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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfParams {
    // ~64 MiB, 3 passes - well above the Argon2 default for new master-password vaults.
    pub const STRONG: KdfParams = KdfParams { m_cost: 65536, t_cost: 3, p_cost: 1 };

    pub fn tuple(self) -> (u32, u32, u32) {
        (self.m_cost, self.t_cost, self.p_cost)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultFile {
    pub version: u32,
    pub key_source: KeySource,
    pub kdf_salt: Option<String>,
    #[serde(default)]
    pub kdf_params: Option<KdfParams>,
    #[serde(default)]
    pub verifier: Option<SealedSecret>,
    pub secrets: BTreeMap<String, SealedSecret>,
}

impl Default for VaultFile {
    fn default() -> Self {
        VaultFile {
            version: 1,
            key_source: KeySource::Keychain,
            kdf_salt: None,
            kdf_params: None,
            verifier: None,
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
