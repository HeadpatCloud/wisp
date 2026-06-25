use std::io::{Read, Write};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use suppaftp::list::File as FtpFile;
use suppaftp::native_tls::TlsConnector;
use suppaftp::types::FileType;
use suppaftp::{NativeTlsConnector, NativeTlsFtpStream};

use crate::error::{AppError, AppResult};
use crate::sftp::{join_remote, sort_entries, SftpEntry};

pub type FtpStream = NativeTlsFtpStream;

pub fn connect(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    secure: bool,
    allow_invalid_cert: bool,
    ignore_hostname: bool,
) -> AppResult<FtpStream> {
    let mut ftp = NativeTlsFtpStream::connect(format!("{host}:{port}"))?;
    if secure {
        let mut builder = TlsConnector::builder();
        // Two separate opt-ins: accept an untrusted/self-signed chain vs. ignore a hostname
        // mismatch. Keeping them apart means "self-signed" doesn't silently also disable the
        // MITM-relevant hostname check.
        if allow_invalid_cert {
            builder.danger_accept_invalid_certs(true);
        }
        if ignore_hostname {
            builder.danger_accept_invalid_hostnames(true);
        }
        let connector = builder.build().map_err(|e| AppError::Ftp(e.to_string()))?;
        ftp = ftp.into_secure(NativeTlsConnector::from(connector), host)?;
    }
    ftp.login(username, password)?;
    ftp.transfer_type(FileType::Binary)?;
    Ok(ftp)
}

fn entry_from(parent: &str, f: &FtpFile) -> SftpEntry {
    let name = f.name().to_string();
    SftpEntry {
        path: join_remote(parent, &name),
        name,
        is_dir: f.is_directory(),
        is_symlink: f.is_symlink(),
        size: f.size() as u64,
        modified: f.modified().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()),
    }
}

pub fn list(ftp: &mut FtpStream, path: &str) -> AppResult<Vec<SftpEntry>> {
    let lines = ftp.list(Some(path))?;
    let mut entries: Vec<SftpEntry> = lines
        .iter()
        .filter_map(|l| FtpFile::from_str(l).ok())
        .map(|f| entry_from(path, &f))
        .collect();
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn exists(ftp: &mut FtpStream, path: &str) -> bool {
    if ftp.size(path).is_ok() {
        return true;
    }
    // SIZE only works on files; fall back to a parent listing so directories count too.
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return false,
    };
    ftp.list(Some(parent))
        .map(|lines| {
            lines.iter().filter_map(|l| FtpFile::from_str(l).ok()).any(|f| f.name() == name)
        })
        .unwrap_or(false)
}

pub fn mkdir(ftp: &mut FtpStream, path: &str) -> AppResult<()> {
    ftp.mkdir(path)?;
    Ok(())
}

pub fn rename(ftp: &mut FtpStream, from: &str, to: &str) -> AppResult<()> {
    ftp.rename(from, to)?;
    Ok(())
}

pub fn remove(ftp: &mut FtpStream, path: &str, is_dir: bool) -> AppResult<()> {
    if is_dir {
        ftp.rmdir(path)?;
    } else {
        ftp.rm(path)?;
    }
    Ok(())
}

pub fn download(
    ftp: &mut FtpStream,
    remote: &str,
    local: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> AppResult<()> {
    let total = ftp.size(remote).unwrap_or(0) as u64;
    // Drive the data stream by hand so a cancel/error sends a real ABOR (which reads the
    // aborted-transfer responses), leaving the control connection in sync. A clean EOF
    // finalizes normally instead.
    let mut stream = ftp.retr_as_stream(remote)?;
    let mut file = std::fs::File::create(local)?;
    let mut buf = [0u8; 65536];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            ftp.abort(stream)?;
            let _ = std::fs::remove_file(local);
            return Err(AppError::Ftp("transfer cancelled".into()));
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = file.write_all(&buf[..n]) {
                    let _ = ftp.abort(stream);
                    let _ = std::fs::remove_file(local);
                    return Err(AppError::Io(e.to_string()));
                }
                done += n as u64;
                progress(done, total);
            }
            Err(e) => {
                let _ = ftp.abort(stream);
                let _ = std::fs::remove_file(local);
                return Err(AppError::Io(e.to_string()));
            }
        }
    }
    ftp.finalize_retr_stream(stream)?;
    Ok(())
}

struct ProgressReader<'a, R, F> {
    inner: R,
    done: u64,
    total: u64,
    cancel: &'a AtomicBool,
    cb: F,
}

impl<R: Read, F: FnMut(u64, u64)> Read for ProgressReader<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Signal EOF on cancel so put_file finishes cleanly and the control connection stays in sync.
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(0);
        }
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        (self.cb)(self.done, self.total);
        Ok(n)
    }
}

pub fn upload(
    ftp: &mut FtpStream,
    local: &str,
    remote: &str,
    cancel: &AtomicBool,
    progress: impl FnMut(u64, u64),
) -> AppResult<()> {
    let total = std::fs::metadata(local)?.len();
    let file = std::fs::File::open(local)?;
    let mut reader = ProgressReader { inner: file, done: 0, total, cancel, cb: progress };
    ftp.put_file(remote, &mut reader)?;
    if cancel.load(Ordering::Relaxed) {
        let _ = ftp.rm(remote);
        return Err(AppError::Ftp("transfer cancelled".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_posix_list_line_into_entry() {
        let f = FtpFile::from_str("-rw-rw-r-- 1 1000 1001 8192 Nov 5 2018 omar.txt").unwrap();
        let e = entry_from("/home/me", &f);
        assert_eq!(e.name, "omar.txt");
        assert_eq!(e.path, "/home/me/omar.txt");
        assert!(!e.is_dir);
        assert_eq!(e.size, 8192);
    }

    #[test]
    fn parses_directory_line() {
        let f = FtpFile::from_str("drwxr-xr-x 2 1000 1001 4096 Nov 5 2018 pub").unwrap();
        let e = entry_from("/", &f);
        assert_eq!(e.path, "/pub");
        assert!(e.is_dir);
    }
}
