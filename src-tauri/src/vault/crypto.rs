use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};

pub fn random_bytes<const N: usize>() -> AppResult<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|_| AppError::Crypto)?;
    Ok(buf)
}

pub fn derive_key(password: &[u8], salt: &[u8; 16]) -> AppResult<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| AppError::Crypto)?;
    Ok(key)
}

pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> AppResult<([u8; 24], Vec<u8>)> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_bytes = random_bytes::<24>()?;
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|_| AppError::Crypto)?;
    Ok((nonce_bytes, ciphertext))
}

pub fn open(key: &[u8; 32], nonce: &[u8; 24], ciphertext: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = XNonce::from_slice(nonce);
    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| AppError::Crypto)?;
    Ok(Zeroizing::new(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trips() {
        let key = derive_key(b"hunter2", &[7u8; 16]).unwrap();
        let (nonce, ct) = seal(&key, b"my-passphrase").unwrap();
        let pt = open(&key, &nonce, &ct).unwrap();
        assert_eq!(pt.as_slice(), b"my-passphrase");
    }

    #[test]
    fn wrong_key_fails() {
        let k1 = derive_key(b"a", &[1u8; 16]).unwrap();
        let k2 = derive_key(b"b", &[1u8; 16]).unwrap();
        let (nonce, ct) = seal(&k1, b"secret").unwrap();
        assert!(matches!(open(&k2, &nonce, &ct), Err(AppError::Crypto)));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = derive_key(b"a", &[1u8; 16]).unwrap();
        let (nonce, mut ct) = seal(&key, b"secret").unwrap();
        ct[0] ^= 0xff;
        assert!(matches!(open(&key, &nonce, &ct), Err(AppError::Crypto)));
    }

    #[test]
    fn same_plaintext_uses_distinct_nonces() {
        let key = derive_key(b"a", &[1u8; 16]).unwrap();
        let (n1, _) = seal(&key, b"x").unwrap();
        let (n2, _) = seal(&key, b"x").unwrap();
        assert_ne!(n1, n2);
    }
}
