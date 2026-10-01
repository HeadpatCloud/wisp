use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::Store;
use crate::transfer::bundle::{self, Opened, Payload};
use crate::transfer::{
    apply, export, plan, ApplySummary, ExportOptions, ExportSelection, ExportSummary, ImportReview,
    ItemDecision, ItemProblem, LocalEnv, ReadOutcome,
};
use crate::vault::Vault;

// Decoded bundles (possibly with secrets) wait here between review and apply, so nothing
// sensitive ever crosses into the webview.
#[derive(Default)]
pub struct PendingImports(pub Mutex<HashMap<String, Payload>>);

fn poisoned() -> AppError {
    AppError::Internal("lock poisoned".into())
}

fn config_dir(app: &AppHandle) -> AppResult<PathBuf> {
    app.path().app_config_dir().map_err(|e| AppError::Io(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub fn transfer_export(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    vault: State<'_, Mutex<Vault>>,
    selection: ExportSelection,
    options: ExportOptions,
    password: Option<String>,
    path: String,
) -> AppResult<ExportSummary> {
    let password = password.map(Zeroizing::new);
    let password = if options.include_secrets || options.include_keys {
        match password {
            Some(p) if !p.is_empty() => Some(p),
            _ => {
                return Err(AppError::Import(
                    "set an export password to include passwords or key files".into(),
                ))
            }
        }
    } else {
        None
    };
    let dir = config_dir(&app)?;
    let (payload, warnings) = {
        let s = store.lock().map_err(|_| poisoned())?;
        let v = vault.lock().map_err(|_| poisoned())?;
        export::build(
            &s.snapshot(),
            &LocalEnv { vault: &v, config_dir: &dir },
            &selection,
            &options,
        )
    };
    let bytes = bundle::write(&payload, password.as_deref().map(String::as_str))?;
    std::fs::write(&path, bytes).map_err(|e| AppError::Io(format!("{path}: {e}")))?;
    Ok(ExportSummary {
        profiles: (payload.profiles.len()
            + payload.sftp_profiles.len()
            + payload.s3_profiles.len()) as u32,
        secrets: payload.secrets.len() as u32,
        key_files: payload.key_files.len() as u32,
        warnings,
    })
}

#[tauri::command]
#[specta::specta]
pub fn transfer_read(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    vault: State<'_, Mutex<Vault>>,
    pending: State<'_, PendingImports>,
    path: String,
    password: Option<String>,
) -> AppResult<ReadOutcome> {
    let password = password.map(Zeroizing::new);
    let bytes =
        Zeroizing::new(std::fs::read(&path).map_err(|e| AppError::Io(format!("{path}: {e}")))?);
    let payload = match bundle::read(&bytes, password.as_deref().map(String::as_str))? {
        Opened::NeedsPassword => return Ok(ReadOutcome::NeedsPassword),
        Opened::Payload(p) => p,
    };
    let dir = config_dir(&app)?;
    let planned = {
        let s = store.lock().map_err(|_| poisoned())?;
        let v = vault.lock().map_err(|_| poisoned())?;
        plan::plan(&s.snapshot(), &payload, &LocalEnv { vault: &v, config_dir: &dir })
    };
    let review_id = uuid::Uuid::new_v4().to_string();
    pending.0.lock().map_err(|_| poisoned())?.insert(review_id.clone(), payload);
    Ok(ReadOutcome::Review {
        review: ImportReview { review_id, items: planned.items, unchanged: planned.unchanged },
    })
}

#[tauri::command]
#[specta::specta]
pub fn transfer_validate(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    vault: State<'_, Mutex<Vault>>,
    pending: State<'_, PendingImports>,
    review_id: String,
    decisions: Vec<ItemDecision>,
) -> AppResult<Vec<ItemProblem>> {
    let dir = config_dir(&app)?;
    let pending = pending.0.lock().map_err(|_| poisoned())?;
    let payload = pending
        .get(&review_id)
        .ok_or_else(|| AppError::NotFound(format!("import {review_id}")))?;
    let s = store.lock().map_err(|_| poisoned())?;
    let v = vault.lock().map_err(|_| poisoned())?;
    let snapshot = s.snapshot();
    let env = LocalEnv { vault: &v, config_dir: &dir };
    let planned = plan::plan(&snapshot, payload, &env);
    Ok(apply::stage(&snapshot, payload, &planned, &decisions, &env).err().unwrap_or_default())
}

#[tauri::command]
#[specta::specta]
pub fn transfer_apply(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    vault: State<'_, Mutex<Vault>>,
    pending: State<'_, PendingImports>,
    review_id: String,
    decisions: Vec<ItemDecision>,
) -> AppResult<ApplySummary> {
    let dir = config_dir(&app)?;
    let mut pending = pending.0.lock().map_err(|_| poisoned())?;
    let payload = pending
        .get(&review_id)
        .ok_or_else(|| AppError::NotFound(format!("import {review_id}")))?;
    let mut s = store.lock().map_err(|_| poisoned())?;
    let mut v = vault.lock().map_err(|_| poisoned())?;
    let snapshot = s.snapshot();
    let staged = {
        let env = LocalEnv { vault: &v, config_dir: &dir };
        let planned = plan::plan(&snapshot, payload, &env);
        apply::stage(&snapshot, payload, &planned, &decisions, &env)
            .map_err(|p| AppError::Import(format!("{} item(s) still need attention", p.len())))?
    };
    let summary = apply::execute(&mut s, &mut v, &dir.join("keys"), staged)?;
    pending.remove(&review_id);
    Ok(summary)
}

#[tauri::command]
#[specta::specta]
pub fn transfer_discard(pending: State<'_, PendingImports>, review_id: String) -> AppResult<()> {
    pending.0.lock().map_err(|_| poisoned())?.remove(&review_id);
    Ok(())
}
