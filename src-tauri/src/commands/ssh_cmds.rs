use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_specta::Event;
use tokio::sync::{mpsc, Mutex as TokioMutex};
use zeroize::Zeroizing;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::error::{AppError, AppResult};
use crate::ssh::client::{self, SshHandle};
use crate::ssh::jump;
use crate::ssh::known_hosts::KnownHosts;
use crate::ssh::session::{open_pty, run_session};
use crate::store::hosts::SshTarget;
use crate::store::model::{AuthMethod, ProfileKey};
use crate::store::Store;
use crate::vault::Vault;

pub struct PtyIo {
    pub input_tx: mpsc::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u32, u32)>,
}

pub struct Session {
    pub handle: Arc<SshHandle>,
    // held only to keep bastion connections alive for the session; dropped on disconnect
    #[allow(dead_code)]
    pub bastions: Vec<SshHandle>,
    pub remote_forwards: crate::ssh::client::RemoteForwards,
    // None for a background connection that only carries tunnels.
    pub pty: Option<PtyIo>,
    // The terminal's task, or the task watching a background connection.
    pub abort: tokio::task::AbortHandle,
}

#[derive(Default)]
pub struct Sessions(pub TokioMutex<HashMap<String, Session>>);

pub struct KnownHostsState(pub Arc<StdMutex<KnownHosts>>);

#[derive(Clone, Serialize, Deserialize, Type, Event)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SshStatus {
    Connected {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    Disconnected {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<u32>,
    },
}

pub(crate) fn secret_string(
    vault: &StdMutex<Vault>,
    secret_id: &str,
) -> AppResult<Zeroizing<String>> {
    let v = vault.lock().map_err(|_| AppError::Internal("vault lock poisoned".into()))?;
    let bytes = v.get_secret(secret_id)?; // Zeroizing<Vec<u8>>
    let text = std::str::from_utf8(&bytes).map_err(|_| AppError::Vault("secret not utf-8".into()))?;
    Ok(Zeroizing::new(text.to_string()))
}

fn secret_for(
    vault: &StdMutex<Vault>,
    target: &SshTarget,
) -> AppResult<Option<Zeroizing<String>>> {
    match &target.secret_id {
        Some(id) => Ok(Some(secret_string(vault, id)?)),
        None => Ok(None),
    }
}

pub(crate) struct ResolvedKey {
    pub path: String,
    pub passphrase: Option<Zeroizing<String>>,
}

// Pulls each key's passphrase out of the vault up front, so authenticate() needs no vault access.
pub(crate) fn resolve_keys(
    vault: &StdMutex<Vault>,
    keys: &[ProfileKey],
) -> AppResult<Vec<ResolvedKey>> {
    keys.iter()
        .map(|k| {
            Ok(ResolvedKey {
                path: k.path.clone(),
                passphrase: match &k.secret_id {
                    Some(id) => Some(secret_string(vault, id)?),
                    None => None,
                },
            })
        })
        .collect()
}

pub(crate) async fn connect_via_chain(
    store: &StdMutex<Store>,
    vault: &StdMutex<Vault>,
    known: &KnownHostsState,
    target_id: &str,
) -> AppResult<(SshHandle, Vec<SshHandle>, crate::ssh::client::RemoteForwards)> {
    let hosts = store.lock().map_err(|_| AppError::Internal("store lock poisoned".into()))?.hosts();
    let chain = jump::resolve_jump_chain(|id| hosts.ssh_target(id), target_id)?;

    let root = &chain[0];
    let root_forwards = client::new_forwards();
    let mut prev =
        client::connect(&root.host, root.port, known.0.clone(), root_forwards.clone()).await?;
    authenticate(
        &mut prev,
        &root.username,
        root.auth_method,
        &resolve_keys(vault, &root.keys)?,
        secret_for(vault, root)?,
    )
    .await?;

    let mut bastions: Vec<SshHandle> = Vec::new();
    let mut prev_forwards = root_forwards;
    for hop in &chain[1..] {
        let channel = prev
            .channel_open_direct_tcpip(hop.host.clone(), hop.port as u32, "127.0.0.1", 0)
            .await?;
        bastions.push(prev); // keep the previous hop alive so its channel stays open
        let hop_forwards = client::new_forwards();
        let mut next = client::connect_over(
            channel.into_stream(),
            &hop.host,
            hop.port,
            known.0.clone(),
            hop_forwards.clone(),
        )
        .await?;
        authenticate(
            &mut next,
            &hop.username,
            hop.auth_method,
            &resolve_keys(vault, &hop.keys)?,
            secret_for(vault, hop)?,
        )
        .await?;
        prev = next;
        prev_forwards = hop_forwards;
    }
    Ok((prev, bastions, prev_forwards))
}

