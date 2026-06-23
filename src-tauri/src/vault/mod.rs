pub mod crypto;
pub mod model;

use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::store::io;
use model::{KeySource, SealedSecret, VaultFile, VaultStatus};

const KEYRING_SERVICE: &str = "de.headpat.wisp";
const KEYRING_USER: &str = "vault-key";

pub struct Vault {
    path: PathBuf,
    file: VaultFile,
    key: Option<Zeroizing<[u8; 32]>>,
}

impl Vault {
    #[cfg(test)]
    pub fn open_with_key(path: PathBuf, key: Zeroizing<[u8; 32]>) -> AppResult<Self> {
        let file: VaultFile = io::read_json(&path)?;
        Ok(Self { path, file, key: Some(key) })
    }

    pub fn open_from_keychain(path: PathBuf) -> AppResult<Self> {
        let mut file: VaultFile = io::read_json(&path)?;
        let key = load_or_create_keychain_key()?;
        let changed = file.key_source != KeySource::Keychain;
        file.key_source = KeySource::Keychain;
        let vault = Self { path, file, key: Some(key) };
        if changed {
            vault.persist()?;
        }
        Ok(vault)
    }

    // Fallback when no keystore is usable: vault opens locked. The master-password
    // unlock UI is a later phase; the file already records the salt so it can be derived.
    pub fn open_locked(path: PathBuf) -> AppResult<Self> {
        let mut file: VaultFile = io::read_json(&path)?;
        let changed = file.key_source != KeySource::Password || file.kdf_salt.is_none();
        file.key_source = KeySource::Password;
        if file.kdf_salt.is_none() {
            file.kdf_salt = Some(STANDARD.encode(crypto::random_bytes::<16>()?));
        }
        let vault = Self { path, file, key: None };
        if changed {
            vault.persist()?;
        }
        Ok(vault)
    }

    pub fn status(&self) -> VaultStatus {
        match (&self.key, self.file.key_source) {
            (Some(_), _) => VaultStatus::Unlocked,
            (None, KeySource::Password) => VaultStatus::NeedsPassword,
            (None, KeySource::Keychain) => VaultStatus::Locked,
        }
    }

    fn key(&self) -> AppResult<&[u8; 32]> {
        self.key.as_deref().ok_or_else(|| AppError::Vault("vault is locked".into()))
    }

    fn salt(&self) -> AppResult<[u8; 16]> {
        let b64 = self.file.kdf_salt.as_ref().ok_or(AppError::Crypto)?;
        STANDARD.decode(b64).map_err(|_| AppError::Crypto)?.try_into().map_err(|_| AppError::Crypto)
    }

    // Derive the key from a master password and adopt it. With existing secrets the
    // password is verified by decrypting one; with none it simply becomes the key.
    pub fn unlock(&mut self, password: &str) -> AppResult<()> {
        if self.file.key_source != KeySource::Password {
            return Err(AppError::Vault("vault is not password-protected".into()));
        }
        let key = crypto::derive_key(password.as_bytes(), &self.salt()?)?;
        if let Some(sealed) = self.file.secrets.values().next() {
            let nonce: [u8; 24] = STANDARD
                .decode(&sealed.nonce)
                .map_err(|_| AppError::Crypto)?
                .try_into()
                .map_err(|_| AppError::Crypto)?;
            let ciphertext = STANDARD.decode(&sealed.ciphertext).map_err(|_| AppError::Crypto)?;
            crypto::open(&key, &nonce, &ciphertext).map_err(|_| AppError::WrongPassphrase)?;
        }
        self.key = Some(key);
        Ok(())
    }

    // Re-encrypt every secret under a key derived from a new master password + fresh salt.
    pub fn change_master_password(&mut self, new_password: &str) -> AppResult<()> {
        let plaintexts: Vec<(String, Zeroizing<Vec<u8>>)> = self
            .file
            .secrets
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|id| self.get_secret(&id).map(|pt| (id, pt)))
            .collect::<AppResult<_>>()?;
        let salt = crypto::random_bytes::<16>()?;
        let key = crypto::derive_key(new_password.as_bytes(), &salt)?;
        for (id, pt) in &plaintexts {
            let (nonce, ciphertext) = crypto::seal(&key, pt)?;
            self.file.secrets.insert(
                id.clone(),
                SealedSecret {
                    nonce: STANDARD.encode(nonce),
                    ciphertext: STANDARD.encode(ciphertext),
                },
            );
        }
        self.file.key_source = KeySource::Password;
        self.file.kdf_salt = Some(STANDARD.encode(salt));
        self.key = Some(key);
        self.persist()
    }

    fn persist(&self) -> AppResult<()> {
        io::write_json_atomic(&self.path, &self.file)
    }

    pub fn has_secret(&self, id: &str) -> bool {
        self.file.secrets.contains_key(id)
    }

    pub fn set_secret(&mut self, plaintext: &[u8]) -> AppResult<String> {
        let id = Uuid::new_v4().to_string();
        let (nonce, ciphertext) = crypto::seal(self.key()?, plaintext)?;
        self.file.secrets.insert(
            id.clone(),
            SealedSecret { nonce: STANDARD.encode(nonce), ciphertext: STANDARD.encode(ciphertext) },
        );
        self.persist()?;
        Ok(id)
    }

    pub fn delete_secret(&mut self, id: &str) -> AppResult<()> {
        if self.file.secrets.remove(id).is_none() {
            return Err(AppError::NotFound(format!("secret {id}")));
        }
        self.persist()
    }

    pub fn get_secret(&self, id: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        let key = self.key()?;
        let sealed = self
            .file
            .secrets
            .get(id)
            .ok_or_else(|| AppError::NotFound(format!("secret {id}")))?;
        let nonce_vec = STANDARD.decode(&sealed.nonce).map_err(|_| AppError::Crypto)?;
        let ciphertext = STANDARD.decode(&sealed.ciphertext).map_err(|_| AppError::Crypto)?;
        let nonce: [u8; 24] = nonce_vec.try_into().map_err(|_| AppError::Crypto)?;
        crypto::open(key, &nonce, &ciphertext)
    }
}

