pub mod socks;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Buf, Bytes};
use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::{AbortHandle, JoinHandle};

use crate::error::{AppError, AppResult};
use crate::ssh::client::{RemoteForwards, RemoteTarget, SshHandle};

pub struct RemoteCleanup {
    pub handle: Arc<SshHandle>,
    pub bind_host: String,
    pub bound_port: u32,
    pub registry: RemoteForwards,
}

pub type Conns = Arc<Mutex<Vec<AbortHandle>>>;

pub struct TunnelHandle {
    pub abort: AbortHandle,
    pub conns: Conns,
    pub session_id: String,
    pub remote: Option<RemoteCleanup>,
}

fn track(conns: &Conns, h: AbortHandle) {
    if let Ok(mut v) = conns.lock() {
        v.retain(|a| !a.is_finished());
        v.push(h);
    }
}

// Pump one direction, counting bytes. Ends when the reader hits EOF.
pub async fn pump<R, W>(mut r: R, mut w: W, counter: Arc<AtomicU64>) -> std::io::Result<()>
where
    R: AsyncReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).await?;
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
    let _ = w.shutdown().await;
    Ok(())
}

// The halves of a split channel do not close it when they go, and a relay's task can be
// aborted at any point.
struct CloseOnDrop(Option<ChannelWriteHalf<Msg>>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let Some(half) = self.0.take() {
            tokio::spawn(async move {
                // Fails only when the session is gone, and the channel with it.
                let _ = half.close().await;
            });
        }
    }
}

pub const BACKLOG_LIMIT: usize = 2 * 1024 * 1024;
pub const GRACE: Duration = Duration::from_secs(3);