pub(crate) async fn connect_adhoc(
    known: &KnownHostsState,
    host: &str,
    port: u16,
    username: &str,
    auth_method: AuthMethod,
    keys: &[ResolvedKey],
    secret: Option<Zeroizing<String>>,
) -> AppResult<SshHandle> {
    let mut handle = client::connect(host, port, known.0.clone(), client::new_forwards()).await?;
    authenticate(&mut handle, username, auth_method, keys, secret).await?;
    Ok(handle)
}

async fn authenticate(
    handle: &mut SshHandle,
    username: &str,
    auth_method: AuthMethod,
    keys: &[ResolvedKey],
    secret: Option<Zeroizing<String>>,
) -> AppResult<()> {
    match auth_method {
        AuthMethod::Password => {
            let pw = secret.ok_or_else(|| AppError::Auth("no password stored".into()))?;
            match client::auth_password(handle, username, &pw).await {
                Ok(()) => Ok(()),
                // Some servers only offer keyboard-interactive for password login. Report
                // both attempts, or the fallback's error hides the plain password rejection.
                Err(AppError::Auth(_)) => {
                    match client::auth_keyboard_interactive(handle, username, &pw).await {
                        Err(AppError::Auth(_)) => Err(AppError::Auth(
                            "password rejected (tried password and keyboard-interactive). \
                             Check the password, or the server may require a key or MFA."
                                .into(),
                        )),
                        other => other,
                    }
                }
                Err(e) => Err(e),
            }
        }
        AuthMethod::Key => {
            if keys.is_empty() {
                return Err(AppError::Auth("no private key configured".into()));
            }
            let mut last: Option<AppError> = None;
            // A passphrase problem is the user's to fix, so it outranks a plain rejection in
            // the final message - but it must not stop the loop, because an unreadable key
            // file reports the same way and the next key may well work.
            let mut passphrase_err: Option<AppError> = None;
            for key in keys {
                match client::auth_key(
                    handle,
                    username,
                    &key.path,
                    key.passphrase.as_ref().map(|s| s.as_str()),
                )
                .await
                {
                    Ok(()) => return Ok(()),
                    Err(e @ (AppError::PassphraseRequired(_) | AppError::WrongPassphrase)) => {
                        passphrase_err.get_or_insert(e);
                    }
                    Err(e) => last = Some(e),
                }
            }
            Err(match (passphrase_err, last) {
                (Some(e), _) => e,
                (None, Some(e)) if keys.len() == 1 => e,
                (None, Some(e)) => {
                    AppError::Auth(format!("all {} keys rejected (last: {e})", keys.len()))
                }
                (None, None) => AppError::Auth("no private key configured".into()),
            })
        }
        AuthMethod::Agent => client::auth_agent(handle, username).await,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_connect(
    app: AppHandle,
    store: State<'_, StdMutex<Store>>,
    vault: State<'_, StdMutex<Vault>>,
    known: State<'_, KnownHostsState>,
    sessions: State<'_, Sessions>,
    profile_id: String,
    cols: u32,
    rows: u32,
    on_output: Channel<String>,
) -> AppResult<String> {
    let (handle, bastions, remote_forwards) =
        connect_via_chain(&store, &vault, &known, &profile_id).await?;
    start_session(&app, &sessions, handle, bastions, remote_forwards, cols, rows, on_output).await
}

#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AdhocTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub keys: Vec<ProfileKey>,
    pub secret_id: Option<String>,
}

// Connect once: nothing is saved, and the secret ids belong to the tab.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn ssh_connect_adhoc(
    app: AppHandle,
    vault: State<'_, StdMutex<Vault>>,
    known: State<'_, KnownHostsState>,
    sessions: State<'_, Sessions>,
    target: AdhocTarget,
    cols: u32,
    rows: u32,
    on_output: Channel<String>,
) -> AppResult<String> {
    let secret = match &target.secret_id {
        Some(id) => Some(secret_string(&vault, id)?),
        None => None,
    };
    let keys = resolve_keys(&vault, &target.keys)?;
    let forwards = client::new_forwards();
    let mut handle = client::connect(&target.host, target.port, known.0.clone(), forwards.clone()).await?;
    authenticate(&mut handle, &target.username, target.auth_method, &keys, secret).await?;
    start_session(&app, &sessions, handle, vec![], forwards, cols, rows, on_output).await
}

