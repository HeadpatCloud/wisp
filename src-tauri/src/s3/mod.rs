use std::fmt::Write as _;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::region::Region;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::sftp::{sort_entries, SftpEntry};

type HmacSha256 = Hmac<Sha256>;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn s3err(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("s3: {e}"))
}

pub struct S3Config {
    endpoint: String,
    host: String,
    region: String,
    access_key: String,
    secret_key: Zeroizing<String>,
    path_style: bool,
}

pub fn build_config(
    host: &str,
    port: Option<u16>,
    region: &str,
    use_tls: bool,
    path_style: bool,
    access_key_id: &str,
    secret_key: &str,
) -> AppResult<S3Config> {
    let scheme = if use_tls { "https" } else { "http" };
    let default_port = if use_tls { 443 } else { 80 };
    // The Host header (and the signed host) carries the port only when it's non-default.
    let authority = match port {
        Some(p) if p != default_port => format!("{host}:{p}"),
        _ => host.to_string(),
    };
    Ok(S3Config {
        endpoint: format!("{scheme}://{authority}"),
        host: authority,
        region: region.to_string(),
        access_key: access_key_id.to_string(),
        secret_key: Zeroizing::new(secret_key.to_string()),
        path_style,
    })
}

impl S3Config {
    fn bucket(&self, name: &str) -> AppResult<Box<Bucket>> {
        let region =
            Region::Custom { region: self.region.clone(), endpoint: self.endpoint.clone() };
        let creds = Credentials::new(
            Some(self.access_key.as_str()),
            Some(self.secret_key.as_str()),
            None,
            None,
            None,
        )
        .map_err(s3err)?;
        let b = Bucket::new(name, region, creds).map_err(s3err)?;
        Ok(if self.path_style { b.with_path_style() } else { b })
    }
}

fn leaf(key: &str) -> String {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key).to_string()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

// AWS canonical-URI encoding: everything but the unreserved set and the path separator.
fn aws_uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

// Sign an empty-payload request (GET/HEAD/DELETE/copy-PUT) and return the date + Authorization
// header. rust-s3 mis-signs HEAD/DELETE against Ceph RGW, so those go through this instead.
// `extra` holds any additional signed headers (e.g. x-amz-copy-source), beyond the fixed three.
fn sign_v4(cfg: &S3Config, method: &str, canonical_uri: &str, extra: &[(&str, &str)]) -> (String, String) {
    let now = time::OffsetDateTime::now_utc();
    let amz_date = format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let date = &amz_date[..8];

    let mut headers: Vec<(&str, &str)> = vec![
        ("host", cfg.host.as_str()),
        ("x-amz-content-sha256", EMPTY_SHA256),
        ("x-amz-date", amz_date.as_str()),
    ];
    headers.extend_from_slice(extra);
    headers.sort_by(|a, b| a.0.cmp(b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(";");
    let canonical_request =
        format!("{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{EMPTY_SHA256}");
    let scope = format!("{date}/{region}/s3/aws4_request", region = cfg.region);
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", {
        let mut h = Sha256::new();
        h.update(canonical_request.as_bytes());
        hex(&h.finalize())
    });

    let mut secret_material = Zeroizing::new(Vec::with_capacity(4 + cfg.secret_key.len()));
    secret_material.extend_from_slice(b"AWS4");
    secret_material.extend_from_slice(cfg.secret_key.as_bytes());
    let k_date = Zeroizing::new(hmac(&secret_material, date.as_bytes()));
    let k_region = Zeroizing::new(hmac(&k_date, cfg.region.as_bytes()));
    let k_service = Zeroizing::new(hmac(&k_region, b"s3"));
    let k_signing = Zeroizing::new(hmac(&k_service, b"aws4_request"));
    let signature = hex(&hmac(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={ak}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        ak = cfg.access_key
    );
    (amz_date, authorization)
}

// No redirects (don't resend Authorization elsewhere) and a timeout against a hostile endpoint.
fn signed_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(s3err)
}