#[cfg(test)]
thread_local! {
    static GRACE_FOR_TEST: std::cell::Cell<Duration> = const { std::cell::Cell::new(GRACE) };
    // The most a relay on this thread has held for its local peer, with the message in hand.
    static HIGH_WATER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct AbortOnDrop(JoinHandle<std::io::Result<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// Carries one forwarded connection. A far side that is done sending (EOF) may still wait for
// its answer, so that only ends its own direction; a CLOSE, the end of the session or an
// error on either side ends both. `over` tells whether the session has ended.
//
// What the far side sends waits here for the local peer, up to BACKLOG_LIMIT. The session
// serves all its channels in one loop, and that loop stands still for as long as one of them
// does not take a message, so a full backlog holds up the whole session. A local peer that
// takes nothing for GRACE with the backlog full loses its connection.
pub async fn relay(
    channel: Channel<Msg>,
    local: tokio::net::TcpStream,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
    over: impl Fn() -> bool + Send + Sync,
) {
    let (lr, lw) = local.into_split();
    let (far, near) = channel.split();
    let writer = near.make_writer();
    let _close = CloseOnDrop(Some(near));
    // In a task of its own: sharing one with the receiving side, the local peer is read too
    // slowly when much arrives for it, and the session stalls more often when a peer sends
    // back what it gets.
    let sending = tokio::spawn(pump(lr, writer, up));
    relay_loop(far, lw, sending, down, over).await;
}

async fn relay_loop<W: AsyncWrite + Unpin>(
    mut far: ChannelReadHalf,
    mut lw: W,
    sending: JoinHandle<std::io::Result<()>>,
    down: Arc<AtomicU64>,
    over: impl Fn() -> bool + Send + Sync,
) {
    let mut sending = AbortOnDrop(sending);
    #[cfg(test)]
    let grace = GRACE_FOR_TEST.with(|grace| grace.get());
    #[cfg(not(test))]
    let grace = GRACE;
    let mut backlog: VecDeque<Bytes> = VecDeque::new();
    let mut waiting = 0;
    let mut progress = tokio::time::Instant::now();
    let (mut sent, mut received) = (false, false);
    let (mut ending, mut closed) = (false, false);

    loop {
        if backlog.is_empty() {
            if closed {
                return;
            }
            if ending && !received {
                received = true;
                if lw.shutdown().await.is_err() {
                    return;
                }
            }
        }
        if sent && received {
            return;
        }
        let next = backlog.front().map_or(&[][..], |data| &data[..]);
        // Biased, so that a write that is ready wins over the end of the grace time.
        tokio::select! {
            biased;
            written = lw.write(next), if !backlog.is_empty() => {
                let Ok(n) = written else { return };
                down.fetch_add(n as u64, Ordering::Relaxed);
                waiting -= n;
                progress = tokio::time::Instant::now();
                backlog[0].advance(n);
                if backlog[0].is_empty() {
                    backlog.pop_front();
                }
            }
            ended = &mut sending.0, if !sent && !closed => {
                if !matches!(ended, Ok(Ok(()))) {
                    return;
                }
                sent = true;
            }
            msg = far.wait(), if !closed && waiting < BACKLOG_LIMIT => match msg {
                Some(ChannelMsg::Data { data }) => {
                    #[cfg(test)]
                    HIGH_WATER.with(|most| most.set(most.get().max(waiting + data.len())));
                    waiting += data.len();
                    backlog.push_back(data);
                    if waiting >= BACKLOG_LIMIT {
                        progress = tokio::time::Instant::now();
                    }
                }
                Some(ChannelMsg::Eof) => ending = true,
                // What is queued still goes to the local peer; only the pump stops.
                Some(ChannelMsg::Close) => {
                    closed = true;
                    sending.0.abort();
                }
                None => return,
                Some(_) => {}
            },
            _ = tokio::time::sleep_until(progress + grace), if waiting >= BACKLOG_LIMIT => return,
            // Nothing more comes from a closed channel, the end of the session neither.
            _ = tokio::time::sleep(Duration::from_millis(100)), if closed => {
                if over() {
                    return;
                }
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub tunnel_id: String,
    pub state: String,
    #[specta(type = specta_typescript::Number)]
    pub bytes_up: u64,
    #[specta(type = specta_typescript::Number)]
    pub bytes_down: u64,
    pub message: Option<String>,
}

fn emit<R: Runtime>(app: &AppHandle<R>, status: TunnelStatus) {
    let _ = status.emit(app);
}

// Bridge one accepted local connection to a fresh direct-tcpip channel.
fn bridge(
    handle: Arc<SshHandle>,
    local: tokio::net::TcpStream,
    target_host: String,
    target_port: u16,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) -> AbortHandle {
    tokio::spawn(async move {
        let peer = local.peer_addr().ok();
        let (oa, op) = peer
            .map(|p| (p.ip().to_string(), p.port() as u32))
            .unwrap_or_else(|| ("127.0.0.1".into(), 0));
        let Ok(channel) = handle
            .channel_open_direct_tcpip(target_host, target_port as u32, oa, op)
            .await
        else {
            return;
        };
        relay(channel, local, up, down, || handle.is_closed()).await;
    })
    .abort_handle()
}

pub fn run_local<R: Runtime>(
    app: AppHandle<R>,
    tunnel_id: String,
    session_id: String,
    handle: Arc<SshHandle>,
    bind: String,
    target_host: String,
    target_port: u16,
) -> AppResult<TunnelHandle> {
    let bytes_up = Arc::new(AtomicU64::new(0));
    let bytes_down = Arc::new(AtomicU64::new(0));
    let (up, down) = (bytes_up.clone(), bytes_down.clone());
    let tid = tunnel_id.clone();
    let conns: Conns = Arc::new(Mutex::new(Vec::new()));
    let task_conns = conns.clone();

    let task = tokio::spawn(async move {
        let listener = match TcpListener::bind(&bind).await {
            Ok(l) => l,
            Err(e) => {
                emit(&app, TunnelStatus { tunnel_id: tid, state: "error".into(), bytes_up: 0, bytes_down: 0, message: Some(e.to_string()) });
                return;
            }
        };
        emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: 0, bytes_down: 0, message: None });
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    if let Ok((local, _)) = accepted {
                        let h = bridge(handle.clone(), local, target_host.clone(), target_port, up.clone(), down.clone());
                        track(&task_conns, h);
                    }
                }
                _ = tick.tick() => {
                    emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: up.load(Ordering::Relaxed), bytes_down: down.load(Ordering::Relaxed), message: None });
                }
            }
        }
    });

    Ok(TunnelHandle { abort: task.abort_handle(), conns, session_id, remote: None })
}

