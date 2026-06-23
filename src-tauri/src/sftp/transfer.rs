use std::path::{Path, PathBuf};

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::AppResult;

const CHUNK: usize = 64 * 1024;
const THROTTLE: u64 = 256 * 1024;

fn remote_join(base: &str, rel: &str) -> String {
    format!("{}/{rel}", base.trim_end_matches('/'))
}

// Walk a local directory tree, returning (absolute path, '/'-relative path) for every
// file plus the relative subdirectories to create remotely (parents before children).
fn walk_local(root: &Path) -> std::io::Result<(Vec<(PathBuf, String)>, Vec<String>)> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((abs, rel)) = stack.pop() {
        for entry in std::fs::read_dir(&abs)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let ft = entry.file_type()?;
            if ft.is_dir() {
                dirs.push(child_rel.clone());
                stack.push((entry.path(), child_rel));
            } else if ft.is_file() {
                files.push((entry.path(), child_rel));
            }
        }
    }
    dirs.sort();
    Ok((files, dirs))
}

pub async fn upload(
    sftp: &SftpSession,
    local_path: &str,
    remote_path: &str,
    on: impl FnMut(u64, u64),
) -> AppResult<()> {
    if tokio::fs::metadata(local_path).await?.is_dir() {
        upload_dir(sftp, local_path, remote_path, on).await
    } else {
        upload_file(sftp, local_path, remote_path, on).await
    }
}

async fn upload_file(
    sftp: &SftpSession,
    local_path: &str,
    remote_path: &str,
    mut on: impl FnMut(u64, u64),
) -> AppResult<()> {
    let mut local = tokio::fs::File::open(local_path).await?;
    let total = local.metadata().await?.len();
    let mut remote = sftp.create(remote_path).await?;

    let mut buf = vec![0u8; CHUNK];
    let mut done: u64 = 0;
    let mut last: u64 = 0;
    on(0, total);
    loop {
        let n = local.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        remote.write_all(&buf[..n]).await?;
        done += n as u64;
        if done - last >= THROTTLE {
            last = done;
            on(done, total);
        }
    }
    remote.flush().await?;
    remote.shutdown().await?;
    on(done, total);
    Ok(())
}

async fn upload_dir(
    sftp: &SftpSession,
    local_root: &str,
    remote_root: &str,
    mut on: impl FnMut(u64, u64),
) -> AppResult<()> {
    let (files, dirs) = walk_local(Path::new(local_root))?;
    let _ = sftp.create_dir(remote_root).await;
    for rel in &dirs {
        let _ = sftp.create_dir(remote_join(remote_root, rel)).await;
    }
    let mut total = 0u64;
    for (abs, _) in &files {
        total += tokio::fs::metadata(abs).await?.len();
    }
    on(0, total);
    let mut done = 0u64;
    for (abs, rel) in &files {
        let base = done;
        upload_file(sftp, &abs.to_string_lossy(), &remote_join(remote_root, rel), |t, _| {
            on(base + t, total)
        })
        .await?;
        done += tokio::fs::metadata(abs).await?.len();
    }
    on(done, total);
    Ok(())
}

pub async fn download(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &str,
    on: impl FnMut(u64, u64),
) -> AppResult<()> {
    if sftp.metadata(remote_path).await?.is_dir() {
        download_dir(sftp, remote_path, local_path, on).await
    } else {
        let result = download_file(sftp, remote_path, local_path, on).await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(local_path).await;
        }
        result
    }
}

async fn download_file(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &str,
    mut on: impl FnMut(u64, u64),
) -> AppResult<()> {
    let total = sftp.metadata(remote_path).await?.len();
    let mut remote = sftp.open(remote_path).await?;
    let mut local = tokio::fs::File::create(local_path).await?;

    let mut buf = vec![0u8; CHUNK];
    let mut done: u64 = 0;
    let mut last: u64 = 0;
    on(0, total);
    loop {
        let n = remote.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        local.write_all(&buf[..n]).await?;
        done += n as u64;
        if done - last >= THROTTLE {
            last = done;
            on(done, total);
        }
    }
    local.flush().await?;
    on(done, total);
    Ok(())
}

async fn download_dir(
    sftp: &SftpSession,
    remote_root: &str,
    local_root: &str,
    mut on: impl FnMut(u64, u64),
) -> AppResult<()> {
    tokio::fs::create_dir_all(local_root).await?;
    let mut queue = vec![(remote_root.to_string(), PathBuf::from(local_root))];
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    while let Some((rdir, ldir)) = queue.pop() {
        for entry in sftp.read_dir(&rdir).await? {
            let name = entry.file_name();
            let md = entry.metadata();
            let rpath = remote_join(&rdir, &name);
            let lpath = ldir.join(&name);
            if md.is_dir() {
                tokio::fs::create_dir_all(&lpath).await?;
                queue.push((rpath, lpath));
            } else if !md.is_symlink() {
                files.push((rpath, lpath, md.len()));
            }
        }
    }
    let total: u64 = files.iter().map(|(_, _, s)| s).sum();
    on(0, total);
    let mut done = 0u64;
    for (rpath, lpath, size) in &files {
        let base = done;
        download_file(sftp, rpath, &lpath.to_string_lossy(), |t, _| on(base + t, total)).await?;
        done += size;
    }
    on(done, total);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn walk_local_collects_files_and_dirs_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        fs::create_dir_all(root.join("sub/inner")).unwrap();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join("sub/b.txt"), b"bb").unwrap();
        fs::write(root.join("sub/inner/c.txt"), b"ccc").unwrap();

        let (files, dirs) = walk_local(&root).unwrap();
        let mut rels: Vec<_> = files.iter().map(|(_, r)| r.clone()).collect();
        rels.sort();
        assert_eq!(rels, ["a.txt", "sub/b.txt", "sub/inner/c.txt"]);
        assert_eq!(dirs, ["sub", "sub/inner"]);
    }

    #[test]
    fn remote_join_trims_trailing_slash() {
        assert_eq!(remote_join("/home/me", "a/b.txt"), "/home/me/a/b.txt");
        assert_eq!(remote_join("/home/me/", "a.txt"), "/home/me/a.txt");
    }
}
