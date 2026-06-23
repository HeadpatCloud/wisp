use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::AppResult;
use crate::store::io;

#[derive(Debug, PartialEq, Eq)]
pub enum HostKeyVerdict {
    Trusted,
    Unknown { fingerprint: String },
    Mismatch { stored: String, offered: String },
}

pub struct KnownHosts {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

impl KnownHosts {
    pub fn load(path: PathBuf) -> AppResult<Self> {
        let entries: BTreeMap<String, String> = io::read_json(&path)?;
        Ok(Self { path, entries })
    }

    pub fn verify(&self, host: &str, port: u16, fingerprint: &str) -> HostKeyVerdict {
        let key = format!("{host}:{port}");
        match self.entries.get(&key) {
            Some(stored) if stored == fingerprint => HostKeyVerdict::Trusted,
            Some(stored) => {
                HostKeyVerdict::Mismatch { stored: stored.clone(), offered: fingerprint.to_string() }
            }
            None => HostKeyVerdict::Unknown { fingerprint: fingerprint.to_string() },
        }
    }

    pub fn record(&mut self, host: &str, port: u16, fingerprint: &str) -> AppResult<()> {
        self.entries.insert(format!("{host}:{port}"), fingerprint.to_string());
        io::write_json_atomic(&self.path, &self.entries)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_then_trusted_after_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut kh = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        assert_eq!(kh.verify("h", 22, "fp1"), HostKeyVerdict::Unknown { fingerprint: "fp1".into() });
        kh.record("h", 22, "fp1").unwrap();
        assert_eq!(kh.verify("h", 22, "fp1"), HostKeyVerdict::Trusted);
    }

    #[test]
    fn detects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let mut kh = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        kh.record("h", 22, "fp1").unwrap();
        assert_eq!(
            kh.verify("h", 22, "fp2"),
            HostKeyVerdict::Mismatch { stored: "fp1".into(), offered: "fp2".into() },
        );
    }

    #[test]
    fn record_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        KnownHosts::load(path.clone()).unwrap().record("h", 2222, "fp").unwrap();
        let kh2 = KnownHosts::load(path).unwrap();
        assert_eq!(kh2.verify("h", 2222, "fp"), HostKeyVerdict::Trusted);
    }

    #[test]
    fn different_port_is_separate() {
        let dir = tempfile::tempdir().unwrap();
        let mut kh = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        kh.record("h", 22, "fp1").unwrap();
        assert_eq!(kh.verify("h", 2222, "fp2"), HostKeyVerdict::Unknown { fingerprint: "fp2".into() });
    }

    #[test]
    fn record_overwrites_on_accept() {
        let dir = tempfile::tempdir().unwrap();
        let mut kh = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        kh.record("h", 22, "old").unwrap();
        kh.record("h", 22, "new").unwrap();
        assert_eq!(kh.verify("h", 22, "new"), HostKeyVerdict::Trusted);
    }
}