pub fn run_dynamic<R: Runtime>(
    app: AppHandle<R>,
    tunnel_id: String,
    session_id: String,
    handle: Arc<SshHandle>,
    bind: String,
) -> AppResult<TunnelHandle> {
    let bytes_up = Arc::new(AtomicU64::new(0));
    let bytes_down = Arc::new(AtomicU64::new(0));
    let (up, down) = (bytes_up.clone(), bytes_down.clone());
    let tid = tunnel_id.clone();
    let conns: Conns = Arc::new(Mutex::new(Vec::new()));
    let task_conns = conns.clone();

    let task = tokio::spawn(async move {
        let listener = match TcpListener::bind(&bind).await {
            Ok(l) => l,
            Err(e) => {
                emit(&app, TunnelStatus { tunnel_id: tid, state: "error".into(), bytes_up: 0, bytes_down: 0, message: Some(e.to_string()) });
                return;
            }
        };
        emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: 0, bytes_down: 0, message: None });
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    if let Ok((mut local, _)) = accepted {
                        let handle = handle.clone();
                        let (up, down) = (up.clone(), down.clone());
                        let h = tokio::spawn(async move {
                            let Ok((host, port)) = socks::handshake(&mut local).await else { return };
                            let oa = local.peer_addr().map(|p| p.ip().to_string()).unwrap_or_else(|_| "127.0.0.1".into());
                            match handle.channel_open_direct_tcpip(host, port as u32, oa, 0).await {
                                Ok(channel) => {
                                    if socks::reply(&mut local, 0x00).await.is_err() { return; }
                                    relay(channel, local, up, down, || handle.is_closed()).await;
                                }
                                Err(_) => { let _ = socks::reply(&mut local, 0x05).await; }
                            }
                        });
                        track(&task_conns, h.abort_handle());
                    }
                }
                _ = tick.tick() => {
                    emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: up.load(Ordering::Relaxed), bytes_down: down.load(Ordering::Relaxed), message: None });
                }
            }
        }
    });

    Ok(TunnelHandle { abort: task.abort_handle(), conns, session_id, remote: None })
}

