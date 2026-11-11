use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use specta::Type;

use crate::error::AppResult;
use crate::store::io;

// One trusted key: an SSH host key, or a remote desktop or RD Gateway certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostEntry {
    pub key: String,
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

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

    pub fn list(&self) -> Vec<KnownHostEntry> {
        self.entries
            .iter()
            .map(|(key, fingerprint)| {
                let (kind, address) = ["rdp", "rdg", "vnc"]
                    .iter()
                    .find_map(|k| key.strip_prefix(&format!("{k}/")).map(|rest| (*k, rest)))
                    .unwrap_or(("ssh", key.as_str()));
                // The last colon, so an IPv6 address in brackets keeps its own.
                let (host, port) = address.rsplit_once(':').unwrap_or((address, ""));
                KnownHostEntry {
                    key: key.clone(),
                    kind: kind.to_string(),
                    host: host.to_string(),
                    port: port.parse().unwrap_or(0),
                    fingerprint: fingerprint.clone(),
                }
            })
            .collect()
    }

    pub fn forget(&mut self, key: &str) -> AppResult<()> {
        if self.entries.remove(key).is_some() {
            io::write_json_atomic(&self.path, &self.entries)?;
        }
        Ok(())
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
    fn known_hosts_list_reads_every_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        std::fs::write(
            &path,
            r#"{
              "nas.local:22": "SHA256:abc",
              "rdp/win-dev.corp:3389": "SHA256:0a1b",
              "rdg/gw.corp:443": "SHA256:ff00",
              "vnc/[::1]:5900": "no certificate (unencrypted login)"
            }"#,
        )
        .unwrap();
        let mut kh = KnownHosts::load(path.clone()).unwrap();
        let entry = |key: &str, kind: &str, host: &str, port: u16, fp: &str| KnownHostEntry {
            key: key.into(),
            kind: kind.into(),
            host: host.into(),
            port,
            fingerprint: fp.into(),
        };
        assert_eq!(
            kh.list(),
            vec![
                entry("nas.local:22", "ssh", "nas.local", 22, "SHA256:abc"),
                entry("rdg/gw.corp:443", "rdg", "gw.corp", 443, "SHA256:ff00"),
                entry("rdp/win-dev.corp:3389", "rdp", "win-dev.corp", 3389, "SHA256:0a1b"),
                entry("vnc/[::1]:5900", "vnc", "[::1]", 5900, "no certificate (unencrypted login)"),
            ]
        );
        kh.forget("rdp/win-dev.corp:3389").unwrap();
        assert_eq!(KnownHosts::load(path).unwrap().list().len(), 3);
    }

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
