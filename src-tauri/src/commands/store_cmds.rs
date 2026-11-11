use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use crate::commands::tunnel_cmds;
use crate::error::{AppError, AppResult};
use crate::store::model::Settings;
use crate::store::themes::ThemesFile;
use crate::store::hosts::{changed_tunnels, ConnectionMove, Folder, FolderMove, Host, HostMove, HostsFile, SavedLogin, SshKey, Tag};
use crate::store::migrate::MigrationReport;
use crate::store::{is_icon_path, named_by_profiles, Store};
use crate::transfer::apply::remove_unreferenced_key_files;
use crate::vault::Vault;

fn poisoned() -> AppError {
    AppError::Internal("store lock poisoned".into())
}

pub(crate) fn secrets_to_delete(before: &HostsFile, after: &HostsFile) -> Vec<String> {
    before.referenced_secrets().difference(&after.referenced_secrets()).cloned().collect()
}

// Only what the change dropped is deleted, never a sweep: a vault entry hosts.json never
// referred to may still be 0.5.0's, or a tab's own.
pub(crate) fn delete_secrets(vault: &Mutex<Vault>, before: &HostsFile, after: &HostsFile) {
    let Ok(mut vault) = vault.lock() else { return };
    for id in secrets_to_delete(before, after) {
        // Best-effort: the change is saved already, and a leftover secret is only unused.
        let _ = vault.delete_secret(&id);
    }
}

pub(crate) fn after_change(app: &AppHandle, before: &HostsFile, after: &HostsFile) {
    let changed = changed_tunnels(before, after);
    if !changed.is_empty() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            for id in changed {
                tunnel_cmds::stop(&app, id, Some("Its settings changed. Start it again to use them.")).await;
            }
        });
    }
    delete_secrets(app.state::<Mutex<Vault>>().inner(), before, after);
    let Ok(dir) = app.path().app_config_dir() else { return };
    remove_dropped_files(&dir, before, after);
}

// The icons and key files the change dropped, except what 0.5.0's profiles.json still names.
pub(crate) fn remove_dropped_files(dir: &Path, before: &HostsFile, after: &HostsFile) {
    let Some((old_icons, old_keys)) = named_by_profiles(dir) else { return };
    for icon in before.custom_icons().difference(&after.custom_icons()) {
        if is_icon_path(icon) && !old_icons.contains(icon) {
            let _ = std::fs::remove_file(dir.join(icon));
        }
    }
    let mut kept = after.key_paths();
    kept.extend(old_keys);
    remove_unreferenced_key_files(&dir.join("keys"), &before.key_paths(), &kept);
}

#[tauri::command]
#[specta::specta]
pub fn get_hosts(store: State<'_, Mutex<Store>>) -> AppResult<HostsFile> {
    Ok(store.lock().map_err(|_| poisoned())?.hosts())
}

#[tauri::command]
#[specta::specta]
pub fn take_migration_report(store: State<'_, Mutex<Store>>) -> AppResult<Option<MigrationReport>> {
    Ok(store.lock().map_err(|_| poisoned())?.take_migration_report())
}

pub(crate) fn change<T>(app: &AppHandle, store: &Mutex<Store>, f: impl FnOnce(&mut Store) -> AppResult<T>) -> AppResult<T> {
    let (before, after, out) = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let before = s.hosts();
        let out = f(&mut s)?;
        (before, s.hosts(), out)
    };
    after_change(app, &before, &after);
    Ok(out)
}

// Pending deletes left from the last run go at start, once the vault is there to clean up after them.
pub(crate) fn purge_left_over_deletes(app: &AppHandle) -> AppResult<()> {
    change(app, app.state::<Mutex<Store>>().inner(), |s| s.purge_deletes(None))
}

#[tauri::command]
#[specta::specta]
pub fn upsert_host(
    app: AppHandle,
    store: State<'_, Mutex<Store>>,
    host: Host,
    moves: Vec<ConnectionMove>,
) -> AppResult<Vec<String>> {
    change(&app, &store, |s| s.upsert_host(host, moves))
}

