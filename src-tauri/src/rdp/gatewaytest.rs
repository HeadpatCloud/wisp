use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_util::stream::SplitStream;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{
    AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream, ReadBuf,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::testserver::tls;

pub const HANDSHAKE_RESPONSE: u16 = 0x02;
pub const TUNNEL_RESPONSE: u16 = 0x05;
const TUNNEL_AUTH_RESPONSE: u16 = 0x07;
const CHANNEL_CREATE: u16 = 0x08;
pub const CHANNEL_RESPONSE: u16 = 0x09;
const DATA: u16 = 0x0a;

// How the stand-in answers the HTTP request.
pub enum Answer {
    // 101, then MS-TSGU.
    Upgrade,
    // These bytes as they are: a response, or anything else.
    Reply(Vec<u8>),
    // Nothing, until the client goes away.
    Silent,
}

// How the stand-in hangs up in place of an answer.
pub enum Hangup {
    // A WebSocket Close.
    Close,
    // A TCP reset.
    Reset,
}

// What the stand-in answers the client's handshake, tunnel, tunnel auth and channel requests
// with, as it goes on the wire; an empty answer is none, and the stand-in waits for the client to
// go away. `hangup` takes the place of the answer to the request with that number, 0 being the
// handshake. `after` follows the channel response, before the relay starts.
pub struct Script {
    pub answer: Answer,
    pub handshake: Vec<u8>,
    pub tunnel: Vec<u8>,
    pub tunnel_auth: Vec<u8>,
    pub channel: Vec<u8>,
    pub hangup: Option<(usize, Hangup)>,
    pub after: Vec<Message>,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            answer: Answer::Upgrade,
            handshake: handshake_response(),
            tunnel: tunnel_response(0),
            tunnel_auth: tunnel_auth_response(0),
            channel: channel_response(0),
            hangup: None,
            after: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct Heard {
    // Every byte the client sent after the TLS handshake.
    pub bytes: Vec<u8>,
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    // The resource and port of the channel request.
    pub channel: Option<(String, u16)>,
    // The size of every binary message the client sent after the channel response.
    pub packets: Vec<usize>,
    // Whether the client sent a WebSocket Close.
    pub closed: bool,
}

impl Heard {
    pub fn header(&self, name: &str) -> Option<&str> {
        let mut found = self.headers.iter().filter(|(key, _)| key.eq_ignore_ascii_case(name));
        found.next().map(|(_, value)| value.as_str())
    }
}

pub struct StandIn {
    pub port: u16,
    // Ends once the client has gone away or the connection was cut.
    pub heard: JoinHandle<Heard>,
    // Sent, the stand-in drops the TCP connection, with no WebSocket or TLS close before it.
    pub cut: oneshot::Sender<()>,
}

// An RD Gateway for one connection on 127.0.0.1, with the test certificate. The channel data goes
// to `server` and back, the RDP server's end of a duplex stream; without one, what the client
// sends after the channel response is read and dropped.
pub async fn stand_in(script: Script, server: Option<DuplexStream>) -> StandIn {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (cut, cutting) = oneshot::channel();
    let heard = tokio::spawn(async move {
        let heard = Mutex::new(Heard::default());
        let bytes = Arc::new(Mutex::new(Vec::new()));
        if let Ok((tcp, _)) = listener.accept().await {
            if matches!(script.hangup, Some((_, Hangup::Reset))) {
                tcp.set_zero_linger().unwrap();
            }
            if let Ok(stream) = tls().0.accept(tcp).await {
                let stream = Recorded { inner: stream, bytes: Arc::clone(&bytes) };
                tokio::select! {
                    _ = serve(stream, script, server, &heard) => {}
                    Ok(()) = cutting => {}
                }
            }
        }
        let mut heard = heard.into_inner().unwrap();
        heard.bytes = std::mem::take(&mut *bytes.lock().unwrap());
        heard
    });
    StandIn { port, heard, cut }
}

async fn serve<S>(mut stream: S, script: Script, server: Option<DuplexStream>, heard: &Mutex<Heard>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(byte) => head.push(byte),
            Err(_) => return,
        }
    }
    let key = {
        let mut heard = heard.lock().unwrap();
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = text.split("\r\n").filter(|line| !line.is_empty());
        heard.request_line = lines.next().unwrap_or_default().to_owned();
        heard.headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        heard.header("sec-websocket-key").unwrap_or_default().to_owned()
    };
    match script.answer {
        Answer::Upgrade => {}
        Answer::Reply(bytes) => {
            if stream.write_all(&bytes).await.is_ok() && stream.flush().await.is_ok() {
                stream.read_to_end(&mut Vec::new()).await.ok();
            }
            return;
        }
        Answer::Silent => {
            stream.read_to_end(&mut Vec::new()).await.ok();
            return;
        }
    }
    let accept = derive_accept_key(key.as_bytes());
    let upgraded = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(upgraded.as_bytes()).await.is_err() {
        return;
    }
    let mut ws = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    let answers = [script.handshake, script.tunnel, script.tunnel_auth, script.channel];
    for (n, answer) in answers.into_iter().enumerate() {
        let Some(Ok(request)) = ws.next().await else { return };
        if let Some(channel) = channel_request(&request.into_data()) {
            heard.lock().unwrap().channel = Some(channel);
        }
        match script.hangup {
            Some((at, Hangup::Close)) if at == n => {
                ws.close(None).await.ok();
                return;
            }
            // Dropped with a zero linger, the connection is reset.
            Some((at, Hangup::Reset)) if at == n => return,
            _ => {}
        }
        if answer.is_empty() {
            while let Some(Ok(_)) = ws.next().await {}
            return;
        }
        if ws.send(Message::binary(answer)).await.is_err() {
            return;
        }
    }
    for message in script.after {
        if ws.send(message).await.is_err() {
            return;
        }
    }
    let (mut to_client, mut from_client) = ws.split();
    let Some(server) = server else {
        while let Some(packet) = binary(&mut from_client, heard).await {
            heard.lock().unwrap().packets.push(packet.len());
        }
        return;
    };
    let (mut from_server, mut to_server) = tokio::io::split(server);
    let up = async {
        while let Some(packet) = binary(&mut from_client, heard).await {
            heard.lock().unwrap().packets.push(packet.len());
            let Some(data) = data(&packet) else { continue };
            if to_server.write_all(data).await.is_err() {
                return;
            }
        }
    };
    let down = async {
        let mut buf = vec![0; 4096];
        loop {
            let n = match from_server.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            if to_client.send(Message::binary(data_packet(&buf[..n]))).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}

// The next binary message from the client, until it closes or goes away.
async fn binary<S>(
    from_client: &mut SplitStream<WebSocketStream<S>>,
    heard: &Mutex<Heard>,
) -> Option<Vec<u8>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        match from_client.next().await? {
            Ok(Message::Binary(data)) => return Some(data.to_vec()),
            Ok(Message::Close(_)) => heard.lock().unwrap().closed = true,
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

// A request whose header gives another length than it has is none, as for a gateway.
fn channel_request(packet: &[u8]) -> Option<(String, u16)> {
    if packet.get(..2)? != CHANNEL_CREATE.to_le_bytes() {
        return None;
    }
    if u32::from_le_bytes(packet.get(4..8)?.try_into().ok()?) as usize != packet.len() {
        return None;
    }
    // Resource count, alternate name count, port, protocol, then each resource as a byte length
    // and UTF-16 with a terminator.
    let body = packet.get(8..)?;
    let port = u16::from_le_bytes([*body.get(2)?, *body.get(3)?]);
    let length = usize::from(u16::from_le_bytes([*body.get(6)?, *body.get(7)?]));
    let name = body.get(8..8 + length)?;
    let units = name.chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0], unit[1]]));
    let name = String::from_utf16(&units.take_while(|&unit| unit != 0).collect::<Vec<_>>()).ok()?;
    Some((name, port))
}

