#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
mod macros;

mod proto;

use core::fmt;
use core::fmt::Display;
use core::pin::Pin;
use core::task::{Poll, ready};
use core::time::Duration;
use std::io;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use hyper::body::Bytes;
use ironrdp_core::{Decode as _, Encode, ReadCursor, WriteCursor};
use log::{error, warn};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::{Message, http};
use tokio_util::sync::PollSender;

use self::proto::{
    ChannelPkt, ChannelResp, DataPkt, HandshakeReqPkt, HandshakeRespPkt, HttpCapsTy, KeepalivePkt, PktHdr, PktTy,
    TunnelAuthPkt, TunnelAuthRespPkt, TunnelReqPkt, TunnelRespPkt,
};

#[derive(Clone, Debug)]
pub struct GwConnectTarget {
    pub gw_endpoint: String,
    pub gw_user: String,
    pub gw_pass: String,

    pub server: String,
    pub server_port: u16,
}

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
#[non_exhaustive]
pub enum GwErrorKind {
    InvalidGwTarget,
    Connect,
    PacketEof,
    UnsupportedFeature,
    Custom,
    Encode,
    Decode,
    /// A 401 to the upgrade, and whether a `WWW-Authenticate` header offers Basic.
    Unauthorized { basic: bool },
    /// Any other status than 101 to the upgrade.
    HttpStatus(u16),
    /// The HRESULT of a refused tunnel, tunnel authorization or channel.
    TunnelRefused(u32),
    TunnelAuthRefused(u32),
    ChannelRefused(u32),
    /// The gateway closed the WebSocket or the connection under it.
    Closed,
    /// The gateway closed the connection after the channel request, before its answer.
    ClosedAtChannel,
}

trait GwErrorExt {
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for ironrdp_error::Error<GwErrorKind> {
    #[track_caller]
    fn custom<E>(context: &'static str, e: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, GwErrorKind::Custom).with_source(e)
    }
}

impl Display for GwErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let x = match self {
            GwErrorKind::InvalidGwTarget => "invalid GW Target",
            GwErrorKind::Connect => "connection error",
            GwErrorKind::PacketEof => "PacketEOF",
            GwErrorKind::UnsupportedFeature => "unsupported feature",
            GwErrorKind::Custom => "custom",
            GwErrorKind::Encode => "encode",
            GwErrorKind::Decode => "decode",
            GwErrorKind::Unauthorized { basic } => return write!(f, "HTTP 401 (Basic offered: {basic})"),
            GwErrorKind::HttpStatus(status) => return write!(f, "HTTP {status}"),
            GwErrorKind::TunnelRefused(code) => return write!(f, "tunnel refused ({code:#010x})"),
            GwErrorKind::TunnelAuthRefused(code) => return write!(f, "tunnel authorization refused ({code:#010x})"),
            GwErrorKind::ChannelRefused(code) => return write!(f, "channel refused ({code:#010x})"),
            GwErrorKind::Closed => "the gateway closed the connection",
            GwErrorKind::ClosedAtChannel => "the gateway closed the connection after the channel request",
        };
        f.write_str(x)
    }
}

impl core::error::Error for GwErrorKind {}

struct GwConn<S> {
    client_name: String,
    target: GwConnectTarget,
    ws_sink: SplitSink<WebSocketStream<S>, Message>,
    ws_stream: SplitStream<WebSocketStream<S>>,
}

/// The buffer a packet to the gateway is encoded into.
const PACKET_BUFFER: usize = 8192;

/// The most a data packet carries: the buffer less the packet header and the data length.
const MAX_WRITE: usize = PACKET_BUFFER - 8 - 2;

/// How long the task goes on once the client is gone, to send what it was given and a Close: a
/// gateway that stops reading would otherwise keep it and the connection for good.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// What the client hands the task. A flush or a close is answered once everything before it has
/// gone out.
#[derive(Debug)]
enum Out {
    Data(Bytes),
    Flush(oneshot::Sender<()>),
    Close(oneshot::Sender<()>),
}

