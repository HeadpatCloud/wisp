pub mod socks;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelWriteHalf};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::AbortHandle;

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

// Carries one forwarded connection. A far side that is done sending (EOF) may still wait for
// its answer, so that only ends its own direction; a CLOSE, the end of the session or an
// error on either side ends both.
pub async fn relay(
    channel: Channel<Msg>,
    local: tokio::net::TcpStream,
    up: Arc<AtomicU64>,
    down: Arc<AtomicU64>,
) {
    let (lr, mut lw) = local.into_split();
    let (mut far, near) = channel.split();
    let writer = near.make_writer();
    let _close = CloseOnDrop(Some(near));
    let (sent, received) = (AtomicBool::new(false), AtomicBool::new(false));

    let sending = async {
        let ended = pump(lr, writer, up).await.is_ok();
        sent.store(ended, Ordering::Relaxed);
        ended
    };
    let receiving = async {
        loop {
            match far.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    if lw.write_all(&data).await.is_err() {
                        return;
                    }
                    down.fetch_add(data.len() as u64, Ordering::Relaxed);
                }
                Some(ChannelMsg::Eof) => {
                    received.store(true, Ordering::Relaxed);
                    if lw.shutdown().await.is_err() || sent.load(Ordering::Relaxed) {
                        return;
                    }
                }
                Some(ChannelMsg::Close) | None => return,
                Some(_) => {}
            }
        }
    };
    tokio::pin!(receiving);
    tokio::select! {
        _ = &mut receiving => {}
        ended = sending => {
            if ended && !received.load(Ordering::Relaxed) {
                receiving.await;
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
        relay(channel, local, up, down).await;
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
                                    relay(channel, local, up, down).await;
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
    use std::time::Instant;

    use tauri::test::MockRuntime;
    use tokio::net::TcpStream;
    use tokio::time::{sleep, timeout};

    use super::*;
    use crate::ssh::client::testing::session;

    const LIMIT: Duration = Duration::from_secs(10);

    // One connection through a forward: the program that connected to the local port, and the
    // channel as the server holds it for the service behind it.
    struct Forward {
        client: TcpStream,
        service: Channel<russh::server::Msg>,
        task: AbortHandle,
        _handle: Arc<SshHandle>,
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
        Forward { client, service, task, _handle: handle, _dynamic: None }
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
        Forward { client, service, task, _handle: handle, _dynamic: Some((app, tunnel)) }
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
}
