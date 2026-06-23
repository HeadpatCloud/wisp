use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;

use crate::commands::ssh_cmds::{self, KnownHostsState, Sessions};
use crate::error::{AppError, AppResult};
use crate::sftp::{self, transfer, SftpEntry};
use crate::ssh::client::SshHandle;
use crate::store::Store;
use crate::vault::Vault;

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    #[specta(type = specta_typescript::Number)]
    pub transferred: u64,
    #[specta(type = specta_typescript::Number)]
    pub total: u64,
}

#[derive(Default)]
pub struct SftpSessions(pub TokioMutex<HashMap<String, Arc<SftpSession>>>);

#[derive(Default)]
pub struct Transfers(pub TokioMutex<HashMap<String, tokio::task::AbortHandle>>);

// Holds the SSH connection alive for a standalone (no-terminal) SFTP session; dropping
// it closes the connection. The terminal SFTP path keeps the handle in `Sessions` instead.
pub struct SftpConn {
    #[allow(dead_code)]
    handle: Arc<SshHandle>,
    #[allow(dead_code)]
    bastions: Vec<SshHandle>,
}

#[derive(Default)]
pub struct SftpConns(pub TokioMutex<HashMap<String, SftpConn>>);

// Connect SSH for a saved profile and open an SFTP session without a PTY. The session is
// pre-cached in SftpSessions so the existing sftp_* commands resolve it by id.
#[tauri::command]
#[specta::specta]
pub async fn sftp_connect(
    store: State<'_, StdMutex<Store>>,
    vault: State<'_, StdMutex<Vault>>,
    known: State<'_, KnownHostsState>,
    sftps: State<'_, SftpSessions>,
    conns: State<'_, SftpConns>,
    profile_id: String,
) -> AppResult<String> {
    let (handle, bastions, _forwards) =
        ssh_cmds::connect_via_chain(&store, &vault, &known, &profile_id).await?;
    let handle = Arc::new(handle);
    let sftp = Arc::new(sftp::open_sftp(&handle).await?);
    let id = uuid::Uuid::new_v4().to_string();
    sftps.0.lock().await.insert(id.clone(), sftp);
    conns.0.lock().await.insert(id.clone(), SftpConn { handle, bastions });
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_disconnect(
    sftps: State<'_, SftpSessions>,
    conns: State<'_, SftpConns>,
    session_id: String,
) -> AppResult<()> {
    sftps.0.lock().await.remove(&session_id);
    conns.0.lock().await.remove(&session_id);
    Ok(())
}

// Get the cached SFTP session for a connection, or open + cache one on the session's Arc<Handle>.
async fn sftp_for(
    sessions: &State<'_, Sessions>,
    sftps: &State<'_, SftpSessions>,
    session_id: &str,
) -> AppResult<Arc<SftpSession>> {
    if let Some(s) = sftps.0.lock().await.get(session_id) {
        return Ok(s.clone());
    }
    let handle = {
        let map = sessions.0.lock().await;
        map.get(session_id)
            .map(|s| s.handle.clone())
            .ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?
    };
    let sftp = Arc::new(sftp::open_sftp(&handle).await?);
    sftps.0.lock().await.insert(session_id.to_string(), sftp.clone());
    Ok(sftp)
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_list(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    session_id: String,
    path: String,
) -> AppResult<Vec<SftpEntry>> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    sftp::list(&sftp, &path).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_stat(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    session_id: String,
    path: String,
) -> AppResult<SftpEntry> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    sftp::stat(&sftp, &path).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_mkdir(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    sftp::mkdir(&sftp, &path).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_rename(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    sftp::rename(&sftp, &from, &to).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_remove(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    session_id: String,
    path: String,
    is_dir: bool,
) -> AppResult<()> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    sftp::remove(&sftp, &path, is_dir).await
}

async fn run_tracked(
    transfers: &State<'_, Transfers>,
    transfer_id: String,
    fut: impl std::future::Future<Output = AppResult<()>> + Send + 'static,
) -> AppResult<()> {
    let task = tokio::spawn(fut);
    transfers.0.lock().await.insert(transfer_id.clone(), task.abort_handle());
    let res = task.await;
    transfers.0.lock().await.remove(&transfer_id);
    match res {
        Ok(r) => r,
        Err(_) => Err(AppError::Sftp("transfer cancelled".into())),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_upload(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    transfers: State<'_, Transfers>,
    session_id: String,
    transfer_id: String,
    local_path: String,
    remote_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    run_tracked(&transfers, transfer_id, async move {
        transfer::upload(&sftp, &local_path, &remote_path, |transferred, total| {
            let _ = on_progress.send(TransferProgress { transferred, total });
        })
        .await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_download(
    sessions: State<'_, Sessions>,
    sftps: State<'_, SftpSessions>,
    transfers: State<'_, Transfers>,
    session_id: String,
    transfer_id: String,
    remote_path: String,
    local_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let sftp = sftp_for(&sessions, &sftps, &session_id).await?;
    run_tracked(&transfers, transfer_id, async move {
        transfer::download(&sftp, &remote_path, &local_path, |transferred, total| {
            let _ = on_progress.send(TransferProgress { transferred, total });
        })
        .await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_cancel(transfers: State<'_, Transfers>, transfer_id: String) -> AppResult<()> {
    if let Some(h) = transfers.0.lock().await.remove(&transfer_id) {
        h.abort();
    }
    Ok(())
}