pub struct GwClient {
    /// `None` once the task's end was reported: its handle panics when polled again.
    work: Option<tokio::task::JoinHandle<Result<(), Error>>>,
    /// The kind of the error the task ended with, for every read and write after the first.
    ended: io::ErrorKind,
    rx: tokio::sync::mpsc::Receiver<Bytes>,
    rx_bufs: Vec<Bytes>,
    tx: PollSender<Out>,
    /// The answer to the flush or close under way.
    answer: Option<oneshot::Receiver<()>>,
    closing: bool,
    /// Whether a read has met the task's end.
    read_end: bool,
}

impl GwClient {
    /// `stream` is the connection to the gateway, with TLS done and its certificate checked: the
    /// login goes out on it at once. `target.gw_endpoint` is the value of the Host header.
    pub async fn connect<S>(stream: S, target: &GwConnectTarget, client_name: &str) -> Result<GwClient, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let auth_val: String = STANDARD.encode(format!("{}:{}", target.gw_user, target.gw_pass));
        let req = http::Request::builder()
            .method("RDG_OUT_DATA")
            .header(hyper::header::HOST, &target.gw_endpoint)
            .header("Rdg-Connection-Id", format!("{{{}}}", uuid::Uuid::new_v4()))
            .uri("/remoteDesktopGateway/")
            .header(hyper::header::AUTHORIZATION, format!("Basic {auth_val}"))
            .header(hyper::header::CONNECTION, "Upgrade")
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::SEC_WEBSOCKET_VERSION, "13")
            .header(hyper::header::SEC_WEBSOCKET_KEY, generate_key())
            .body(http_body_util::Empty::<Bytes>::new())
            .map_err(|e| custom_err!("failed to build request", e))?;

        let stream = hyper_util::rt::tokio::TokioIo::new(stream);
        let (mut sender, mut conn) = hyper::client::conn::http1::handshake(stream)
            .await
            .map_err(|e| custom_err!("H1 Handshake", e))?;
        let (tx, rx) = oneshot::channel();

        let jh = tokio::task::spawn(async move {
            tokio::select! {
                Err(e) = &mut conn => error!("Handshake error: {:?}", e),
                _ = rx => (),
            }
            conn.into_parts()
        });
        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| custom_err!("WS Upgrade Send error", e))?;

        if resp.status() == http::StatusCode::UNAUTHORIZED {
            let mut challenges = resp.headers().get_all(hyper::header::WWW_AUTHENTICATE).iter();
            let basic = challenges.any(offers_basic);
            return Err(Error::new("WS Upgrade", GwErrorKind::Unauthorized { basic }));
        }
        if resp.status() != http::StatusCode::SWITCHING_PROTOCOLS {
            let status = resp.status().as_u16();
            return Err(Error::new("WS Upgrade", GwErrorKind::HttpStatus(status)));
        }

        let _ = tx.send(()); // TODO: Not needed since it doesnt keep alive conn?
        let stream = jh.await.map_err(|e| custom_err!("WS join", e))?.io.into_inner();

        Self::connect_ws(target.clone(), client_name, stream).await
    }

    async fn connect_ws<S>(target: GwConnectTarget, client_name: &str, tls_stream: S) -> Result<GwClient, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ws_stream: WebSocketStream<_> = WebSocketStream::from_raw_socket(tls_stream, Role::Client, None).await;
        let (ws_sink, ws_stream) = ws_stream.split();
        let mut gw = GwConn {
            client_name: client_name.to_owned(),
            target,
            ws_sink,
            ws_stream,
        };

        // Closed before its handshake answer, it has not shown itself a gateway.
        gw.handshake().await.map_err(|e| match e.kind() {
            GwErrorKind::Closed => Error::new("Handshake", GwErrorKind::Connect),
            _ => e,
        })?;
        gw.tunnel().await?;
        gw.tunnel_auth().await?;
        gw.channel().await?;

        let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Out>(4);

        // One loop per direction: while the reader waits for room in `in_tx`, writes still go out.
        // The reading ends the task, so that what the gateway sent before its end is all read.
        let GwConn { ws_sink, ws_stream, .. } = gw;
        let work = tokio::spawn(async move {
            let gone = in_tx.clone();
            let writing = async {
                write_packets(ws_sink, out_rx).await?;
                core::future::pending::<Result<(), Error>>().await
            };
            tokio::select! {
                ended = async { tokio::try_join!(read_packets(ws_stream, in_tx), writing).map(|_| ()) } => ended,
                () = async {
                    gone.closed().await;
                    tokio::time::sleep(DRAIN_LIMIT).await;
                } => Ok(()),
            }
        });

        Ok(GwClient {
            work: Some(work),
            ended: io::ErrorKind::Other,
            rx: in_rx,
            rx_bufs: vec![],
            tx: PollSender::new(out_tx),
            answer: None,
            closing: false,
            read_end: false,
        })
    }

    fn poll_work(&mut self, cx: &mut core::task::Context<'_>) -> Poll<io::Error> {
        let Some(work) = self.work.as_mut() else {
            return Poll::Ready(io::Error::new(self.ended, "Work Task ended"));
        };
        let ended = match work.poll_unpin(cx) {
            Poll::Ready(Err(e)) => io::Error::other(e),
            Poll::Ready(Ok(Err(e))) if matches!(e.kind(), GwErrorKind::Closed) => {
                io::Error::new(io::ErrorKind::UnexpectedEof, e)
            }
            Poll::Ready(Ok(Err(e))) => io::Error::other(e),
            Poll::Ready(_) => io::Error::other("Premature Work Task end?"),
            Poll::Pending => return Poll::Pending,
        };
        self.work = None;
        self.ended = ended.kind();
        Poll::Ready(ended)
    }

    /// The task's end as a write or a flush meets it. After the gateway's close, as on a socket
    /// after the peer's FIN, what is written goes nowhere until a read has met the end: the
    /// server's last PDUs must not be lost to the error of a write in between. Whether they have
    /// been read cannot be told from what is queued here, as TLS above reads ahead.
    fn poll_write_end(&mut self, cx: &mut core::task::Context<'_>) -> Poll<io::Result<()>> {
        match self.poll_work(cx) {
            Poll::Ready(e) if e.kind() == io::ErrorKind::UnexpectedEof && !self.read_end => Poll::Ready(Ok(())),
            Poll::Ready(e) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Hands the task a flush or a close, once, and waits for its answer.
    fn poll_answer(
        &mut self,
        cx: &mut core::task::Context<'_>,
        out: fn(oneshot::Sender<()>) -> Out,
    ) -> Poll<io::Result<()>> {
        if self.answer.is_none() {
            if let Poll::Ready(end) = self.poll_write_end(cx) {
                return Poll::Ready(end);
            }
            // The task is ending, or has sent its Close; its handle tells how once it has ended.
            if ready!(self.tx.poll_reserve(cx)).is_err() {
                return self.poll_write_end(cx);
            }
            let (done, answer) = oneshot::channel();
            if self.tx.send_item(out(done)).is_err() {
                return Poll::Ready(Err(io::Error::other("Sender closed")));
            }
            self.answer = Some(answer);
        }
        let Some(answer) = self.answer.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let answered = match answer.poll_unpin(cx) {
            Poll::Ready(answered) => answered,
            // Sent as the task ended, a flush or close can stay in its channel unanswered for good.
            Poll::Pending => {
                let end = ready!(self.poll_write_end(cx));
                self.answer = None;
                return Poll::Ready(end);
            }
        };
        self.answer = None;
        match answered {
            Ok(()) => Poll::Ready(Ok(())),
            // Dropped unanswered: the task is ending.
            Err(_) => self.poll_write_end(cx),
        }
    }
}

/// While connecting, a reset or an aborted connection is the gateway closing it too.
fn closed_at_connect(e: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::Error as WsError;

    let reset = |e: &io::Error| matches!(e.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted);
    closed(e) || matches!(e, WsError::Io(e) if reset(e))
}

/// An error that only says the gateway or the connection to it has gone.
fn closed(e: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::Error as WsError;

    match e {
        WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        WsError::Io(e) => e.kind() == io::ErrorKind::UnexpectedEof,
        _ => false,
    }
}

async fn read_packets<S: AsyncRead + AsyncWrite + Unpin>(
    mut ws_stream: SplitStream<WebSocketStream<S>>,
    in_tx: tokio::sync::mpsc::Sender<Bytes>,
) -> Result<(), Error> {
    loop {
        let msg = match ws_stream.next().await {
            None | Some(Ok(Message::Close(_))) => return Err(Error::new("WS Stream", GwErrorKind::Closed)),
            // tungstenite answers a ping itself.
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(msg)) => msg.into_data(),
            Some(Err(e)) if closed(&e) => return Err(Error::new("WS Stream", GwErrorKind::Closed).with_source(e)),
            Some(Err(e)) => return Err(custom_err!("Stream", e)),
        };
        let mut cur = ReadCursor::new(&msg);
        let hdr = PktHdr::decode(&mut cur).map_err(|e| custom_err!("Header Decode", e))?;

        let header_length =
            usize::try_from(hdr.length).map_err(|_| Error::new("PktHdr too big", GwErrorKind::Decode))?;
        let body_length = header_length
            .checked_sub(hdr.size())
            .ok_or_else(|| Error::new("PktHdr too small", GwErrorKind::Decode))?;
        if cur.len() < body_length {
            return Err(Error::new("Packet", GwErrorKind::PacketEof));
        }
        match hdr.ty {
            PktTy::Keepalive => {
                continue;
            }
            PktTy::Data => {
                let p = DataPkt::decode(&mut cur).map_err(|e| custom_err!("PktDecode", e))?;
                // Once the client is gone what comes is dropped, and the reading goes on until the
                // gateway answers the Close.
                if in_tx.send(Bytes::from(p.data.to_vec())).await.is_err() {
                    continue;
                }
            }
            x => {
                warn!("Unhandled gw packet type {x:?}");
            }
        }
    }
}

