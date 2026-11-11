use std::collections::{BTreeMap, HashSet};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::hosts::{ConnectionKind, Folder, Host, SavedLogin, SshKey, Tag};
use crate::store::model::{Group, Profile, RdpProfile, S3Profile, SftpProfile, VncProfile};
use crate::vault::crypto;
use crate::vault::model::KdfParams;

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFile {
    pub file_name: String,
    pub data: Zeroizing<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyPayload {
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub sftp_profiles: Vec<SftpProfile>,
    #[serde(default)]
    pub s3_profiles: Vec<S3Profile>,
    #[serde(default)]
    pub vnc_profiles: Vec<VncProfile>,
    #[serde(default)]
    pub rdp_profiles: Vec<RdpProfile>,
    // Keyed by the exporting machine's secret id and key path, which the profiles still reference.
    #[serde(default)]
    pub secrets: BTreeMap<String, Zeroizing<String>>,
    #[serde(default)]
    pub key_files: BTreeMap<String, KeyFile>,
}

// Version 3: hosts with their connections. `LegacyPayload` above is what versions 1 and 2 hold.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadV3 {
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub logins: Vec<SavedLogin>,
    #[serde(default)]
    pub keys: Vec<SshKey>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub secrets: BTreeMap<String, Zeroizing<String>>,
    #[serde(default)]
    pub key_files: BTreeMap<String, KeyFile>,
    // Read from a version 1 or 2 bundle: its hosts were put together from whichever profiles were
    // exported, so their names, folders and the connection labels say nothing about changes.
    #[serde(skip)]
    pub legacy: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u32,
    #[serde(default)]
    encrypted: bool,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    kdf: Option<KdfParams>,
    #[serde(default)]
    salt: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    ciphertext: Option<String>,
}

pub enum Opened {
    NeedsPassword,
    Legacy(LegacyPayload),
    V3(PayloadV3),
}

fn write_envelope<P: Serialize>(
    version: u32,
    payload: &P,
    carries_secrets: bool,
    password: Option<&str>,
) -> AppResult<Vec<u8>> {
    let envelope = match password {
        None if carries_secrets => {
            return Err(AppError::Import(
                "an export with passwords or key files must be encrypted".into(),
            ));
        }
        None => Envelope {
            version,
            encrypted: false,
            payload: Some(serde_json::to_value(payload)?),
            kdf: None,
            salt: None,
            nonce: None,
            ciphertext: None,
        },
        Some(password) => {
            let plain = Zeroizing::new(serde_json::to_vec(payload)?);
            let salt = crypto::random_bytes::<16>()?;
            let key =
                crypto::derive_key(password.as_bytes(), &salt, Some(KdfParams::STRONG.tuple()))?;
            let (nonce, ciphertext) = crypto::seal(&key, &plain)?;
            Envelope {
                version,
                encrypted: true,
                payload: None,
                kdf: Some(KdfParams::STRONG),
                salt: Some(STANDARD.encode(salt)),
                nonce: Some(STANDARD.encode(nonce)),
                ciphertext: Some(STANDARD.encode(ciphertext)),
            }
        }
    };
    Ok(serde_json::to_vec_pretty(&envelope)?)
}

pub fn write(payload: &PayloadV3, password: Option<&str>) -> AppResult<Vec<u8>> {
    let carries_secrets = !payload.secrets.is_empty() || !payload.key_files.is_empty();
    write_envelope(3, payload, carries_secrets, password)
}

#[cfg(test)]
pub fn write_v2(payload: &LegacyPayload, password: Option<&str>) -> AppResult<Vec<u8>> {
    let carries_secrets = !payload.secrets.is_empty() || !payload.key_files.is_empty();
    write_envelope(2, payload, carries_secrets, password)
}

fn corrupt() -> AppError {
    AppError::Import("the export file is corrupt".into())
}

fn decode(field: Option<String>) -> AppResult<Vec<u8>> {
    STANDARD.decode(field.ok_or_else(corrupt)?).map_err(|_| corrupt())
}

// A repeated id within a kind would be imported as two items sharing that id.
fn checked(payload: LegacyPayload) -> AppResult<Opened> {
    let unique = |ids: Vec<&String>| ids.iter().collect::<HashSet<_>>().len() == ids.len();
    if unique(payload.groups.iter().map(|g| &g.id).collect())
        && unique(payload.profiles.iter().map(|p| &p.id).collect())
        && unique(payload.sftp_profiles.iter().map(|p| &p.id).collect())
        && unique(payload.s3_profiles.iter().map(|p| &p.id).collect())
        && unique(payload.vnc_profiles.iter().map(|p| &p.id).collect())
        && unique(payload.rdp_profiles.iter().map(|p| &p.id).collect())
    {
        Ok(Opened::Legacy(payload))
    } else {
        Err(corrupt())
    }
}

