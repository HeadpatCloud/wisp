use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;
use zeroize::Zeroizing;

use crate::commands::sftp_cmds::TransferProgress;
use crate::error::{AppError, AppResult};
use crate::ftp::{self, FtpStream};
use crate::sftp::SftpEntry;

// suppaftp is a synchronous client; each connection lives behind a blocking mutex and
// every operation runs on a blocking thread so it never stalls the tokio runtime.
type Conn = Arc<StdMutex<FtpStream>>;

#[derive(Default)]
pub struct FtpSessions(pub TokioMutex<HashMap<String, Conn>>);

// spawn_blocking work can't be aborted mid-flight, so transfers cancel cooperatively:
// ftp_cancel flips this flag and the transfer loop bails at the next chunk boundary.
#[derive(Default)]
pub struct FtpTransfers(pub TokioMutex<HashMap<String, Arc<AtomicBool>>>);

async fn conn_for(sessions: &State<'_, FtpSessions>, id: &str) -> AppResult<Conn> {
    sessions
        .0
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("ftp {id}")))
}

async fn blocking<T, F>(conn: Conn, op: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce(&mut FtpStream) -> AppResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut s = conn.lock().map_err(|_| AppError::Internal("ftp lock poisoned".into()))?;
        op(&mut s)
    })
    .await
    .map_err(|e| AppError::Internal(format!("ftp task: {e}")))?
}

async fn run_cancellable<F>(
    transfers: &State<'_, FtpTransfers>,
    conn: Conn,
    transfer_id: String,
    op: F,
) -> AppResult<()>
where
    F: FnOnce(&mut FtpStream, Arc<AtomicBool>) -> AppResult<()> + Send + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.0.lock().await.insert(transfer_id.clone(), cancel.clone());
    let result = blocking(conn, move |s| op(s, cancel)).await;
    transfers.0.lock().await.remove(&transfer_id);
    result
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_connect(
    sessions: State<'_, FtpSessions>,
    host: String,
    port: u16,
    username: String,
    password: String,
    secure: bool,
    allow_invalid_cert: bool,
    ignore_hostname: bool,
) -> AppResult<String> {
    let password = Zeroizing::new(password);
    let stream = tokio::task::spawn_blocking(move || {
        ftp::connect(&host, port, &username, &password, secure, allow_invalid_cert, ignore_hostname)
    })
    .await
    .map_err(|e| AppError::Internal(format!("ftp task: {e}")))??;
    let id = uuid::Uuid::new_v4().to_string();
    sessions.0.lock().await.insert(id.clone(), Arc::new(StdMutex::new(stream)));
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_list(
    sessions: State<'_, FtpSessions>,
    session_id: String,
    path: String,
) -> AppResult<Vec<SftpEntry>> {
    let conn = conn_for(&sessions, &session_id).await?;
    blocking(conn, move |s| ftp::list(s, &path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_exists(
    sessions: State<'_, FtpSessions>,
    session_id: String,
    path: String,
) -> AppResult<bool> {
    let conn = conn_for(&sessions, &session_id).await?;
    blocking(conn, move |s| Ok(ftp::exists(s, &path))).await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_mkdir(
    sessions: State<'_, FtpSessions>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    blocking(conn, move |s| ftp::mkdir(s, &path)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_rename(
    sessions: State<'_, FtpSessions>,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    blocking(conn, move |s| ftp::rename(s, &from, &to)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_remove(
    sessions: State<'_, FtpSessions>,
    session_id: String,
    path: String,
    is_dir: bool,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    blocking(conn, move |s| ftp::remove(s, &path, is_dir)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_upload(
    sessions: State<'_, FtpSessions>,
    transfers: State<'_, FtpTransfers>,
    session_id: String,
    transfer_id: String,
    local_path: String,
    remote_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    run_cancellable(&transfers, conn, transfer_id, move |s, cancel| {
        ftp::upload(s, &local_path, &remote_path, &cancel, |transferred, total| {
            let _ = on_progress.send(TransferProgress { transferred, total });
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_download(
    sessions: State<'_, FtpSessions>,
    transfers: State<'_, FtpTransfers>,
    session_id: String,
    transfer_id: String,
    remote_path: String,
    local_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    run_cancellable(&transfers, conn, transfer_id, move |s, cancel| {
        ftp::download(s, &remote_path, &local_path, &cancel, |transferred, total| {
            let _ = on_progress.send(TransferProgress { transferred, total });
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_cancel(
    transfers: State<'_, FtpTransfers>,
    transfer_id: String,
) -> AppResult<()> {
    if let Some(flag) = transfers.0.lock().await.get(&transfer_id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn ftp_disconnect(
    sessions: State<'_, FtpSessions>,
    session_id: String,
) -> AppResult<()> {
    // Bind first so the sessions guard drops here, not across the QUIT round-trip below.
    let conn = sessions.0.lock().await.remove(&session_id);
    if let Some(conn) = conn {
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(mut s) = conn.lock() {
                let _ = s.quit();
            }
        })
        .await;
    }
    Ok(())
}