// rust-s3's own `list_buckets` signs a path-style request against an *empty* bucket name, which
// S3-compatible servers (Ceph RGW) reject with SignatureDoesNotMatch. So sign a plain `GET /`
// ListBuckets ourselves with SigV4 - the same signing real bucket ops use, just done right.
pub async fn list_buckets(cfg: &S3Config) -> AppResult<Vec<SftpEntry>> {
    let (amz_date, authorization) = sign_v4(cfg, "GET", "/", &[]);
    let mut resp = signed_client()?
        .get(format!("{}/", cfg.endpoint))
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", EMPTY_SHA256)
        .header("authorization", &authorization)
        .send()
        .await
        .map_err(s3err)?;
    let status = resp.status();
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(s3err)? {
        if buf.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(s3err("list buckets response too large"));
        }
        buf.extend_from_slice(&chunk);
    }
    let body = String::from_utf8_lossy(&buf).into_owned();
    if !status.is_success() {
        return Err(s3err(format!("list buckets failed ({status}): {body}")));
    }

    // ListAllMyBucketsResult puts each bucket name in a <Name> tag (Owner uses <DisplayName>/<ID>).
    let mut entries = Vec::new();
    let mut rest = body.as_str();
    while let Some(start) = rest.find("<Name>") {
        let after = &rest[start + 6..];
        let Some(end) = after.find("</Name>") else {
            break;
        };
        let name = after[..end].to_string();
        entries.push(SftpEntry {
            path: name.clone(),
            name,
            is_dir: true,
            is_symlink: false,
            size: 0,
            modified: None,
        });
        rest = &after[end + 7..];
    }
    sort_entries(&mut entries);
    Ok(entries)
}

pub async fn list_objects(cfg: &S3Config, bucket_name: &str, prefix: &str) -> AppResult<Vec<SftpEntry>> {
    let bucket = cfg.bucket(bucket_name)?;
    let results = bucket.list(prefix.to_string(), Some("/".to_string())).await.map_err(s3err)?;
    let mut entries = Vec::new();
    for res in results {
        for cp in res.common_prefixes.into_iter().flatten() {
            entries.push(SftpEntry {
                name: leaf(&cp.prefix),
                path: cp.prefix,
                is_dir: true,
                is_symlink: false,
                size: 0,
                modified: None,
            });
        }
        for obj in res.contents {
            if obj.key == prefix {
                continue; // the folder's own zero-byte marker
            }
            entries.push(SftpEntry {
                name: leaf(&obj.key),
                path: obj.key,
                is_dir: false,
                is_symlink: false,
                size: obj.size,
                modified: None,
            });
        }
    }
    sort_entries(&mut entries);
    Ok(entries)
}

// Wraps a reader so each chunk consumed by the uploader reports cumulative bytes; a cancel
// surfaces as a read error so the upload aborts instead of writing a truncated object.
struct ProgressReader<'a, R, F> {
    inner: R,
    read: u64,
    total: u64,
    cancel: &'a AtomicBool,
    on_progress: F,
}

impl<R: AsyncRead + Unpin, F: FnMut(u64, u64) + Unpin> AsyncRead for ProgressReader<'_, R, F> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.cancel.load(Ordering::Relaxed) {
            return Poll::Ready(Err(std::io::Error::other("transfer cancelled")));
        }
        let before = buf.filled().len();
        let r = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &r {
            let n = (buf.filled().len() - before) as u64;
            if n > 0 {
                this.read += n;
                (this.on_progress)(this.read, this.total);
            }
        }
        r
    }
}

pub async fn upload<F: FnMut(u64, u64) + Send + Unpin>(
    cfg: &S3Config,
    bucket_name: &str,
    key: &str,
    local_path: &str,
    cancel: &AtomicBool,
    on_progress: F,
) -> AppResult<()> {
    let bucket = cfg.bucket(bucket_name)?;
    let file = tokio::fs::File::open(local_path).await.map_err(s3err)?;
    let total = file.metadata().await.map_err(s3err)?.len();
    let mut reader = ProgressReader { inner: file, read: 0, total, cancel, on_progress };
    bucket.put_object_stream(&mut reader, key).await.map_err(s3err)?;
    Ok(())
}

// Stream one object to disk, reporting cumulative bytes (across a whole folder) as it goes.
async fn download_object<F: FnMut(u64, u64) + Send>(
    bucket: &Bucket,
    key: &str,
    dest: &Path,
    transferred: &mut u64,
    total: u64,
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> AppResult<()> {
    let mut stream = bucket.get_object_stream(key).await.map_err(s3err)?;
    let mut file = tokio::fs::File::create(dest).await.map_err(s3err)?;
    while let Some(chunk) = stream.bytes().next().await {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(s3err("transfer cancelled"));
        }
        let chunk = chunk.map_err(s3err)?;
        file.write_all(&chunk).await.map_err(s3err)?;
        *transferred += chunk.len() as u64;
        on_progress(*transferred, total);
    }
    file.flush().await.map_err(s3err)?;
    Ok(())
}

