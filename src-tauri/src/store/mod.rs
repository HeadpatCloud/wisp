pub mod io;
pub mod model;

use std::path::PathBuf;

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use model::{
    AuthMethod, Group, Profile, ProfileKey, ProfileStore, S3Profile, SftpProfile, Settings,
    VncProfile,
};

pub struct Store {
    dir: PathBuf,
    data: ProfileStore,
    settings: Settings,
}

// Folds a pre-multi-key profile's single key_path into the keys list. Runs on every read and
// write path, not just load, so profiles arriving from an import or the ssh-config importer
// are normalized too. The secret *moves* rather than being copied: two owners of one vault
// entry means whichever side is edited first deletes the other's passphrase.
pub(crate) fn normalize_keys(p: &mut Profile) {
    let Some(path) = p.key_path.take() else { return };
    if !p.keys.is_empty() {
        return;
    }
    let secret_id = if p.auth_method == AuthMethod::Key { p.secret_id.take() } else { None };
    p.keys.push(ProfileKey { path, secret_id });
}

pub(crate) const ICON_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];

// A custom icon is always `icons/<uuid>.<ext>`, exactly as `import_icon` writes it. The path
// can come from an imported bundle and is joined onto the config dir, where it is read and
// later deleted. Another spelling of the same file would get past the shared-icon check.
pub(crate) fn is_icon_path(rel: &str) -> bool {
    rel.strip_prefix("icons/").and_then(|file| file.split_once('.')).is_some_and(|(id, ext)| {
        Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id)
            && ICON_EXTENSIONS.contains(&ext)
    })
}

impl Store {
    pub fn load(dir: PathBuf) -> AppResult<Self> {
        let mut data: ProfileStore = io::read_json(&dir.join("profiles.json"))?;
        if data.version < ProfileStore::CURRENT_VERSION {
            data.version = ProfileStore::CURRENT_VERSION;
        }
        for p in data.profiles.iter_mut() {
            normalize_keys(p);
        }
        let settings: Settings = io::read_json(&dir.join("settings.json"))?;
        Ok(Self { dir, data, settings })
    }

    fn persist_profiles(&self) -> AppResult<()> {
        io::write_json_atomic(&self.dir.join("profiles.json"), &self.data)
    }

    fn persist_settings(&self) -> AppResult<()> {
        io::write_json_atomic(&self.dir.join("settings.json"), &self.settings)
    }

    pub fn snapshot(&self) -> ProfileStore {
        self.data.clone()
    }

    // Writes first and swaps only on success, so a failed import leaves memory and disk as they
    // were.
    pub fn commit(&mut self, mut data: ProfileStore) -> AppResult<()> {
        data.version = ProfileStore::CURRENT_VERSION;
        for p in data.profiles.iter_mut() {
            normalize_keys(p);
        }
        io::write_json_atomic(&self.dir.join("profiles.json"), &data)?;
        self.data = data;
        Ok(())
    }

    pub fn groups(&self) -> Vec<Group> {
        self.data.groups.clone()
    }

    pub fn profiles(&self) -> Vec<Profile> {
        self.data.profiles.clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.clone()
    }

    pub fn upsert_group(&mut self, group: Group) -> AppResult<()> {
        match self.data.groups.iter_mut().find(|g| g.id == group.id) {
            Some(existing) => *existing = group,
            None => self.data.groups.push(group),
        }
        self.persist_profiles()
    }

    pub fn delete_group(&mut self, id: &str) -> AppResult<()> {
        let before = self.data.groups.len();
        self.data.groups.retain(|g| g.id != id);
        if self.data.groups.len() == before {
            return Err(AppError::NotFound(format!("group {id}")));
        }
        for p in self.data.profiles.iter_mut() {
            if p.group_id.as_deref() == Some(id) {
                p.group_id = None;
            }
        }
        self.persist_profiles()
    }

