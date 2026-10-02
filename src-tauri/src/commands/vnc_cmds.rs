use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;
use zeroize::Zeroizing;

use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::{FrameBytes, FrameOp};
use crate::vnc::decode::ENCODINGS;
use crate::vnc::handshake::Login;
use crate::vnc::session::Session;

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VncOpened {
    pub id: String,
    pub width: u16,
    pub height: u16,
    pub name: String,
}

#[derive(Default)]
pub struct VncSessions(pub TokioMutex<HashMap<String, Arc<Session>>>);

#[tauri::command]
#[specta::specta]
pub async fn vnc_open(
    vault: State<'_, std::sync::Mutex<crate::vault::Vault>>,
    known: State<'_, KnownHostsState>,
    vncs: State<'_, VncSessions>,
    host: String,
    port: u16,
    username: Option<String>,
    secret_id: Option<String>,
    on_frame: Channel<FrameBytes>,
) -> AppResult<VncOpened> {
    // The password lives in the vault; the caller only ever holds a reference to it.
    let password = match &secret_id {
        Some(id) => crate::commands::ssh_cmds::secret_string(&vault, id)?,
        None => Zeroizing::new(String::new()),
    };
    let login = Login { username: username.as_deref().unwrap_or(""), password: &password };
    // A send only fails once the tab is gone, and the tab's `vnc_close` ends the session.
    let sink = move |op: FrameOp| {
        let _ = on_frame.send(op.encode());
    };
    let session = Session::connect(&host, port, &login, &known, &ENCODINGS, sink).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let opened = VncOpened {
        id: id.clone(),
        width: session.width,
        height: session.height,
        name: session.name.clone(),
    };
    vncs.0.lock().await.insert(id, Arc::new(session));
    Ok(opened)
}

// The lock is released before the session is used: its input waits while the queue to a stalled
// server is full, and that must not hold up `vnc_close` or the other sessions.
async fn session_for(vncs: &State<'_, VncSessions>, id: &str) -> Option<Arc<Session>> {
    vncs.0.lock().await.get(id).cloned()
}

// The view still sends key releases and a close after the server has ended a session, so input
// for a session that is over, or no longer in the map, is dropped without an error.
fn unless_ended(sent: AppResult<()>) -> AppResult<()> {
    match sent {
        Err(AppError::NotFound(_)) => Ok(()),
        sent => sent,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_pointer(
    vncs: State<'_, VncSessions>,
    id: String,
    buttons: u8,
    x: u16,
    y: u16,
) -> AppResult<()> {
    let Some(session) = session_for(&vncs, &id).await else { return Ok(()) };
    unless_ended(session.pointer(buttons, x, y).await)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_key(
    vncs: State<'_, VncSessions>,
    id: String,
    down: bool,
    keysym: u32,
) -> AppResult<()> {
    let Some(session) = session_for(&vncs, &id).await else { return Ok(()) };
    unless_ended(session.key(down, keysym).await)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_cut_text(vncs: State<'_, VncSessions>, id: String, text: String) -> AppResult<()> {
    let Some(session) = session_for(&vncs, &id).await else { return Ok(()) };
    unless_ended(session.clipboard(&text).await)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_ack(vncs: State<'_, VncSessions>, id: String) -> AppResult<()> {
    if let Some(session) = session_for(&vncs, &id).await {
        session.ack();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_close(vncs: State<'_, VncSessions>, id: String) -> AppResult<()> {
    let session = vncs.0.lock().await.remove(&id);
    if let Some(session) = session {
        session.close().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vnc::session::closed;

    #[test]
    fn only_the_end_of_a_session_is_no_error() {
        assert!(unless_ended(Ok(())).is_ok());
        assert!(unless_ended(Err(closed())).is_ok());
        let failed = unless_ended(Err(AppError::Io("broken pipe".into())));
        assert!(matches!(failed, Err(AppError::Io(_))));
    }
}