#[tauri::command]
#[specta::specta]
pub fn delete_host(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<String> {
    change(&app, &store, |s| s.delete_host(&id))
}

#[tauri::command]
#[specta::specta]
pub fn undo_delete(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    change(&app, &store, |s| s.undo_delete(&id))
}

#[tauri::command]
#[specta::specta]
pub fn purge_deletes(app: AppHandle, store: State<'_, Mutex<Store>>, ids: Vec<String>) -> AppResult<()> {
    change(&app, &store, |s| s.purge_deletes(Some(&ids)))
}

#[tauri::command]
#[specta::specta]
pub fn upsert_key(app: AppHandle, store: State<'_, Mutex<Store>>, key: SshKey) -> AppResult<()> {
    change(&app, &store, |s| s.upsert_key(key))
}

#[tauri::command]
#[specta::specta]
pub fn upsert_tag(store: State<'_, Mutex<Store>>, tag: Tag) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.upsert_tag(tag)
}

#[tauri::command]
#[specta::specta]
pub fn delete_tag(store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.delete_tag(&id)
}

// The three go through change(), so a replaced password or passphrase leaves the vault.
#[tauri::command]
#[specta::specta]
pub fn upsert_login(app: AppHandle, store: State<'_, Mutex<Store>>, login: SavedLogin) -> AppResult<()> {
    change(&app, &store, |s| s.upsert_login(login))
}

#[tauri::command]
#[specta::specta]
pub fn delete_login(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    change(&app, &store, |s| s.delete_login(&id))
}

#[tauri::command]
#[specta::specta]
pub fn delete_key(app: AppHandle, store: State<'_, Mutex<Store>>, id: String) -> AppResult<()> {
    change(&app, &store, |s| s.delete_key(&id))
}

#[tauri::command]
#[specta::specta]
pub fn upsert_folder(app: AppHandle, store: State<'_, Mutex<Store>>, folder: Folder) -> AppResult<()> {
    let (before, after) = {
        let mut s = store.lock().map_err(|_| poisoned())?;
        let before = s.hosts();
        s.upsert_folder(folder)?;
        (before, s.hosts())
    };
    after_change(&app, &before, &after);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn move_hosts(store: State<'_, Mutex<Store>>, moves: Vec<HostMove>) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.move_hosts(&moves)
}

#[tauri::command]
#[specta::specta]
pub fn move_folders(store: State<'_, Mutex<Store>>, moves: Vec<FolderMove>) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.move_folders(&moves)
}

#[tauri::command]
#[specta::specta]
pub fn get_settings(store: State<'_, Mutex<Store>>) -> AppResult<Settings> {
    Ok(store.lock().map_err(|_| poisoned())?.settings())
}

#[tauri::command]
#[specta::specta]
pub fn set_settings(store: State<'_, Mutex<Store>>, settings: Settings) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.set_settings(settings)
}

#[tauri::command]
#[specta::specta]
pub fn get_themes(store: State<'_, Mutex<Store>>) -> AppResult<ThemesFile> {
    Ok(store.lock().map_err(|_| poisoned())?.themes())
}

#[tauri::command]
#[specta::specta]
pub fn set_themes(store: State<'_, Mutex<Store>>, themes: ThemesFile) -> AppResult<()> {
    store.lock().map_err(|_| poisoned())?.set_themes(themes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hosts::fixtures::{file, host, own, ssh, vnc};
    use crate::store::hosts::{Login, OwnLogin, SshKey};
    use crate::store::model::AuthMethod;
    use zeroize::Zeroizing;

    #[test]
    fn files_profiles_json_names_are_kept() {
        use crate::store::model::IconRef;
        let dir = tempfile::tempdir().unwrap();
        let (old_icon, new_icon) = (
            "icons/0a5f6c2e-3b1d-4c8e-9f7a-1b2c3d4e5f60.png",
            "icons/1b6a7d3f-4c2e-4d9f-8a6b-2c3d4e5f6071.png",
        );
        let keys = dir.path().join("keys");
        let old_key = keys.join("2c7b8e4a-5d3f-4eaf-9b7c-3d4e5f607182-id_old");
        let new_key = keys.join("3d8c9f5b-6e4a-4fb0-8c8d-4e5f60718293-id_new");
        let make = || {
            std::fs::create_dir_all(dir.path().join("icons")).unwrap();
            std::fs::create_dir_all(&keys).unwrap();
            for f in [dir.path().join(old_icon), dir.path().join(new_icon), old_key.clone(), new_key.clone()] {
                std::fs::write(f, b"x").unwrap();
            }
        };
        make();
        // What 0.5.0 still works from after the update.
        let profiles = serde_json::json!({
            "version": 1,
            "groups": [],
            "profiles": [{
                "id": "p", "name": "nas", "groupId": null, "host": "nas.lan", "port": 22,
                "username": "u", "authMethod": "key", "keyPath": null,
                "keys": [{ "path": old_key.to_string_lossy(), "secretId": null }],
                "secretId": null, "icon": { "kind": "custom", "path": old_icon }, "order": 0,
                "jumpHostId": null, "tunnels": [], "appearance": null
            }]
        });
        std::fs::write(dir.path().join("profiles.json"), profiles.to_string()).unwrap();

        let mut before = file(vec![
            host("a", "a", "a", vec![]),
            host("b", "b", "b", vec![]),
        ]);
        before.hosts[0].icon = IconRef::Custom { path: old_icon.into() };
        before.hosts[1].icon = IconRef::Custom { path: new_icon.into() };
        for (id, path) in [("k1", &old_key), ("k2", &new_key)] {
            before.keys.push(SshKey {
                id: id.into(),
                path: path.to_string_lossy().into_owned(),
                secret_id: None,
                extra: Default::default(),
            });
        }
        let after = file(vec![]);

        remove_dropped_files(dir.path(), &before, &after);
        assert!(dir.path().join(old_icon).exists());
        assert!(old_key.exists());
        assert!(!dir.path().join(new_icon).exists());
        assert!(!new_key.exists());

        // A profiles.json that can't be read might name anything, so nothing goes.
        make();
        std::fs::write(dir.path().join("profiles.json"), "{bad").unwrap();
        remove_dropped_files(dir.path(), &before, &after);
        assert!(dir.path().join(new_icon).exists());
        assert!(new_key.exists());
    }

    #[test]
    fn cleanup_deletes_only_what_the_change_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Mutex::new(
            Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([7u8; 32])).unwrap(),
        );
        let (a, b, c) = {
            let mut v = vault.lock().unwrap();
            (v.set_secret(b"a").unwrap(), v.set_secret(b"b").unwrap(), v.set_secret(b"c").unwrap())
        };
        let before = file(vec![host("h", "h", "h", vec![vnc("x", Some(&a)), vnc("y", Some(&b))])]);
        let after = file(vec![host("h", "h", "h", vec![vnc("y", Some(&b))])]);
        assert_eq!(secrets_to_delete(&before, &after), [a.clone()]);

        delete_secrets(&vault, &before, &after);
        let v = vault.lock().unwrap();
        assert!(!v.has_secret(&a));
        assert!(v.has_secret(&b));
        assert!(v.has_secret(&c));
    }

    #[test]
    fn editing_one_login_keeps_a_shared_key_passphrase() {
        let key_login = |user: &str| {
            Login::Own(OwnLogin {
                username: user.into(),
                auth_method: AuthMethod::Key,
                key_ids: vec!["k1".into()],
                secret_id: None,
                extra: Default::default(),
            })
        };
        let mut before = file(vec![host(
            "h",
            "h",
            "h",
            vec![ssh("a", 22, key_login("a")), ssh("b", 22, key_login("b"))],
        )]);
        before.keys.push(SshKey {
            id: "k1".into(),
            path: "/k".into(),
            secret_id: Some("p".into()),
            extra: Default::default(),
        });
        let mut after = before.clone();
        after.hosts[0].connections[1] = ssh("b", 22, own("b", Some("q")));
        assert!(secrets_to_delete(&before, &after).is_empty());
    }

}