/// Ends once it has sent a Close: on a close from the client, or once the client is gone and
/// everything it gave has gone out.
async fn write_packets<S: AsyncRead + AsyncWrite + Unpin>(
    mut ws_sink: SplitSink<WebSocketStream<S>, Message>,
    mut out_rx: tokio::sync::mpsc::Receiver<Out>,
) -> Result<(), Error> {
    let iv = Duration::from_secs(15 * 60);
    let mut keepalive_interval = tokio::time::interval_at(tokio::time::Instant::now() + iv, iv);

    loop {
        let mut wsbuf = [0u8; PACKET_BUFFER];

        tokio::select!(
            _ = keepalive_interval.tick() => {
                let pos = {
                    let mut cur = WriteCursor::new(&mut wsbuf);
                    KeepalivePkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                    cur.pos()
                };

                ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await.map_err(|e| custom_err!("ws send", e))?;
            },
            next = out_rx.recv() => {
                let next = match next {
                    Some(Out::Data(next)) => next,
                    // A flush its caller gave up has nobody to answer.
                    Some(Out::Flush(done)) => {
                        let _ = done.send(());
                        continue;
                    }
                    Some(Out::Close(done)) => {
                        ws_sink.close().await.map_err(|e| custom_err!("ws close", e))?;
                        let _ = done.send(());
                        return Ok(());
                    }
                    None => {
                        ws_sink.close().await.map_err(|e| custom_err!("ws close", e))?;
                        return Ok(());
                    }
                };
                let pkt = DataPkt { data: &next };

                let pos = {
                    let mut cur = WriteCursor::new(&mut wsbuf);
                    pkt.encode(&mut cur).map_err(|e| custom_err!("PktEncode", e))?;
                    cur.pos()
                };
                ws_sink.send(Message::Binary(Bytes::copy_from_slice(&wsbuf[..pos]))).await.map_err(|e| custom_err!("ws send", e))?;
            }
        );
    }
}

