use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex as TokioMutex;

use crate::commands::ssh_cmds::{self, KnownHostsState, Sessions};
use crate::error::{AppError, AppResult};
use crate::sftp::{self, transfer, Sftp, SftpEntry};
use crate::ssh::client::SshHandle;
use crate::store::model::{AuthMethod, ProfileKey};
use crate::store::Store;
use crate::vault::Vault;

// A half-open connection never answers, so the liveness probe needs its own deadline.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    #[specta(type = specta_typescript::Number)]
    pub transferred: u64,
    #[specta(type = specta_typescript::Number)]
    pub total: u64,
}

#[derive(Default)]
pub struct SftpSessions(pub TokioMutex<HashMap<String, Arc<Sftp>>>);

#[derive(Default)]
pub struct Transfers(pub TokioMutex<HashMap<String, tokio::task::AbortHandle>>);

// Holds the SSH connection alive for a standalone (no-terminal) SFTP session; dropping
// it closes the connection. The terminal SFTP path keeps the handle in `Sessions` instead.
pub struct SftpConn {
    #[allow(dead_code)]
    handle: Arc<SshHandle>,
    #[allow(dead_code)]
    bastions: Vec<SshHandle>,
    reconnect: Reconnect,
}

// How to dial a standalone session again once its connection drops. Only ids and vault
// references, never a plaintext secret.
#[derive(Clone)]
enum Reconnect {
    Profile(String),
    Saved(String),
    Adhoc {
        host: String,
        port: u16,
        username: String,
        auth_method: AuthMethod,
        keys: Vec<ProfileKey>,
        secret_id: Option<String>,
    },
}

#[derive(Default)]
pub struct SftpConns {
    map: TokioMutex<HashMap<String, SftpConn>>,
    // One reconnect at a time: sessions failing together dial once, and a tab closed
    // mid-reconnect can't be brought back to life afterwards.
    heal: TokioMutex<()>,
}

struct Ctx<'a> {
    store: &'a StdMutex<Store>,
    vault: &'a StdMutex<Vault>,
    known: &'a KnownHostsState,
    sessions: &'a Sessions,
    sftps: &'a SftpSessions,
    conns: &'a SftpConns,
}

impl<'a> Ctx<'a> {
    fn new(app: &'a AppHandle) -> Self {
        Self {
            store: app.state::<StdMutex<Store>>().inner(),
            vault: app.state::<StdMutex<Vault>>().inner(),
            known: app.state::<KnownHostsState>().inner(),
            sessions: app.state::<Sessions>().inner(),
            sftps: app.state::<SftpSessions>().inner(),
            conns: app.state::<SftpConns>().inner(),
        }
    }
}

// Dial a standalone session's connection and cache it under `id`, replacing whatever was there.
async fn open_standalone(ctx: &Ctx<'_>, id: &str, reconnect: Reconnect) -> AppResult<Arc<Sftp>> {
    let (handle, bastions) = match &reconnect {
        Reconnect::Profile(profile_id) => {
            let (handle, bastions, _forwards) =
                ssh_cmds::connect_via_chain(ctx.store, ctx.vault, ctx.known, profile_id).await?;
            (handle, bastions)
        }
        Reconnect::Saved(profile_id) => {
            let profile = {
                let s =
                    ctx.store.lock().map_err(|_| AppError::Internal("store lock poisoned".into()))?;
                s.sftp_profiles()
                    .into_iter()
                    .find(|p| &p.id == profile_id)
                    .ok_or_else(|| AppError::NotFound(format!("sftp profile {profile_id}")))?
            };
            let secret = match &profile.secret_id {
                Some(id) => Some(ssh_cmds::secret_string(ctx.vault, id)?),
                None => None,
            };
            let resolved = ssh_cmds::resolve_keys(ctx.vault, &profile.keys)?;
            let handle = ssh_cmds::connect_adhoc(
                ctx.known,
                &profile.host,
                profile.port,
                &profile.username,
                profile.auth_method,
                &resolved,
                secret,
            )
            .await?;
            (handle, Vec::new())
        }
        Reconnect::Adhoc { host, port, username, auth_method, keys, secret_id } => {
            // Passwords and key passphrases live in the vault; the caller only holds references.
            let secret = match secret_id {
                Some(id) => Some(ssh_cmds::secret_string(ctx.vault, id)?),
                None => None,
            };
            let resolved = ssh_cmds::resolve_keys(ctx.vault, keys)?;
            let handle = ssh_cmds::connect_adhoc(
                ctx.known,
                host,
                *port,
                username,
                *auth_method,
                &resolved,
                secret,
            )
            .await?;
            (handle, Vec::new())
        }
    };
    let handle = Arc::new(handle);
    let sftp = Arc::new(sftp::open_sftp(&handle).await?);
    ctx.sftps.0.lock().await.insert(id.to_string(), sftp.clone());
    ctx.conns.map.lock().await.insert(id.to_string(), SftpConn { handle, bastions, reconnect });
    Ok(sftp)
}