fn load_or_create_keychain_key() -> AppResult<Zeroizing<[u8; 32]>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| AppError::Keyring(e.to_string()))?;
    match entry.get_password() {
        Ok(hex) => {
            let hex = Zeroizing::new(hex);
            decode_key_hex(&hex)
        }
        Err(keyring::Error::NoEntry) => {
            let key = Zeroizing::new(crypto::random_bytes::<32>()?);
            let hex = Zeroizing::new(key.iter().map(|b| format!("{b:02x}")).collect::<String>());
            entry.set_password(&hex).map_err(|e| AppError::Keyring(e.to_string()))?;
            Ok(key)
        }
        Err(e) => Err(AppError::Keyring(e.to_string())),
    }
}

fn decode_key_hex(hex: &str) -> AppResult<Zeroizing<[u8; 32]>> {
    if hex.len() != 64 {
        return Err(AppError::Crypto);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| AppError::Crypto)?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vault(dir: &std::path::Path) -> Vault {
        Vault::open_with_key(dir.join("vault.enc"), Zeroizing::new([9u8; 32])).unwrap()
    }

    #[test]
    fn set_get_delete_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = test_vault(dir.path());
        let id = v.set_secret(b"s3cr3t").unwrap();
        assert!(v.has_secret(&id));
        assert_eq!(v.get_secret(&id).unwrap().as_slice(), b"s3cr3t");
        v.delete_secret(&id).unwrap();
        assert!(!v.has_secret(&id));
    }

    #[test]
    fn secret_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let mut v = test_vault(dir.path());
            v.set_secret(b"keep-me").unwrap()
        };
        let v2 =
            Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([9u8; 32])).unwrap();
        assert_eq!(v2.get_secret(&id).unwrap().as_slice(), b"keep-me");
    }

    #[test]
    fn wrong_key_cannot_open_secret() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let mut v = test_vault(dir.path());
            v.set_secret(b"keep-me").unwrap()
        };
        let v2 =
            Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([1u8; 32])).unwrap();
        assert!(matches!(v2.get_secret(&id), Err(AppError::Crypto)));
    }

    #[test]
    fn get_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let v = test_vault(dir.path());
        assert!(matches!(v.get_secret("nope"), Err(AppError::NotFound(_))));
    }

    #[test]
    fn open_locked_needs_password_and_refuses_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Vault::open_locked(dir.path().join("vault.enc")).unwrap();
        assert_eq!(v.status(), VaultStatus::NeedsPassword);
        assert!(matches!(v.set_secret(b"x"), Err(AppError::Vault(_))));
        assert!(matches!(v.get_secret("anything"), Err(AppError::Vault(_))));
    }

    #[test]
    fn unlock_with_correct_password_reads_existing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let id = {
            let mut v = Vault::open_locked(path.clone()).unwrap();
            v.unlock("master-pw").unwrap();
            v.set_secret(b"top").unwrap()
        };
        let mut v2 = Vault::open_locked(path.clone()).unwrap();
        assert_eq!(v2.status(), VaultStatus::NeedsPassword);
        v2.unlock("master-pw").unwrap();
        assert_eq!(v2.status(), VaultStatus::Unlocked);
        assert_eq!(v2.get_secret(&id).unwrap().as_slice(), b"top");
    }

    #[test]
    fn unlock_with_wrong_password_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        {
            let mut v = Vault::open_locked(path.clone()).unwrap();
            v.unlock("right").unwrap();
            v.set_secret(b"top").unwrap();
        }
        let mut v2 = Vault::open_locked(path.clone()).unwrap();
        assert!(matches!(v2.unlock("wrong"), Err(AppError::WrongPassphrase)));
        assert_eq!(v2.status(), VaultStatus::NeedsPassword);
    }

    #[test]
    fn change_master_password_reencrypts_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let id = {
            let mut v = Vault::open_locked(path.clone()).unwrap();
            v.unlock("old-pw").unwrap();
            let id = v.set_secret(b"keep").unwrap();
            v.change_master_password("new-pw").unwrap();
            id
        };
        let mut v2 = Vault::open_locked(path.clone()).unwrap();
        assert!(matches!(v2.unlock("old-pw"), Err(AppError::WrongPassphrase)));
        v2.unlock("new-pw").unwrap();
        assert_eq!(v2.get_secret(&id).unwrap().as_slice(), b"keep");
    }
}