fn data(packet: &[u8]) -> Option<&[u8]> {
    if packet.get(..2)? != DATA.to_le_bytes() {
        return None;
    }
    let length = usize::from(u16::from_le_bytes([*packet.get(8)?, *packet.get(9)?]));
    packet.get(10..10 + length)
}

// A packet header (type, reserved, length of the whole packet) and the body.
pub fn packet(kind: u16, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(8 + body.len()).unwrap();
    [&kind.to_le_bytes()[..], &[0, 0], &length.to_le_bytes(), body].concat()
}

// Error code 0, version 1.0, server version 0, no extended authentication.
pub fn handshake_response() -> Vec<u8> {
    packet(HANDSHAKE_RESPONSE, &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0])
}

// Server version, status, then a tunnel id and capabilities, as Windows sends them.
pub fn tunnel_response(status: u32) -> Vec<u8> {
    let mut body = vec![0, 0];
    body.extend(status.to_le_bytes());
    body.extend([3, 0, 0, 0]);
    body.extend(1u32.to_le_bytes());
    body.extend(0x3fu32.to_le_bytes());
    packet(TUNNEL_RESPONSE, &body)
}

// A tunnel response with a consent message the user would have to accept.
pub fn consent_response(message: &str) -> Vec<u8> {
    let text: Vec<u8> = message.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut body = vec![0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0];
    body.extend(u16::try_from(text.len()).unwrap().to_le_bytes());
    body.extend(text);
    packet(TUNNEL_RESPONSE, &body)
}

pub fn tunnel_auth_response(code: u32) -> Vec<u8> {
    packet(TUNNEL_AUTH_RESPONSE, &[&code.to_le_bytes()[..], &[0, 0, 0, 0]].concat())
}

// Error code, then a channel id.
pub fn channel_response(code: u32) -> Vec<u8> {
    let body = [&code.to_le_bytes()[..], &[1, 0, 0, 0], &7u32.to_le_bytes()].concat();
    packet(CHANNEL_RESPONSE, &body)
}

pub fn data_packet(data: &[u8]) -> Vec<u8> {
    let length = u16::try_from(data.len()).unwrap();
    packet(DATA, &[&length.to_le_bytes()[..], data].concat())
}

struct Recorded<S> {
    inner: S,
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Recorded<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.bytes.lock().unwrap().extend_from_slice(&buf.filled()[before..]);
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Recorded<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
