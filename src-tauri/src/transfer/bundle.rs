use std::collections::{BTreeMap, HashSet};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::model::{Group, Profile, S3Profile, SftpProfile, VncProfile};
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
pub struct Payload {
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
    // Keyed by the exporting machine's secret id and key path, which the profiles still reference.
    #[serde(default)]
    pub secrets: BTreeMap<String, Zeroizing<String>>,
    #[serde(default)]
    pub key_files: BTreeMap<String, KeyFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u32,
    #[serde(default)]
    encrypted: bool,
    #[serde(default)]
    payload: Option<Payload>,
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
    Payload(Payload),
}

pub fn write(payload: &Payload, password: Option<&str>) -> AppResult<Vec<u8>> {
    let envelope = match password {
        None if !payload.secrets.is_empty() || !payload.key_files.is_empty() => {
            return Err(AppError::Import(
                "an export with passwords or key files must be encrypted".into(),
            ));
        }
        None => Envelope {
            version: 2,
            encrypted: false,
            payload: Some(payload.clone()),
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
                version: 2,
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

fn corrupt() -> AppError {
    AppError::Import("the export file is corrupt".into())
}

fn decode(field: Option<String>) -> AppResult<Vec<u8>> {
    STANDARD.decode(field.ok_or_else(corrupt)?).map_err(|_| corrupt())
}

// A repeated id within a kind would be imported as two items sharing that id.
fn checked(payload: Payload) -> AppResult<Opened> {
    let unique = |ids: Vec<&String>| ids.iter().collect::<HashSet<_>>().len() == ids.len();
    if unique(payload.groups.iter().map(|g| &g.id).collect())
        && unique(payload.profiles.iter().map(|p| &p.id).collect())
        && unique(payload.sftp_profiles.iter().map(|p| &p.id).collect())
        && unique(payload.s3_profiles.iter().map(|p| &p.id).collect())
        && unique(payload.vnc_profiles.iter().map(|p| &p.id).collect())
    {
        Ok(Opened::Payload(payload))
    } else {
        Err(corrupt())
    }
}

pub fn load(path: &str) -> AppResult<Zeroizing<Vec<u8>>> {
    let io = |e: std::io::Error| AppError::Io(format!("{path}: {e}"));
    if std::fs::metadata(path).map_err(io)?.len() > MAX_FILE_BYTES {
        return Err(AppError::Import("the export file is too large".into()));
    }
    Ok(Zeroizing::new(std::fs::read(path).map_err(io)?))
}

pub fn read(bytes: &[u8], password: Option<&str>) -> AppResult<Opened> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value.get("version").and_then(|v| v.as_u64()) {
        Some(1) if value.get("groups").is_some() && value.get("profiles").is_some() => {
            checked(serde_json::from_value(value)?)
        }
        Some(2) => {
            let envelope: Envelope = serde_json::from_value(value)?;
            if !envelope.encrypted {
                return checked(envelope.payload.ok_or_else(corrupt)?);
            }
            let Some(password) = password else { return Ok(Opened::NeedsPassword) };
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
            let plain =
                crypto::open(&key, &nonce, &ciphertext).map_err(|_| AppError::WrongPassphrase)?;
            checked(serde_json::from_slice(&plain)?)
        }
        Some(1) | None => Err(AppError::Import("this isn't a wisp export file".into())),
        Some(v) => Err(AppError::Import(format!("unsupported export version {v}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::{AuthMethod, IconRef, Profile, S3Profile};

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
        }
    }

    fn payload() -> Payload {
        let mut p = Payload { profiles: vec![profile("p1")], ..Default::default() };
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
        let mut with_secret = Payload { profiles: vec![profile("p1")], ..Default::default() };
        with_secret.secrets.insert("sec-1".into(), Zeroizing::new("hunter2".into()));
        let with_key = Payload { key_files: payload().key_files, ..Default::default() };
        for payload in [&with_secret, &with_key] {
            assert!(matches!(write(payload, None), Err(AppError::Import(_))));
        }
    }

    #[test]
    fn encrypted_write_uses_the_strong_kdf_parameters() {
        let bytes = write(&payload(), Some("pw")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let kdf: KdfParams = serde_json::from_value(value["kdf"].clone()).unwrap();
        assert_eq!(kdf, KdfParams::STRONG);
    }

    #[test]
    fn smallest_kdf_params_are_accepted() {
        let bytes = sealed_with(KdfParams { m_cost: 8, t_cost: 1, p_cost: 1 });
        assert!(matches!(
            read(&bytes, Some("pw")).unwrap(),
            Opened::Payload(back) if back == payload()
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
        let p = Payload { profiles: vec![profile("p1")], ..Default::default() };
        let bytes = write(&p, None).unwrap();
        assert!(matches!(read(&bytes, None).unwrap(), Opened::Payload(back) if back == p));
    }

    #[test]
    fn encrypted_round_trips_and_hides_secrets() {
        let bytes = write(&payload(), Some("pw")).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("id_ed25519"));
        assert!(matches!(read(&bytes, None).unwrap(), Opened::NeedsPassword));
        assert!(matches!(
            read(&bytes, Some("pw")).unwrap(),
            Opened::Payload(back) if back == payload()
        ));
    }

    #[test]
    fn wrong_password_is_wrong_passphrase() {
        let bytes = write(&payload(), Some("pw")).unwrap();
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
        };
        let v1 = serde_json::json!({
            "version": 1, "groups": [], "profiles": [profile("p1")], "s3Profiles": [s3]
        });
        let Opened::Payload(p) = read(v1.to_string().as_bytes(), None).unwrap() else { panic!() };
        assert_eq!(p.profiles.len(), 1);
        assert_eq!(p.s3_profiles.len(), 1);
        assert!(p.secrets.is_empty());
    }

    #[test]
    fn vnc_profiles_round_trip_in_a_version_two_file() {
        let plain = Payload { vnc_profiles: vec![vnc("v1")], ..Default::default() };
        let mut sealed = plain.clone();
        sealed.secrets.insert("vnc-1".into(), Zeroizing::new("hunter2".into()));
        for (payload, password) in [(&plain, None), (&sealed, Some("pw"))] {
            let bytes = write(payload, password).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["version"], 2);
            assert!(matches!(
                read(&bytes, password).unwrap(),
                Opened::Payload(back) if &back == payload
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&write(&plain, None).unwrap()).unwrap();
        assert_eq!(value["payload"]["vncProfiles"][0]["id"], "v1");
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
            let Opened::Payload(p) = read(file.to_string().as_bytes(), password).unwrap() else {
                panic!()
            };
            assert_eq!(p.profiles.len(), 1);
            assert!(p.vnc_profiles.is_empty());
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
        let bytes = write(&payload(), Some("pw")).unwrap();
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
        };
        let groups = Payload { groups: vec![group.clone(), group], ..Default::default() };
        let profiles =
            Payload { profiles: vec![profile("p1"), profile("p1")], ..Default::default() };
        let vnc_profiles =
            Payload { vnc_profiles: vec![vnc("v1"), vnc("v1")], ..Default::default() };
        for (payload, password) in
            [(&groups, None), (&profiles, None), (&profiles, Some("pw")), (&vnc_profiles, None)]
        {
            let bytes = write(payload, password).unwrap();
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
            assert!(matches!(read(v1.to_string().as_bytes(), None), Ok(Opened::Payload(_))));
            v1[kind] = serde_json::json!([item.clone(), item]);
            assert!(matches!(read(v1.to_string().as_bytes(), None), Err(AppError::Import(_))));
        }
    }
}
