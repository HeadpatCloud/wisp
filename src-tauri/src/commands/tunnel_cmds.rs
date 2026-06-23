use std::collections::HashMap;

use tauri::{AppHandle, State};
use tokio::sync::Mutex as TokioMutex;

use crate::commands::ssh_cmds::Sessions;
use crate::error::{AppError, AppResult};
use crate::store::model::{Tunnel, TunnelKind};
use crate::tunnel::{self, TunnelHandle};

#[derive(Default)]
pub struct Tunnels(pub TokioMutex<HashMap<String, TunnelHandle>>);

#[tauri::command]
#[specta::specta]
pub async fn tunnel_start(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    tunnels: State<'_, Tunnels>,
    session_id: String,
    tunnel: Tunnel,
) -> AppResult<()> {
    let (handle, remote_forwards) = {
        let map = sessions.0.lock().await;
        let s = map.get(&session_id).ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
        (s.handle.clone(), s.remote_forwards.clone())
    };
    let bind = format!("{}:{}", tunnel.bind_host, tunnel.bind_port);
    let started = match tunnel.kind {
        TunnelKind::Local => {
            let host = tunnel.target_host.ok_or_else(|| AppError::Tunnel("local tunnel needs a target host".into()))?;
            let port = tunnel.target_port.ok_or_else(|| AppError::Tunnel("local tunnel needs a target port".into()))?;
            tunnel::run_local(app, tunnel.id.clone(), session_id, handle, bind, host, port)?
        }
        TunnelKind::Dynamic => tunnel::run_dynamic(app, tunnel.id.clone(), session_id, handle, bind)?,
        TunnelKind::Remote => {
            let host = tunnel
                .target_host
                .ok_or_else(|| AppError::Tunnel("remote tunnel needs a target host".into()))?;
            let port = tunnel
                .target_port
                .ok_or_else(|| AppError::Tunnel("remote tunnel needs a target port".into()))?;
            tunnel::run_remote(
                app,
                tunnel.id.clone(),
                session_id,
                handle,
                remote_forwards,
                tunnel.bind_host,
                tunnel.bind_port,
                host,
                port,
            )
            .await?
        }
    };
    tunnels.0.lock().await.insert(tunnel.id, started);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn tunnel_stop(tunnels: State<'_, Tunnels>, tunnel_id: String) -> AppResult<()> {
    if let Some(h) = tunnels.0.lock().await.remove(&tunnel_id) {
        h.abort.abort();
        if let Ok(conns) = h.conns.lock() {
            for c in conns.iter() {
                c.abort();
            }
        }
        if let Some(rc) = h.remote {
            let _ = rc.handle.cancel_tcpip_forward(rc.bind_host.clone(), rc.bound_port).await;
            if let Ok(mut map) = rc.registry.lock() {
                map.remove(&(rc.bind_host, rc.bound_port));
            }
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn tunnel_list(tunnels: State<'_, Tunnels>) -> AppResult<Vec<String>> {
    Ok(tunnels.0.lock().await.keys().cloned().collect())
}
