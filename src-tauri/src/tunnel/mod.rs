pub mod socks;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
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

fn emit(app: &AppHandle, status: TunnelStatus) {
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
        let (lr, lw) = local.into_split();
        let (rr, rw) = tokio::io::split(channel.into_stream());
        let up_task = tokio::spawn(pump(lr, rw, up));
        let _ = pump(rr, lw, down).await;
        up_task.abort();
    })
    .abort_handle()
}

pub fn run_local(
    app: AppHandle,
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

pub fn run_dynamic(
    app: AppHandle,
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
                                    let (lr, lw) = local.into_split();
                                    let (rr, rw) = tokio::io::split(channel.into_stream());
                                    let up_task = tokio::spawn(pump(lr, rw, up));
                                    let _ = pump(rr, lw, down).await;
                                    up_task.abort();
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
pub async fn run_remote(
    app: AppHandle,
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
