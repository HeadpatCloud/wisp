use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::store::model::{Group, IconRef, Profile, S3Profile, Settings, SftpProfile};
use crate::store::Store;

fn poisoned() -> AppError {
    AppError::Internal("store lock poisoned".into())
}

fn remove_custom_icon(app: &AppHandle, icon: &IconRef) {
    // Best-effort: a leftover icon file must never block deleting the profile.
    if let IconRef::Custom { path } = icon {
        if path.contains("..") {
            return;
        }
        if let Ok(dir) = app.path().app_config_dir() {
            let _ = std::fs::remove_file(dir.join(path));
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn list_groups(store: State<'_, Mutex<Store>>) -> AppResult<Vec<Group>> {
    Ok(store.lock().map_err(|_| poisoned())?.groups())
}

#[tauri::command]
#[specta::specta]
pub fn list_profiles(store: State<'_, Mutex<Store>>) -> AppResult<Vec<Profile>> {
    Ok(store.lock().map_err(|_| poisoned())?.profiles())
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(store: State<'_, Mutex<Store>>) -> AppResult<Settings> {
    Ok(store.lock().map_err(|_| poisoned())?.settings())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_group(store: State<'_, Mutex<Store>>, group: Group) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_group(group)
}

#[tauri::command]
#[specta::specta]
pub fn delete_group(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    let icon = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let icon = s.groups().into_iter().find(|g| g.id == id).map(|g| g.icon);
        s.delete_group(&id)?;
        icon
    };
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_profile(store: State<'_, Mutex<Store>>, profile: Profile) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_profile(profile)
}

#[tauri::command]
#[specta::specta]
pub fn delete_profile(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    let icon = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let icon = s.profiles().into_iter().find(|p| p.id == id).map(|p| p.icon);
        s.delete_profile(&id)?;
        icon
    };
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_settings(store: State<'_, Mutex<Store>>, settings: Settings) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.set_settings(settings)
}

#[tauri::command]
#[specta::specta]
pub fn list_s3_profiles(store: State<'_, Mutex<Store>>) -> AppResult<Vec<S3Profile>> {
    Ok(store.lock().map_err(|_| poisoned())?.s3_profiles())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_s3_profile(store: State<'_, Mutex<Store>>, profile: S3Profile) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_s3_profile(profile)
}

#[tauri::command]
#[specta::specta]
pub fn delete_s3_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    id: String,
) -> AppResult<()> {
    let icon = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let icon = s.s3_profiles().into_iter().find(|p| p.id == id).map(|p| p.icon);
        s.delete_s3_profile(&id)?;
        icon
    };
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn list_sftp_profiles(store: State<'_, Mutex<Store>>) -> AppResult<Vec<SftpProfile>> {
    Ok(store.lock().map_err(|_| poisoned())?.sftp_profiles())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_sftp_profile(store: State<'_, Mutex<Store>>, profile: SftpProfile) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_sftp_profile(profile)
}

#[tauri::command]
#[specta::specta]
pub fn delete_sftp_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    id: String,
) -> AppResult<()> {
    let icon = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let icon = s.sftp_profiles().into_iter().find(|p| p.id == id).map(|p| p.icon);
        s.delete_sftp_profile(&id)?;
        icon
    };
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon);
    }
    Ok(())
}
