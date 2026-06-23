use serde::Serialize;
use specta::Type;

#[derive(Debug, thiserror::Error, Serialize, Type)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum AppError {
    #[error("io error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serde(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("vault error: {0}")]
    Vault(String),
    #[error("crypto error")]
    Crypto,
    #[error("keyring error: {0}")]
    Keyring(String),
    #[error("ssh error: {0}")]
    Ssh(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("host key unknown for {host}:{port}")]
    HostKeyUnknown { host: String, port: u16, fingerprint: String },
    #[error("host key mismatch for {host}:{port}")]
    HostKeyMismatch { host: String, port: u16, stored: String, offered: String },
    #[error("key is encrypted - passphrase required")]
    PassphraseRequired,
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("sftp error: {0}")]
    Sftp(String),
    #[error("ftp error: {0}")]
    Ftp(String),
    #[error("tunnel error: {0}")]
    Tunnel(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Serde(e.to_string())
    }
}

impl From<russh::Error> for AppError {
    fn from(e: russh::Error) -> Self {
        AppError::Ssh(e.to_string())
    }
}

impl From<russh::keys::Error> for AppError {
    fn from(e: russh::keys::Error) -> Self {
        match e {
            russh::keys::Error::KeyIsEncrypted => AppError::PassphraseRequired,
            russh::keys::Error::SshKey(_) => AppError::WrongPassphrase,
            other => AppError::Ssh(other.to_string()),
        }
    }
}

impl From<russh_sftp::client::error::Error> for AppError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        AppError::Sftp(e.to_string())
    }
}

impl From<suppaftp::FtpError> for AppError {
    fn from(e: suppaftp::FtpError) -> Self {
        AppError::Ftp(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_kind_tag() {
        let json = serde_json::to_value(AppError::NotFound("p1".into())).unwrap();
        assert_eq!(json["kind"], "notFound");
        assert_eq!(json["message"], "p1");
    }

    #[test]
    fn crypto_variant_has_no_message() {
        let json = serde_json::to_value(AppError::Crypto).unwrap();
        assert_eq!(json["kind"], "crypto");
        assert!(json.get("message").is_none());
    }

    #[test]
    fn host_key_mismatch_serializes() {
        let json = serde_json::to_value(AppError::HostKeyMismatch {
            host: "h".into(),
            port: 22,
            stored: "a".into(),
            offered: "b".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "hostKeyMismatch");
        assert_eq!(json["message"]["stored"], "a");
        assert_eq!(json["message"]["offered"], "b");
        assert_eq!(json["message"]["port"], 22);
    }

    #[test]
    fn host_key_unknown_serializes() {
        let json = serde_json::to_value(AppError::HostKeyUnknown {
            host: "h".into(),
            port: 22,
            fingerprint: "fp".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "hostKeyUnknown");
        assert_eq!(json["message"]["fingerprint"], "fp");
    }

    #[test]
    fn passphrase_required_has_no_message() {
        let json = serde_json::to_value(AppError::PassphraseRequired).unwrap();
        assert_eq!(json["kind"], "passphraseRequired");
        assert!(json.get("message").is_none());
    }
}