/// Whether a `WWW-Authenticate` value offers Basic. A value can list several challenges, whose
/// parameters can hold commas in quotes.
fn offers_basic(value: &http::HeaderValue) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    let mut quoted = false;
    let mut challenges = value.split(|c: char| {
        if c == '"' {
            quoted = !quoted;
        }
        c == ',' && !quoted
    });
    challenges.any(|challenge| {
        challenge
            .split_whitespace()
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("basic"))
    })
}

impl<S: AsyncRead + AsyncWrite + Unpin> GwConn<S> {
    async fn send_packet<E: Encode>(&mut self, payload: &E) -> Result<(), Error> {
        let mut buf = [0u8; 4096];
        let pos = {
            let mut cur = WriteCursor::new(&mut buf);
            payload
                .encode(&mut cur)
                .map_err(|e| Error::new("packet encode", GwErrorKind::Encode).with_source(e))?;
            cur.pos()
        };
        match self.ws_sink.send(Message::Binary(Bytes::copy_from_slice(&buf[..pos]))).await {
            Ok(()) => Ok(()),
            Err(e) if closed_at_connect(&e) => Err(Error::new("WebSocket send", GwErrorKind::Closed).with_source(e)),
            Err(e) => Err(custom_err!("WebSocket send error", e)),
        }
    }

