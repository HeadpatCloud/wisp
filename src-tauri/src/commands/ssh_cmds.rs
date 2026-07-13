use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
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
use crate::store::model::{AuthMethod, Profile};
use crate::store::Store;
use crate::vault::Vault;

pub struct Session {
    pub handle: Arc<SshHandle>,
    // held only to keep bastion connections alive for the session; dropped on disconnect
    #[allow(dead_code)]
    pub bastions: Vec<SshHandle>,
    pub remote_forwards: crate::ssh::client::RemoteForwards,
    pub input_tx: mpsc::Sender<Vec<u8>>,
    pub resize_tx: mpsc::Sender<(u32, u32)>,
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

fn secret_string(vault: &State<'_, StdMutex<Vault>>, secret_id: &str) -> AppResult<Zeroizing<String>> {
    let v = vault.lock().map_err(|_| AppError::Internal("vault lock poisoned".into()))?;
    let bytes = v.get_secret(secret_id)?; // Zeroizing<Vec<u8>>
    let text = std::str::from_utf8(&bytes).map_err(|_| AppError::Vault("secret not utf-8".into()))?;
    Ok(Zeroizing::new(text.to_string()))
}

fn secret_for(
    vault: &State<'_, StdMutex<Vault>>,
    profile: &Profile,
) -> AppResult<Option<Zeroizing<String>>> {
    match &profile.secret_id {
        Some(id) => Ok(Some(secret_string(vault, id)?)),
        None => Ok(None),
    }
}

pub(crate) async fn connect_via_chain(
    store: &State<'_, StdMutex<Store>>,
    vault: &State<'_, StdMutex<Vault>>,
    known: &State<'_, KnownHostsState>,
    target_id: &str,
) -> AppResult<(SshHandle, Vec<SshHandle>, crate::ssh::client::RemoteForwards)> {
    let profiles = {
        let store = store.lock().map_err(|_| AppError::Internal("store lock poisoned".into()))?;
        store.profiles()
    };
    let chain = jump::resolve_jump_chain(&profiles, target_id)?;

    let root = &chain[0];
    let root_forwards = client::new_forwards();
    let mut prev =
        client::connect(&root.host, root.port, known.0.clone(), root_forwards.clone()).await?;
    authenticate(
        &mut prev,
        &root.username,
        root.auth_method,
        root.key_path.as_deref(),
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
            hop.key_path.as_deref(),
            secret_for(vault, hop)?,
        )
        .await?;
        prev = next;
        prev_forwards = hop_forwards;
    }
    Ok((prev, bastions, prev_forwards))
}

pub(crate) async fn connect_adhoc(
    known: &State<'_, KnownHostsState>,
    host: &str,
    port: u16,
    username: &str,
    auth_method: AuthMethod,
    key_path: Option<&str>,
    secret: Option<Zeroizing<String>>,
) -> AppResult<SshHandle> {
    let mut handle = client::connect(host, port, known.0.clone(), client::new_forwards()).await?;
    authenticate(&mut handle, username, auth_method, key_path, secret).await?;
    Ok(handle)
}

async fn authenticate(
    handle: &mut SshHandle,
    username: &str,
    auth_method: AuthMethod,
    key_path: Option<&str>,
    secret: Option<Zeroizing<String>>,
) -> AppResult<()> {
    match auth_method {
        AuthMethod::Password => {
            let pw = secret.ok_or_else(|| AppError::Auth("no password stored".into()))?;
            match client::auth_password(handle, username, &pw).await {
                Ok(()) => Ok(()),
                // Some servers only offer keyboard-interactive for password login.
                Err(AppError::Auth(_)) => {
                    client::auth_keyboard_interactive(handle, username, &pw).await
                }
                Err(e) => Err(e),
            }
        }
        AuthMethod::Key => {
            let key_path = key_path.ok_or_else(|| AppError::Auth("no key path".into()))?;
            let passphrase = secret.as_ref().map(|s| s.as_str());
            client::auth_key(handle, username, key_path, passphrase).await
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
        let _ = SshStatus::Disconnected { session_id: sid, exit_code: exit }.emit(&app_task);
    });

    sessions
        .0
        .lock()
        .await
        .insert(session_id.clone(), Session { handle, bastions, remote_forwards, input_tx, resize_tx, abort: task.abort_handle() });

    let _ = SshStatus::Connected { session_id: session_id.clone() }.emit(&app);
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
        map.get(&session_id).map(|s| s.input_tx.clone())
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
        map.get(&session_id).map(|s| s.resize_tx.clone())
    };
    let tx = tx.ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
    tx.send((cols, rows)).await.map_err(|_| AppError::Ssh("session closed".into()))
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

#[tauri::command]
#[specta::specta]
pub async fn ssh_disconnect(
    sessions: State<'_, Sessions>,
    sftps: State<'_, crate::commands::sftp_cmds::SftpSessions>,
    tunnels: State<'_, crate::commands::tunnel_cmds::Tunnels>,
    session_id: String,
) -> AppResult<()> {
    sftps.0.lock().await.remove(&session_id);
    {
        let mut tmap = tunnels.0.lock().await;
        let ids: Vec<String> = tmap
            .iter()
            .filter(|(_, h)| h.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(h) = tmap.remove(&id) {
                h.abort.abort();
                if let Ok(conns) = h.conns.lock() {
                    for c in conns.iter() {
                        c.abort();
                    }
                }
            }
        }
    }
    if let Some(session) = sessions.0.lock().await.remove(&session_id) {
        session.abort.abort();
    }
    Ok(())
}
