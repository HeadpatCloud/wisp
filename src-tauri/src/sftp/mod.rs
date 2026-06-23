pub mod transfer;

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use serde::Serialize;
use specta::Type;

use crate::error::AppResult;
use crate::ssh::client::SshHandle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    #[specta(type = specta_typescript::Number)]
    pub size: u64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub modified: Option<u64>,
}

pub(crate) fn join_remote(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

fn modified_secs(md: &FileAttributes) -> Option<u64> {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()))
}

fn entry_from(parent: &str, name: &str, md: &FileAttributes) -> SftpEntry {
    SftpEntry {
        name: name.to_string(),
        path: join_remote(parent, name),
        is_dir: md.is_dir(),
        is_symlink: md.is_symlink(),
        size: md.len(),
        modified: modified_secs(md),
    }
}

pub(crate) fn sort_entries(entries: &mut [SftpEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub async fn open_sftp(handle: &SshHandle) -> AppResult<SftpSession> {
    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = SftpSession::new(channel.into_stream()).await?;
    Ok(sftp)
}

pub async fn list(sftp: &SftpSession, path: &str) -> AppResult<Vec<SftpEntry>> {
    let read = sftp.read_dir(path).await?;
    let mut entries: Vec<SftpEntry> =
        read.map(|e| entry_from(path, &e.file_name(), &e.metadata())).collect();
    sort_entries(&mut entries);
    Ok(entries)
}

pub async fn stat(sftp: &SftpSession, path: &str) -> AppResult<SftpEntry> {
    let md = sftp.metadata(path).await?;
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string();
    Ok(SftpEntry {
        name,
        path: path.to_string(),
        is_dir: md.is_dir(),
        is_symlink: md.is_symlink(),
        size: md.len(),
        modified: modified_secs(&md),
    })
}

pub async fn mkdir(sftp: &SftpSession, path: &str) -> AppResult<()> {
    sftp.create_dir(path).await?;
    Ok(())
}

pub async fn rename(sftp: &SftpSession, from: &str, to: &str) -> AppResult<()> {
    sftp.rename(from, to).await?;
    Ok(())
}

pub async fn remove(sftp: &SftpSession, path: &str, is_dir: bool) -> AppResult<()> {
    if is_dir {
        sftp.remove_dir(path).await?;
    } else {
        sftp.remove_file(path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(is_dir: bool) -> FileAttributes {
        let mut a = FileAttributes::empty();
        a.size = Some(if is_dir { 0 } else { 42 });
        a.permissions = Some(if is_dir { 0o040000 | 0o755 } else { 0o100000 | 0o644 });
        a
    }

    #[test]
    fn join_remote_handles_root_and_nested() {
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/home/me", "f.txt"), "/home/me/f.txt");
        assert_eq!(join_remote("/home/me/", "f.txt"), "/home/me/f.txt");
    }

    #[test]
    fn entry_from_maps_fields() {
        let e = entry_from("/home/me", "f.txt", &attrs(false));
        assert_eq!(e.path, "/home/me/f.txt");
        assert!(!e.is_dir);
        assert_eq!(e.size, 42);
    }

    #[test]
    fn sort_puts_dirs_first_then_case_insensitive_name() {
        let mut v = vec![
            entry_from("/", "Zeta", &attrs(false)),
            entry_from("/", "alpha", &attrs(false)),
            entry_from("/", "Mid", &attrs(true)),
        ];
        sort_entries(&mut v);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Mid", "alpha", "Zeta"]);
    }
}
