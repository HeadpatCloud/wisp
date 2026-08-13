use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::AppResult;

// Matches russh-sftp's default max_packet_len, so a request carries as much as the protocol
// allows; the server's advertised limit clamps it lower during negotiation if needed.
const CHUNK: usize = 256 * 1024;
const THROTTLE: u64 = 256 * 1024;
// russh-sftp keeps only one READ in flight per file handle, so a download runs at
// chunk-per-round-trip no matter the bandwidth. Reading disjoint regions through separate
// handles is what puts multiple requests on the wire at once.
const READ_STREAMS: u64 = 8;
// Below this a second handle costs more (extra OPEN round trips) than it saves.
const PARALLEL_MIN: u64 = 2 * 1024 * 1024;

fn remote_join(base: &str, rel: &str) -> String {
    format!("{}/{rel}", base.trim_end_matches('/'))
}

// Half-open [start, end) byte ranges, one per concurrent reader. They must tile the file
// exactly - a gap leaves a hole of zeros and an overlap corrupts what the other wrote.
fn read_regions(total: u64) -> Vec<(u64, u64)> {
    let streams = READ_STREAMS.min(total.div_ceil(CHUNK as u64)).max(1);
    let region = total.div_ceil(streams);
    (0..streams)
        .map(|i| (i * region, ((i + 1) * region).min(total)))
        .filter(|(start, end)| start < end)
        .collect()
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
    on: impl FnMut(u64, u64) + Send,
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
    mut on: impl FnMut(u64, u64) + Send,
) -> AppResult<()> {
    let total = sftp.metadata(remote_path).await?.len();
    if total < PARALLEL_MIN {
        return download_stream(sftp, remote_path, local_path, total, on).await;
    }

    // Sized up front so each worker can write straight into its own region.
    let file = tokio::fs::File::create(local_path).await?;
    file.set_len(total).await?;
    drop(file);

    let done = AtomicU64::new(0);
    on(0, total);
    let on = Mutex::new(on);

    let workers = read_regions(total).into_iter().map(|(start, end)| {
        let (done, on) = (&done, &on);
        async move {
            let mut remote = sftp.open(remote_path).await?;
            remote.seek(SeekFrom::Start(start)).await?;
            let mut local = tokio::fs::OpenOptions::new().write(true).open(local_path).await?;
            local.seek(SeekFrom::Start(start)).await?;

            let mut buf = vec![0u8; CHUNK];
            let mut pos = start;
            let mut since: u64 = 0;
            while pos < end {
                let want = ((end - pos) as usize).min(CHUNK);
                let n = remote.read(&mut buf[..want]).await?;
                if n == 0 {
                    break;
                }
                local.write_all(&buf[..n]).await?;
                pos += n as u64;
                since += n as u64;
                let total_done = done.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                if since >= THROTTLE {
                    since = 0;
                    if let Ok(mut on) = on.lock() {
                        on(total_done, total);
                    }
                }
            }
            local.flush().await?;
            Ok::<(), crate::error::AppError>(())
        }
    });
    futures_util::future::try_join_all(workers).await?;

    if let Ok(mut on) = on.lock() {
        on(total, total);
    }
    Ok(())
}

async fn download_stream(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &str,
    total: u64,
    mut on: impl FnMut(u64, u64),
) -> AppResult<()> {
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
    mut on: impl FnMut(u64, u64) + Send,
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
    fn read_regions_tile_the_file_exactly() {
        for total in [
            PARALLEL_MIN,
            PARALLEL_MIN + 1,
            CHUNK as u64 * 9,
            CHUNK as u64 * 8,
            100 * 1024 * 1024,
            3_000_000_001,
        ] {
            let regions = read_regions(total);
            assert!(!regions.is_empty(), "no regions for {total}");
            assert!(regions.len() as u64 <= READ_STREAMS);
            assert_eq!(regions[0].0, 0, "first region must start at 0 for {total}");
            assert_eq!(regions.last().unwrap().1, total, "last region must end at {total}");
            for w in regions.windows(2) {
                assert_eq!(w[0].1, w[1].0, "gap or overlap at {total}");
            }
            let covered: u64 = regions.iter().map(|(s, e)| e - s).sum();
            assert_eq!(covered, total, "coverage mismatch for {total}");
        }
    }

    // Every region must carry real work, or a worker opens a remote handle for nothing.
    #[test]
    fn read_regions_are_never_empty() {
        for total in [1, CHUNK as u64, CHUNK as u64 + 1, PARALLEL_MIN] {
            assert!(read_regions(total).iter().all(|(s, e)| e > s), "empty region at {total}");
        }
    }

    #[test]
    fn remote_join_trims_trailing_slash() {
        assert_eq!(remote_join("/home/me", "a/b.txt"), "/home/me/a/b.txt");
        assert_eq!(remote_join("/home/me/", "a.txt"), "/home/me/a.txt");
    }
}
