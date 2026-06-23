use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Keychain,
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedSecret {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultFile {
    pub version: u32,
    pub key_source: KeySource,
    pub kdf_salt: Option<String>,
    pub secrets: BTreeMap<String, SealedSecret>,
}

impl Default for VaultFile {
    fn default() -> Self {
        VaultFile {
            version: 1,
            key_source: KeySource::Keychain,
            kdf_salt: None,
            secrets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VaultStatus {
    Unlocked,
    NeedsPassword,
    Locked,
}
