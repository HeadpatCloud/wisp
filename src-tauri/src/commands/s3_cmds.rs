use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;
use zeroize::Zeroizing;

use crate::commands::sftp_cmds::TransferProgress;
use crate::error::{AppError, AppResult};
use crate::s3::{self, S3Config};
use crate::sftp::SftpEntry;
use crate::store::Store;
use crate::vault::Vault;

type Conn = Arc<S3Config>;

#[derive(Default)]
pub struct S3Sessions(pub TokioMutex<HashMap<String, Conn>>);

// transfer_id -> cancel flag, so s3_cancel can flip an in-flight upload/download.
#[derive(Default)]
pub struct S3Transfers(pub TokioMutex<HashMap<String, Arc<AtomicBool>>>);

fn poisoned(what: &str) -> AppError {
    AppError::Internal(format!("{what} lock poisoned"))
}

async fn conn_for(sessions: &State<'_, S3Sessions>, id: &str) -> AppResult<Conn> {
    sessions
        .0
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("s3 {id}")))
}

#[tauri::command]
#[specta::specta]
pub async fn s3_connect(
    store: State<'_, StdMutex<Store>>,
    vault: State<'_, StdMutex<Vault>>,
    sessions: State<'_, S3Sessions>,
    profile_id: String,
) -> AppResult<String> {
    // Resolve the profile and its vault secret, then build the client - no await while the
    // store/vault locks are held.
    let config = {
        let profile = store
            .lock()
            .map_err(|_| poisoned("store"))?
            .s3_profiles()
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| AppError::NotFound(format!("s3 profile {profile_id}")))?;
        let secret = match &profile.secret_id {
            Some(id) => {
                let bytes = vault.lock().map_err(|_| poisoned("vault"))?.get_secret(id)?;
                Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned())
            }
            None => Zeroizing::new(String::new()),
        };
        s3::build_config(
            &profile.endpoint,
            profile.port,
            &profile.region,
            profile.use_tls,
            profile.path_style,
            &profile.access_key_id,
            &secret,
        )?
    };
    let id = uuid::Uuid::new_v4().to_string();
    sessions.0.lock().await.insert(id.clone(), Arc::new(config));
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn s3_list_buckets(
    sessions: State<'_, S3Sessions>,
    session_id: String,
) -> AppResult<Vec<SftpEntry>> {
    let conn = conn_for(&sessions, &session_id).await?;
    s3::list_buckets(&conn).await
}

#[tauri::command]
#[specta::specta]
pub async fn s3_list(
    sessions: State<'_, S3Sessions>,
    session_id: String,
    bucket: String,
    prefix: String,
) -> AppResult<Vec<SftpEntry>> {
    let conn = conn_for(&sessions, &session_id).await?;
    s3::list_objects(&conn, &bucket, &prefix).await
}

#[tauri::command]
#[specta::specta]
pub async fn s3_upload(
    sessions: State<'_, S3Sessions>,
    transfers: State<'_, S3Transfers>,
    session_id: String,
    bucket: String,
    key: String,
    local_path: String,
    transfer_id: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.0.lock().await.insert(transfer_id.clone(), cancel.clone());
    let r = s3::upload(&conn, &bucket, &key, &local_path, &cancel, move |transferred, total| {
        let _ = on_progress.send(TransferProgress { transferred, total });
    })
    .await;
    transfers.0.lock().await.remove(&transfer_id);
    r
}

#[tauri::command]
#[specta::specta]
pub async fn s3_download(
    sessions: State<'_, S3Sessions>,
    transfers: State<'_, S3Transfers>,
    session_id: String,
    bucket: String,
    key: String,
    local_path: String,
    is_dir: bool,
    size: f64,
    transfer_id: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.0.lock().await.insert(transfer_id.clone(), cancel.clone());
    let r = s3::download(&conn, &bucket, &key, &local_path, is_dir, size as u64, &cancel, move |transferred, total| {
        let _ = on_progress.send(TransferProgress { transferred, total });
    })
    .await;
    transfers.0.lock().await.remove(&transfer_id);
    r
}

#[tauri::command]
#[specta::specta]
pub async fn s3_cancel(transfers: State<'_, S3Transfers>, transfer_id: String) -> AppResult<()> {
    if let Some(flag) = transfers.0.lock().await.get(&transfer_id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn s3_delete(
    sessions: State<'_, S3Sessions>,
    session_id: String,
    bucket: String,
    key: String,
    is_dir: bool,
) -> AppResult<()> {
    // Guard against an empty key reaching delete (which would target a whole bucket); the
    // frontend blocks this too, but don't rely on the renderer alone.
    if key.is_empty() || key == "/" {
        return Err(AppError::Internal("s3: refusing to delete an empty key".into()));
    }
    let conn = conn_for(&sessions, &session_id).await?;
    if is_dir {
        s3::delete_prefix(&conn, &bucket, &key).await
    } else {
        s3::delete_object(&conn, &bucket, &key).await
    }
}

#[tauri::command]
#[specta::specta]
pub async fn s3_rename(
    sessions: State<'_, S3Sessions>,
    session_id: String,
    bucket: String,
    from: String,
    to: String,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    s3::rename(&conn, &bucket, &from, &to).await
}

#[tauri::command]
#[specta::specta]
pub async fn s3_mkdir(
    sessions: State<'_, S3Sessions>,
    session_id: String,
    bucket: String,
    prefix: String,
) -> AppResult<()> {
    let conn = conn_for(&sessions, &session_id).await?;
    s3::create_folder(&conn, &bucket, &prefix).await
}

#[tauri::command]
#[specta::specta]
pub async fn s3_disconnect(sessions: State<'_, S3Sessions>, session_id: String) -> AppResult<()> {
    sessions.0.lock().await.remove(&session_id);
    Ok(())
}
