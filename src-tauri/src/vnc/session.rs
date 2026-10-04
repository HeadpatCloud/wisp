use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;

use super::decode::Decoder;
use super::handshake::{handshake, Login, Stream, TlsContext};
use super::proto::{
    client_cut_text, fb_update_request, key_event, latin1_decode, latin1_encode, pointer_event,
    set_encodings,
};
use super::{err, MAX_TEXT, PIXEL_FORMAT};
use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::FrameOp;

pub struct Session {
    pub width: u16,
    pub height: u16,
    pub name: String,
    input: mpsc::Sender<Vec<u8>>,
    acks: Arc<Semaphore>,
    buttons: AtomicU8,
    task: Mutex<JoinSet<()>>,
}

impl Session {
    pub async fn connect(
        host: &str,
        port: u16,
        login: &Login<'_>,
        known: &KnownHostsState,
        encodings: &[i32],
        sink: impl Fn(FrameOp) + Send + 'static,
    ) -> AppResult<Session> {
        let limit = Duration::from_secs(20);
        Self::connect_within(host, port, login, known, encodings, sink, limit).await
    }

    async fn connect_within(
        host: &str,
        port: u16,
        login: &Login<'_>,
        known: &KnownHostsState,
        encodings: &[i32],
        sink: impl Fn(FrameOp) + Send + 'static,
        limit: Duration,
    ) -> AppResult<Session> {
        let opening = async {
            let tcp = TcpStream::connect((crate::net::normalize_host(host).as_str(), port)).await?;
            tcp.set_nodelay(true)?;
            let keepalive = TcpKeepalive::new()
                .with_time(Duration::from_secs(30))
                .with_interval(Duration::from_secs(10));
            SockRef::from(&tcp).set_tcp_keepalive(&keepalive)?;
            handshake(Box::new(tcp), login, &TlsContext { host, port, known }).await
        };
        let (mut stream, init) = tokio::time::timeout(limit, opening)
            .await
            .map_err(|_| err("connection timed out"))??;
        let decoder = Decoder::new(init.width, init.height)?;

        let mut setup = vec![0u8; 4];
        setup.extend_from_slice(&PIXEL_FORMAT);
        setup.extend(set_encodings(encodings));
        setup.extend(fb_update_request(false, 0, 0, init.width, init.height));
        stream.write_all(&setup).await?;
        stream.flush().await?;

        let (input, messages) = mpsc::channel(256);
        let acks = Arc::new(Semaphore::new(0));
        let mut task = JoinSet::new();
        task.spawn(run(stream, decoder, input.clone(), messages, acks.clone(), sink));
        Ok(Session {
            width: init.width,
            height: init.height,
            name: init.name.chars().filter(|c| !c.is_control()).take(200).collect(),
            input,
            acks,
            buttons: AtomicU8::new(0),
            task: Mutex::new(task),
        })
    }

    async fn send(&self, message: Vec<u8>) -> AppResult<()> {
        self.input.send(message).await.map_err(|_| closed())
    }

    pub async fn pointer(&self, buttons: u8, x: u16, y: u16) -> AppResult<()> {
        let message = pointer_event(buttons, x, y).to_vec();
        if self.buttons.swap(buttons, Ordering::Relaxed) != buttons {
            return self.send(message).await;
        }
        // Only a move: the next one says where the pointer is, so it does not wait for room.
        match self.input.try_send(message) {
            Err(TrySendError::Closed(_)) => Err(closed()),
            _ => Ok(()),
        }
    }

    pub async fn key(&self, down: bool, keysym: u32) -> AppResult<()> {
        self.send(key_event(down, keysym).to_vec()).await
    }

    pub async fn clipboard(&self, text: &str) -> AppResult<()> {
        let text = latin1_encode(text);
        // Servers drop a client that sends more, so a longer text is not synced.
        if text.len() > MAX_TEXT {
            return Ok(());
        }
        self.send(client_cut_text(&text)).await
    }

    // The view has applied everything up to one more `Sync`.
    pub fn ack(&self) {
        self.acks.add_permits(1);
    }

    pub async fn close(&self) {
        self.close_within(Duration::from_secs(1)).await
    }

    async fn close_within(&self, limit: Duration) {
        let mut task = self.task.lock().await;
        // The empty message stops the writer; what was queued before it, key releases above all,
        // still goes out first. The task ends once the server has closed its side as well.
        let finished = async {
            if self.input.send(Vec::new()).await.is_ok() {
                task.join_next().await;
            }
        };
        tokio::select! {
            _ = finished => {}
            _ = tokio::time::sleep(limit) => {}
        }
        task.shutdown().await;
    }
}

fn closed() -> AppError {
    AppError::NotFound("vnc session closed".into())
}