    pub fn upsert_profile(&mut self, mut profile: Profile) -> AppResult<()> {
        normalize_keys(&mut profile);
        match self.data.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.data.profiles.push(profile),
        }
        self.persist_profiles()
    }

    pub fn delete_profile(&mut self, id: &str) -> AppResult<()> {
        let before = self.data.profiles.len();
        self.data.profiles.retain(|p| p.id != id);
        if self.data.profiles.len() == before {
            return Err(AppError::NotFound(format!("profile {id}")));
        }
        self.persist_profiles()
    }

    pub fn set_settings(&mut self, settings: Settings) -> AppResult<()> {
        self.settings = settings;
        self.persist_settings()
    }

    pub fn sftp_profiles(&self) -> Vec<SftpProfile> {
        self.data.sftp_profiles.clone()
    }

    pub fn upsert_sftp_profile(&mut self, profile: SftpProfile) -> AppResult<()> {
        match self.data.sftp_profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.data.sftp_profiles.push(profile),
        }
        self.persist_profiles()
    }

    pub fn delete_sftp_profile(&mut self, id: &str) -> AppResult<()> {
        let before = self.data.sftp_profiles.len();
        self.data.sftp_profiles.retain(|p| p.id != id);
        if self.data.sftp_profiles.len() == before {
            return Err(AppError::NotFound(format!("sftp profile {id}")));
        }
        self.persist_profiles()
    }

    pub fn s3_profiles(&self) -> Vec<S3Profile> {
        self.data.s3_profiles.clone()
    }

    pub fn upsert_s3_profile(&mut self, profile: S3Profile) -> AppResult<()> {
        match self.data.s3_profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.data.s3_profiles.push(profile),
        }
        self.persist_profiles()
    }

    pub fn delete_s3_profile(&mut self, id: &str) -> AppResult<()> {
        let before = self.data.s3_profiles.len();
        self.data.s3_profiles.retain(|p| p.id != id);
        if self.data.s3_profiles.len() == before {
            return Err(AppError::NotFound(format!("s3 profile {id}")));
        }
        self.persist_profiles()
    }

    pub fn vnc_profiles(&self) -> Vec<VncProfile> {
        self.data.vnc_profiles.clone()
    }

    // A copy is changed and written, and swapped in only on success. Changed in place, a write
    // that fails would leave memory ahead of the file: the profile would then use a password the
    // caller has already taken back, and the next save of anything would write that out.
    pub fn upsert_vnc_profile(&mut self, profile: VncProfile) -> AppResult<()> {
        let mut data = self.data.clone();
        match data.vnc_profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => data.vnc_profiles.push(profile),
        }
        io::write_json_atomic(&self.dir.join("profiles.json"), &data)?;
        self.data = data;
        Ok(())
    }

    pub fn delete_vnc_profile(&mut self, id: &str) -> AppResult<()> {
        let mut data = self.data.clone();
        data.vnc_profiles.retain(|p| p.id != id);
        if data.vnc_profiles.len() == self.data.vnc_profiles.len() {
            return Err(AppError::NotFound(format!("vnc profile {id}")));
        }
        io::write_json_atomic(&self.dir.join("profiles.json"), &data)?;
        self.data = data;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{AuthMethod, IconRef};

    fn profile(id: &str, group: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            group_id: group.map(Into::into),
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            keys: vec![],
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
        }
    }

    fn group(id: &str) -> Group {
        Group { id: id.into(), name: id.into(), parent_id: None, icon: IconRef::default(), order: 0 }
    }

    #[test]
    fn icon_path_is_only_what_import_icon_writes() {
        for ext in ICON_EXTENSIONS {
            let path = format!("icons/{}.{ext}", uuid::Uuid::new_v4());
            assert!(is_icon_path(&path), "{path}");
        }

        let id = "6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8";
        let others = [
            format!("icons/{}.png", id.to_uppercase()),
            format!("icons/{id}.PNG"),
            format!("icons\\{id}.png"),
            format!("icons//{id}.png"),
            format!("Icons/{id}.png"),
            format!("icons/{id}.png."),
            format!("icons/{id}.png "),
            format!("icons/{id}.png::$DATA"),
            format!("icons/{id}.bmp"),
            format!("icons/{id}"),
            format!("icons/{}.png", id.replace('-', "")),
            "icons/NUL".to_string(),
            "icons/a.png".to_string(),
        ];
        let accepted: Vec<_> = others.iter().filter(|p| is_icon_path(p)).collect();
        assert!(accepted.is_empty(), "{accepted:?}");
    }

    #[test]
    fn upsert_inserts_then_updates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.upsert_profile(profile("p1", None)).unwrap();
        let mut p = profile("p1", None);
        p.host = "changed".into();
        store.upsert_profile(p).unwrap();
        assert_eq!(store.profiles().len(), 1);
        assert_eq!(store.profiles()[0].host, "changed");

        let reopened = Store::load(dir.path().to_path_buf()).unwrap();
        assert_eq!(reopened.profiles()[0].host, "changed");
    }

    #[test]
    fn delete_group_detaches_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.upsert_group(group("g1")).unwrap();
        store.upsert_profile(profile("p1", Some("g1"))).unwrap();
        store.delete_group("g1").unwrap();
        assert_eq!(store.profiles()[0].group_id, None);
    }

    // Profiles written before multi-key support carry key_path + a passphrase in secret_id.
    #[test]
    fn load_migrates_a_legacy_single_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("profiles.json"),
            r#"{"version":1,"groups":[],"profiles":[
                {"id":"p1","name":"p1","groupId":null,"host":"h","port":22,"username":"u",
                 "authMethod":"key","keyPath":"/keys/id_ed25519","secretId":"vault-1",
                 "order":0,"jumpHostId":null},
                {"id":"p2","name":"p2","groupId":null,"host":"h","port":22,"username":"u",
                 "authMethod":"password","keyPath":null,"secretId":"vault-2",
                 "order":1,"jumpHostId":null}
            ]}"#,
        )
        .unwrap();
        let store = Store::load(dir.path().to_path_buf()).unwrap();
        let profiles = store.profiles();

        let key_profile = profiles.iter().find(|p| p.id == "p1").unwrap();
        assert_eq!(key_profile.key_path, None);
        assert_eq!(key_profile.keys.len(), 1);
        assert_eq!(key_profile.keys[0].path, "/keys/id_ed25519");
        // The old profile secret was that key's passphrase, and it MOVED - leaving it on the
        // profile too would give one vault entry two owners, and whichever is edited first
        // would delete the other's secret.
        assert_eq!(key_profile.keys[0].secret_id.as_deref(), Some("vault-1"));
        assert_eq!(key_profile.secret_id, None);

        // Password auth keeps its secret where it was, with no phantom key.
        let pw_profile = profiles.iter().find(|p| p.id == "p2").unwrap();
        assert!(pw_profile.keys.is_empty());
        assert_eq!(pw_profile.secret_id.as_deref(), Some("vault-2"));
    }

    // Imports and the ssh-config importer write through upsert, never through load.
    #[test]
    fn upsert_normalizes_a_legacy_key_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut p = profile("p1", None);
        p.auth_method = AuthMethod::Key;
        p.key_path = Some("/keys/id_rsa".into());
        p.secret_id = Some("vault-9".into());
        store.upsert_profile(p).unwrap();

        let saved = &store.profiles()[0];
        assert_eq!(saved.key_path, None);
        assert_eq!(saved.keys.len(), 1);
        assert_eq!(saved.keys[0].path, "/keys/id_rsa");
        assert_eq!(saved.keys[0].secret_id.as_deref(), Some("vault-9"));
        assert_eq!(saved.secret_id, None);
    }

    #[test]
    fn sftp_profiles_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store
            .upsert_sftp_profile(SftpProfile {
                id: "s1".into(),
                name: "files".into(),
                host: "h".into(),
                port: 22,
                username: "u".into(),
                auth_method: AuthMethod::Key,
                keys: vec![ProfileKey { path: "/keys/a".into(), secret_id: None }],
                secret_id: None,
                icon: IconRef::default(),
                order: 0,
            })
            .unwrap();
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().sftp_profiles().len(), 1);
        store.delete_sftp_profile("s1").unwrap();
        assert!(store.sftp_profiles().is_empty());
    }

    fn vnc(id: &str) -> VncProfile {
        VncProfile {
            id: id.into(),
            name: id.into(),
            host: "h".into(),
            port: 5900,
            username: Some("faye".into()),
            secret_id: Some("vault-1".into()),
            icon: IconRef::default(),
            order: 0,
        }
    }

    #[test]
    fn vnc_profiles_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut desk = vnc("v1");
        store.upsert_vnc_profile(desk.clone()).unwrap();
        desk.host = "changed".into();
        store.upsert_vnc_profile(desk.clone()).unwrap();
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles(), [desk]);

        store.delete_vnc_profile("v1").unwrap();
        assert!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles().is_empty());
        assert!(matches!(store.delete_vnc_profile("v1"), Err(AppError::NotFound(_))));
    }

    // A store holding one VNC profile, and the directory that stands where the temp file of a
    // store write should be, so that every write fails until it is removed.
    fn store_with_blocked_writes(dir: &std::path::Path) -> (Store, VncProfile, PathBuf) {
        let mut store = Store::load(dir.to_path_buf()).unwrap();
        let desk = vnc("v1");
        store.upsert_vnc_profile(desk.clone()).unwrap();
        let blocker = dir.join("profiles.json.tmp");
        std::fs::create_dir(&blocker).unwrap();
        (store, desk, blocker)
    }

    #[test]
    fn vnc_upsert_that_cannot_be_written_leaves_the_profiles_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, desk, blocker) = store_with_blocked_writes(dir.path());
        let edited = VncProfile { secret_id: Some("vault-2".into()), ..desk.clone() };
        assert!(matches!(store.upsert_vnc_profile(edited), Err(AppError::Io(_))));
        assert!(matches!(store.upsert_vnc_profile(vnc("v2")), Err(AppError::Io(_))));
        assert_eq!(store.vnc_profiles(), [desk.clone()]);

        std::fs::remove_dir(blocker).unwrap();
        store.upsert_group(group("g1")).unwrap();
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles(), [desk]);
    }

    #[test]
    fn vnc_delete_that_cannot_be_written_keeps_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, desk, blocker) = store_with_blocked_writes(dir.path());
        assert!(matches!(store.delete_vnc_profile("v1"), Err(AppError::Io(_))));
        assert_eq!(store.vnc_profiles(), [desk.clone()]);

        std::fs::remove_dir(blocker).unwrap();
        store.upsert_group(group("g1")).unwrap();
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().vnc_profiles(), [desk]);
    }

    #[test]
    fn profiles_file_from_before_vnc_profiles_loads_with_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("profiles.json"),
            r#"{"version":1,"groups":[{"id":"g1","name":"g1","parentId":null,"order":0}],
                "profiles":[],"sftpProfiles":[],"s3Profiles":[]}"#,
        )
        .unwrap();
        let store = Store::load(dir.path().to_path_buf()).unwrap();
        // A file that does not parse loads as an empty store, so the group shows it was read.
        assert_eq!(store.groups(), [group("g1")]);
        assert!(store.vnc_profiles().is_empty());
    }

    // What a later build that knows more kinds of profile leaves in the file.
    #[test]
    fn sections_this_build_does_not_know_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let rdp = serde_json::json!([{
            "id": "r1", "name": "büro", "host": "10.0.0.9", "port": 3389, "gateway": null,
            "scale": 1.5, "icon": { "kind": "builtin", "name": "server" }, "order": 0
        }]);
        let file = serde_json::json!({
            "version": 1, "groups": [], "profiles": [], "vncProfiles": [vnc("v1")],
            "rdpProfiles": rdp, "syncedAt": 1759622400
        });
        std::fs::write(&path, file.to_string()).unwrap();
        let saved = || -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
        };

        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.upsert_vnc_profile(VncProfile { name: "renamed".into(), ..vnc("v1") }).unwrap();
        assert_eq!(saved()["vncProfiles"][0]["name"], "renamed");
        assert_eq!(saved()["rdpProfiles"].to_string(), rdp.to_string());
        assert_eq!(saved()["syncedAt"], 1759622400);

        // Every other save writes the same data: the plain ones, an import, and a later start.
        store.upsert_group(group("g1")).unwrap();
        assert_eq!(saved()["rdpProfiles"].to_string(), rdp.to_string());
        store.commit(store.snapshot()).unwrap();
        assert_eq!(saved()["rdpProfiles"].to_string(), rdp.to_string());
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.delete_vnc_profile("v1").unwrap();
        assert_eq!(saved()["rdpProfiles"].to_string(), rdp.to_string());
        assert_eq!(saved()["groups"][0]["id"], "g1");
    }

    #[test]
    fn delete_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        assert!(matches!(store.delete_profile("ghost"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn load_migrates_version_zero_to_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        std::fs::write(&path, r#"{"version":0,"groups":[],"profiles":[]}"#).unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        store.upsert_group(group("g1")).unwrap(); // a write persists the bumped version
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["version"].as_u64(), Some(ProfileStore::CURRENT_VERSION as u64));
    }

    #[test]
    fn commit_persists_and_a_failed_commit_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::load(dir.path().to_path_buf()).unwrap();
        let mut data = store.snapshot();
        data.profiles.push(profile("p1", None));
        store.commit(data).unwrap();
        assert_eq!(Store::load(dir.path().to_path_buf()).unwrap().profiles().len(), 1);

        // A directory where profiles.json should be makes the final rename fail.
        let blocked = tempfile::tempdir().unwrap();
        let mut store = Store::load(blocked.path().to_path_buf()).unwrap();
        std::fs::create_dir(blocked.path().join("profiles.json")).unwrap();
        let mut data = store.snapshot();
        data.profiles.push(profile("p2", None));
        assert!(store.commit(data).is_err());
        assert!(store.profiles().is_empty());
    }
}
