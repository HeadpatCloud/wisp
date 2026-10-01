pub mod apply;
pub mod bundle;
pub mod export;
pub mod plan;

#[cfg(test)]
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, Prefix};

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroizing;

use crate::store::is_icon_path;
use crate::vault::Vault;

pub trait Env {
    fn secret(&self, id: &str) -> Option<Zeroizing<Vec<u8>>>;
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;
    fn icon_exists(&self, rel: &str) -> bool;
}

pub struct LocalEnv<'a> {
    pub vault: &'a Vault,
    pub config_dir: &'a Path,
}

impl Env for LocalEnv<'_> {
    fn secret(&self, id: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.vault.get_secret(id).ok()
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        // The path can come from an imported bundle: a UNC path would send the user's credentials
        // to that server, and an unbounded read is a memory risk. Checked before touching disk.
        let path = Path::new(path);
        let mut components = path.components();
        #[cfg(windows)]
        let on_local_drive = matches!(
            components.next(),
            Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_))
        ) && components.next() == Some(Component::RootDir);
        #[cfg(not(windows))]
        let on_local_drive = components.next() == Some(Component::RootDir);
        if !on_local_drive {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > 1_048_576 {
            return None;
        }
        std::fs::read(path).ok()
    }

    fn icon_exists(&self, rel: &str) -> bool {
        // The path comes from an imported bundle and is later joined onto the config dir.
        is_icon_path(rel) && self.config_dir.join(rel).is_file()
    }
}

// True for Windows paths that leave the local drives: UNC shares, verbatim and device paths, and
// the NT `\??\` form, which parses as a plain rooted path but the OS still resolves.
pub(crate) fn is_network_path(path: &str) -> bool {
    path.starts_with(r"\??\")
        || matches!(
            Path::new(path).components().next(),
            Some(Component::Prefix(p)) if !matches!(p.kind(), Prefix::Disk(_))
        )
}

#[cfg(test)]
#[derive(Default)]
pub struct MapEnv {
    pub secrets: HashMap<String, Vec<u8>>,
    pub files: HashMap<String, Vec<u8>>,
    pub icons: HashSet<String>,
}

#[cfg(test)]
impl Env for MapEnv {
    fn secret(&self, id: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.secrets.get(id).cloned().map(Zeroizing::new)
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }

    fn icon_exists(&self, rel: &str) -> bool {
        self.icons.contains(rel)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportSelection {
    pub group_ids: Vec<String>,
    pub profile_ids: Vec<String>,
    pub sftp_ids: Vec<String>,
    pub s3_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub include_secrets: bool,
    pub include_keys: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub profiles: u32,
    pub secrets: u32,
    pub key_files: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Group,
    Ssh,
    Sftp,
    S3,
}

impl ItemKind {
    pub fn key(self, id: &str) -> String {
        let prefix = match self {
            ItemKind::Group => "group",
            ItemKind::Ssh => "ssh",
            ItemKind::Sftp => "sftp",
            ItemKind::S3 => "s3",
        };
        format!("{prefix}:{id}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    New,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldDiff {
    pub field: String,
    pub label: String,
    pub local: String,
    pub incoming: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalMatch {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub key: String,
    pub kind: ItemKind,
    pub name: String,
    pub status: ItemStatus,
    pub matched: Option<LocalMatch>,
    pub fields: Vec<FieldDiff>,
    pub notes: Vec<String>,
    // What `notes` would say with no local match, for a match the user adds as new instead.
    pub notes_as_new: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportReview {
    pub review_id: String,
    pub items: Vec<ReviewItem>,
    pub unchanged: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReadOutcome {
    NeedsPassword,
    Review { review: ImportReview },
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemDecision {
    pub key: String,
    pub accept: bool,
    pub as_new: bool,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemProblem {
    pub key: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApplySummary {
    pub added: u32,
    pub updated: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_exists_only_for_files_in_the_icons_folder() {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([1u8; 32])).unwrap();
        let env = LocalEnv { vault: &vault, config_dir: dir.path() };
        std::fs::create_dir(dir.path().join("icons")).unwrap();
        let name = "icons/6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c8.png";
        let icon = dir.path().join(name);
        std::fs::write(&icon, b"png").unwrap();

        assert!(env.icon_exists(name));
        assert!(!env.icon_exists("icons/6f1c2a9e-8d0b-4c57-9a3e-2b7d5e41f0c9.png"));
        assert!(!env.icon_exists(""));
        assert!(!env.icon_exists(icon.to_str().unwrap()));

        std::fs::create_dir(dir.path().join("icons").join("a")).unwrap();
        std::fs::create_dir(dir.path().join("keys")).unwrap();
        for other in [
            "vault.enc",
            "profiles.json",
            "keys/x",
            "icons/a.png",
            "icons/a/b.png",
            "icons/../vault.enc",
        ] {
            std::fs::write(dir.path().join(other), b"x").unwrap();
            assert!(dir.path().join(other).is_file());
            assert!(!env.icon_exists(other), "{other}");
        }

        let dir_name = dir.path().file_name().unwrap().to_str().unwrap();
        let outside = format!("../{dir_name}/{name}");
        assert!(dir.path().join(&outside).is_file());
        assert!(!env.icon_exists(&outside));

        #[cfg(windows)]
        {
            let rooted: std::path::PathBuf = icon
                .components()
                .filter(|c| !matches!(c, std::path::Component::Prefix(_)))
                .collect();
            let rooted = rooted.to_str().unwrap();
            assert!(rooted.starts_with('\\'));
            let forward = icon.to_str().unwrap().replace('\\', "/");
            for path in [rooted.to_string(), rooted.replace('\\', "/"), forward] {
                assert!(dir.path().join(&path).is_file());
                assert!(!env.icon_exists(&path));
            }

            // Other spellings Windows resolves to the same file.
            let spellings = [
                name.to_uppercase(),
                name.replace('/', "\\"),
                format!("{name}."),
                format!("{name}::$DATA"),
            ];
            for path in &spellings {
                assert!(dir.path().join(path).is_file(), "{path}");
            }
            let accepted: Vec<_> = spellings.iter().filter(|p| env.icon_exists(p)).collect();
            assert!(accepted.is_empty(), "{accepted:?}");
        }
    }

    #[test]
    fn read_file_only_reads_small_files_on_a_local_drive() {
        let dir = tempfile::tempdir().unwrap();
        let vault =
            Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([1u8; 32])).unwrap();
        let env = LocalEnv { vault: &vault, config_dir: dir.path() };
        let key = dir.path().join("id");
        std::fs::write(&key, b"KEY").unwrap();
        assert_eq!(env.read_file(key.to_str().unwrap()), Some(b"KEY".to_vec()));

        assert!(Path::new("Cargo.toml").is_file());
        assert!(env.read_file("Cargo.toml").is_none());

        let big = dir.path().join("big");
        std::fs::write(&big, vec![0u8; 1_048_576]).unwrap();
        assert_eq!(env.read_file(big.to_str().unwrap()).map(|b| b.len()), Some(1_048_576));
        std::fs::write(&big, vec![0u8; 1_048_577]).unwrap();
        assert!(env.read_file(big.to_str().unwrap()).is_none());

        assert!(env.read_file(dir.path().to_str().unwrap()).is_none());

        #[cfg(windows)]
        {
            let verbatim = format!("\\\\?\\{}", key.to_str().unwrap());
            assert!(Path::new(&verbatim).is_file());
            for path in
                ["\\\\localhost\\nonexistent-share\\x", "\\\\?\\C:\\Windows\\win.ini", &verbatim]
            {
                assert!(env.read_file(path).is_none());
            }
        }
    }
}