// Runs a connection whose first update has been asked for, until it ends. `requests` and `input`
// are the two ends of the one channel everything written goes through.
pub async fn run(
    stream: Box<dyn Stream>,
    mut decoder: Decoder,
    requests: mpsc::Sender<Vec<u8>>,
    mut input: mpsc::Receiver<Vec<u8>>,
    acks: Arc<Semaphore>,
    mut sink: impl FnMut(FrameOp),
) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut writing = JoinSet::new();
    writing.spawn(async move {
        // An empty message is `close` asking to stop once everything before it is written.
        while let Some(message) = input.recv().await.filter(|message| !message.is_empty()) {
            writer.write_all(&message).await?;
            writer.flush().await?;
        }
        // Tells the server that nothing more comes, so that it closes its side.
        writer.shutdown().await
    });

    // `unacked` counts the updates and clipboard texts the view has not acknowledged, `requested`
    // says an update is on its way, and `full` that the next request is for the whole screen.
    let (mut unacked, mut requested, mut full) = (0u32, true, false);
    let ended: AppResult<()> = async {
        loop {
            // A message is read to its end once its type is in: `Decoder::update` cannot be
            // dropped halfway, so only the wait for the type byte is raced. With two updates
            // unacknowledged nothing more is read, and TCP holds a server that pushes them back.
            tokio::select! {
                kind = reader.read_u8(), if unacked < 2 => match kind? {
                    0 => {
                        let mut forward = |op| {
                            full |= matches!(op, FrameOp::Resize { .. });
                            sink(op);
                        };
                        decoder.update(&mut reader, &mut forward).await?;
                        sink(FrameOp::Sync);
                        unacked += 1;
                        requested = false;
                    }
                    1 => {
                        let mut head = [0u8; 5];
                        reader.read_exact(&mut head).await?;
                        let colours = u16::from_be_bytes([head[3], head[4]]) as usize;
                        reader.read_exact(&mut vec![0u8; colours * 6]).await?;
                    }
                    2 => {}
                    3 => {
                        reader.read_exact(&mut [0u8; 3]).await?;
                        let len = reader.read_u32().await? as usize;
                        if len > MAX_TEXT {
                            let mut skipped = [0u8; 4096];
                            for done in (0..len).step_by(skipped.len()) {
                                let part = (len - done).min(skipped.len());
                                reader.read_exact(&mut skipped[..part]).await?;
                            }
                        } else {
                            let mut text = vec![0u8; len];
                            reader.read_exact(&mut text).await?;
                            text.retain(|byte| *byte != 0);
                            // x11vnc sends an empty text on some first connections, and the view
                            // would write it over the local clipboard.
                            if !text.is_empty() {
                                sink(FrameOp::Clipboard(latin1_decode(&text)));
                                // Acknowledged like an update, or a server could push texts
                                // faster than the view takes them.
                                sink(FrameOp::Sync);
                                unacked += 1;
                            }
                        }
                    }
                    other => {
                        return Err(err(format!("the server sent an unknown message ({other})")));
                    }
                },
                Ok(ack) = acks.acquire() => {
                    ack.forget();
                    unacked = unacked.saturating_sub(1);
                }
                Ok(slot) = requests.reserve(), if unacked == 0 && !requested => {
                    let (w, h) = decoder.size();
                    slot.send(fb_update_request(!full, 0, 0, w, h).to_vec());
                    (requested, full) = (true, false);
                }
                // The writer ends with an error when a write failed, and without one when
                // `close` asked it to stop.
                Some(stopped) = writing.join_next() => {
                    return Ok(stopped.map_err(std::io::Error::other)??);
                }
            }
        }
    }
    .await;
    // A read that fails once `close` has stopped the writer is the server closing in answer.
    let ended = match writing.try_join_next() {
        Some(Ok(Ok(()))) => Ok(()),
        _ => ended,
    };
    // Nothing more can be sent once the view hears that the session is over.
    writing.shutdown().await;
    let reason = match ended {
        // `close` ends this task too, and the view is told nothing. What the server still sends
        // is read and thrown away until it closes: a socket dropped with data unread resets the
        // connection, and the server loses what was written to it last.
        Ok(()) => {
            let mut discarded = [0u8; 4096];
            while matches!(reader.read(&mut discarded).await, Ok(1..)) {}
            return;
        }
        Err(AppError::Internal(text)) => text.trim_start_matches("vnc: ").to_string(),
        Err(_) => "network connection lost".to_string(),
    };
    sink(FrameOp::Closed(reason));
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::OnceLock;
    use std::time::Instant;

    use futures_util::future::join_all;
    use futures_util::FutureExt;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use super::*;
    use crate::ssh::known_hosts::KnownHosts;
    use crate::vnc::decode::ENCODINGS;
    use crate::vnc::proto::{client_cut_text, key_event, pointer_event};
    use crate::vnc::testserver::{cut_text, rect, update, After, Script, Seen, Server};
    use crate::vnc::PIXEL_FORMAT;

    const WAIT: Duration = Duration::from_secs(5);
    const LOST: &str = "network connection lost";
    const FULL: Seen = Seen::Request { incremental: false, x: 0, y: 0, w: 4, h: 2 };
    const NEXT: Seen = Seen::Request { incremental: true, x: 0, y: 0, w: 4, h: 2 };

    // A 3.8 server with a 4x2 screen that wants the password "hunter2".
    fn script(updates: Vec<Vec<u8>>, then: After) -> Script {
        Script {
            version: "RFB 003.008\n",
            security: vec![2],
            password: Some("hunter2"),
            size: (4, 2),
            name: "desk".into(),
            updates,
            then,
        }
    }

    // What every client has written once it is connected.
    fn setup() -> Vec<Seen> {
        vec![Seen::SetPixelFormat(PIXEL_FORMAT), Seen::SetEncodings(ENCODINGS.to_vec()), FULL]
    }

    // An update of one Raw pixel, and the operation it is drawn as.
    fn dot(x: u16, y: u16) -> Vec<u8> {
        update(&[rect(x, y, 1, 1, 0, &[1, 2, 3, 4])])
    }

    fn drawn(x: u16, y: u16) -> FrameOp {
        FrameOp::Rect { x, y, w: 1, h: 1, rgba: vec![3, 2, 1, 255] }
    }

    fn key(keysym: u32) -> Seen {
        Seen::Key { down: true, keysym }
    }

    fn known_hosts() -> KnownHostsState {
        let dir = tempfile::tempdir().unwrap();
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        KnownHostsState(Arc::new(std::sync::Mutex::new(hosts)))
    }

    async fn connect(
        port: u16,
        password: &str,
        sink: impl Fn(FrameOp) + Send + 'static,
    ) -> AppResult<Session> {
        let known = known_hosts();
        let login = Login { username: "", password };
        let connecting = Session::connect("127.0.0.1", port, &login, &known, &ENCODINGS, sink);
        timeout(Duration::from_secs(60), connecting).await.expect("connect hung")
    }

    // A connection whose operations arrive on the returned channel.
    async fn open(
        port: u16,
        password: &str,
    ) -> (AppResult<Session>, mpsc::UnboundedReceiver<FrameOp>) {
        let (sink, ops) = mpsc::unbounded_channel();
        (connect(port, password, move |op| sink.send(op).unwrap()).await, ops)
    }

    async fn next(ops: &mut mpsc::UnboundedReceiver<FrameOp>) -> FrameOp {
        timeout(WAIT, ops.recv()).await.expect("no operation arrived").expect("the session is gone")
    }

    // Everything the sink is still given until the session is gone.
    async fn rest(ops: &mut mpsc::UnboundedReceiver<FrameOp>) -> Vec<FrameOp> {
        let mut left = Vec::new();
        while let Some(op) = timeout(WAIT, ops.recv()).await.expect("the session did not end") {
            left.push(op);
        }
        left
    }

    // The server's log once a key pressed now has arrived, so with all that was written before it.
    async fn log_now(session: &Session, server: &mut Server, keysym: u32) -> Vec<Seen> {
        session.key(true, keysym).await.unwrap();
        server.wait(|log| log.contains(&key(keysym))).await
    }

    fn refusal(session: AppResult<Session>) -> String {
        match session {
            Ok(_) => panic!("connect should have failed"),
            Err(e) => e.to_string(),
        }
    }

    #[tokio::test]
    async fn password_login_draws_an_update_and_asks_for_the_next() {
        let mut server = Server::start(script(vec![dot(1, 1)], After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!((session.width, session.height, session.name.as_str()), (4, 2, "desk"));
        assert_eq!(next(&mut ops).await, drawn(1, 1));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        session.ack();
        let log = server.wait(|log| log.len() >= 5).await;
        assert_eq!(log, [setup(), vec![Seen::Sent, NEXT]].concat());
        session.close().await;
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn a_request_is_written_only_once_the_last_update_is_acknowledged() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        let mut log = setup();
        for n in 0..3 {
            log.push(key(n * 2));
            assert_eq!(log_now(&session, &mut server, n * 2).await, log);
            server.send(dot(n as u16, 0));
            assert_eq!(next(&mut ops).await, drawn(n as u16, 0));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            log.extend([Seen::Sent, key(n * 2 + 1)]);
            assert_eq!(log_now(&session, &mut server, n * 2 + 1).await, log);
            session.ack();
            log.push(NEXT);
            assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        }
        let (mut requests, mut updates) = (0, 0);
        for entry in &log {
            match entry {
                Seen::Request { .. } => requests += 1,
                Seen::Sent => updates += 1,
                _ => {}
            }
            assert!(requests <= updates + 1, "two requests outstanding in {log:?}");
        }
    }

    #[tokio::test]
    async fn an_acknowledgement_given_inside_the_sink_is_not_lost() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let slot = Arc::new(OnceLock::<Session>::new());
        let acking = slot.clone();
        let sink = move |op| {
            if op == FrameOp::Sync {
                acking.get().unwrap().ack();
            }
        };
        let session = connect(server.port, "hunter2", sink).await.unwrap();
        assert!(slot.set(session).is_ok());
        assert_eq!(server.wait(|log| log.len() >= 3).await, setup());
        server.send(dot(0, 0));
        let log = server.wait(|log| log.len() >= 5).await;
        assert_eq!(log, [setup(), vec![Seen::Sent, NEXT]].concat());
    }

    #[tokio::test]
    async fn acknowledgements_nobody_waits_for_bring_no_request() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        session.ack();
        let mut log = [setup(), vec![key(1)]].concat();
        assert_eq!(log_now(&session, &mut server, 1).await, log);

        server.send(dot(0, 0));
        assert_eq!(next(&mut ops).await, drawn(0, 0));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        log.extend([Seen::Sent, key(2)]);
        assert_eq!(log_now(&session, &mut server, 2).await, log);
        for _ in 0..3 {
            session.ack();
        }
        log.push(NEXT);
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);

        server.send(dot(1, 0));
        assert_eq!(next(&mut ops).await, drawn(1, 0));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        log.extend([Seen::Sent, key(3)]);
        assert_eq!(log_now(&session, &mut server, 3).await, log);
        session.ack();
        log.push(NEXT);
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
    }

    #[tokio::test]
    async fn an_update_nobody_asked_for_is_drawn_and_acknowledged_too() {
        let mut server = Server::start(script(vec![dot(0, 0)], After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(next(&mut ops).await, drawn(0, 0));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        server.send(dot(1, 0));
        assert_eq!(next(&mut ops).await, drawn(1, 0));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);

        session.ack();
        let mut log = [setup(), vec![Seen::Sent, Seen::Sent, key(1)]].concat();
        assert_eq!(log_now(&session, &mut server, 1).await, log);
        session.ack();
        log.push(NEXT);
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
    }

    #[tokio::test]
    async fn a_resize_is_followed_by_a_full_request_for_the_new_size() {
        let resize = update(&[rect(0, 0, 6, 3, -223, &[])]);
        let mut server = Server::start(script(vec![resize, dot(5, 2)], After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(next(&mut ops).await, FrameOp::Resize { w: 6, h: 3 });
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        session.ack();
        assert_eq!(next(&mut ops).await, drawn(5, 2));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        session.ack();
        let log = server.wait(|log| log.len() >= 7).await;
        let resized = |incremental| Seen::Request { incremental, x: 0, y: 0, w: 6, h: 3 };
        let after = vec![Seen::Sent, resized(false), Seen::Sent, resized(true)];
        assert_eq!(log, [setup(), after].concat());
        assert_eq!((session.width, session.height), (4, 2));
    }

    #[tokio::test]
    async fn an_update_without_operations_still_waits_for_its_acknowledgement() {
        // What TigerVNC, x11vnc and QEMU answer a full request with: the size they already have.
        let same_size = update(&[rect(0, 0, 4, 2, -308, &[0; 4])]);
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        let mut log = setup();
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        for (keysym, empty) in [(1, update(&[])), (2, same_size)] {
            server.send(empty);
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            log.extend([Seen::Sent, key(keysym)]);
            assert_eq!(log_now(&session, &mut server, keysym).await, log);
            session.ack();
            log.push(NEXT);
            assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        }
    }

    #[tokio::test]
    async fn waiting_for_an_acknowledgement_keeps_input_and_reading_going() {
        let mut server = Server::start(script(vec![dot(0, 0)], After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(next(&mut ops).await, drawn(0, 0));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);

        session.pointer(1, 10, 20).await.unwrap();
        let log = log_now(&session, &mut server, 0x61).await;
        let input = vec![Seen::Sent, Seen::Pointer { buttons: 1, x: 10, y: 20 }, key(0x61)];
        assert_eq!(log, [setup(), input].concat());

        server.send(cut_text(b"hi"));
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("hi".into()));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        session.ack();
        drop(server);
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(LOST.into())]);
    }

    #[tokio::test]
    async fn clipboard_text_is_latin1_in_both_directions() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        session.clipboard("héllo ✓").await.unwrap();
        let log = server.wait(|log| log.len() >= 4).await;
        let text = Seen::CutText(vec![b'h', 0xE9, b'l', b'l', b'o', 0x20, 0x3F]);
        assert_eq!(log, [setup(), vec![text]].concat());
        server.send(cut_text(&[0xE9]));
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("é".into()));
    }

    #[tokio::test]
    async fn keys_and_pointer_events_reach_the_server() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, _ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        session.key(true, 0xFFE1).await.unwrap();
        session.pointer(1, 10, 20).await.unwrap();
        session.pointer(1, 300, 400).await.unwrap();
        session.key(false, 0xFFE1).await.unwrap();
        let log = server.wait(|log| log.len() >= 7).await;
        let input = vec![
            Seen::Key { down: true, keysym: 0xFFE1 },
            Seen::Pointer { buttons: 1, x: 10, y: 20 },
            Seen::Pointer { buttons: 1, x: 300, y: 400 },
            Seen::Key { down: false, keysym: 0xFFE1 },
        ];
        assert_eq!(log, [setup(), input].concat());
    }

    #[tokio::test]
    async fn colour_maps_and_bells_are_skipped() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        let colours = [&[1, 0, 0, 5, 0, 2][..], &[9; 12]].concat();
        server.send([colours, vec![2], cut_text(b"after")].concat());
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("after".into()));
    }

    #[tokio::test]
    async fn empty_clipboard_text_is_skipped() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        server.send([cut_text(b""), cut_text(b"after")].concat());
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("after".into()));
    }

    #[tokio::test]
    async fn nul_bytes_are_dropped_from_clipboard_text() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        server.send([cut_text(&[0, 0]), cut_text(b"af\0ter")].concat());
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("after".into()));
    }

    #[tokio::test]
    async fn a_server_that_closes_ends_the_session() {
        let server = Server::start(script(Vec::new(), After::Close)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(next(&mut ops).await, FrameOp::Closed(LOST.into()));
        let sent = [
            session.key(true, 0x61).await,
            session.pointer(0, 1, 1).await,
            session.pointer(1, 1, 1).await,
            session.clipboard("x").await,
        ];
        for sent in sent {
            match sent {
                Err(AppError::NotFound(what)) => assert_eq!(what, "vnc session closed"),
                other => panic!("expected the closed error, got {other:?}"),
            }
        }
        session.ack();
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn input_is_refused_from_the_moment_the_view_hears_of_the_end() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let slot = Arc::new(OnceLock::<Session>::new());
        let (results, mut tried) = mpsc::unbounded_channel();
        let ended = slot.clone();
        let sink = move |op| {
            if let FrameOp::Closed(_) = op {
                results.send(ended.get().unwrap().key(true, 0x61).now_or_never()).unwrap();
            }
        };
        let session = connect(server.port, "hunter2", sink).await.unwrap();
        assert!(slot.set(session).is_ok());
        drop(server);
        let sent = timeout(WAIT, tried.recv()).await.expect("the session did not end");
        match sent {
            Some(Some(Err(AppError::NotFound(what)))) => assert_eq!(what, "vnc session closed"),
            other => panic!("expected the closed error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_connection_lost_inside_an_update_ends_the_session() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        server.send(dot(0, 0)[..18].to_vec());
        drop(server);
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(LOST.into())]);
    }

    #[tokio::test]
    async fn a_failed_write_ends_the_session() {
        let (quiet, _server) = tokio::io::duplex(64);
        let (broken, gone) = tokio::io::duplex(64);
        drop(gone);
        let (requests, input) = mpsc::channel(256);
        requests.send(key_event(true, 0x61).to_vec()).await.unwrap();
        let mut ops = Vec::new();
        let running = run(
            Box::new(tokio::io::join(quiet, broken)),
            Decoder::new(4, 2).unwrap(),
            requests.clone(),
            input,
            Arc::new(Semaphore::new(0)),
            |op| ops.push(op),
        );
        timeout(WAIT, running).await.expect("the session did not end");
        assert_eq!(ops, [FrameOp::Closed(LOST.into())]);
        assert!(requests.is_closed());
    }

    #[tokio::test]
    async fn an_unknown_message_ends_the_session() {
        let server = Server::start(script(Vec::new(), After::Garbage(vec![200]))).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        let closed = FrameOp::Closed("the server sent an unknown message (200)".into());
        assert_eq!(rest(&mut ops).await, [closed]);
    }

    #[tokio::test]
    async fn a_rectangle_outside_the_screen_ends_the_session() {
        let outside = update(&[rect(0, 0, 1, 1, 0, &[1, 2, 3, 4]), rect(4, 0, 1, 1, 0, &[0; 4])]);
        let server = Server::start(script(vec![outside], After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        let closed = FrameOp::Closed("the server sent an invalid update".into());
        assert_eq!(rest(&mut ops).await, [drawn(0, 0), closed]);
    }

    #[tokio::test]
    async fn server_clipboard_text_over_a_mebibyte_is_skipped() {
        let server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        server.send(cut_text(&vec![b'x'; MAX_TEXT]));
        match next(&mut ops).await {
            FrameOp::Clipboard(text) => assert_eq!(text.len(), MAX_TEXT),
            other => panic!("expected the clipboard text, got {other:?}"),
        }
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        server.send([cut_text(&vec![b'x'; MAX_TEXT + 1]), cut_text(b"after")].concat());
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("after".into()));
    }

    #[tokio::test]
    async fn a_connection_lost_inside_skipped_clipboard_text_ends_the_session() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        // Everything the client wrote is read first, so the server closes without a reset.
        assert_eq!(server.wait(|log| log.len() >= 3).await, setup());
        server.send(cut_text(&vec![b'x'; MAX_TEXT + 1])[..10_000].to_vec());
        drop(server);
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(LOST.into())]);
    }

    #[tokio::test]
    async fn clipboard_text_over_a_mebibyte_is_not_sent() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, _ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        session.clipboard(&"é".repeat(MAX_TEXT + 1)).await.unwrap();
        session.clipboard(&"é".repeat(MAX_TEXT)).await.unwrap();
        let log = server.wait(|log| log.len() >= 4).await;
        assert_eq!(log[..3], setup());
        match &log[3..] {
            [Seen::CutText(text)] => assert!(text.len() == MAX_TEXT && text[0] == 0xE9),
            other => panic!("expected one text of a mebibyte, got {} messages", other.len()),
        }
    }

    #[tokio::test]
    async fn updates_pushed_without_acknowledgement_stop_after_two() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        let mut log = setup();
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        let place = |n: u32| ((n % 4) as u16, (n / 4 % 2) as u16);
        server.send((0..10).flat_map(|n| dot(place(n).0, place(n).1)).collect());
        log.push(Seen::Sent);
        for n in 0..10 {
            if n >= 2 {
                log.push(key(n));
                assert_eq!(log_now(&session, &mut server, n).await, log);
                assert!(ops.try_recv().is_err(), "update {n} was read before an acknowledgement");
                session.ack();
            }
            assert_eq!(next(&mut ops).await, drawn(place(n).0, place(n).1));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
        }
        log.push(key(10));
        assert_eq!(log_now(&session, &mut server, 10).await, log);
        session.ack();
        session.ack();
        log.push(NEXT);
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        assert!(ops.try_recv().is_err());
    }

    #[tokio::test]
    async fn clipboard_texts_pushed_without_acknowledgement_stop_after_two() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        let mut log = setup();
        assert_eq!(server.wait(|seen| seen.len() >= log.len()).await, log);
        server.send((b'0'..=b'9').flat_map(|digit| cut_text(&[digit])).collect());
        log.push(Seen::Sent);
        for n in 0..10 {
            if n >= 2 {
                log.push(key(n));
                assert_eq!(log_now(&session, &mut server, n).await, log);
                assert!(ops.try_recv().is_err(), "text {n} was read before an acknowledgement");
                session.ack();
            }
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard(n.to_string()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
        }
        assert!(ops.try_recv().is_err());
    }

    #[tokio::test]
    async fn closing_while_pushed_updates_wait_tells_the_view_nothing() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(server.wait(|log| log.len() >= 3).await, setup());
        server.send([dot(0, 0), dot(1, 0), dot(2, 0)].concat());
        for x in 0..2 {
            assert_eq!(next(&mut ops).await, drawn(x, 0));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
        }
        timeout(WAIT, session.close()).await.expect("close hung");
        assert!(ops.is_closed());
        assert_eq!(rest(&mut ops).await, []);
        let log = server.wait(|log| log.last() == Some(&Seen::Gone)).await;
        assert_eq!(log, [setup(), vec![Seen::Sent, Seen::Gone]].concat());
    }

    #[tokio::test]
    async fn keys_queued_before_close_still_reach_the_server() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        session.key(true, 0xFFE1).await.unwrap();
        session.key(false, 0xFFE1).await.unwrap();
        timeout(WAIT, session.close()).await.expect("close hung");
        assert!(ops.is_closed());
        assert_eq!(rest(&mut ops).await, []);
        let log = server.wait(|log| log.last() == Some(&Seen::Gone)).await;
        let keys = [key(0xFFE1), Seen::Key { down: false, keysym: 0xFFE1 }, Seen::Gone];
        assert_eq!(log, [setup(), keys.to_vec()].concat());
        match session.key(true, 0x61).await {
            Err(AppError::NotFound(what)) => assert_eq!(what, "vnc session closed"),
            other => panic!("expected the closed error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dropped_session_disconnects() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        drop(session.unwrap());
        let log = server.wait(|log| log.last() == Some(&Seen::Gone)).await;
        assert_eq!(log, [setup(), vec![Seen::Gone]].concat());
        assert_eq!(rest(&mut ops).await, []);
    }

    // A server without a login that pushes 40 KB of updates and reads nothing the client writes
    // after its ClientInit until it is told to; it returns all of that once the client is gone.
    async fn late_reader() -> (u16, oneshot::Sender<()>, JoinSet<std::io::Result<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (read, told) = oneshot::channel();
        let mut served = JoinSet::new();
        served.spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            stream.write_all(b"RFB 003.003\n").await?;
            stream.read_exact(&mut [0u8; 12]).await?;
            stream.write_u32(1).await?;
            stream.read_u8().await?;
            stream.write_all(&[&[0, 4, 0, 2][..], &PIXEL_FORMAT, &[0; 4]].concat()).await?;
            stream.write_all(&dot(0, 0).repeat(2048)).await?;
            told.await.map_err(std::io::Error::other)?;
            let mut written = Vec::new();
            stream.read_to_end(&mut written).await?;
            Ok(written)
        });
        (port, read, served)
    }

    #[tokio::test]
    async fn close_delivers_queued_input_to_a_server_that_reads_late() {
        let mut expected = vec![0u8; 4];
        expected.extend(PIXEL_FORMAT);
        expected.extend(set_encodings(&ENCODINGS));
        expected.extend(fb_update_request(false, 0, 0, 4, 2));
        expected.extend(key_event(false, 0xFFE1));
        let rounds = (0..50).map(|_| async {
            let (port, read, mut served) = late_reader().await;
            let (session, mut ops) = open(port, "").await;
            let session = session.unwrap();
            // Two updates are read; the rest of the 40 KB stays unread.
            for _ in 0..2 {
                assert_eq!(next(&mut ops).await, drawn(0, 0));
                assert_eq!(next(&mut ops).await, FrameOp::Sync);
            }
            session.key(false, 0xFFE1).await.unwrap();
            timeout(WAIT, session.close()).await.expect("close hung");
            assert!(ops.is_closed());
            read.send(()).unwrap();
            let written = timeout(WAIT, served.join_next()).await.expect("the server hung");
            written.unwrap().unwrap().ok()
        });
        let delivered = join_all(rounds).await;
        let whole = delivered.iter().filter(|written| written.as_ref() == Some(&expected)).count();
        assert_eq!(whole, 50, "of 50 closes");
    }

    #[tokio::test]
    async fn closing_in_the_middle_of_an_update_tells_the_view_nothing() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(server.wait(|log| log.len() >= 3).await, setup());
        server.send(dot(0, 0)[..18].to_vec());
        server.wait(|log| log.last() == Some(&Seen::Sent)).await;
        // The server answers the close by closing, which cuts the update off.
        timeout(WAIT, session.close()).await.expect("close hung");
        assert!(ops.is_closed());
        assert_eq!(rest(&mut ops).await, []);
        server.wait(|log| log.last() == Some(&Seen::Gone)).await;
    }

    #[tokio::test]
    async fn closing_an_idle_session_does_not_wait_for_the_limit() {
        let mut server = Server::start(script(Vec::new(), After::Hold)).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let session = session.unwrap();
        assert_eq!(server.wait(|log| log.len() >= 3).await, setup());
        let started = Instant::now();
        timeout(WAIT, session.close()).await.expect("close hung");
        let took = started.elapsed();
        assert!(took < Duration::from_millis(250), "close took {took:?}");
        assert_eq!(rest(&mut ops).await, []);
        let log = server.wait(|log| log.last() == Some(&Seen::Gone)).await;
        assert_eq!(log, [setup(), vec![Seen::Gone]].concat());
    }

    #[tokio::test]
    async fn close_gives_a_stuck_writer_its_limit_and_no_more() {
        let (client, mut server) = tokio::io::duplex(64);
        let (input, messages) = mpsc::channel(256);
        let acks = Arc::new(Semaphore::new(0));
        let (sink, mut ops) = mpsc::unbounded_channel();
        let decoder = Decoder::new(4, 2).unwrap();
        let sink = move |op| sink.send(op).unwrap();
        let mut task = JoinSet::new();
        task.spawn(run(Box::new(client), decoder, input.clone(), messages, acks.clone(), sink));
        let session = Session {
            width: 4,
            height: 2,
            name: String::new(),
            input,
            acks,
            buttons: AtomicU8::new(0),
            task: Mutex::new(task),
        };
        // 160 bytes for a pipe that takes 64 and is not read.
        for keysym in 0..20 {
            session.key(true, keysym).await.unwrap();
        }
        let limit = Duration::from_millis(200);
        let started = Instant::now();
        timeout(WAIT, session.close_within(limit)).await.expect("close hung");
        let took = started.elapsed();
        assert!(took >= limit && took < limit * 5, "close took {took:?}");
        assert!(ops.is_closed());
        assert_eq!(rest(&mut ops).await, []);
        let mut written = Vec::new();
        let ended = timeout(WAIT, server.read_to_end(&mut written)).await;
        ended.expect("the connection stayed open").unwrap();
        let keys: Vec<u8> = (0..8).flat_map(|keysym| key_event(true, keysym)).collect();
        assert_eq!(written, keys);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn nothing_reaches_the_view_once_close_has_returned() {
        for round in 0..100 {
            let server = Server::start(script(vec![dot(0, 0)], After::Hold)).await;
            let (session, ops) = open(server.port, "hunter2").await;
            let session = session.unwrap();
            drop(server);
            timeout(WAIT, session.close()).await.expect("close hung");
            assert!(ops.is_closed(), "round {round}: the sink outlived close");
        }
    }

    #[tokio::test]
    async fn v33_server_without_a_login() {
        let old = Script {
            version: "RFB 003.003\n",
            security: vec![1],
            password: None,
            ..script(vec![dot(0, 0)], After::Hold)
        };
        let server = Server::start(old).await;
        let (session, mut ops) = open(server.port, "").await;
        let _session = session.unwrap();
        assert_eq!(next(&mut ops).await, drawn(0, 0));
    }

    #[tokio::test]
    async fn v37_server_with_a_password() {
        let old = Script { version: "RFB 003.007\n", ..script(vec![dot(0, 0)], After::Hold) };
        let server = Server::start(old).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        let _session = session.unwrap();
        assert_eq!(next(&mut ops).await, drawn(0, 0));
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_before_anything_is_drawn() {
        for version in ["RFB 003.003\n", "RFB 003.007\n", "RFB 003.008\n"] {
            let wants = Script { version, ..script(vec![dot(0, 0)], After::Hold) };
            let server = Server::start(wants).await;
            let (session, mut ops) = open(server.port, "hunter3").await;
            assert!(refusal(session).contains("wrong password"), "{version:?}");
            assert_eq!(rest(&mut ops).await, [], "{version:?}");
        }
    }

    #[tokio::test]
    async fn a_screen_without_pixels_is_refused() {
        let flat = Script { size: (0, 2), ..script(Vec::new(), After::Hold) };
        let server = Server::start(flat).await;
        let (session, mut ops) = open(server.port, "hunter2").await;
        assert!(refusal(session).ends_with("vnc: the server reported an invalid desktop size"));
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn the_desktop_name_is_cleaned_and_cut_short() {
        let name = format!("a\u{1}b\u{7f}c\u{9b}\n{}", "é".repeat(300));
        let server = Server::start(Script { name, ..script(Vec::new(), After::Hold) }).await;
        let (session, _ops) = open(server.port, "hunter2").await;
        assert_eq!(session.unwrap().name, format!("abc{}", "é".repeat(197)));
    }

    #[tokio::test]
    async fn a_port_nothing_listens_on_is_an_error() {
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let (session, mut ops) = open(port, "").await;
        assert!(matches!(session, Err(AppError::Io(_))));
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn a_server_that_never_speaks_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _silent = listener.accept().await;
            std::future::pending::<()>().await
        });
        let known = known_hosts();
        let login = Login { username: "", password: "" };
        let (sink, mut ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let limit = Duration::from_millis(300);
        let started = Instant::now();
        let connecting =
            Session::connect_within("127.0.0.1", port, &login, &known, &ENCODINGS, sink, limit);
        let session = timeout(WAIT, connecting).await.expect("connect hung");
        assert!(refusal(session).ends_with("vnc: connection timed out"));
        assert!(started.elapsed() >= limit);
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn a_full_channel_drops_pointer_moves_and_holds_everything_else() {
        let (input, mut messages) = mpsc::channel(256);
        let session = Session {
            width: 4,
            height: 2,
            name: String::new(),
            input,
            acks: Arc::new(Semaphore::new(0)),
            buttons: AtomicU8::new(0),
            task: Mutex::new(JoinSet::new()),
        };
        // One poll outside tokio's cooperative budget, which makes the 129th send in a row wait
        // whether there is room or not.
        fn at_once<T>(sending: impl Future<Output = T>) -> Option<T> {
            tokio::task::unconstrained(sending).now_or_never()
        }
        for keysym in 0..256 {
            at_once(session.key(true, keysym)).expect("a key waited for room").unwrap();
        }
        for x in 0..10 {
            at_once(session.pointer(0, x, 0)).expect("a move waited for room").unwrap();
        }
        let mut press = pin!(session.pointer(1, 5, 5));
        let mut release = pin!(session.key(false, 7));
        let mut text = pin!(session.clipboard("x"));
        assert!(at_once(&mut press).is_none());
        assert!(at_once(&mut release).is_none());
        assert!(at_once(&mut text).is_none());

        for keysym in 0..256 {
            assert_eq!(messages.try_recv().unwrap(), key_event(true, keysym));
        }
        at_once(press).expect("the press still waits").unwrap();
        at_once(release).expect("the key still waits").unwrap();
        at_once(text).expect("the text still waits").unwrap();
        assert_eq!(messages.try_recv().unwrap(), pointer_event(1, 5, 5));
        assert_eq!(messages.try_recv().unwrap(), key_event(false, 7));
        assert_eq!(messages.try_recv().unwrap(), client_cut_text(b"x"));
        assert!(messages.try_recv().is_err());
    }
}