fn checked_v3(payload: PayloadV3) -> AppResult<Opened> {
    let unique = |ids: Vec<&String>| ids.iter().collect::<HashSet<_>>().len() == ids.len();
    if unique(payload.folders.iter().map(|f| &f.id).collect())
        && unique(payload.hosts.iter().map(|h| &h.id).collect())
        && unique(payload.hosts.iter().flat_map(|h| &h.connections).map(|c| &c.id).collect())
        && unique(payload.logins.iter().map(|l| &l.id).collect())
        && unique(payload.keys.iter().map(|k| &k.id).collect())
        && unique(payload.tags.iter().map(|t| &t.id).collect())
    {
        Ok(Opened::V3(payload))
    } else {
        Err(corrupt())
    }
}

fn newer() -> AppError {
    AppError::Import("this file was made by a newer version of Wisp".into())
}

// A key this build does not write holds a kind of item it would leave out without a word.
fn known_keys<'a, P: Serialize + Default>(
    keys: impl IntoIterator<Item = &'a String>,
) -> AppResult<()> {
    let known = serde_json::to_value(P::default())?;
    if keys.into_iter().all(|key| known.get(key).is_some()) {
        return Ok(());
    }
    Err(newer())
}

// A profile field this build does not know, such as a gateway, would be left out on import and the
// profile would then connect without it.
fn known_fields(payload: &LegacyPayload) -> AppResult<()> {
    let known = payload.groups.iter().all(|g| g.extra.is_empty())
        && payload.profiles.iter().all(|p| p.extra.is_empty())
        && payload.sftp_profiles.iter().all(|p| p.extra.is_empty())
        && payload.s3_profiles.iter().all(|p| p.extra.is_empty())
        && payload.vnc_profiles.iter().all(|p| p.extra.is_empty())
        && payload.rdp_profiles.iter().all(|p| {
            p.extra.is_empty()
                && p.shared_folder.as_ref().is_none_or(|f| f.extra.is_empty())
                && p.gateway.as_ref().is_none_or(|g| g.extra.is_empty())
        });
    if known {
        Ok(())
    } else {
        Err(newer())
    }
}

fn kind_is_known(kind: &ConnectionKind) -> bool {
    match kind {
        ConnectionKind::Ssh(c) => c.extra.is_empty(),
        ConnectionKind::Sftp(c) => c.extra.is_empty(),
        ConnectionKind::Ftp(c) => c.extra.is_empty(),
        ConnectionKind::S3(c) => c.extra.is_empty(),
        ConnectionKind::Rdp(c) => {
            c.extra.is_empty()
                && c.shared_folder.as_ref().is_none_or(|f| f.extra.is_empty())
                && c.gateway.as_ref().is_none_or(|g| g.extra.is_empty())
        }
        ConnectionKind::Vnc(c) => c.extra.is_empty(),
        ConnectionKind::Local(c) => c.extra.is_empty(),
    }
}

fn known_fields_v3(payload: &PayloadV3) -> AppResult<()> {
    let known = payload.folders.iter().all(|f| f.extra.is_empty())
        && payload.hosts.iter().all(|h| {
            h.extra.is_empty()
                && h.connections.iter().all(|c| c.extra.is_empty() && kind_is_known(&c.kind))
        })
        && payload.logins.iter().all(|l| l.extra.is_empty())
        && payload.keys.iter().all(|k| k.extra.is_empty())
        && payload.tags.iter().all(|t| t.extra.is_empty());
    if known {
        Ok(())
    } else {
        Err(newer())
    }
}

pub fn load(path: &str) -> AppResult<Zeroizing<Vec<u8>>> {
    let io = |e: std::io::Error| AppError::Io(format!("{path}: {e}"));
    if std::fs::metadata(path).map_err(io)?.len() > MAX_FILE_BYTES {
        return Err(AppError::Import("the export file is too large".into()));
    }
    Ok(Zeroizing::new(std::fs::read(path).map_err(io)?))
}