// A file goes to `local_path`; a folder downloads every object under its prefix into the
// `local_path` directory, preserving the sub-prefix structure. Progress is byte counts.
pub async fn download<F: FnMut(u64, u64) + Send>(
    cfg: &S3Config,
    bucket_name: &str,
    key: &str,
    local_path: &str,
    is_dir: bool,
    size: u64,
    cancel: &AtomicBool,
    mut on_progress: F,
) -> AppResult<()> {
    let bucket = cfg.bucket(bucket_name)?;
    if !is_dir {
        // `size` is the object size the listing already reported, so no HEAD round-trip.
        let mut transferred = 0;
        return download_object(&bucket, key, Path::new(local_path), &mut transferred, size, cancel, &mut on_progress).await;
    }

    let prefix = if key.ends_with('/') { key.to_string() } else { format!("{key}/") };
    let results = bucket.list(prefix.clone(), None).await.map_err(s3err)?;
    let objects: Vec<(String, u64)> = results
        .into_iter()
        .flat_map(|r| r.contents)
        .filter(|o| !o.key.ends_with('/')) // skip zero-byte folder markers
        .map(|o| (o.key, o.size))
        .collect();
    let total: u64 = objects.iter().map(|(_, size)| *size).sum();
    let base = Path::new(local_path);
    tokio::fs::create_dir_all(base).await.map_err(s3err)?;
    let mut transferred = 0;
    for (okey, _) in objects {
        if cancel.load(Ordering::Relaxed) {
            return Err(s3err("transfer cancelled"));
        }
        let rel = okey.strip_prefix(&prefix).unwrap_or(&okey);
        let dest = base.join(rel);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(s3err)?;
        }
        download_object(&bucket, &okey, &dest, &mut transferred, total, cancel, &mut on_progress).await?;
    }
    Ok(())
}

// rust-s3 mis-signs DELETE against Ceph RGW (SignatureDoesNotMatch), so sign it ourselves.
pub async fn delete_object(cfg: &S3Config, bucket_name: &str, key: &str) -> AppResult<()> {
    let uri = format!("/{bucket_name}/{}", aws_uri_encode(key));
    let (amz_date, authorization) = sign_v4(cfg, "DELETE", &uri, &[]);
    let resp = signed_client()?
        .delete(format!("{}{uri}", cfg.endpoint))
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", EMPTY_SHA256)
        .header("authorization", &authorization)
        .send()
        .await
        .map_err(s3err)?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(s3err(format!("delete failed ({status}): {body}")));
    }
    Ok(())
}

// S3 has no folders, so a delete of one removes every object under its prefix.
pub async fn delete_prefix(cfg: &S3Config, bucket_name: &str, prefix: &str) -> AppResult<()> {
    let prefix = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
    let bucket = cfg.bucket(bucket_name)?;
    let results = bucket.list(prefix, None).await.map_err(s3err)?;
    for res in results {
        for obj in res.contents {
            bucket.delete_object(&obj.key).await.map_err(s3err)?;
        }
    }
    Ok(())
}

// S3 has no rename: server-side copy then delete the original. rust-s3 mis-signs these against
// Ceph RGW, so the copy (a PUT with x-amz-copy-source) is hand-signed, and delete already is.
pub async fn rename(cfg: &S3Config, bucket_name: &str, from: &str, to: &str) -> AppResult<()> {
    let dest_uri = format!("/{bucket_name}/{}", aws_uri_encode(to));
    let copy_source = format!("/{bucket_name}/{}", aws_uri_encode(from));
    let (amz_date, authorization) =
        sign_v4(cfg, "PUT", &dest_uri, &[("x-amz-copy-source", &copy_source)]);
    let resp = signed_client()?
        .put(format!("{}{dest_uri}", cfg.endpoint))
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", EMPTY_SHA256)
        .header("x-amz-copy-source", &copy_source)
        .header("authorization", &authorization)
        .send()
        .await
        .map_err(s3err)?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    // S3 copy can return 200 with an error embedded in the body, so check both.
    if !status.is_success() || body.contains("<Error") {
        return Err(s3err(format!("copy failed ({status}): {body}")));
    }
    delete_object(cfg, bucket_name, from).await
}

// A folder is just a zero-byte object whose key ends in `/`.
pub async fn create_folder(cfg: &S3Config, bucket_name: &str, prefix: &str) -> AppResult<()> {
    let key = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
    let bucket = cfg.bucket(bucket_name)?;
    bucket.put_object(&key, &[]).await.map_err(s3err)?;
    Ok(())
}
