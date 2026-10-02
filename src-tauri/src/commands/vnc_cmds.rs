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

#[derive(Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VncInput {
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    Clipboard { text: String },
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
    // A send fails only once the webview itself is gone, and then nobody is left to tell.
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

// The view still sends key releases and a close after the server has ended a session, so the
// rest of a batch for a session that is over is dropped without an error.
async fn send_events(session: &Session, events: Vec<VncInput>) -> AppResult<()> {
    for event in events {
        let sent = match event {
            VncInput::Key { down, keysym } => session.key(down, keysym).await,
            VncInput::Pointer { buttons, x, y } => session.pointer(buttons, x, y).await,
            VncInput::Clipboard { text } => session.clipboard(&text).await,
        };
        match sent {
            Err(AppError::NotFound(_)) => return Ok(()),
            sent => sent?,
        }
    }
    Ok(())
}

// One command for all input: commands run as tasks of their own, so only what is sent in one
// call is certain to arrive in the order it was made.
#[tauri::command]
#[specta::specta]
pub async fn vnc_input(
    vncs: State<'_, VncSessions>,
    id: String,
    events: Vec<VncInput>,
) -> AppResult<()> {
    let Some(session) = session_for(&vncs, &id).await else { return Ok(()) };
    send_events(&session, events).await
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
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::*;
    use crate::ssh::known_hosts::KnownHosts;
    use crate::vnc::testserver::{After, Script, Seen, Server};

    const WAIT: Duration = Duration::from_secs(5);

    // A session on a server without a login, and the operations it hands to the view.
    async fn open(then: After) -> (Server, Session, mpsc::UnboundedReceiver<FrameOp>) {
        let script = Script {
            version: "RFB 003.008\n",
            security: vec![1],
            password: None,
            size: (4, 2),
            name: "desk".into(),
            updates: Vec::new(),
            then,
        };
        let server = Server::start(script).await;
        let dir = tempfile::tempdir().unwrap();
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        let known = KnownHostsState(Arc::new(std::sync::Mutex::new(hosts)));
        let login = Login { username: "", password: "" };
        let (sink, ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let port = server.port;
        let connecting = Session::connect("127.0.0.1", port, &login, &known, &ENCODINGS, sink);
        let session = timeout(WAIT, connecting).await.expect("connect hung").unwrap();
        (server, session, ops)
    }

    fn batch() -> Vec<VncInput> {
        vec![
            VncInput::Key { down: true, keysym: 0xFFE1 },
            VncInput::Pointer { buttons: 1, x: 10, y: 20 },
            VncInput::Clipboard { text: "hi".into() },
            VncInput::Pointer { buttons: 0, x: 10, y: 20 },
            VncInput::Key { down: false, keysym: 0xFFE1 },
        ]
    }

    #[tokio::test]
    async fn events_reach_the_server_in_the_order_given() {
        let (mut server, session, _ops) = open(After::Hold).await;
        timeout(WAIT, send_events(&session, batch())).await.expect("input hung").unwrap();
        let log = server.wait(|log| log.len() >= 8).await;
        assert_eq!(
            log[3..],
            [
                Seen::Key { down: true, keysym: 0xFFE1 },
                Seen::Pointer { buttons: 1, x: 10, y: 20 },
                Seen::CutText(b"hi".to_vec()),
                Seen::Pointer { buttons: 0, x: 10, y: 20 },
                Seen::Key { down: false, keysym: 0xFFE1 },
            ],
        );
    }

    #[tokio::test]
    async fn a_batch_for_a_session_that_has_ended_is_no_error() {
        let (_server, session, mut ops) = open(After::Close).await;
        let ended = timeout(WAIT, ops.recv()).await.expect("the session did not end");
        assert!(matches!(ended, Some(FrameOp::Closed(_))));
        timeout(WAIT, send_events(&session, batch())).await.expect("input hung").unwrap();

        let (_server, session, _ops) = open(After::Hold).await;
        timeout(WAIT, session.close()).await.expect("close hung");
        timeout(WAIT, send_events(&session, batch())).await.expect("input hung").unwrap();
    }
}