// The payload of an envelope, or None when it is encrypted and no password was given.
fn open_envelope<P: Serialize + DeserializeOwned + Default>(
    value: serde_json::Value,
    password: Option<&str>,
) -> AppResult<Option<P>> {
    if let Some(serde_json::Value::Object(payload)) = value.get("payload") {
        known_keys::<P>(payload.keys())?;
    }
    let envelope: Envelope = serde_json::from_value(value)?;
    if !envelope.encrypted {
        let payload = envelope.payload.ok_or_else(corrupt)?;
        return Ok(Some(serde_json::from_value(payload)?));
    }
    let Some(password) = password else { return Ok(None) };
    let salt: [u8; 16] = decode(envelope.salt)?.try_into().map_err(|_| corrupt())?;
    let nonce: [u8; 24] = decode(envelope.nonce)?.try_into().map_err(|_| corrupt())?;
    let ciphertext = decode(envelope.ciphertext)?;
    let params = envelope.kdf.ok_or_else(corrupt)?;
    // The file is untrusted and these are spent before the password can be checked.
    if !(1..=8).contains(&params.t_cost)
        || !(1..=16).contains(&params.p_cost)
        || !(8 * params.p_cost..=262_144).contains(&params.m_cost)
    {
        return Err(corrupt());
    }
    let key = crypto::derive_key(password.as_bytes(), &salt, Some(params.tuple()))?;
    let plain = crypto::open(&key, &nonce, &ciphertext).map_err(|_| AppError::WrongPassphrase)?;
    // Only the keys are read here, so that no secret is copied out of `plain`.
    let keys: BTreeMap<String, IgnoredAny> = serde_json::from_slice(&plain)?;
    known_keys::<P>(keys.keys())?;
    Ok(Some(serde_json::from_slice(&plain)?))
}

