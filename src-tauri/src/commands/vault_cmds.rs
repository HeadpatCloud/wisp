use std::sync::Mutex;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::vault::model::VaultStatus;
use crate::vault::Vault;

fn poisoned() -> AppError {
    AppError::Internal("vault lock poisoned".into())
}

#[tauri::command]
#[specta::specta]
pub fn vault_status(vault: State<'_, Mutex<Vault>>) -> AppResult<VaultStatus> {
    Ok(vault.lock().map_err(|_| poisoned())?.status())
}

#[tauri::command]
#[specta::specta]
pub fn set_secret(vault: State<'_, Mutex<Vault>>, value: String) -> AppResult<String> {
    vault.lock().map_err(|_| poisoned())?.set_secret(value.as_bytes())
}

#[tauri::command]
#[specta::specta]
pub fn delete_secret(vault: State<'_, Mutex<Vault>>, id: String) -> AppResult<()> {
    vault.lock().map_err(|_| poisoned())?.delete_secret(&id)
}

#[tauri::command]
#[specta::specta]
pub fn has_secret(vault: State<'_, Mutex<Vault>>, id: String) -> AppResult<bool> {
    Ok(vault.lock().map_err(|_| poisoned())?.has_secret(&id))
}

#[tauri::command]
#[specta::specta]
pub fn vault_unlock(vault: State<'_, Mutex<Vault>>, password: String) -> AppResult<()> {
    vault.lock().map_err(|_| poisoned())?.unlock(&password)
}

#[tauri::command]
#[specta::specta]
pub fn vault_change_password(vault: State<'_, Mutex<Vault>>, password: String) -> AppResult<()> {
    vault.lock().map_err(|_| poisoned())?.change_master_password(&password)
}
