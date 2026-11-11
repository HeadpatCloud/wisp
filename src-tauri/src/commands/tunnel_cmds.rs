use std::collections::HashMap;

use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex as TokioMutex;

use crate::commands::ssh_cmds::{self, Sessions};
use crate::error::{AppError, AppResult};
use crate::store::model::{Tunnel, TunnelKind};
use crate::tunnel::{self, TunnelHandle};

#[derive(Default)]
pub struct Tunnels(pub TokioMutex<HashMap<String, TunnelHandle>>);

// A second start of one tunnel would fight the first for its port.
fn refuse_running<V>(running: &HashMap<String, V>, tunnel_id: &str) -> AppResult<()> {
    if running.contains_key(tunnel_id) {
        return Err(AppError::Invalid("This tunnel is already running.".into()));
    }
    Ok(())
}

// A background connection exists only for its tunnels, so the last one to stop takes it along.
fn last_on_session<'a>(mut remaining: impl Iterator<Item = &'a str>, session_id: &str) -> bool {
    !remaining.any(|s| s == session_id)
}

#[tauri::command]
#[specta::specta]
pub async fn tunnel_start(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    tunnels: State<'_, Tunnels>,
    session_id: String,
    tunnel: Tunnel,
) -> AppResult<()> {
    // Held for the whole start, so two starts of one tunnel can't both pass the check.
    let mut running = tunnels.0.lock().await;
    refuse_running(&running, &tunnel.id)?;
    let (handle, remote_forwards) = {
        let map = sessions.0.lock().await;
        let s = map.get(&session_id).ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
        (s.handle.clone(), s.remote_forwards.clone())
    };
    let bind = crate::net::authority(&tunnel.bind_host, tunnel.bind_port);
    let started = match tunnel.kind {
        TunnelKind::Local => {
            let host = tunnel.target_host.ok_or_else(|| AppError::Tunnel("local tunnel needs a target host".into()))?;
            let port = tunnel.target_port.ok_or_else(|| AppError::Tunnel("local tunnel needs a target port".into()))?;
            tunnel::run_local(app, tunnel.id.clone(), session_id, handle, bind, host, port).await?
        }
        TunnelKind::Dynamic => tunnel::run_dynamic(app, tunnel.id.clone(), session_id, handle, bind).await?,
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
    running.insert(tunnel.id, started);
    Ok(())
}

// Ends the listener, its open connections and, for a remote forward, the forward on the server.
pub(crate) async fn end_tunnel(h: TunnelHandle) {
    h.abort.abort();
    if let Ok(conns) = h.conns.lock() {
        for c in conns.iter() {
            c.abort();
        }
    }
    if let Some(rc) = h.remote {
        // Fails when the session is already gone, which ends the forward on its own.
        let _ = rc.handle.cancel_tcpip_forward(rc.bind_host.clone(), rc.bound_port).await;
        if let Ok(mut map) = rc.registry.lock() {
            map.remove(&(rc.bind_host, rc.bound_port));
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn tunnel_stop(app: AppHandle, tunnel_id: String) -> AppResult<()> {
    stop(&app, tunnel_id, None).await;
    Ok(())
}

// A tunnel that isn't running is left alone. The last one on a background connection closes it.
pub(crate) async fn stop(app: &AppHandle, tunnel_id: String, reason: Option<&str>) {
    let tunnels = app.state::<Tunnels>();
    let (ended, closes) = {
        let mut running = tunnels.0.lock().await;
        let Some(h) = running.remove(&tunnel_id) else { return };
        let closes = last_on_session(running.values().map(|t| t.session_id.as_str()), &h.session_id);
        (h, closes)
    };
    let session_id = ended.session_id.clone();
    end_tunnel(ended).await;
    tunnel::emit_stopped(app, tunnel_id, reason.map(str::to_string));
    if closes && ssh_cmds::is_background(app, &session_id).await {
        ssh_cmds::close_session(app, &session_id, None, None).await;
    }
}

#[tauri::command]
#[specta::specta]
pub async fn tunnel_list(tunnels: State<'_, Tunnels>) -> AppResult<Vec<String>> {
    Ok(tunnels.0.lock().await.keys().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_tunnel_is_not_started_twice() {
        let mut running = HashMap::new();
        assert!(refuse_running(&running, "t1").is_ok());
        running.insert("t1".to_string(), ());
        match refuse_running(&running, "t1") {
            Err(AppError::Invalid(m)) => assert_eq!(m, "This tunnel is already running."),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(refuse_running(&running, "t2").is_ok());
    }

    #[test]
    fn the_last_tunnel_closes_its_background_connection() {
        assert!(last_on_session(["other"].into_iter(), "bg"));
        assert!(last_on_session(std::iter::empty(), "bg"));
        assert!(!last_on_session(["other", "bg"].into_iter(), "bg"));
    }
}
