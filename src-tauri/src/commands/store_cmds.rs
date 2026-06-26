use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::store::model::{Group, IconRef, Profile, S3Profile, Settings};
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileExport {
    pub version: u32,
    pub groups: Vec<Group>,
    pub profiles: Vec<Profile>,
}

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

// Secrets stay in the vault and are never written here; an export carries only the
// profile's secretId reference, which resolves again on the same machine.
#[tauri::command]
#[specta::specta]
pub fn export_profiles(store: State<'_, Mutex<Store>>, path: String) -> AppResult<()> {
    let export = {
        let s = store.lock().map_err(|_| poisoned())?;
        ProfileExport { version: 1, groups: s.groups(), profiles: s.profiles() }
    };
    let json = serde_json::to_string_pretty(&export)?;
    std::fs::write(&path, json).map_err(|e| AppError::Io(format!("{path}: {e}")))?;
    Ok(())
}

// Merge an exported bundle by id (re-importing your own file is idempotent). Returns
// how many profiles were written.
#[tauri::command]
#[specta::specta]
pub fn import_profiles(store: State<'_, Mutex<Store>>, path: String) -> AppResult<u32> {
    let text = std::fs::read_to_string(&path).map_err(|e| AppError::Io(format!("{path}: {e}")))?;
    let export: ProfileExport = serde_json::from_str(&text)?;
    if export.version != 1 {
        return Err(AppError::Serde(format!("unsupported export version {}", export.version)));
    }
    let mut s = store.lock().map_err(|_| poisoned())?;
    for g in export.groups {
        s.upsert_group(g)?;
    }
    let mut count = 0;
    for p in export.profiles {
        s.upsert_profile(p)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::AuthMethod;

    fn sample_profile(id: &str, name: &str) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            group_id: None,
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
        }
    }

    #[test]
    fn profile_export_round_trips_json() {
        let export = ProfileExport {
            version: 1,
            groups: vec![Group {
                id: "g1".into(),
                name: "Prod".into(),
                parent_id: None,
                icon: IconRef::default(),
                order: 0,
            }],
            profiles: vec![sample_profile("p1", "web")],
        };
        let json = serde_json::to_string_pretty(&export).unwrap();
        assert!(json.contains("\"version\": 1"));
        let back: ProfileExport = serde_json::from_str(&json).unwrap();
        assert_eq!(export, back);
    }

    #[test]
    fn import_merges_bundle_into_a_fresh_store() {
        let src = tempfile::tempdir().unwrap();
        let mut store = Store::load(src.path().to_path_buf()).unwrap();
        store.upsert_profile(sample_profile("p1", "web")).unwrap();
        let export =
            ProfileExport { version: 1, groups: store.groups(), profiles: store.profiles() };
        let json = serde_json::to_string(&export).unwrap();

        let dst = tempfile::tempdir().unwrap();
        let mut store2 = Store::load(dst.path().to_path_buf()).unwrap();
        let parsed: ProfileExport = serde_json::from_str(&json).unwrap();
        for g in parsed.groups {
            store2.upsert_group(g).unwrap();
        }
        for p in parsed.profiles {
            store2.upsert_profile(p).unwrap();
        }
        assert_eq!(store2.profiles().len(), 1);
        assert_eq!(store2.profiles()[0].name, "web");
    }
}