#[allow(clippy::too_many_arguments)]
pub async fn run_remote<R: Runtime>(
    app: AppHandle<R>,
    tunnel_id: String,
    session_id: String,
    handle: Arc<SshHandle>,
    registry: RemoteForwards,
    bind_host: String,
    bind_port: u16,
    target_host: String,
    target_port: u16,
) -> AppResult<TunnelHandle> {
    let bound = handle.tcpip_forward(bind_host.clone(), bind_port as u32).await?;
    // russh 0.61.2 returns 0 when a specific port was requested (the success reply
    // is empty); only a port-0 request yields the server-assigned port. Normalize so
    // the registry key matches the connected_port the server sends back later and so
    // cancel_tcpip_forward targets the right port.
    let bound_port = if bound == 0 { bind_port as u32 } else { bound };
    let bytes_up = Arc::new(AtomicU64::new(0));
    let bytes_down = Arc::new(AtomicU64::new(0));
    registry
        .lock()
        .map_err(|_| AppError::Tunnel("remote_forwards poisoned".into()))?
        .insert(
            (bind_host.clone(), bound_port),
            RemoteTarget { target_host, target_port, up: bytes_up.clone(), down: bytes_down.clone() },
        );

    let (up, down) = (bytes_up.clone(), bytes_down.clone());
    let tid = tunnel_id.clone();
    let task = tokio::spawn(async move {
        emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: 0, bytes_down: 0, message: None });
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            emit(&app, TunnelStatus { tunnel_id: tid.clone(), state: "active".into(), bytes_up: up.load(Ordering::Relaxed), bytes_down: down.load(Ordering::Relaxed), message: None });
        }
    });

    Ok(TunnelHandle {
        abort: task.abort_handle(),
        conns: Arc::new(Mutex::new(Vec::new())),
        session_id,
        remote: Some(RemoteCleanup { handle, bind_host, bound_port, registry }),
    })
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Instant;

    use tauri::test::MockRuntime;
    use tokio::net::TcpStream;
    use tokio::task::JoinHandle;
    use tokio::sync::mpsc;
    use tokio::time::{sleep, sleep_until, timeout};

    use super::*;
    use crate::ssh::client::testing::session;

    const LIMIT: Duration = Duration::from_secs(10);

    // One connection through a forward: the program that connected to the local port, and the
    // channel as the server holds it for the service behind it.
    struct Forward {
        client: TcpStream,
        service: Channel<russh::server::Msg>,
        task: AbortHandle,
        handle: Arc<SshHandle>,
        _dynamic: Option<(tauri::App<MockRuntime>, TunnelHandle)>,
    }

    async fn local() -> Forward {
        let (handle, _forwards, _server, mut opened) = session().await;
        let handle = Arc::new(handle);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        let counters = (Arc::default(), Arc::default());
        let task = bridge(handle.clone(), accepted, "service".into(), 80, counters.0, counters.1);
        let service = timeout(LIMIT, opened.recv()).await.expect("no channel").unwrap();
        Forward { client, service, task, handle, _dynamic: None }
    }

    async fn dynamic() -> Forward {
        let (handle, _forwards, _server, mut opened) = session().await;
        let handle = Arc::new(handle);
        let app = tauri::test::mock_app();
        tauri_specta::Builder::<MockRuntime>::new()
            .events(tauri_specta::collect_events![TunnelStatus])
            .mount_events(&app);
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let app_handle = app.handle().clone();
        let (bind, session) = (free.to_string(), handle.clone());
        let tunnel = run_dynamic(app_handle, "t".into(), "s".into(), session, bind).unwrap();

        let started = Instant::now();
        let mut client = loop {
            match TcpStream::connect(free).await {
                Ok(client) => break client,
                Err(e) => assert!(started.elapsed() < LIMIT, "the tunnel does not listen: {e}"),
            }
            sleep(Duration::from_millis(10)).await;
        };
        let mut reply = [0u8; 12];
        let greeting = async {
            client.write_all(&[5, 1, 0]).await.unwrap();
            client.read_exact(&mut reply[..2]).await.unwrap();
            client.write_all(&[5, 1, 0, 1, 10, 0, 0, 1, 0, 80]).await.unwrap();
            client.read_exact(&mut reply[2..]).await.unwrap();
        };
        timeout(LIMIT, greeting).await.expect("no SOCKS5 reply");
        assert_eq!(reply, [5, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        let service = timeout(LIMIT, opened.recv()).await.expect("no channel").unwrap();
        let task = tunnel.conns.lock().unwrap()[0].clone();
        Forward { client, service, task, handle, _dynamic: Some((app, tunnel)) }
    }

    async fn gone(task: &AbortHandle) {
        let started = Instant::now();
        while !task.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(1), "the forward's task lives on");
            sleep(Duration::from_millis(10)).await;
        }
    }

    // The service sends, closes its sending side and goes on reading; then it closes.
    async fn late_data_arrives_and_a_close_ends_it(mut forward: Forward) {
        forward.service.data(&b"hello"[..]).await.unwrap();
        forward.service.eof().await.unwrap();
        let mut got = Vec::new();
        timeout(LIMIT, forward.client.read_to_end(&mut got)).await.expect("no end").unwrap();
        assert_eq!(got, b"hello");

        forward.client.write_all(b"late").await.unwrap();
        loop {
            match timeout(LIMIT, forward.service.wait()).await.expect("no data") {
                Some(ChannelMsg::Data { data }) => {
                    assert_eq!(&data[..], b"late");
                    break;
                }
                Some(_) => {}
                None => panic!("the channel closed without the client's data"),
            }
        }

        forward.service.close().await.unwrap();
        gone(&forward.task).await;
        let failing = async { while forward.client.write_all(b"more").await.is_ok() {} };
        timeout(LIMIT, failing).await.expect("the client's writes still go somewhere");
    }

    // The client leaves; the forward ends as soon as the service sends into it.
    async fn ends_once_the_client_has_gone(mut forward: Forward) {
        drop(forward.client);
        loop {
            match timeout(LIMIT, forward.service.wait()).await.expect("no end of input") {
                Some(ChannelMsg::Eof) => break,
                Some(_) => {}
                None => panic!("the channel closed while the service could still answer"),
            }
        }
        let started = Instant::now();
        while !forward.task.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(1), "the forward's task lives on");
            if forward.service.data(&b"answer"[..]).await.is_err() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        gone(&forward.task).await;
    }

    // More arrives than the client's socket takes unread, in fewer packets than the channel's
    // queue holds: with that queue full the session itself would hang.
    async fn ends_with_the_session_while_the_client_does_not_read(forward: Forward) {
        let Forward { client: _client, service, task, handle, _dynamic } = forward;
        let sending = tokio::spawn(async move {
            service.data_bytes(vec![0x55u8; 2 * 1024 * 1024]).await.unwrap();
            service
        });
        let _service = timeout(LIMIT, sending).await.expect("the service could not send").unwrap();
        sleep(Duration::from_millis(200)).await;
        handle.disconnect(russh::Disconnect::ByApplication, "", "en").await.unwrap();
        gone(&task).await;
    }

    #[tokio::test]
    async fn local_forward_ends_with_the_session_while_the_client_does_not_read() {
        ends_with_the_session_while_the_client_does_not_read(local().await).await;
    }

    #[tokio::test]
    async fn dynamic_forward_ends_with_the_session_while_the_client_does_not_read() {
        ends_with_the_session_while_the_client_does_not_read(dynamic().await).await;
    }

    #[tokio::test]
    async fn local_forward_carries_what_the_client_sends_after_the_service_half_closed() {
        late_data_arrives_and_a_close_ends_it(local().await).await;
    }

    #[tokio::test]
    async fn local_forward_ends_once_the_client_has_gone() {
        ends_once_the_client_has_gone(local().await).await;
    }

    #[tokio::test]
    async fn dynamic_forward_carries_what_the_client_sends_after_the_service_half_closed() {
        late_data_arrives_and_a_close_ends_it(dynamic().await).await;
    }

    #[tokio::test]
    async fn dynamic_forward_ends_once_the_client_has_gone() {
        ends_once_the_client_has_gone(dynamic().await).await;
    }

    const MIB: usize = 1024 * 1024;
    const PACKET: usize = 32 * 1024;
    const SHORT_GRACE: Duration = Duration::from_millis(500);

    async fn full(task: &AbortHandle) {
        let started = Instant::now();
        while HIGH_WATER.with(|most| most.get()) < BACKLOG_LIMIT {
            assert!(!task.is_finished(), "the forward ended before it was full");
            assert!(started.elapsed() < LIMIT, "the forward never filled");
            sleep(Duration::from_millis(5)).await;
        }
    }

    // A session on which a test opens local forwards one after the other.
    struct Link {
        handle: Arc<SshHandle>,
        opened: mpsc::UnboundedReceiver<Channel<russh::server::Msg>>,
    }

    struct Bridged {
        client: TcpStream,
        service: Channel<russh::server::Msg>,
        task: AbortHandle,
        // The forward's tasks hold a clone each for as long as they live.
        up: Arc<AtomicU64>,
        down: Arc<AtomicU64>,
    }

    async fn link() -> Link {
        let (handle, _forwards, _server, opened) = session().await;
        Link { handle: Arc::new(handle), opened }
    }

    impl Link {
        async fn forward(&mut self) -> Bridged {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
            let (accepted, _) = listener.accept().await.unwrap();
            let (up, down) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
            let handle = self.handle.clone();
            let task = bridge(handle, accepted, "service".into(), 80, up.clone(), down.clone());
            let service = timeout(LIMIT, self.opened.recv()).await.expect("no channel").unwrap();
            Bridged { client, service, task, up, down }
        }

        // A fresh forward on the same session carries a question and its answer.
        async fn answers(&mut self, within: Duration) {
            let exchange = async {
                let mut second = self.forward().await;
                second.service.data(&b"ping"[..]).await.unwrap();
                let mut asked = [0u8; 4];
                second.client.read_exact(&mut asked).await.unwrap();
                second.client.write_all(b"pong").await.unwrap();
                loop {
                    match second.service.wait().await {
                        Some(ChannelMsg::Data { data }) => break assert_eq!(&data[..], b"pong"),
                        Some(_) => {}
                        None => panic!("the second forward closed"),
                    }
                }
            };
            let answered = timeout(within, exchange).await;
            answered.expect("a second forward on the session got no answer");
        }
    }

    // Every byte depends on where it stands.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len as u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect()
    }

    async fn ended_within(task: &AbortHandle, limit: Duration) {
        let started = Instant::now();
        while !task.is_finished() {
            assert!(started.elapsed() < limit, "the forward did not end in {limit:?}");
            sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_client_that_stopped_reading_loses_its_forward_and_nothing_else() {
        GRACE_FOR_TEST.with(|grace| grace.set(SHORT_GRACE));
        HIGH_WATER.with(|most| most.set(0));
        let one_second = Duration::from_secs(1);
        let mut link = link().await;
        let Bridged { mut client, service, task, up, down } = link.forward().await;
        let (mut far, sender) = service.split();

        // Until the client's socket takes no more: from then on the forward holds data back.
        let mut sent = 0;
        'filling: loop {
            assert!(sent < 12 * MIB, "the client's socket took {sent} bytes unread");
            let sending = sender.data_bytes(vec![0x55u8; 256 * 1024]);
            timeout(LIMIT, sending).await.expect("the service could not send").unwrap();
            sent += 256 * 1024;
            let started = Instant::now();
            while down.load(Ordering::Relaxed) < sent as u64 {
                if started.elapsed() > Duration::from_millis(300) {
                    break 'filling;
                }
                sleep(Duration::from_millis(10)).await;
            }
        }
        link.answers(one_second).await;
        assert!(!task.is_finished(), "the forward ended with {sent} bytes sent");

        let rest = vec![0x55u8; 16 * MIB - sent];
        let flooding = tokio::spawn(async move { sender.data_bytes(rest).await });
        full(&task).await;
        // The session waits for this forward: the answer comes when it has ended.
        link.answers(SHORT_GRACE + one_second).await;
        ended_within(&task, SHORT_GRACE + Duration::from_secs(2)).await;
        loop {
            match timeout(LIMIT, far.wait()).await.expect("the service's channel stays open") {
                Some(ChannelMsg::Close) => break,
                Some(_) => {}
                None => panic!("the channel went without a close"),
            }
        }
        link.answers(one_second).await;

        let started = Instant::now();
        while Arc::strong_count(&up) > 1 || Arc::strong_count(&down) > 1 {
            assert!(started.elapsed() < Duration::from_secs(1), "a task of the forward lives on");
            sleep(Duration::from_millis(10)).await;
        }
        // Ended or reset, as the system has it.
        let closed = timeout(LIMIT, client.read_to_end(&mut Vec::new())).await;
        assert!(closed.is_ok(), "the client's connection was left open");
        flooding.abort();
    }

    #[tokio::test]
    async fn a_client_that_reads_slowly_but_steadily_gets_everything() {
        let mut link = link().await;
        let Bridged { mut client, service, task, .. } = link.forward().await;
        let sent = pattern(3 * MIB);
        // 1 MiB a second on both sides, by the clock.
        let (started, step) = (tokio::time::Instant::now(), Duration::from_micros(62_500));
        let outgoing = sent.clone();
        let sending = tokio::spawn(async move {
            for (i, piece) in outgoing.chunks(64 * 1024).enumerate() {
                sleep_until(started + step * i as u32).await;
                service.data_bytes(piece.to_vec()).await.unwrap();
            }
            service
        });
        let mut got = vec![0u8; sent.len()];
        let reading = async {
            for (i, piece) in got.chunks_mut(64 * 1024).enumerate() {
                sleep_until(started + step * (i as u32 + 1)).await;
                client.read_exact(piece).await.unwrap();
            }
        };
        timeout(LIMIT, reading).await.expect("the data does not arrive");
        assert!(got == sent, "what arrived is not what was sent");
        let _service = timeout(LIMIT, sending).await.expect("the service could not send").unwrap();
        assert!(!task.is_finished(), "the forward ended");
    }

    #[tokio::test]
    async fn a_client_that_pauses_gets_everything() {
        let mut link = link().await;
        let Bridged { mut client, service, .. } = link.forward().await;
        let sent = pattern(MIB);
        let sending = service.data_bytes(sent.clone());
        timeout(LIMIT, sending).await.expect("the service could not send").unwrap();
        sleep(Duration::from_millis(300)).await;

        let mut got = vec![0u8; sent.len()];
        let reading = client.read_exact(&mut got);
        timeout(LIMIT, reading).await.expect("the data does not arrive").unwrap();
        assert!(got == sent, "what arrived is not what was sent");
        service.data(&b"more"[..]).await.unwrap();
        let mut more = [0u8; 4];
        timeout(LIMIT, client.read_exact(&mut more)).await.expect("nothing after it").unwrap();
        assert_eq!(&more, b"more");
    }

    #[tokio::test]
    async fn a_client_that_reads_late_gets_everything_and_then_the_end() {
        let mut link = link().await;
        let Bridged { mut client, service, .. } = link.forward().await;
        let sent = pattern(MIB);
        let sending = async {
            service.data_bytes(sent.clone()).await.unwrap();
            service.eof().await.unwrap();
        };
        timeout(LIMIT, sending).await.expect("the service could not send");
        sleep(Duration::from_millis(300)).await;

        let mut got = Vec::new();
        timeout(LIMIT, client.read_to_end(&mut got)).await.expect("no end").unwrap();
        assert!(got == sent, "what arrived is not what was sent");
    }

    #[tokio::test]
    async fn a_far_close_waits_for_what_the_client_has_not_read() {
        let mut link = link().await;
        let Bridged { mut client, service, task, .. } = link.forward().await;
        let sent = pattern(MIB);
        let sending = async {
            service.data_bytes(sent.clone()).await.unwrap();
            service.eof().await.unwrap();
            service.close().await.unwrap();
        };
        timeout(LIMIT, sending).await.expect("the service could not send");
        sleep(Duration::from_millis(300)).await;

        let mut got = Vec::new();
        timeout(LIMIT, client.read_to_end(&mut got)).await.expect("no end").unwrap();
        assert!(got == sent, "what arrived is not what was sent");
        gone(&task).await;
    }

    // As much as a forward may hold, so that the client's socket cannot have taken it all.
    #[tokio::test]
    async fn a_closed_forward_ends_with_the_session_while_the_client_does_not_read() {
        let mut link = link().await;
        let Bridged { client: _client, service, task, .. } = link.forward().await;
        let sending = async {
            service.data_bytes(vec![0x55u8; BACKLOG_LIMIT]).await.unwrap();
            service.close().await.unwrap();
        };
        timeout(LIMIT, sending).await.expect("the service could not send");
        sleep(Duration::from_millis(200)).await;
        link.handle.disconnect(russh::Disconnect::ByApplication, "", "en").await.unwrap();
        gone(&task).await;
    }

    #[tokio::test]
    async fn a_client_that_reads_slowly_with_the_backlog_full_is_never_cut_off() {
        GRACE_FOR_TEST.with(|grace| grace.set(Duration::from_secs(2)));
        HIGH_WATER.with(|most| most.set(0));
        let mut link = link().await;
        let Bridged { mut client, service, task, .. } = link.forward().await;
        let sent = pattern(16 * MIB);
        let outgoing = sent.clone();
        let (_far, sender) = service.split();
        // Pieces that do not add up to the limit: the backlog ends up a little over it.
        let flooding = tokio::spawn(async move {
            for piece in outgoing.chunks(30_000) {
                sender.data(piece).await.unwrap();
            }
        });
        full(&task).await;

        let mut got = vec![0u8; 16 * 1024];
        let started = Instant::now();
        let mut taken = 0;
        while started.elapsed() < Duration::from_secs(6) {
            timeout(LIMIT, client.read_exact(&mut got)).await.expect("no data").unwrap();
            assert!(got == sent[taken..taken + got.len()], "what arrived is not what was sent");
            taken += got.len();
            assert!(!task.is_finished(), "the forward ended after {taken} bytes");
            sleep(Duration::from_millis(100)).await;
        }
        assert!(!flooding.is_finished(), "the service ran out of data to send");
        flooding.abort();
    }

    // Takes 100 bytes at a time and then nothing until `gap` has passed.
    struct Trickle {
        gap: Duration,
        ready: tokio::time::Instant,
        wait: Pin<Box<tokio::time::Sleep>>,
        taken: Arc<AtomicU64>,
        broken: bool,
    }

    impl AsyncWrite for Trickle {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.broken {
                return Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()));
            }
            let now = tokio::time::Instant::now();
            if now < self.ready {
                let ready = self.ready;
                self.wait.as_mut().reset(ready);
                if self.wait.as_mut().poll(cx).is_ready() {
                    cx.waker().wake_by_ref();
                }
                return Poll::Pending;
            }
            let n = buf.len().min(100);
            self.ready = now + self.gap;
            self.taken.fetch_add(n as u64, Ordering::SeqCst);
            Poll::Ready(Ok(n))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    // A relay loop on a new channel whose local side is a `Trickle`.
    async fn trickled(
        link: &mut Link,
        grace: Duration,
        gap: Duration,
        broken: bool,
    ) -> (JoinHandle<()>, Channel<russh::server::Msg>, Arc<AtomicU64>) {
        GRACE_FOR_TEST.with(|most| most.set(grace));
        HIGH_WATER.with(|most| most.set(0));
        let opening = link.handle.channel_open_direct_tcpip("service", 80, "127.0.0.1", 0);
        let channel = timeout(LIMIT, opening).await.expect("no channel").unwrap();
        let service = timeout(LIMIT, link.opened.recv()).await.expect("no channel").unwrap();
        let (far, near) = channel.split();
        let taken = Arc::new(AtomicU64::new(0));
        let local = Trickle {
            gap,
            ready: tokio::time::Instant::now(),
            wait: Box::pin(sleep(gap)),
            taken: taken.clone(),
            broken,
        };
        let sending = tokio::spawn(std::future::pending::<std::io::Result<()>>());
        let task = tokio::spawn(async move {
            let _near = near;
            relay_loop(far, local, sending, Arc::default(), || false).await;
        });
        (task, service, taken)
    }

    // The backlog ends up 32 KiB - 1 over the limit.
    async fn overfill(service: &Channel<russh::server::Msg>) {
        let sending = async {
            service.data_bytes(vec![0x55u8; BACKLOG_LIMIT - 1]).await.unwrap();
            service.data(&[0x55u8; PACKET][..]).await.unwrap();
        };
        timeout(LIMIT, sending).await.expect("the service could not send");
    }

    #[tokio::test]
    async fn every_write_to_the_local_peer_restarts_the_grace_time() {
        let mut link = link().await;
        let gap = Duration::from_millis(200);
        let grace = Duration::from_millis(400);
        let (task, service, taken) = trickled(&mut link, grace, gap, false).await;
        overfill(&service).await;
        full(&task.abort_handle()).await;

        // 7.5 times the grace time, with far fewer bytes taken than the backlog is over.
        sleep(Duration::from_secs(3)).await;
        assert!(!task.is_finished(), "the forward ended");
        let taken = taken.load(Ordering::SeqCst);
        assert!((1000..=2000).contains(&taken), "the peer took {taken} bytes");
    }

    // The grace time is over and the peer is ready in the same poll.
    #[tokio::test]
    async fn a_peer_that_is_ready_when_the_grace_time_ends_keeps_its_forward() {
        let mut link = link().await;
        let grace = Duration::from_millis(200);
        let gap = Duration::from_millis(150);
        let (task, service, taken) = trickled(&mut link, grace, gap, false).await;
        overfill(&service).await;
        full(&task.abort_handle()).await;

        for trial in 0..12 {
            let seen = taken.load(Ordering::SeqCst);
            let started = Instant::now();
            while taken.load(Ordering::SeqCst) == seen {
                assert!(started.elapsed() < LIMIT, "the peer took nothing more");
                sleep(Duration::from_millis(5)).await;
            }
            // Stand still past the end of the grace time: the timer and the peer are due together.
            std::thread::sleep(Duration::from_millis(230));
            sleep(Duration::from_millis(30)).await;
            assert!(!task.is_finished(), "the forward ended in trial {trial}");
        }
    }

    #[tokio::test]
    async fn a_local_peer_that_fails_with_data_waiting_ends_its_forward() {
        let mut link = link().await;
        let (task, service, _) =
            trickled(&mut link, GRACE, Duration::from_millis(10), true).await;
        service.data(&b"waiting"[..]).await.unwrap();
        ended_within(&task.abort_handle(), Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn a_forward_holds_no_more_than_the_limit_and_one_message() {
        GRACE_FOR_TEST.with(|grace| grace.set(SHORT_GRACE));
        HIGH_WATER.with(|most| most.set(0));
        let mut link = link().await;
        let Bridged { client: _client, service, task, .. } = link.forward().await;
        let (_far, sender) = service.split();
        let flooding = tokio::spawn(async move { sender.data_bytes(vec![0x55u8; 16 * MIB]).await });
        ended_within(&task, LIMIT).await;
        flooding.abort();

        let most = HIGH_WATER.with(|most| most.get());
        assert!(most >= BACKLOG_LIMIT, "the forward was never full: it held {most} bytes");
        assert!(most <= BACKLOG_LIMIT + PACKET, "the forward held {most} bytes");
    }
}