    async fn read_packet(&mut self) -> Result<(PktHdr, Bytes), Error> {
        let mut msg = match self.ws_stream.next().await {
            None | Some(Ok(Message::Close(_))) => return Err(Error::new("Stream closed", GwErrorKind::Closed)),
            Some(Ok(msg)) => msg.into_data(),
            Some(Err(e)) if closed_at_connect(&e) => {
                return Err(Error::new("WS err", GwErrorKind::Closed).with_source(e));
            }
            Some(Err(e)) => return Err(custom_err!("WS err", e)),
        };
        let mut cur = ReadCursor::new(&msg);

        let hdr = PktHdr::decode(&mut cur).map_err(|_| Error::new("PktHdr", GwErrorKind::Decode))?;

        let header_length =
            usize::try_from(hdr.length).map_err(|_| Error::new("PktHdr too big", GwErrorKind::Decode))?;
        let body_length = header_length
            .checked_sub(hdr.size())
            .ok_or_else(|| Error::new("PktHdr too small", GwErrorKind::Decode))?;
        if cur.len() != body_length {
            return Err(Error::new("read_packet", GwErrorKind::PacketEof));
        }

        Ok((hdr, msg.split_off(cur.pos())))
    }

    async fn handshake(&mut self) -> Result<(), Error> {
        // For NTLM we would include extended_auth: NTLM_SSPI in this handshake req here.
        let hs = HandshakeReqPkt {
            ver_major: 1,
            ver_minor: 0,
            ..HandshakeReqPkt::default()
        };
        self.send_packet(&hs).await?;
        let (_hdr, bytes) = self.read_packet().await?;

        let mut cur = ReadCursor::new(&bytes);
        let resp = HandshakeRespPkt::decode(&mut cur).map_err(|_| Error::new("Handshake", GwErrorKind::Decode))?;
        if resp.error_code != 0 || resp.ver_major != 1 || resp.ver_minor != 0 || resp.server_version != 0 {
            return Err(Error::new("Handshake", GwErrorKind::Connect));
        }
        Ok(())
    }

    async fn tunnel(&mut self) -> Result<(), Error> {
        let req = TunnelReqPkt {
            // Havent seen any server working without this.
            caps: HttpCapsTy::MessagingConsentSign.as_u32(),
            fields_present: 0,
            ..TunnelReqPkt::default()
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);

        let resp = TunnelRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelDecode", GwErrorKind::Decode))?;
        if resp.status_code != 0 {
            return Err(Error::new("Tunnel", GwErrorKind::TunnelRefused(resp.status_code)));
        }
        if !cur.eof() {
            return Err(Error::new("TunnelDecode", GwErrorKind::Decode));
        }
        if !resp.consent_msg.is_empty() {
            return Err(Error::new(
                "Received consent message but showing it not implemented",
                GwErrorKind::UnsupportedFeature,
            ));
        }
        Ok(())
    }

