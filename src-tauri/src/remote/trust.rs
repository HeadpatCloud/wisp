use sha2::{Digest, Sha256};

use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::ssh::known_hosts::HostKeyVerdict;

#[derive(Clone, Copy)]
pub enum Scheme {
    Vnc,
}

pub const UNENCRYPTED: &str = "no certificate (unencrypted login)";

pub fn fingerprint(cert_der: &[u8]) -> String {
    format!("SHA256:{:x}", Sha256::digest(cert_der))
}

fn lookup(
    known: &KnownHostsState,
    scheme: Scheme,
    host: &str,
    port: u16,
    fingerprint: &str,
) -> AppResult<(String, HostKeyVerdict)> {
    let prefix = match scheme {
        Scheme::Vnc => "vnc/",
    };
    let host = format!("{prefix}{}", crate::net::normalize_host(host));
    let verdict = {
        let kh = known
            .0
            .lock()
            .map_err(|_| AppError::Internal("known_hosts lock poisoned".into()))?;
        kh.verify(&host, port, fingerprint)
    };
    Ok((host, verdict))
}

pub fn check(
    known: &KnownHostsState,
    scheme: Scheme,
    host: &str,
    port: u16,
    cert_der: &[u8],
) -> AppResult<()> {
    let (host, verdict) = lookup(known, scheme, host, port, &fingerprint(cert_der))?;
    match verdict {
        HostKeyVerdict::Trusted => Ok(()),
        HostKeyVerdict::Unknown { fingerprint } => {
            Err(AppError::HostKeyUnknown { host, port, fingerprint })
        }
        HostKeyVerdict::Mismatch { stored, offered } => {
            Err(AppError::HostKeyMismatch { host, port, stored, offered })
        }
    }
}

// For a login that runs without a certificate. A host that never showed one is let through
// unasked; one that did must not drop it unnoticed.
pub fn check_unencrypted(
    known: &KnownHostsState,
    scheme: Scheme,
    host: &str,
    port: u16,
) -> AppResult<()> {
    match lookup(known, scheme, host, port, UNENCRYPTED)? {
        (host, HostKeyVerdict::Mismatch { stored, offered }) => {
            Err(AppError::HostKeyMismatch { host, port, stored, offered })
        }
        _ => Ok(()),
    }
}

// Whether the host has shown a certificate that was accepted; an accepted unencrypted login is
// not one.
pub fn has_certificate_pin(
    known: &KnownHostsState,
    scheme: Scheme,
    host: &str,
    port: u16,
) -> AppResult<bool> {
    let (_, verdict) = lookup(known, scheme, host, port, UNENCRYPTED)?;
    Ok(matches!(verdict, HostKeyVerdict::Mismatch { .. }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::ssh::known_hosts::KnownHosts;

    fn known(dir: &tempfile::TempDir) -> KnownHostsState {
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        KnownHostsState(Arc::new(Mutex::new(hosts)))
    }

    #[test]
    fn fingerprint_is_prefixed_sha256_hex() {
        assert_eq!(
            fingerprint(b"abc"),
            "SHA256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
    }

    #[test]
    fn unknown_certificate_reports_prefixed_host() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        match check(&known, Scheme::Vnc, "h", 5900, b"cert") {
            Err(AppError::HostKeyUnknown { host, port, fingerprint: offered }) => {
                assert_eq!(host, "vnc/h");
                assert_eq!(port, 5900);
                assert_eq!(offered, fingerprint(b"cert"));
            }
            other => panic!("expected HostKeyUnknown, got {other:?}"),
        }
    }

    #[test]
    fn recorded_certificate_is_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("vnc/h", 5900, &fingerprint(b"cert")).unwrap();
        assert!(check(&known, Scheme::Vnc, "h", 5900, b"cert").is_ok());
    }

    #[test]
    fn different_certificate_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("vnc/h", 5900, &fingerprint(b"cert")).unwrap();
        match check(&known, Scheme::Vnc, "h", 5900, b"other") {
            Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
                assert_eq!(host, "vnc/h");
                assert_eq!(port, 5900);
                assert_eq!(stored, fingerprint(b"cert"));
                assert_eq!(offered, fingerprint(b"other"));
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
    }

    #[test]
    fn bracketed_ipv6_shares_the_bare_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("vnc/::1", 5900, &fingerprint(b"cert")).unwrap();
        assert!(check(&known, Scheme::Vnc, "[::1]", 5900, b"cert").is_ok());
    }

    #[test]
    fn unknown_bracketed_ipv6_reports_bare_host() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        assert!(matches!(
            check(&known, Scheme::Vnc, "[::1]", 5900, b"cert"),
            Err(AppError::HostKeyUnknown { host, .. }) if host == "vnc/::1"
        ));
    }

    #[test]
    fn unencrypted_login_is_fine_without_a_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        assert!(check_unencrypted(&known, Scheme::Vnc, "h", 5900).is_ok());
        assert!(matches!(
            check(&known, Scheme::Vnc, "h", 5900, b"cert"),
            Err(AppError::HostKeyUnknown { .. })
        ));
    }

    #[test]
    fn unencrypted_login_after_a_certificate_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("vnc/::1", 5900, &fingerprint(b"cert")).unwrap();
        match check_unencrypted(&known, Scheme::Vnc, "[::1]", 5900) {
            Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
                assert_eq!(host, "vnc/::1");
                assert_eq!(port, 5900);
                assert_eq!(stored, fingerprint(b"cert"));
                assert_eq!(offered, "no certificate (unencrypted login)");
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
        assert!(check_unencrypted(&known, Scheme::Vnc, "::1", 5901).is_ok());
    }

    #[test]
    fn accepted_unencrypted_login_holds_until_a_certificate_appears() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("vnc/h", 5900, UNENCRYPTED).unwrap();
        assert!(check_unencrypted(&known, Scheme::Vnc, "h", 5900).is_ok());
        match check(&known, Scheme::Vnc, "h", 5900, b"cert") {
            Err(AppError::HostKeyMismatch { stored, offered, .. }) => {
                assert_eq!(stored, UNENCRYPTED);
                assert_eq!(offered, fingerprint(b"cert"));
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
    }

    #[test]
    fn only_a_recorded_certificate_is_a_certificate_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        assert!(!has_certificate_pin(&known, Scheme::Vnc, "::1", 5900).unwrap());
        known.0.lock().unwrap().record("vnc/::1", 5900, &fingerprint(b"cert")).unwrap();
        assert!(has_certificate_pin(&known, Scheme::Vnc, "::1", 5900).unwrap());
        assert!(has_certificate_pin(&known, Scheme::Vnc, "[::1]", 5900).unwrap());
        assert!(!has_certificate_pin(&known, Scheme::Vnc, "::1", 5901).unwrap());
        known.0.lock().unwrap().record("vnc/::1", 5900, UNENCRYPTED).unwrap();
        assert!(!has_certificate_pin(&known, Scheme::Vnc, "::1", 5900).unwrap());
    }

    #[test]
    fn ssh_entry_does_not_satisfy_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        known.0.lock().unwrap().record("h", 5900, &fingerprint(b"cert")).unwrap();
        assert!(matches!(
            check(&known, Scheme::Vnc, "h", 5900, b"cert"),
            Err(AppError::HostKeyUnknown { host, .. }) if host == "vnc/h"
        ));
    }
}
