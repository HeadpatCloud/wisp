use std::collections::HashMap;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_specta::Event;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::Mutex as TokioMutex;

use crate::error::{AppError, AppResult};
use crate::vnc::{self, client_cut_text, fb_update_request, key_event, pointer_event};

#[derive(Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct VncClipboard {
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrameUpdate {
    Raw { x: u16, y: u16, w: u16, h: u16, data: String },
    Copy { x: u16, y: u16, w: u16, h: u16, src_x: u16, src_y: u16 },
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VncOpened {
    pub id: String,
    pub width: u16,
    pub height: u16,
}

pub struct VncHandle {
    writer: Arc<TokioMutex<OwnedWriteHalf>>,
    abort: tokio::task::AbortHandle,
}

#[derive(Default)]
pub struct VncSessions(pub TokioMutex<HashMap<String, VncHandle>>);

fn io(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("vnc: {e}"))
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_open(
    app: AppHandle,
    vncs: State<'_, VncSessions>,
    host: String,
    port: u16,
    password: String,
    on_frame: Channel<FrameUpdate>,
) -> AppResult<VncOpened> {
    let init = vnc::connect(&host, port, &password).await?;
    let (mut reader, width, height) = (init.reader, init.width, init.height);
    let writer = Arc::new(TokioMutex::new(init.writer));
    writer.lock().await.write_all(&fb_update_request(false, 0, 0, width, height)).await.map_err(io)?;

    let id = uuid::Uuid::new_v4().to_string();
    let loop_writer = writer.clone();
    let task = tokio::spawn(async move {
        loop {
            match vnc::read_message(&mut reader).await {
                Ok(vnc::ServerMsg::Frame(ops)) => {
                    for op in ops {
                        let frame = match op {
                            vnc::DrawOp::Raw { x, y, w, h, rgba } => {
                                FrameUpdate::Raw { x, y, w, h, data: STANDARD.encode(&rgba) }
                            }
                            vnc::DrawOp::Copy { x, y, w, h, src_x, src_y } => {
                                FrameUpdate::Copy { x, y, w, h, src_x, src_y }
                            }
                        };
                        let _ = on_frame.send(frame);
                    }
                    let req = fb_update_request(true, 0, 0, width, height);
                    if loop_writer.lock().await.write_all(&req).await.is_err() {
                        break;
                    }
                }
                Ok(vnc::ServerMsg::Clipboard(text)) => {
                    let _ = VncClipboard { text }.emit(&app);
                }
                Ok(vnc::ServerMsg::Ignored) => {}
                Err(_) => break,
            }
        }
    });

    vncs.0.lock().await.insert(id.clone(), VncHandle { writer, abort: task.abort_handle() });
    Ok(VncOpened { id, width, height })
}

async fn writer_for(
    vncs: &State<'_, VncSessions>,
    id: &str,
) -> AppResult<Arc<TokioMutex<OwnedWriteHalf>>> {
    vncs.0
        .lock()
        .await
        .get(id)
        .map(|h| h.writer.clone())
        .ok_or_else(|| AppError::NotFound(format!("vnc {id}")))
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
    let w = writer_for(&vncs, &id).await?;
    let r = w.lock().await.write_all(&pointer_event(buttons, x, y)).await;
    r.map_err(io)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_key(
    vncs: State<'_, VncSessions>,
    id: String,
    down: bool,
    keysym: u32,
) -> AppResult<()> {
    let w = writer_for(&vncs, &id).await?;
    let r = w.lock().await.write_all(&key_event(down, keysym)).await;
    r.map_err(io)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_cut_text(vncs: State<'_, VncSessions>, id: String, text: String) -> AppResult<()> {
    let w = writer_for(&vncs, &id).await?;
    let r = w.lock().await.write_all(&client_cut_text(&text)).await;
    r.map_err(io)
}

#[tauri::command]
#[specta::specta]
pub async fn vnc_close(vncs: State<'_, VncSessions>, id: String) -> AppResult<()> {
    if let Some(h) = vncs.0.lock().await.remove(&id) {
        h.abort.abort();
    }
    Ok(())
}
