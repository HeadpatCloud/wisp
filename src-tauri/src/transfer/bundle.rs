use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::model::{Group, Profile, S3Profile, SftpProfile};
use crate::vault::crypto;
use crate::vault::model::KdfParams;

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

pub fn read(bytes: &[u8], password: Option<&str>) -> AppResult<Opened> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value.get("version").and_then(|v| v.as_u64()) {
        Some(1) if value.get("groups").is_some() && value.get("profiles").is_some() => {
            Ok(Opened::Payload(serde_json::from_value(value)?))
        }
        Some(2) => {
            let envelope: Envelope = serde_json::from_value(value)?;
            if !envelope.encrypted {
                return envelope.payload.map(Opened::Payload).ok_or_else(corrupt);
            }
            let Some(password) = password else { return Ok(Opened::NeedsPassword) };
            let salt: [u8; 16] = decode(envelope.salt)?.try_into().map_err(|_| corrupt())?;
            let nonce: [u8; 24] = decode(envelope.nonce)?.try_into().map_err(|_| corrupt())?;
            let ciphertext = decode(envelope.ciphertext)?;
            let params = envelope.kdf.ok_or_else(corrupt)?;
            // The file is untrusted and these are spent before the password can be checked.
            if !(1..=16).contains(&params.t_cost)
                || !(1..=16).contains(&params.p_cost)
                || !(8 * params.p_cost..=1_048_576).contains(&params.m_cost)
            {
                return Err(corrupt());
            }
            let key = crypto::derive_key(password.as_bytes(), &salt, Some(params.tuple()))?;
            let plain =
                crypto::open(&key, &nonce, &ciphertext).map_err(|_| AppError::WrongPassphrase)?;
            Ok(Opened::Payload(serde_json::from_slice(&plain)?))
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
    fn unknown_version_and_foreign_json_are_import_errors() {
        assert!(matches!(read(br#"{"version":3}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(br#"{"hello":1}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(br#"{"version":1,"hello":1}"#, None), Err(AppError::Import(_))));
        assert!(matches!(read(b"not json", None), Err(AppError::Serde(_))));
    }

    #[test]
    fn out_of_range_kdf_params_are_import_errors() {
        let bytes = write(&payload(), Some("pw")).unwrap();
        for (field, cost) in [("mCost", u32::MAX), ("tCost", 0)] {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["kdf"][field] = cost.into();
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(matches!(read(&bytes, Some("pw")), Err(AppError::Import(_))));
        }
    }
}