#[allow(clippy::too_many_arguments)]
async fn start_session(
    app: &AppHandle,
    sessions: &Sessions,
    handle: SshHandle,
    bastions: Vec<SshHandle>,
    remote_forwards: crate::ssh::client::RemoteForwards,
    cols: u32,
    rows: u32,
    on_output: Channel<String>,
) -> AppResult<String> {
    let channel = open_pty(&handle, cols, rows).await?;
    let handle = Arc::new(handle);

    let session_id = uuid::Uuid::new_v4().to_string();
    let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>(64);
    let (resize_tx, resize_rx) = mpsc::channel::<(u32, u32)>(8);

    let app_task = app.clone();
    let sid = session_id.clone();
    let keep_alive = handle.clone();
    let task = tokio::spawn(async move {
        let _keep = keep_alive; // hold an Arc<Handle> so the connection lives as long as the PTY
        let out = on_output;
        let exit = run_session(channel, input_rx, resize_rx, move |bytes| {
            let _ = out.send(STANDARD.encode(&bytes));
        })
        .await;
        close_session(&app_task, &sid, Some(CLOSED), exit).await;
    });

    sessions.0.lock().await.insert(
        session_id.clone(),
        Session {
            handle,
            bastions,
            remote_forwards,
            pty: Some(PtyIo { input_tx, resize_tx }),
            abort: task.abort_handle(),
        },
    );

    let _ = SshStatus::Connected { session_id: session_id.clone() }.emit(app);
    Ok(session_id)
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_write(
    sessions: State<'_, Sessions>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    let tx = {
        let map = sessions.0.lock().await;
        map.get(&session_id).and_then(|s| s.pty.as_ref()).map(|p| p.input_tx.clone())
    };
    let tx = tx.ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
    tx.send(data).await.map_err(|_| AppError::Ssh("session closed".into()))
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_resize(
    sessions: State<'_, Sessions>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let tx = {
        let map = sessions.0.lock().await;
        map.get(&session_id).and_then(|s| s.pty.as_ref()).map(|p| p.resize_tx.clone())
    };
    let tx = tx.ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
    tx.send((cols, rows)).await.map_err(|_| AppError::Ssh("session closed".into()))
}

#[tauri::command]
#[specta::specta]
pub fn known_hosts_list(known: State<'_, KnownHostsState>) -> AppResult<Vec<crate::ssh::known_hosts::KnownHostEntry>> {
    let kh = known.0.lock().map_err(|_| AppError::Internal("known_hosts lock poisoned".into()))?;
    Ok(kh.list())
}

#[tauri::command]
#[specta::specta]
pub fn known_hosts_forget(known: State<'_, KnownHostsState>, key: String) -> AppResult<()> {
    let mut kh = known.0.lock().map_err(|_| AppError::Internal("known_hosts lock poisoned".into()))?;
    kh.forget(&key)
}

#[tauri::command]
#[specta::specta]
pub async fn trust_host_key(
    known: State<'_, KnownHostsState>,
    host: String,
    port: u16,
    fingerprint: String,
) -> AppResult<()> {
    let mut kh = known
        .0
        .lock()
        .map_err(|_| AppError::Internal("known_hosts lock poisoned".into()))?;
    kh.record(&host, port, &fingerprint)
}

const CLOSED: &str = "The SSH connection closed.";

