use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::store::model::{
    Group, IconRef, Profile, ProfileKey, ProfileStore, S3Profile, Settings, SftpProfile,
    VncProfile,
};
use crate::store::{is_icon_path, Store};
use crate::transfer::apply::remove_unreferenced_keys;

fn poisoned() -> AppError {
    AppError::Internal("store lock poisoned".into())
}

fn icon_in_use(data: &ProfileStore, path: &str) -> bool {
    let same = |icon: &IconRef| matches!(icon, IconRef::Custom { path: p } if p == path);
    data.groups.iter().any(|g| same(&g.icon))
        || data.profiles.iter().any(|p| same(&p.icon))
        || data.sftp_profiles.iter().any(|p| same(&p.icon))
        || data.s3_profiles.iter().any(|p| same(&p.icon))
        || data.vnc_profiles.iter().any(|p| same(&p.icon))
}

fn remove_custom_icon(app: &AppHandle, icon: &IconRef, data: &ProfileStore) {
    // Best-effort: a leftover icon file must never block deleting the profile.
    if let IconRef::Custom { path } = icon {
        if !is_icon_path(path) || icon_in_use(data, path) {
            return;
        }
        if let Ok(dir) = app.path().app_config_dir() {
            let _ = std::fs::remove_file(dir.join(path));
        }
    }
}

fn remove_unused_keys(app: &AppHandle, previous: &[ProfileKey], store: &Store) {
    if let Ok(dir) = app.path().app_config_dir() {
        remove_unreferenced_keys(&dir.join("keys"), previous, &store.snapshot());
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
    let mut s = store.lock().map_err(|_| poisoned())?;
    let icon = s.groups().into_iter().find(|g| g.id == id).map(|g| g.icon);
    s.delete_group(&id)?;
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon, &s.snapshot());
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    profile: Profile,
) -> AppResult<()> {
    let mut s = store.lock().map_err(|_| poisoned())?;
    let previous = s.profiles().into_iter().find(|p| p.id == profile.id);
    s.upsert_profile(profile)?;
    if let Some(previous) = previous {
        remove_unused_keys(&app, &previous.keys, &s);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn delete_profile(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    let mut s = store.lock().map_err(|_| poisoned())?;
    let deleted = s.profiles().into_iter().find(|p| p.id == id);
    s.delete_profile(&id)?;
    if let Some(deleted) = deleted {
        remove_custom_icon(&app, &deleted.icon, &s.snapshot());
        remove_unused_keys(&app, &deleted.keys, &s);
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
    let mut s = store.lock().map_err(|_| poisoned())?;
    let icon = s.s3_profiles().into_iter().find(|p| p.id == id).map(|p| p.icon);
    s.delete_s3_profile(&id)?;
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon, &s.snapshot());
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
pub fn upsert_sftp_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    profile: SftpProfile,
) -> AppResult<()> {
    let mut s = store.lock().map_err(|_| poisoned())?;
    let previous = s.sftp_profiles().into_iter().find(|p| p.id == profile.id);
    s.upsert_sftp_profile(profile)?;
    if let Some(previous) = previous {
        remove_unused_keys(&app, &previous.keys, &s);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn delete_sftp_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    id: String,
) -> AppResult<()> {
    let mut s = store.lock().map_err(|_| poisoned())?;
    let deleted = s.sftp_profiles().into_iter().find(|p| p.id == id);
    s.delete_sftp_profile(&id)?;
    if let Some(deleted) = deleted {
        remove_custom_icon(&app, &deleted.icon, &s.snapshot());
        remove_unused_keys(&app, &deleted.keys, &s);
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn list_vnc_profiles(store: State<'_, Mutex<Store>>) -> AppResult<Vec<VncProfile>> {
    Ok(store.lock().map_err(|_| poisoned())?.vnc_profiles())
}

#[tauri::command]
#[specta::specta]
pub fn upsert_vnc_profile(store: State<'_, Mutex<Store>>, profile: VncProfile) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_vnc_profile(profile)
}

#[tauri::command]
#[specta::specta]
pub fn delete_vnc_profile(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    id: String,
) -> AppResult<()> {
    let mut s = store.lock().map_err(|_| poisoned())?;
    let icon = s.vnc_profiles().into_iter().find(|p| p.id == id).map(|p| p.icon);
    s.delete_vnc_profile(&id)?;
    if let Some(icon) = icon {
        remove_custom_icon(&app, &icon, &s.snapshot());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_in_use_while_any_group_or_profile_points_at_it() {
        let path = |n: u8| format!("icons/6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c{n}.png");
        let custom = |n: u8| serde_json::json!({ "kind": "custom", "path": path(n) });
        let data: ProfileStore = serde_json::from_value(serde_json::json!({
            "version": 1,
            "groups": [{
                "id": "g", "name": "g", "parentId": null, "icon": custom(1), "order": 0
            }],
            "profiles": [{
                "id": "p", "name": "p", "groupId": null, "host": "h", "port": 22, "username": "u",
                "authMethod": "agent", "secretId": null, "icon": custom(2), "order": 0,
                "jumpHostId": null
            }],
            "sftpProfiles": [{
                "id": "f", "name": "f", "host": "h", "port": 22, "username": "u",
                "authMethod": "agent", "secretId": null, "icon": custom(3), "order": 0
            }],
            "s3Profiles": [{
                "id": "s", "name": "s", "endpoint": "e", "port": null, "region": "r",
                "useTls": true, "pathStyle": false, "accessKeyId": "AK", "secretId": null,
                "bucket": null, "icon": custom(4), "order": 0
            }],
            "vncProfiles": [{
                "id": "v", "name": "v", "host": "h", "port": 5900, "username": null,
                "secretId": null, "icon": custom(5), "order": 0
            }]
        }))
        .unwrap();
        for n in 1..=5 {
            assert!(icon_in_use(&data, &path(n)), "{n}");
        }
        assert!(!icon_in_use(&data, &path(6)));
        assert!(!icon_in_use(&ProfileStore::default(), &path(1)));
    }
}