pub fn read(bytes: &[u8], password: Option<&str>) -> AppResult<Opened> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value.get("version").and_then(|v| v.as_u64()) {
        Some(1) if value.get("groups").is_some() && value.get("profiles").is_some() => {
            let payload = serde_json::from_value(value)?;
            known_fields(&payload)?;
            checked(payload)
        }
        Some(2) => match open_envelope::<LegacyPayload>(value, password)? {
            None => Ok(Opened::NeedsPassword),
            Some(payload) => {
                known_fields(&payload)?;
                checked(payload)
            }
        },
        Some(3) => match open_envelope::<PayloadV3>(value, password)? {
            None => Ok(Opened::NeedsPassword),
            Some(payload) => {
                known_fields_v3(&payload)?;
                checked_v3(payload)
            }
        },
        Some(1) | None => Err(AppError::Import("this isn't a wisp export file".into())),
        Some(v) => Err(AppError::Import(format!("unsupported export version {v}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, IconRef, Profile, RdpGateway, S3Profile, SharedFolder};

    fn profile(id: &str) -> Profile {
        Profile {
            id: id.into(),
            name: "web".into(),
            group_id: None,
            host: "h".into(),
            port: 22,
            username: "u".into(),
            auth_method: AuthMethod::Password,
            key_path: None,
            keys: vec![],
            secret_id: Some("sec-1".into()),
            icon: IconRef::default(),
            order: 0,
            jump_host_id: None,
            tunnels: vec![],
            appearance: None,
            extra: Default::default(),
        }
    }

    fn vnc(id: &str) -> VncProfile {
        VncProfile {
            id: id.into(),
            name: "desk".into(),
            host: "10.0.0.5".into(),
            port: 5901,
            username: Some("faye".into()),
            secret_id: Some("vnc-1".into()),
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        }
    }

    fn rdp(id: &str) -> RdpProfile {
        RdpProfile {
            id: id.into(),
            name: "office".into(),
            host: "10.0.0.9".into(),
            port: 3390,
            username: "faye".into(),
            domain: Some("CORP".into()),
            secret_id: Some("rdp-1".into()),
            keyboard_layout: Some(0x407),
            audio: true,
            shared_folder: None,
            gateway: None,
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        }
    }

    fn v3() -> PayloadV3 {
        use crate::store::hosts::fixtures::{host, own, ssh, vnc};
        let mut p = PayloadV3 {
            hosts: vec![host(
                "h1",
                "nas",
                "nas.local",
                vec![ssh("c1", 22, own("admin", Some("sec-1"))), vnc("c2", None)],
            )],
            ..Default::default()
        };
        p.secrets.insert("sec-1".into(), Zeroizing::new("hunter2".into()));
        p
    }

    fn opened_v3(bytes: &[u8], password: Option<&str>) -> PayloadV3 {
        match read(bytes, password).unwrap() {
            Opened::V3(p) => p,
            _ => panic!("not a version 3 payload"),
        }
    }

    #[test]
    fn v3_round_trips_plain_and_encrypted() {
        let plain = PayloadV3 { secrets: Default::default(), ..v3() };
        let bytes = write(&plain, None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["version"], 3);
        assert!(opened_v3(&bytes, None) == plain);

        let bytes = write(&v3(), Some("pw")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("hunter2"));
        assert!(matches!(read(&bytes, None).unwrap(), Opened::NeedsPassword));
        assert!(opened_v3(&bytes, Some("pw")) == v3());
        assert!(matches!(read(&bytes, Some("nope")), Err(AppError::WrongPassphrase)));
    }

    #[test]
    fn v3_with_secrets_must_be_encrypted() {
        assert!(matches!(write(&v3(), None), Err(AppError::Import(_))));
    }

    #[test]
    fn v3_rejects_an_unknown_connection_field() {
        let plain = PayloadV3 { secrets: Default::default(), ..v3() };
        let mut value: serde_json::Value =
            serde_json::from_slice(&write(&plain, None).unwrap()).unwrap();
        value["payload"]["hosts"][0]["connections"][0]["kind"]["x"] = 1.into();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(matches!(read(&bytes, None), Err(AppError::Import(m)) if m.contains("newer")));

        let mut value: serde_json::Value =
            serde_json::from_slice(&write(&plain, None).unwrap()).unwrap();
        value["payload"]["themes"] = serde_json::json!([]);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(matches!(read(&bytes, None), Err(AppError::Import(m)) if m.contains("newer")));
    }

    #[test]
    fn v3_repeated_connection_ids_are_import_errors() {
        let mut p = PayloadV3 { secrets: Default::default(), ..v3() };
        let copy = p.hosts[0].connections[0].clone();
        p.hosts.push(crate::store::hosts::fixtures::host("h2", "b", "b", vec![copy]));
        let bytes = write(&p, None).unwrap();
        assert!(matches!(read(&bytes, None), Err(AppError::Import(m)) if m.contains("corrupt")));
    }

    #[test]
    fn legacy_v2_still_reads() {
        let p = LegacyPayload { profiles: vec![profile("p1")], ..Default::default() };
        let bytes = write_v2(&p, None).unwrap();
        assert!(matches!(read(&bytes, None).unwrap(), Opened::Legacy(read) if read.profiles.len() == 1));
    }

    fn payload() -> LegacyPayload {
        let mut p = LegacyPayload { profiles: vec![profile("p1")], ..Default::default() };
        p.secrets.insert("sec-1".into(), Zeroizing::new("hunter2".into()));
        p.key_files.insert(
            "/Users/me/.ssh/id_ed25519".into(),
            KeyFile {
                file_name: "id_ed25519".into(),
                data: Zeroizing::new(STANDARD.encode(b"KEYDATA")),
            },
        );
        p
    }

    fn sealed_with(kdf: KdfParams) -> Vec<u8> {
        let salt = crypto::random_bytes::<16>().unwrap();
        let key = crypto::derive_key(b"pw", &salt, Some(kdf.tuple())).unwrap();
        let (nonce, ciphertext) =
            crypto::seal(&key, &serde_json::to_vec(&payload()).unwrap()).unwrap();
        let envelope = Envelope {
            version: 2,
            encrypted: true,
            payload: None,
            kdf: Some(kdf),
            salt: Some(STANDARD.encode(salt)),
            nonce: Some(STANDARD.encode(nonce)),
            ciphertext: Some(STANDARD.encode(ciphertext)),
        };
        serde_json::to_vec(&envelope).unwrap()
    }

    #[test]
    fn plain_write_with_secrets_or_key_files_is_refused() {
        let mut with_secret = LegacyPayload { profiles: vec![profile("p1")], ..Default::default() };
        with_secret.secrets.insert("sec-1".into(), Zeroizing::new("hunter2".into()));
        let with_key = LegacyPayload { key_files: payload().key_files, ..Default::default() };
        for payload in [&with_secret, &with_key] {
            assert!(matches!(write_v2(payload, None), Err(AppError::Import(_))));
        }
    }

    #[test]
    fn encrypted_write_uses_the_strong_kdf_parameters() {
        let bytes = write_v2(&payload(), Some("pw")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let kdf: KdfParams = serde_json::from_value(value["kdf"].clone()).unwrap();
        assert_eq!(kdf, KdfParams::STRONG);
    }

    #[test]
    fn smallest_kdf_params_are_accepted() {
        let bytes = sealed_with(KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 });
        assert!(matches!(
            read(&bytes, Some("pw")).unwrap(),
            Opened::Legacy(back) if back == payload()
        ));
    }

    #[test]
    fn files_over_the_size_limit_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.json");
        let name = path.to_str().unwrap();
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(load(name).unwrap().as_slice(), b"{}");

        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_FILE_BYTES).unwrap();
        assert_eq!(load(name).unwrap().len() as u64, MAX_FILE_BYTES);
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(matches!(load(name), Err(AppError::Import(_))));

        let missing = dir.path().join("missing.json");
        assert!(matches!(load(missing.to_str().unwrap()), Err(AppError::Io(_))));
    }

    #[test]
    fn plain_round_trips() {
        let p = LegacyPayload { profiles: vec![profile("p1")], ..Default::default() };
        let bytes = write_v2(&p, None).unwrap();
        assert!(matches!(read(&bytes, None).unwrap(), Opened::Legacy(back) if back == p));
    }

    #[test]
    fn encrypted_round_trips_and_hides_secrets() {
        let bytes = write_v2(&payload(), Some("pw")).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("id_ed25519"));
        assert!(matches!(read(&bytes, None).unwrap(), Opened::NeedsPassword));
        assert!(matches!(
            read(&bytes, Some("pw")).unwrap(),
            Opened::Legacy(back) if back == payload()
        ));
    }

    #[test]
    fn wrong_password_is_wrong_passphrase() {
        let bytes = write_v2(&payload(), Some("pw")).unwrap();
        assert!(matches!(read(&bytes, Some("nope")), Err(AppError::WrongPassphrase)));
    }

    #[test]
    fn version_one_files_still_read() {
        let s3 = S3Profile {
            id: "s1".into(),
            name: "backups".into(),
            endpoint: "s3.example.com".into(),
            port: None,
            region: "us-east-1".into(),
            use_tls: true,
            path_style: false,
            access_key_id: "AK".into(),
            secret_id: None,
            bucket: None,
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        };
        let v1 = serde_json::json!({
            "version": 1, "groups": [], "profiles": [profile("p1")], "s3Profiles": [s3]
        });
        let Opened::Legacy(p) = read(v1.to_string().as_bytes(), None).unwrap() else { panic!() };
        assert_eq!(p.profiles.len(), 1);
        assert_eq!(p.s3_profiles.len(), 1);
        assert!(p.secrets.is_empty());
    }

    #[test]
    fn version_one_files_with_a_field_this_build_does_not_know_are_refused() {
        let mut newer = serde_json::to_value(profile("p1")).unwrap();
        newer["gateway"] = serde_json::json!({ "host": "gw.lan" });
        let v1 = serde_json::json!({ "version": 1, "groups": [], "profiles": [newer] });
        assert!(matches!(
            read(v1.to_string().as_bytes(), None),
            Err(AppError::Import(told)) if told == "this file was made by a newer version of Wisp"
        ));
    }

    #[test]
    fn vnc_profiles_round_trip_in_a_version_two_file() {
        let plain = LegacyPayload { vnc_profiles: vec![vnc("v1")], ..Default::default() };
        let mut sealed = plain.clone();
        sealed.secrets.insert("vnc-1".into(), Zeroizing::new("hunter2".into()));
        for (payload, password) in [(&plain, None), (&sealed, Some("pw"))] {
            let bytes = write_v2(payload, password).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["version"], 2);
            assert!(matches!(
                read(&bytes, password).unwrap(),
                Opened::Legacy(back) if &back == payload
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&write_v2(&plain, None).unwrap()).unwrap();
        assert_eq!(value["payload"]["vncProfiles"][0]["id"], "v1");
    }

    #[test]
    fn rdp_profiles_round_trip_in_a_version_two_file() {
        let plain = LegacyPayload { rdp_profiles: vec![rdp("r1")], ..Default::default() };
        let mut sealed = plain.clone();
        sealed.secrets.insert("rdp-1".into(), Zeroizing::new("hunter2".into()));
        for (payload, password) in [(&plain, None), (&sealed, Some("pw"))] {
            let bytes = write_v2(payload, password).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["version"], 2);
            assert!(matches!(
                read(&bytes, password).unwrap(),
                Opened::Legacy(back) if &back == payload
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&write_v2(&plain, None).unwrap()).unwrap();
        assert_eq!(value["payload"]["rdpProfiles"][0]["id"], "r1");
    }

    #[test]
    fn version_two_files_from_before_sound_folder_and_gateway_read_with_sound_and_neither() {
        let mut old = serde_json::to_value(rdp("r1")).unwrap();
        for field in ["audio", "sharedFolder", "gateway"] {
            assert!(old.as_object_mut().unwrap().remove(field).is_some());
        }
        let payload = serde_json::json!({
            "groups": [], "profiles": [], "sftpProfiles": [], "s3Profiles": [],
            "vncProfiles": [], "rdpProfiles": [old], "secrets": {}, "keyFiles": {}
        });
        for (file, password) in version_two_files(&payload) {
            let Opened::Legacy(p) = read(file.to_string().as_bytes(), password).unwrap() else {
                panic!()
            };
            let r = &p.rdp_profiles[0];
            assert!(r.audio, "{password:?}");
            assert_eq!((&r.shared_folder, &r.gateway), (&None, &None));
            assert!(r.extra.is_empty());
            assert_eq!(p.rdp_profiles, [rdp("r1")]);
        }
    }

    #[test]
    fn rdp_sound_folder_and_gateway_round_trip_in_a_version_two_file() {
        let office = RdpProfile {
            audio: false,
            shared_folder: Some(SharedFolder {
                path: "C:\\Users\\faye".into(),
                name: "faye".into(),
                extra: Default::default(),
            }),
            gateway: Some(RdpGateway {
                host: "gw.corp".into(),
                port: 8443,
                same_login: false,
                username: "CORP\\gw".into(),
                secret_id: Some("rdp-gw".into()),
                extra: Default::default(),
            }),
            ..rdp("r1")
        };
        let plain = LegacyPayload { rdp_profiles: vec![office], ..Default::default() };
        let mut sealed = plain.clone();
        sealed.secrets.insert("rdp-1".into(), Zeroizing::new("hunter2".into()));
        sealed.secrets.insert("rdp-gw".into(), Zeroizing::new("gateway".into()));
        for (payload, password) in [(&plain, None), (&sealed, Some("pw"))] {
            let bytes = write_v2(payload, password).unwrap();
            assert!(matches!(
                read(&bytes, password).unwrap(),
                Opened::Legacy(back) if &back == payload
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&write_v2(&plain, None).unwrap()).unwrap();
        let written = &value["payload"]["rdpProfiles"][0];
        assert_eq!(written["audio"], false);
        assert_eq!(written["sharedFolder"]["name"], "faye");
        assert_eq!(written["gateway"]["secretId"], "rdp-gw");
    }

    #[test]
    fn version_two_files_from_before_rdp_profiles_still_read() {
        let old = serde_json::json!({
            "groups": [], "profiles": [profile("p1")], "sftpProfiles": [], "s3Profiles": [],
            "vncProfiles": [vnc("v1")], "secrets": {}, "keyFiles": {}
        });
        let kdf = KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 };
        let salt = crypto::random_bytes::<16>().unwrap();
        let key = crypto::derive_key(b"pw", &salt, Some(kdf.tuple())).unwrap();
        let (nonce, ciphertext) = crypto::seal(&key, old.to_string().as_bytes()).unwrap();
        let sealed = serde_json::json!({
            "version": 2, "encrypted": true, "kdf": kdf, "salt": STANDARD.encode(salt),
            "nonce": STANDARD.encode(nonce), "ciphertext": STANDARD.encode(ciphertext)
        });
        let plain = serde_json::json!({ "version": 2, "encrypted": false, "payload": old });
        for (file, password) in [(plain, None), (sealed, Some("pw"))] {
            let Opened::Legacy(p) = read(file.to_string().as_bytes(), password).unwrap() else {
                panic!()
            };
            assert_eq!(p.vnc_profiles.len(), 1);
            assert!(p.rdp_profiles.is_empty());
        }
    }

    #[test]
    fn version_two_files_from_before_vnc_profiles_still_read() {
        let old = serde_json::json!({
            "groups": [], "profiles": [profile("p1")], "sftpProfiles": [], "s3Profiles": [],
            "secrets": {}, "keyFiles": {}
        });
        let kdf = KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 };
        let salt = crypto::random_bytes::<16>().unwrap();
        let key = crypto::derive_key(b"pw", &salt, Some(kdf.tuple())).unwrap();
        let (nonce, ciphertext) = crypto::seal(&key, old.to_string().as_bytes()).unwrap();
        let sealed = serde_json::json!({
            "version": 2, "encrypted": true, "kdf": kdf, "salt": STANDARD.encode(salt),
            "nonce": STANDARD.encode(nonce), "ciphertext": STANDARD.encode(ciphertext)
        });
        let plain = serde_json::json!({ "version": 2, "encrypted": false, "payload": old });
        for (file, password) in [(plain, None), (sealed, Some("pw"))] {
            let Opened::Legacy(p) = read(file.to_string().as_bytes(), password).unwrap() else {
                panic!()
            };
            assert_eq!(p.profiles.len(), 1);
            assert!(p.vnc_profiles.is_empty() && p.rdp_profiles.is_empty());
        }
    }

    #[test]
    fn version_two_files_with_a_kind_this_build_does_not_know_are_refused() {
        let newer = serde_json::json!({
            "groups": [], "profiles": [profile("p1")], "sftpProfiles": [], "s3Profiles": [],
            "vncProfiles": [vnc("v1")], "kioskProfiles": [{ "id": "k1" }], "secrets": {},
            "keyFiles": {}
        });
        let kdf = KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 };
        let salt = crypto::random_bytes::<16>().unwrap();
        let key = crypto::derive_key(b"pw", &salt, Some(kdf.tuple())).unwrap();
        let (nonce, ciphertext) = crypto::seal(&key, newer.to_string().as_bytes()).unwrap();
        let sealed = serde_json::json!({
            "version": 2, "encrypted": true, "kdf": kdf, "salt": STANDARD.encode(salt),
            "nonce": STANDARD.encode(nonce), "ciphertext": STANDARD.encode(ciphertext)
        });
        let plain = serde_json::json!({ "version": 2, "encrypted": false, "payload": newer });
        assert!(matches!(
            read(sealed.to_string().as_bytes(), None).unwrap(),
            Opened::NeedsPassword
        ));
        for (file, password) in [(plain, None), (sealed, Some("pw"))] {
            assert!(matches!(
                read(file.to_string().as_bytes(), password),
                Err(AppError::Import(told))
                    if told == "this file was made by a newer version of Wisp"
            ));
        }
    }

    // A plain and a sealed version two file of `payload`, each with the password it needs.
    fn version_two_files(payload: &serde_json::Value) -> [(serde_json::Value, Option<&str>); 2] {
        let kdf = KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 };
        let salt = crypto::random_bytes::<16>().unwrap();
        let key = crypto::derive_key(b"pw", &salt, Some(kdf.tuple())).unwrap();
        let (nonce, ciphertext) = crypto::seal(&key, payload.to_string().as_bytes()).unwrap();
        let sealed = serde_json::json!({
            "version": 2, "encrypted": true, "kdf": kdf, "salt": STANDARD.encode(salt),
            "nonce": STANDARD.encode(nonce), "ciphertext": STANDARD.encode(ciphertext)
        });
        let plain = serde_json::json!({ "version": 2, "encrypted": false, "payload": payload });
        [(plain, None), (sealed, Some("pw"))]
    }

    #[test]
    fn profiles_with_fields_this_build_does_not_know_are_refused() {
        let group = Group {
            id: "g1".into(),
            name: "g1".into(),
            parent_id: None,
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        };
        let sftp = serde_json::json!({ "id": "s1", "name": "s1", "host": "h", "port": 22,
            "username": "u", "authMethod": "password", "secretId": null, "order": 0 });
        let s3 = serde_json::json!({ "id": "b1", "name": "b1", "endpoint": "e", "port": null,
            "region": "r", "useTls": true, "pathStyle": false, "accessKeyId": "k",
            "secretId": null, "bucket": null, "order": 0 });
        let known = serde_json::json!({
            "groups": [group], "profiles": [profile("p1")], "sftpProfiles": [sftp],
            "s3Profiles": [s3], "vncProfiles": [vnc("v1")], "rdpProfiles": [rdp("r1")],
            "secrets": {}, "keyFiles": {}
        });
        for (file, password) in version_two_files(&known) {
            let Opened::Legacy(p) = read(file.to_string().as_bytes(), password).unwrap() else {
                panic!()
            };
            assert_eq!(p.rdp_profiles, [rdp("r1")]);
        }

        let kinds = ["groups", "profiles", "sftpProfiles", "s3Profiles", "vncProfiles"];
        for kind in ["rdpProfiles"].into_iter().chain(kinds) {
            let mut newer = known.clone();
            newer[kind][0]["later"] = serde_json::json!({ "host": "gw.lan", "port": 443 });
            for (file, password) in version_two_files(&newer) {
                assert!(
                    matches!(
                        read(file.to_string().as_bytes(), password),
                        Err(AppError::Import(told))
                            if told == "this file was made by a newer version of Wisp"
                    ),
                    "{kind} {password:?}"
                );
            }
        }
    }

    #[test]
    fn a_folder_or_gateway_with_fields_this_build_does_not_know_is_refused() {
        let mut rdp = serde_json::to_value(rdp("r1")).unwrap();
        rdp["sharedFolder"] = serde_json::json!({ "path": "/s", "name": "s" });
        rdp["gateway"] = serde_json::json!({ "host": "gw", "port": 443, "sameLogin": true,
            "username": "", "secretId": null });
        let known = serde_json::json!({ "rdpProfiles": [rdp], "secrets": {}, "keyFiles": {} });
        for (file, password) in version_two_files(&known) {
            assert!(read(file.to_string().as_bytes(), password).is_ok(), "{password:?}");
        }
        for nested in ["sharedFolder", "gateway"] {
            let mut newer = known.clone();
            newer["rdpProfiles"][0][nested]["later"] = serde_json::json!({ "auth": "ntlm" });
            for (file, password) in version_two_files(&newer) {
                assert!(
                    matches!(
                        read(file.to_string().as_bytes(), password),
                        Err(AppError::Import(told))
                            if told == "this file was made by a newer version of Wisp"
                    ),
                    "{nested} {password:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_version_and_foreign_json_are_import_errors() {
        assert!(matches!(read(br#"{"version":3}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(br#"{"hello":1}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(br#"{"version":1,"hello":1}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(b"not json", None), Err(AppError::Serde(_))));
    }

    #[test]
    fn out_of_range_kdf_params_are_import_errors() {
        let bytes = write_v2(&payload(), Some("pw")).unwrap();
        for (field, cost) in
            [("mCost", u32::MAX), ("mCost", 262_145), ("tCost", 0), ("tCost", 9), ("pCost", 17)]
        {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["kdf"][field] = cost.into();
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(matches!(read(&bytes, Some("pw")), Err(AppError::Import(_))));
        }
    }

    #[test]
    fn repeated_ids_within_a_kind_are_import_errors() {
        let group = Group {
            id: "g1".into(),
            name: "lab".into(),
            parent_id: None,
            icon: IconRef::default(),
            order: 0,
            extra: Default::default(),
        };
        let groups = LegacyPayload { groups: vec![group.clone(), group], ..Default::default() };
        let profiles =
            LegacyPayload { profiles: vec![profile("p1"), profile("p1")], ..Default::default() };
        let vnc_profiles =
            LegacyPayload { vnc_profiles: vec![vnc("v1"), vnc("v1")], ..Default::default() };
        let rdp_profiles =
            LegacyPayload { rdp_profiles: vec![rdp("r1"), rdp("r1")], ..Default::default() };
        for (payload, password) in [
            (&groups, None),
            (&profiles, None),
            (&profiles, Some("pw")),
            (&vnc_profiles, None),
            (&rdp_profiles, None),
        ] {
            let bytes = write_v2(payload, password).unwrap();
            assert!(matches!(read(&bytes, password), Err(AppError::Import(_))));
        }

        let sftp = serde_json::json!({
            "id": "f1", "name": "files", "host": "h", "port": 22, "username": "u",
            "authMethod": "agent", "secretId": null, "order": 0
        });
        let s3 = serde_json::json!({
            "id": "s1", "name": "backups", "endpoint": "s3.example.com", "port": null,
            "region": "us-east-1", "useTls": true, "pathStyle": false, "accessKeyId": "AK",
            "secretId": null, "bucket": null, "order": 0
        });
        for (kind, item) in [("sftpProfiles", sftp), ("s3Profiles", s3)] {
            let mut v1 = serde_json::json!({ "version": 1, "groups": [], "profiles": [] });
            v1[kind] = serde_json::json!([item]);
            assert!(matches!(read(v1.to_string().as_bytes(), None), Ok(Opened::Legacy(_))));
            v1[kind] = serde_json::json!([item.clone(), item]);
            assert!(matches!(read(v1.to_string().as_bytes(), None), Err(AppError::Import(_))));
        }
    }
}
