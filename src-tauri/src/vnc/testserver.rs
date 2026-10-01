use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};

use super::handshake::Stream;
use super::proto::vnc_auth_response;

pub struct Script {
    pub version: &'static str,
    pub security: Vec<u8>,
    pub password: Option<&'static str>,
    pub size: (u16, u16),
    pub name: String,
    pub updates: Vec<Vec<u8>>,
    pub then: After,
}

// What an update request gets once the scripted updates have run out.
pub enum After {
    Hold,
    Close,
    Garbage(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seen {
    SetPixelFormat([u8; 16]),
    SetEncodings(Vec<i32>),
    Request { incremental: bool, x: u16, y: u16, w: u16, h: u16 },
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    CutText(Vec<u8>),
    // Not from the client: the server wrote an update, or bytes the test gave it, at this point.
    Sent,
}

pub fn rect(x: u16, y: u16, w: u16, h: u16, encoding: i32, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    for field in [x, y, w, h] {
        b.extend(field.to_be_bytes());
    }
    b.extend(encoding.to_be_bytes());
    b.extend(data);
    b
}

pub fn update(rects: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend((rects.len() as u16).to_be_bytes());
    b.extend(rects.concat());
    b
}

pub fn cut_text(text: &[u8]) -> Vec<u8> {
    let mut b = vec![3, 0, 0, 0];
    b.extend((text.len() as u32).to_be_bytes());
    b.extend(text);
    b
}

// Plays the server side: the handshake, then every client message goes into `seen` and each
// update request is answered from the script. Bytes arriving on `push` are written as they come,
// and the connection is closed when `push` is.
pub async fn serve(
    script: Script,
    mut stream: impl Stream,
    seen: watch::Sender<Vec<Seen>>,
    mut push: mpsc::UnboundedReceiver<Vec<u8>>,
) -> std::io::Result<()> {
    stream.write_all(script.version.as_bytes()).await?;
    stream.read_exact(&mut [0u8; 12]).await?;
    let (v33, v38) = (script.version == "RFB 003.003\n", script.version == "RFB 003.008\n");
    let security = if v33 {
        stream.write_u32(script.security[0].into()).await?;
        script.security[0]
    } else {
        stream.write_u8(script.security.len() as u8).await?;
        stream.write_all(&script.security).await?;
        stream.read_u8().await?
    };
    let mut accepted = true;
    if security == 2 {
        let challenge: [u8; 16] = std::array::from_fn(|i| 0xA0 + i as u8);
        stream.write_all(&challenge).await?;
        let mut response = [0u8; 16];
        stream.read_exact(&mut response).await?;
        let password = script.password.expect("a password login needs a password");
        accepted = response == vnc_auth_response(password, &challenge);
    }
    // Before 3.8 a server sends no SecurityResult for type 1.
    if security == 2 || v38 {
        stream.write_u32((!accepted).into()).await?;
    }
    if !accepted {
        if v38 {
            stream.write_u32(0).await?;
        }
        return Ok(());
    }
    stream.read_u8().await?;
    stream.write_u16(script.size.0).await?;
    stream.write_u16(script.size.1).await?;
    stream.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).await?;
    stream.write_u32(script.name.len() as u32).await?;
    stream.write_all(script.name.as_bytes()).await?;

    let mut updates = script.updates.into_iter();
    loop {
        tokio::select! {
            kind = stream.read_u8() => {
                let message = match kind? {
                    0 => {
                        let mut body = [0u8; 19];
                        stream.read_exact(&mut body).await?;
                        Seen::SetPixelFormat(body[3..].try_into().unwrap())
                    }
                    2 => {
                        stream.read_u8().await?;
                        let mut encodings = Vec::new();
                        for _ in 0..stream.read_u16().await? {
                            encodings.push(stream.read_i32().await?);
                        }
                        Seen::SetEncodings(encodings)
                    }
                    3 => Seen::Request {
                        incremental: stream.read_u8().await? != 0,
                        x: stream.read_u16().await?,
                        y: stream.read_u16().await?,
                        w: stream.read_u16().await?,
                        h: stream.read_u16().await?,
                    },
                    4 => {
                        let down = stream.read_u8().await? != 0;
                        stream.read_u16().await?;
                        Seen::Key { down, keysym: stream.read_u32().await? }
                    }
                    5 => Seen::Pointer {
                        buttons: stream.read_u8().await?,
                        x: stream.read_u16().await?,
                        y: stream.read_u16().await?,
                    },
                    6 => {
                        stream.read_exact(&mut [0u8; 3]).await?;
                        let mut text = vec![0u8; stream.read_u32().await? as usize];
                        stream.read_exact(&mut text).await?;
                        Seen::CutText(text)
                    }
                    other => panic!("the client sent message type {other}"),
                };
                let request = matches!(message, Seen::Request { .. });
                seen.send_modify(|log| log.push(message));
                if !request {
                    continue;
                }
                match (updates.next(), &script.then) {
                    (Some(update), _) => {
                        stream.write_all(&update).await?;
                        seen.send_modify(|log| log.push(Seen::Sent));
                    }
                    (None, After::Hold) => {}
                    (None, After::Close) => return Ok(()),
                    (None, After::Garbage(bytes)) => stream.write_all(bytes).await?,
                }
            }
            bytes = push.recv() => {
                let Some(bytes) = bytes else { return Ok(()) };
                stream.write_all(&bytes).await?;
                seen.send_modify(|log| log.push(Seen::Sent));
            }
        }
    }
}

// A scripted server on a local port, for one connection. Dropping this closes the connection.
pub struct Server {
    pub port: u16,
    seen: watch::Receiver<Vec<Seen>>,
    push: mpsc::UnboundedSender<Vec<u8>>,
}

impl Server {
    pub async fn start(script: Script) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (log, seen) = watch::channel(Vec::new());
        let (push, pushed) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            serve(script, stream, log, pushed).await
        });
        Server { port, seen, push }
    }

    // The log as soon as `done` holds for it.
    pub async fn wait(&mut self, done: impl FnMut(&Vec<Seen>) -> bool) -> Vec<Seen> {
        let log = tokio::time::timeout(Duration::from_secs(5), self.seen.wait_for(done)).await;
        log.expect("the client did not send what the test waits for")
            .expect("the server is gone")
            .clone()
    }

    pub fn send(&self, bytes: Vec<u8>) {
        self.push.send(bytes).expect("the server is gone");
    }
}
