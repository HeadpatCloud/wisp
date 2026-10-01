pub mod bundle;
pub mod export;

#[cfg(test)]
use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroizing;

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
        std::fs::read(path).ok()
    }

    fn icon_exists(&self, rel: &str) -> bool {
        !rel.contains("..") && self.config_dir.join(rel).is_file()
    }
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