// Open an SFTP session (no PTY) on a fresh connection and pre-cache it in SftpSessions
// so the existing sftp_* commands resolve it by id.
async fn register_standalone(ctx: &Ctx<'_>, reconnect: Reconnect) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    open_standalone(ctx, &id, reconnect).await?;
    Ok(id)
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_connect(app: AppHandle, profile_id: String) -> AppResult<String> {
    register_standalone(&Ctx::new(&app), Reconnect::Profile(profile_id)).await
}

// Connect a saved SFTP-only profile. Resolving it here means the tab holds just an id, so a
// later edit is picked up on reconnect and no vault reference is duplicated into session state.
#[tauri::command]
#[specta::specta]
pub async fn sftp_connect_saved(app: AppHandle, profile_id: String) -> AppResult<String> {
    register_standalone(&Ctx::new(&app), Reconnect::Saved(profile_id)).await
}

// Same as sftp_connect but for a one-off host with no saved profile (no jump chain).
#[tauri::command]
#[specta::specta]
pub async fn sftp_connect_adhoc(
    app: AppHandle,
    host: String,
    port: u16,
    username: String,
    auth_method: AuthMethod,
    keys: Vec<ProfileKey>,
    secret_id: Option<String>,
) -> AppResult<String> {
    let reconnect = Reconnect::Adhoc { host, port, username, auth_method, keys, secret_id };
    register_standalone(&Ctx::new(&app), reconnect).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_disconnect(
    sftps: State<'_, SftpSessions>,
    conns: State<'_, SftpConns>,
    session_id: String,
) -> AppResult<()> {
    let _heal = conns.heal.lock().await;
    sftps.0.lock().await.remove(&session_id);
    conns.map.lock().await.remove(&session_id);
    Ok(())
}

// Get the cached SFTP session for a connection, or open + cache one on the session's Arc<Handle>.
async fn sftp_for(ctx: &Ctx<'_>, session_id: &str) -> AppResult<Arc<Sftp>> {
    if let Some(s) = ctx.sftps.0.lock().await.get(session_id) {
        return Ok(s.clone());
    }
    let handle = {
        let map = ctx.sessions.0.lock().await;
        map.get(session_id)
            .map(|s| s.handle.clone())
            .ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?
    };
    let sftp = Arc::new(sftp::open_sftp(&handle).await?);
    ctx.sftps.0.lock().await.insert(session_id.to_string(), sftp.clone());
    Ok(sftp)
}

// A refusal from the server still proves the channel answers; any other failure, or no
// reply at all, means the session is gone. A probe on a stream that already ended can sit
// behind russh-sftp's writer for the whole timeout, so that case isn't probed at all.
async fn responsive(sftp: &Sftp) -> bool {
    !sftp.is_closed()
        && matches!(
            tokio::time::timeout(PROBE_TIMEOUT, sftp.canonicalize(".")).await,
            Ok(Ok(_) | Err(russh_sftp::client::error::Error::Status(_)))
        )
}

async fn heal(ctx: &Ctx<'_>, session_id: &str, dead: &Arc<Sftp>) -> AppResult<Arc<Sftp>> {
    let _heal = ctx.conns.heal.lock().await;
    match ctx.sftps.0.lock().await.get(session_id) {
        None => return Err(AppError::NotFound(format!("session {session_id}"))),
        Some(current) if !Arc::ptr_eq(current, dead) => return Ok(current.clone()),
        Some(_) => {}
    }
    let reconnect = ctx.conns.map.lock().await.get(session_id).map(|c| c.reconnect.clone());
    if let Some(reconnect) = reconnect {
        return open_standalone(ctx, session_id, reconnect).await;
    }

    // Terminal-attached: the channel rides the terminal's connection, which only the
    // terminal can re-establish. A fresh channel on a still-live connection is fine.
    let handle = ctx
        .sessions
        .0
        .lock()
        .await
        .get(session_id)
        .map(|s| s.handle.clone())
        .ok_or_else(|| AppError::NotFound(format!("session {session_id}")))?;
    let lost = || AppError::Sftp("connection lost, reconnect the terminal".into());
    if handle.is_closed() {
        return Err(lost());
    }
    let sftp = Arc::new(
        tokio::time::timeout(PROBE_TIMEOUT, sftp::open_sftp(&handle)).await.map_err(|_| lost())??,
    );
    ctx.sftps.0.lock().await.insert(session_id.to_string(), sftp.clone());
    Ok(sftp)
}

// A dropped connection only shows once something is sent over it, so an operation that
// fails on a session that no longer answers gets one reconnect and a retry.
async fn with_session<T, F, Fut>(ctx: &Ctx<'_>, session_id: &str, op: F) -> AppResult<T>
where
    F: Fn(Arc<Sftp>) -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let attempt = |sftp: Arc<Sftp>| {
        let op = &op;
        async move {
            tokio::select! {
                biased;
                result = op(sftp.clone()) => result,
                _ = sftp.closed() => Err(AppError::Sftp("connection lost".into())),
            }
        }
    };
    let sftp = sftp_for(ctx, session_id).await?;
    let result = attempt(sftp.clone()).await;
    if result.is_ok() || responsive(&sftp).await {
        return result;
    }
    attempt(heal(ctx, session_id, &sftp).await?).await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_list(
    app: AppHandle,
    session_id: String,
    path: String,
) -> AppResult<Vec<SftpEntry>> {
    with_session(&Ctx::new(&app), &session_id, |sftp| {
        let path = &path;
        async move { sftp::list(&sftp, path).await }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_stat(app: AppHandle, session_id: String, path: String) -> AppResult<SftpEntry> {
    with_session(&Ctx::new(&app), &session_id, |sftp| {
        let path = &path;
        async move { sftp::stat(&sftp, path).await }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_mkdir(app: AppHandle, session_id: String, path: String) -> AppResult<()> {
    with_session(&Ctx::new(&app), &session_id, |sftp| {
        let path = &path;
        async move { sftp::mkdir(&sftp, path).await }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_rename(
    app: AppHandle,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    with_session(&Ctx::new(&app), &session_id, |sftp| {
        let (from, to) = (&from, &to);
        async move { sftp::rename(&sftp, from, to).await }
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_remove(
    app: AppHandle,
    session_id: String,
    path: String,
    is_dir: bool,
) -> AppResult<()> {
    with_session(&Ctx::new(&app), &session_id, |sftp| {
        let path = &path;
        async move { sftp::remove(&sftp, path, is_dir).await }
    })
    .await
}

async fn run_tracked(
    transfers: &State<'_, Transfers>,
    transfer_id: String,
    fut: impl std::future::Future<Output = AppResult<()>> + Send + 'static,
) -> AppResult<()> {
    let task = tokio::spawn(fut);
    transfers.0.lock().await.insert(transfer_id.clone(), task.abort_handle());
    let res = task.await;
    transfers.0.lock().await.remove(&transfer_id);
    match res {
        Ok(r) => r,
        Err(_) => Err(AppError::Sftp("transfer cancelled".into())),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_upload(
    app: AppHandle,
    transfers: State<'_, Transfers>,
    session_id: String,
    transfer_id: String,
    local_path: String,
    remote_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    run_tracked(&transfers, transfer_id, async move {
        with_session(&Ctx::new(&app), &session_id, |sftp| {
            let (local, remote, progress) = (&local_path, &remote_path, &on_progress);
            async move {
                transfer::upload(&sftp, local, remote, |transferred, total| {
                    let _ = progress.send(TransferProgress { transferred, total });
                })
                .await
            }
        })
        .await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_download(
    app: AppHandle,
    transfers: State<'_, Transfers>,
    session_id: String,
    transfer_id: String,
    remote_path: String,
    local_path: String,
    on_progress: Channel<TransferProgress>,
) -> AppResult<()> {
    run_tracked(&transfers, transfer_id, async move {
        with_session(&Ctx::new(&app), &session_id, |sftp| {
            let (remote, local, progress) = (&remote_path, &local_path, &on_progress);
            async move {
                transfer::download(&sftp, remote, local, |transferred, total| {
                    let _ = progress.send(TransferProgress { transferred, total });
                })
                .await
            }
        })
        .await
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn sftp_cancel(transfers: State<'_, Transfers>, transfer_id: String) -> AppResult<()> {
    if let Some(h) = transfers.0.lock().await.remove(&transfer_id) {
        h.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    use russh::keys::{decode_secret_key, PrivateKey};
    use russh::server::{self as ssh_server, Auth, Msg, Session};
    use russh::{Channel as SshChannel, ChannelId};
    use russh_sftp::protocol::{
        Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::net::{TcpListener, TcpStream};
    use zeroize::Zeroizing;

    use crate::ssh::client;
    use crate::ssh::known_hosts::KnownHosts;

    // Throwaway ed25519 key, used as both the test server's host key and the client key.
    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCRdwGrdNLhRy6B/tKGpcHp939BrEhthavtMn2qqseDOAAAAIiSQlnqkkJZ
6gAAAAtzc2gtZWQyNTUxOQAAACCRdwGrdNLhRy6B/tKGpcHp939BrEhthavtMn2qqseDOA
AAAEB4vAEbSG7dH0l7XzOuEEtQXeoTT/gC6j7NKt+a6UOaSJF3Aat00uFHLoH+0oalwen3
f0GsSG2Fq+0yfaqqx4M4AAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";

    // Big enough that download takes the parallel multi-handle path.
    const SIZE: usize = 3 * 1024 * 1024;

    type Files = Arc<StdMutex<HashMap<String, Vec<u8>>>>;

    fn payload() -> Vec<u8> {
        (0..SIZE).map(|i| (i % 251) as u8).collect()
    }

    struct Fs {
        files: Files,
        open: HashMap<String, String>,
        next: u32,
    }

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".into(),
            language_tag: "en-US".into(),
        }
    }

    impl russh_sftp::server::Handler for Fs {
        type Error = StatusCode;

        fn unimplemented(&self) -> StatusCode {
            StatusCode::OpUnsupported
        }

        async fn realpath(&mut self, id: u32, _path: String) -> Result<Name, StatusCode> {
            Ok(Name { id, files: vec![File::dummy("/")] })
        }

        async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
            let len = self.files.lock().unwrap().get(&path).ok_or(StatusCode::NoSuchFile)?.len();
            let mut attrs = FileAttributes::empty();
            attrs.size = Some(len as u64);
            attrs.permissions = Some(0o100644);
            Ok(Attrs { id, attrs })
        }

        async fn open(
            &mut self,
            id: u32,
            filename: String,
            pflags: OpenFlags,
            _attrs: FileAttributes,
        ) -> Result<Handle, StatusCode> {
            {
                let mut files = self.files.lock().unwrap();
                if pflags.contains(OpenFlags::CREATE) {
                    let f = files.entry(filename.clone()).or_default();
                    if pflags.contains(OpenFlags::TRUNCATE) {
                        f.clear();
                    }
                } else if !files.contains_key(&filename) {
                    return Err(StatusCode::NoSuchFile);
                }
            }
            self.next += 1;
            let handle = self.next.to_string();
            self.open.insert(handle.clone(), filename);
            Ok(Handle { id, handle })
        }

        async fn read(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            len: u32,
        ) -> Result<Data, StatusCode> {
            let path = self.open.get(&handle).ok_or(StatusCode::Failure)?;
            let files = self.files.lock().unwrap();
            let data = files.get(path).ok_or(StatusCode::NoSuchFile)?;
            let start = offset as usize;
            if start >= data.len() {
                return Err(StatusCode::Eof);
            }
            let end = (start + len as usize).min(data.len());
            Ok(Data { id, data: data[start..end].to_vec() })
        }

        async fn write(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            data: Vec<u8>,
        ) -> Result<Status, StatusCode> {
            let path = self.open.get(&handle).ok_or(StatusCode::Failure)?;
            let mut files = self.files.lock().unwrap();
            let f = files.get_mut(path).ok_or(StatusCode::NoSuchFile)?;
            let (start, end) = (offset as usize, offset as usize + data.len());
            if f.len() < end {
                f.resize(end, 0);
            }
            f[start..end].copy_from_slice(&data);
            Ok(ok(id))
        }

        async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
            self.open.remove(&handle);
            Ok(ok(id))
        }
    }

    struct SshServer {
        files: Files,
        channels: HashMap<ChannelId, SshChannel<Msg>>,
        sftp_channels: Arc<StdMutex<Vec<ChannelId>>>,
    }

    impl ssh_server::Handler for SshServer {
        type Error = russh::Error;

        async fn auth_publickey(
            &mut self,
            _user: &str,
            _key: &russh::keys::PublicKey,
        ) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            channel: SshChannel<Msg>,
            _session: &mut Session,
        ) -> Result<bool, Self::Error> {
            self.channels.insert(channel.id(), channel);
            Ok(true)
        }

        async fn subsystem_request(
            &mut self,
            id: ChannelId,
            name: &str,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            match self.channels.remove(&id) {
                Some(channel) if name == "sftp" => {
                    self.sftp_channels.lock().unwrap().push(id);
                    session.channel_success(id)?;
                    let fs = Fs { files: self.files.clone(), open: HashMap::new(), next: 0 };
                    russh_sftp::server::run(channel.into_stream(), fs).await;
                }
                _ => session.channel_failure(id)?,
            }
            Ok(())
        }
    }

    struct Link {
        frozen: Arc<AtomicBool>,
        // Bytes the link may still carry before it dies.
        trap: Arc<AtomicUsize>,
        relay: tokio::task::AbortHandle,
        server: ssh_server::Handle,
        sftp_channels: Arc<StdMutex<Vec<ChannelId>>>,
    }

    // Sits between client and server so a test can break the connection the ways a real
    // network does: drop it, go silent while staying open, die partway through or on the next
    // packet (a server that already forgot the connection), or lose just the SFTP channel.
    #[derive(Clone)]
    struct Proxy {
        port: u16,
        accepted: Arc<AtomicUsize>,
        links: Arc<TokioMutex<Vec<Link>>>,
    }

    // Both directions live in one task, so returning drops the whole link at once rather
    // than leaving one half open.
    async fn relay(
        mut client: TcpStream,
        mut server: DuplexStream,
        frozen: Arc<AtomicBool>,
        trap: Arc<AtomicUsize>,
    ) {
        let (mut up, mut down) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
        loop {
            let (read, to_server) = tokio::select! {
                read = client.read(&mut up) => (read, true),
                read = server.read(&mut down) => (read, false),
            };
            let n = match read {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            if trap.load(Ordering::SeqCst) < n {
                return;
            }
            trap.fetch_sub(n, Ordering::SeqCst);
            if frozen.load(Ordering::SeqCst) {
                continue;
            }
            let sent = if to_server {
                server.write_all(&up[..n]).await
            } else {
                client.write_all(&down[..n]).await
            };
            if sent.is_err() {
                return;
            }
        }
    }

    impl Proxy {
        async fn start(host_key: PrivateKey) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = Proxy {
                port: listener.local_addr().unwrap().port(),
                accepted: Arc::default(),
                links: Arc::default(),
            };
            let config =
                Arc::new(ssh_server::Config { keys: vec![host_key], ..Default::default() });
            let files: Files = Arc::default();
            let p = proxy.clone();
            tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    p.accepted.fetch_add(1, Ordering::SeqCst);
                    let (client_side, server_side) = tokio::io::duplex(1 << 20);
                    let frozen = Arc::new(AtomicBool::new(false));
                    let trap = Arc::new(AtomicUsize::new(usize::MAX));
                    let relay =
                        tokio::spawn(relay(tcp, client_side, frozen.clone(), trap.clone()))
                            .abort_handle();
                    let sftp_channels = Arc::new(StdMutex::new(Vec::new()));
                    let handler = SshServer {
                        files: files.clone(),
                        channels: HashMap::new(),
                        sftp_channels: sftp_channels.clone(),
                    };
                    let running =
                        ssh_server::run_stream(config.clone(), server_side, handler).await.unwrap();
                    p.links.lock().await.push(Link {
                        frozen,
                        trap,
                        relay,
                        server: running.handle(),
                        sftp_channels,
                    });
                }
            });
            proxy
        }

        fn accepted(&self) -> usize {
            self.accepted.load(Ordering::SeqCst)
        }

        async fn kill(&self) {
            for link in self.links.lock().await.drain(..) {
                link.relay.abort();
            }
        }

        async fn freeze(&self) {
            for link in self.links.lock().await.iter() {
                link.frozen.store(true, Ordering::SeqCst);
            }
        }

        async fn trap_after(&self, bytes: usize) {
            for link in self.links.lock().await.iter() {
                link.trap.store(bytes, Ordering::SeqCst);
            }
        }

        async fn close_sftp(&self) {
            for link in self.links.lock().await.iter() {
                let ids: Vec<ChannelId> = link.sftp_channels.lock().unwrap().drain(..).collect();
                for id in ids {
                    link.server.close(id).await.unwrap();
                }
            }
        }
    }

    struct Env {
        dir: tempfile::TempDir,
        store: StdMutex<Store>,
        vault: StdMutex<Vault>,
        known: KnownHostsState,
        sessions: Sessions,
        sftps: SftpSessions,
        conns: SftpConns,
        proxy: Proxy,
    }

    impl Env {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let host_key = decode_secret_key(KEY, None).unwrap();
            let fingerprint = host_key.public_key().fingerprint(Default::default()).to_string();
            let proxy = Proxy::start(host_key).await;
            let mut known = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
            known.record("127.0.0.1", proxy.port, &fingerprint).unwrap();
            std::fs::write(dir.path().join("id_ed25519"), KEY).unwrap();
            Self {
                store: StdMutex::new(Store::load(dir.path().to_path_buf()).unwrap()),
                vault: StdMutex::new(
                    Vault::open_with_key(dir.path().join("vault.enc"), Zeroizing::new([7u8; 32]))
                        .unwrap(),
                ),
                known: KnownHostsState(Arc::new(StdMutex::new(known))),
                sessions: Sessions::default(),
                sftps: SftpSessions::default(),
                conns: SftpConns::default(),
                proxy,
                dir,
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx {
                store: &self.store,
                vault: &self.vault,
                known: &self.known,
                sessions: &self.sessions,
                sftps: &self.sftps,
                conns: &self.conns,
            }
        }

        fn local(&self, name: &str) -> String {
            self.dir.path().join(name).to_string_lossy().into_owned()
        }

        fn keys(&self) -> Vec<ProfileKey> {
            vec![ProfileKey { path: self.local("id_ed25519"), secret_id: None }]
        }

        async fn standalone(&self) -> String {
            let reconnect = Reconnect::Adhoc {
                host: "127.0.0.1".into(),
                port: self.proxy.port,
                username: "me".into(),
                auth_method: AuthMethod::Key,
                keys: self.keys(),
                secret_id: None,
            };
            register_standalone(&self.ctx(), reconnect).await.unwrap()
        }

        async fn terminal(&self) -> String {
            let keys = ssh_cmds::resolve_keys(&self.vault, &self.keys()).unwrap();
            let handle = ssh_cmds::connect_adhoc(
                &self.known,
                "127.0.0.1",
                self.proxy.port,
                "me",
                AuthMethod::Key,
                &keys,
                None,
            )
            .await
            .unwrap();
            let session = ssh_cmds::Session {
                handle: Arc::new(handle),
                bastions: Vec::new(),
                remote_forwards: client::new_forwards(),
                input_tx: tokio::sync::mpsc::channel(1).0,
                resize_tx: tokio::sync::mpsc::channel(1).0,
                abort: tokio::spawn(async {}).abort_handle(),
            };
            self.sessions.0.lock().await.insert("term".into(), session);
            "term".into()
        }

        async fn upload(&self, id: &str, local: &str, remote: &str) -> AppResult<()> {
            with_session(&self.ctx(), id, |sftp| async move {
                transfer::upload(&sftp, local, remote, |_, _| {}).await
            })
            .await
        }

        async fn download(&self, id: &str, remote: &str, local: &str) -> AppResult<()> {
            with_session(&self.ctx(), id, |sftp| async move {
                transfer::download(&sftp, remote, local, |_, _| {}).await
            })
            .await
        }

        // Seeds /a.bin through the session, then breaks the link and times a download of it.
        async fn download_after(
            &self,
            id: &str,
            break_link: impl Future<Output = ()>,
        ) -> (AppResult<()>, Duration) {
            std::fs::write(self.local("up.bin"), payload()).unwrap();
            self.upload(id, &self.local("up.bin"), "/a.bin").await.unwrap();
            break_link.await;
            let started = Instant::now();
            let result = self.download(id, "/a.bin", &self.local("down.bin")).await;
            (result, started.elapsed())
        }

        fn assert_recovered(&self, result: AppResult<()>, dials: usize) {
            result.unwrap();
            assert_eq!(std::fs::read(self.local("down.bin")).unwrap(), payload());
            assert_eq!(self.proxy.accepted(), dials);
        }
    }

    // Anything slower means a request sat out its timeout on a connection already known dead.
    fn assert_fast(took: Duration) {
        assert!(took < PROBE_TIMEOUT, "took {took:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redials_after_connection_drops() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let (result, took) = env.download_after(&id, env.proxy.kill()).await;
        eprintln!("dropped connection: {took:?}");
        env.assert_recovered(result, 2);
        assert_fast(took);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redials_when_server_forgot_connection() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let (result, took) = env.download_after(&id, env.proxy.trap_after(0)).await;
        eprintln!("server forgot connection: {took:?}");
        env.assert_recovered(result, 2);
        assert_fast(took);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redials_when_connection_drops_mid_transfer() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let (result, took) = env.download_after(&id, env.proxy.trap_after(SIZE / 3)).await;
        eprintln!("dropped mid-transfer: {took:?}");
        env.assert_recovered(result, 2);
        assert_fast(took);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upload_redials_when_connection_drops_mid_transfer() {
        let env = Env::new().await;
        let id = env.standalone().await;
        std::fs::write(env.local("up.bin"), payload()).unwrap();
        env.proxy.trap_after(SIZE / 3).await;
        let started = Instant::now();
        let result = env.upload(&id, &env.local("up.bin"), "/b.bin").await;
        let took = started.elapsed();
        eprintln!("upload dropped mid-transfer: {took:?}");
        result.unwrap();
        assert_fast(took);
        let downloaded = env.download(&id, "/b.bin", &env.local("down.bin")).await;
        env.assert_recovered(downloaded, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redials_after_sftp_channel_closes() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let (result, took) = env.download_after(&id, env.proxy.close_sftp()).await;
        eprintln!("closed sftp channel: {took:?}");
        env.assert_recovered(result, 2);
        assert_fast(took);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "waits out the 30s SFTP request timeout"]
    async fn redials_after_connection_goes_silent() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let (result, took) = env.download_after(&id, env.proxy.freeze()).await;
        eprintln!("silent connection: {took:?}");
        env.assert_recovered(result, 2);
    }

    // Pipelined writes have no timeout, so this rides on SSH keepalives closing the session.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "waits for SSH keepalives to give up on the connection"]
    async fn upload_redials_after_connection_goes_silent_mid_transfer() {
        let env = Env::new().await;
        let id = env.standalone().await;
        std::fs::write(env.local("up.bin"), payload()).unwrap();
        let frozen = env.proxy.links.lock().await[0].frozen.clone();
        let local = env.local("up.bin");
        let started = Instant::now();
        let result = with_session(&env.ctx(), &id, |sftp| {
            let (local, frozen) = (&local, frozen.clone());
            async move {
                transfer::upload(&sftp, local, "/b.bin", move |done, _| {
                    if done > 0 {
                        frozen.store(true, Ordering::SeqCst);
                    }
                })
                .await
            }
        })
        .await;
        eprintln!("upload, silent mid-transfer: {:?}", started.elapsed());
        result.unwrap();
        let downloaded = env.download(&id, "/b.bin", &env.local("down.bin")).await;
        env.assert_recovered(downloaded, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn server_refusal_is_returned_without_redialing() {
        let env = Env::new().await;
        let id = env.standalone().await;
        let err = env.download(&id, "/missing.bin", &env.local("x.bin")).await.unwrap_err();
        assert!(err.to_string().contains("No such file"), "{err}");
        assert_eq!(env.proxy.accepted(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnected_tab_is_not_redialed() {
        let env = Env::new().await;
        let id = env.standalone().await;
        env.proxy.kill().await;
        env.sftps.0.lock().await.remove(&id);
        env.conns.map.lock().await.remove(&id);
        let err = env.download(&id, "/a.bin", &env.local("x.bin")).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err}");
        assert_eq!(env.proxy.accepted(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_reopens_channel_on_live_connection() {
        let env = Env::new().await;
        let id = env.terminal().await;
        let (result, took) = env.download_after(&id, env.proxy.close_sftp()).await;
        eprintln!("terminal, closed sftp channel: {took:?}");
        env.assert_recovered(result, 1);
        assert_fast(took);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_reports_lost_connection() {
        let env = Env::new().await;
        let id = env.terminal().await;
        let (result, took) = env.download_after(&id, env.proxy.trap_after(0)).await;
        let err = result.unwrap_err();
        eprintln!("terminal, server forgot connection: {took:?}: {err}");
        assert!(err.to_string().contains("reconnect the terminal"), "{err}");
        assert_eq!(env.proxy.accepted(), 1);
        assert_fast(took);
    }
}