// A connection without a terminal, for tunnels started where no tab of it is open.
#[tauri::command]
#[specta::specta]
pub async fn ssh_open_background(
    app: AppHandle,
    store: State<'_, StdMutex<Store>>,
    vault: State<'_, StdMutex<Vault>>,
    known: State<'_, KnownHostsState>,
    sessions: State<'_, Sessions>,
    connection_id: String,
) -> AppResult<String> {
    let (handle, bastions, remote_forwards) =
        connect_via_chain(&store, &vault, &known, &connection_id).await?;
    let handle = Arc::new(handle);
    let session_id = uuid::Uuid::new_v4().to_string();
    let (watched, app_task, sid) = (handle.clone(), app.clone(), session_id.clone());
    // Without a terminal nothing else notices the server going away.
    let task = tokio::spawn(async move {
        while !watched.is_closed() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        close_session(&app_task, &sid, Some(CLOSED), None).await;
    });
    sessions.0.lock().await.insert(
        session_id.clone(),
        Session { handle, bastions, remote_forwards, pty: None, abort: task.abort_handle() },
    );
    let _ = SshStatus::Connected { session_id: session_id.clone() }.emit(&app);
    Ok(session_id)
}

pub(crate) async fn is_background(app: &AppHandle, session_id: &str) -> bool {
    app.state::<Sessions>().0.lock().await.get(session_id).is_some_and(|s| s.pty.is_none())
}

// Everything a session holds goes with it: its tunnels (each reported stopped, remote forwards
// cancelled), its cached SFTP channel and the connection. Every way a session ends comes here,
// so this is where the frontend hears that it did.
pub(crate) async fn close_session<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    reason: Option<&str>,
    exit_code: Option<u32>,
) {
    use crate::commands::tunnel_cmds::{end_tunnel, Tunnels};
    app.state::<crate::commands::sftp_cmds::SftpSessions>().0.lock().await.remove(session_id);
    let tunnels = app.state::<Tunnels>();
    let ended: Vec<_> = {
        let mut running = tunnels.0.lock().await;
        let ids: Vec<String> = running
            .iter()
            .filter(|(_, h)| h.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().filter_map(|id| running.remove(&id).map(|h| (id, h))).collect()
    };
    for (id, h) in ended {
        end_tunnel(h).await;
        crate::tunnel::emit_stopped(app, id, reason.map(str::to_string));
    }
    // Last, with no await after it: called from the session's own task, the abort ends that task.
    let sessions = app.state::<Sessions>();
    let session = sessions.0.lock().await.remove(session_id);
    if let Some(session) = session {
        let _ = SshStatus::Disconnected { session_id: session_id.to_string(), exit_code }.emit(app);
        session.abort.abort();
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ssh_disconnect(app: AppHandle, session_id: String) -> AppResult<()> {
    close_session(&app, &session_id, Some(CLOSED), None).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc as std_mpsc;

    use tauri::test::MockRuntime;
    use tauri::Listener;

    use super::*;
    use crate::commands::sftp_cmds::SftpSessions;
    use crate::commands::tunnel_cmds::Tunnels;
    use crate::tunnel::TunnelStatus;

    #[tokio::test]
    async fn closing_a_session_says_it_disconnected() {
        let app = tauri::test::mock_app();
        tauri_specta::Builder::<MockRuntime>::new()
            .events(tauri_specta::collect_events![SshStatus, TunnelStatus])
            .mount_events(&app);
        app.manage(Sessions(Default::default()));
        app.manage(Tunnels(Default::default()));
        app.manage(SftpSessions(Default::default()));
        let (handle, remote_forwards, _server, _opened) = client::testing::session().await;
        let watcher = tokio::spawn(std::future::pending::<()>());
        let session = Session {
            handle: Arc::new(handle),
            bastions: Vec::new(),
            remote_forwards,
            pty: None,
            abort: watcher.abort_handle(),
        };
        app.state::<Sessions>().0.lock().await.insert("bg".into(), session);
        let (tx, rx) = std_mpsc::channel();
        app.listen_any("ssh-status", move |e| tx.send(e.payload().to_string()).unwrap());

        close_session(app.handle(), "bg", None, None).await;
        let sent: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(sent, serde_json::json!({ "state": "disconnected", "sessionId": "bg", "exitCode": null }));
        assert!(app.state::<Sessions>().0.lock().await.is_empty());

        // A session already gone says nothing more.
        close_session(app.handle(), "bg", None, None).await;
        assert!(rx.try_recv().is_err());
    }
}
