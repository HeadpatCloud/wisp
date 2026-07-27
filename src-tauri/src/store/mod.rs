pub mod io;
pub mod model;

use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use model::{AuthMethod, Group, Profile, ProfileKey, ProfileStore, S3Profile, SftpProfile, Settings};

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
}