    async fn tunnel_auth(&mut self) -> Result<(), Error> {
        let req = TunnelAuthPkt {
            fields_present: 0,
            client_name: self.client_name.clone(),
        };
        self.send_packet(&req).await?;

        let (_hdr, bytes) = self.read_packet().await?;
        let mut cur = ReadCursor::new(&bytes);
        let resp: TunnelAuthRespPkt =
            TunnelAuthRespPkt::decode(&mut cur).map_err(|_| Error::new("TunnelAuth", GwErrorKind::Decode))?;

        if resp.error_code() != 0 {
            return Err(Error::new("TunnelAuth", GwErrorKind::TunnelAuthRefused(resp.error_code())));
        }
        Ok(())
    }

    async fn channel(&mut self) -> Result<ChannelResp, Error> {
        let req = ChannelPkt {
            resources: vec![self.target.server.clone()],
            port: self.target.server_port,
            protocol: 3,
        };
        self.send_packet(&req).await?;

        let (hdr, bytes) = self.read_packet().await.map_err(|e| match e.kind() {
            GwErrorKind::Closed => Error::new("ChannelResp", GwErrorKind::ClosedAtChannel),
            _ => e,
        })?;
        if hdr.ty != PktTy::ChannelResp {
            return Err(Error::new("ChannelResp", GwErrorKind::Decode));
        }
        let mut cur: ReadCursor<'_> = ReadCursor::new(&bytes);
        let resp: ChannelResp =
            ChannelResp::decode(&mut cur).map_err(|_| Error::new("ChannelResp", GwErrorKind::Decode))?;
        if resp.error_code() != 0 {
            return Err(Error::new("ChannelCreate", GwErrorKind::ChannelRefused(resp.error_code())));
        }
        if !cur.eof() {
            return Err(Error::new("ChannelResp", GwErrorKind::Decode));
        }
        Ok(resp)
    }
}

impl AsyncRead for GwClient {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut n = 0;
        while buf.remaining() > 0 {
            if let Some(rx_buf) = self.rx_bufs.first_mut() {
                let max = core::cmp::min(buf.remaining(), rx_buf.len());
                buf.put_slice(&rx_buf.split_to(max));
                n += max;
                if rx_buf.is_empty() {
                    self.rx_bufs.remove(0);
                }
                continue;
            }
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(new_buf)) => self.rx_bufs.push(new_buf),
                // The task has ended, and everything it read has been read here.
                Poll::Ready(None) if n == 0 => {
                    let end = ready!(self.poll_work(cx));
                    self.read_end = true;
                    return Poll::Ready(Err(end));
                }
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        if n > 0 { Poll::Ready(Ok(())) } else { Poll::Pending }
    }
}

impl AsyncWrite for GwClient {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        // Propagate error or premature exit (?)
        if let Poll::Ready(end) = self.poll_write_end(cx) {
            return Poll::Ready(end.map(|()| buf.len()));
        }
        if self.closing {
            return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        }

        match self.tx.poll_reserve(cx) {
            Poll::Ready(Ok(())) => {
                let len = buf.len().min(MAX_WRITE);
                if self.tx.send_item(Out::Data(Bytes::copy_from_slice(&buf[..len]))).is_err() {
                    return Poll::Ready(Err(io::Error::other("Sender closed")));
                }
                // A flush that was given up must not answer for this write.
                self.answer = None;
                return Poll::Ready(Ok(len));
            }
            // The task is ending; its handle tells how once it has.
            Poll::Ready(Err(_)) => {
                return self.poll_write_end(cx).map(|end| end.map(|()| buf.len()));
            }
            Poll::Pending => (),
        }

        Poll::Pending
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        self.poll_answer(cx, Out::Flush)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Result<(), io::Error>> {
        if !self.closing {
            self.answer = None;
            self.closing = true;
        }
        self.poll_answer(cx, Out::Close)
    }
}
