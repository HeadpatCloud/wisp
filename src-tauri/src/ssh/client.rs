use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Config, Handle, Handler};
use russh::keys::{decode_secret_key, Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};
use russh::{Channel, client::Msg};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{AppError, AppResult};
use crate::ssh::known_hosts::{HostKeyVerdict, KnownHosts};

pub type SshHandle = Handle<ClientHandler>;

#[derive(Clone)]
pub struct RemoteTarget {
    pub target_host: String,
    pub target_port: u16,
    pub up: Arc<AtomicU64>,
    pub down: Arc<AtomicU64>,
}

pub type RemoteForwards = Arc<Mutex<HashMap<(String, u32), RemoteTarget>>>;

pub fn new_forwards() -> RemoteForwards {
    Arc::new(Mutex::new(HashMap::new()))
}

pub struct ClientHandler {
    known_hosts: Arc<Mutex<KnownHosts>>,
    host: String,
    port: u16,
    remote_forwards: RemoteForwards,
}

impl Handler for ClientHandler {
    type Error = AppError;

    async fn check_server_key(&mut self, server_public_key: &PublicKey) -> AppResult<bool> {
        let fingerprint = server_public_key.fingerprint(Default::default()).to_string();
        let verdict = {
            let kh = self
                .known_hosts
                .lock()
                .map_err(|_| AppError::Internal("known_hosts lock poisoned".into()))?;
            kh.verify(&self.host, self.port, &fingerprint)
        };
        match verdict {
            HostKeyVerdict::Trusted => Ok(true),
            HostKeyVerdict::Unknown { fingerprint } => Err(AppError::HostKeyUnknown {
                host: self.host.clone(),
                port: self.port,
                fingerprint,
            }),
            HostKeyVerdict::Mismatch { stored, offered } => Err(AppError::HostKeyMismatch {
                host: self.host.clone(),
                port: self.port,
                stored,
                offered,
            }),
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut russh::client::Session,
    ) -> AppResult<()> {
        let target = {
            let map = self
                .remote_forwards
                .lock()
                .map_err(|_| AppError::Internal("remote_forwards poisoned".into()))?;
            map.get(&(connected_address.to_string(), connected_port)).cloned()
        };
        let Some(target) = target else {
            return Ok(());
        };
        tokio::spawn(async move {
            let Ok(local) =
                tokio::net::TcpStream::connect((target.target_host.as_str(), target.target_port)).await
            else {
                return;
            };
            let (lr, lw) = local.into_split();
            let (rr, rw) = tokio::io::split(channel.into_stream());
            let up = tokio::spawn(crate::tunnel::pump(lr, rw, target.up));
            // A far side that is done sending may still wait for its answer, so only an
            // error here ends the other direction too.
            if crate::tunnel::pump(rr, lw, target.down).await.is_err() {
                up.abort();
            }
        });
        Ok(())
    }
}

pub async fn connect(
    host: &str,
    port: u16,
    known_hosts: Arc<Mutex<KnownHosts>>,
    remote_forwards: RemoteForwards,
) -> AppResult<SshHandle> {
    let config = client_config();
    let host = crate::net::normalize_host(host);
    let handler =
        ClientHandler { known_hosts, host: host.clone(), port, remote_forwards };
    let handle = client::connect(config, (host.as_str(), port), handler).await?;
    Ok(handle)
}

pub async fn connect_over<R>(
    stream: R,
    host: &str,
    port: u16,
    known_hosts: Arc<Mutex<KnownHosts>>,
    remote_forwards: RemoteForwards,
) -> AppResult<SshHandle>
where
    R: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let config = client_config();
    let handler = ClientHandler { known_hosts, host: host.to_string(), port, remote_forwards };
    let handle = client::connect_stream(config, stream, handler).await?;
    Ok(handle)
}

fn client_config() -> Arc<Config> {
    Arc::new(Config {
        inactivity_timeout: Some(Duration::from_secs(3600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}

pub async fn auth_password(handle: &mut SshHandle, user: &str, password: &str) -> AppResult<()> {
    if handle.authenticate_password(user, password).await?.success() {
        Ok(())
    } else {
        Err(AppError::Auth("password rejected".into()))
    }
}

// Answer every server prompt with the stored password. Covers servers that only
// offer keyboard-interactive for password login (common with PAM).
pub async fn auth_keyboard_interactive(
    handle: &mut SshHandle,
    user: &str,
    password: &str,
) -> AppResult<()> {
    use russh::client::KeyboardInteractiveAuthResponse as Resp;
    let mut res = handle.authenticate_keyboard_interactive_start(user.to_string(), None).await?;
    loop {
        match res {
            Resp::Success => return Ok(()),
            Resp::Failure { .. } => return Err(AppError::Auth("keyboard-interactive rejected".into())),
            Resp::InfoRequest { prompts, .. } => {
                let responses = vec![password.to_string(); prompts.len()];
                res = handle.authenticate_keyboard_interactive_respond(responses).await?;
            }
        }
    }
}

fn ppk_to_key(contents: &str, passphrase: Option<&str>) -> AppResult<PrivateKey> {
    let encrypted = contents
        .lines()
        .find_map(|l| l.strip_prefix("Encryption:"))
        .is_some_and(|v| v.trim() != "none");
    if encrypted && passphrase.is_none() {
        return Err(AppError::PassphraseRequired);
    }
    PrivateKey::from_ppk(contents, passphrase.map(String::from)).map_err(|e| {
        if encrypted {
            AppError::WrongPassphrase
        } else {
            AppError::Ssh(e.to_string())
        }
    })
}

// Detect the key format from its contents, not the file name - a key's extension
// may not match its actual format (e.g. an OpenSSH key saved as .ppk).
fn parse_key(contents: &str, passphrase: Option<&str>) -> AppResult<PrivateKey> {
    if contents.contains("PuTTY-User-Key-File") {
        ppk_to_key(contents, passphrase)
    } else {
        Ok(decode_secret_key(contents, passphrase)?)
    }
}

pub async fn auth_key(
    handle: &mut SshHandle,
    user: &str,
    key_path: &str,
    passphrase: Option<&str>,
) -> AppResult<()> {
    let contents = std::fs::read_to_string(key_path)?;
    let key = parse_key(&contents, passphrase)?;
    // RSA must negotiate rsa-sha2 (SHA-512); ed25519/ecdsa ignore the hash alg.
    let hash_alg = if matches!(key.algorithm(), Algorithm::Rsa { .. }) {
        Some(HashAlg::Sha512)
    } else {
        None
    };
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
    if handle.authenticate_publickey(user, key).await?.success() {
        Ok(())
    } else {
        Err(AppError::Auth("key rejected".into()))
    }
}

pub async fn auth_agent(handle: &mut SshHandle, user: &str) -> AppResult<()> {
    use russh::keys::agent::client::AgentClient;

    #[cfg(unix)]
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|e| AppError::Auth(format!("ssh agent unavailable: {e}")))?;

    #[cfg(windows)]
    let mut agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
        .await
        .map_err(|e| AppError::Auth(format!("ssh agent unavailable: {e}")))?;

    let identities = agent.request_identities().await.map_err(AppError::from)?;
    for id in identities {
        let pubkey = id.public_key().into_owned();
        if handle
            .authenticate_publickey_with(user, pubkey, None, &mut agent)
            .await
            .map_err(|e| AppError::Auth(e.to_string()))?
            .success()
        {
            return Ok(());
        }
    }
    Err(AppError::Auth("no agent identity accepted".into()))
}

#[cfg(test)]
mod tests {
    use russh::keys::{Algorithm, HashAlg};
    use crate::error::AppError;

    // A bare literal resolves via the (host, port) tuple on every platform; a bracketed one
    // only happens to work on Windows, which is why connect() normalizes first.
    #[test]
    fn bare_ipv6_literal_resolves() {
        use std::net::ToSocketAddrs;
        let addrs: Vec<_> = (crate::net::normalize_host("[2001:db8::1]").as_str(), 22)
            .to_socket_addrs()
            .unwrap()
            .collect();
        assert_eq!(addrs.len(), 1);
        assert!(addrs[0].is_ipv6());
        assert_eq!(addrs[0].port(), 22);
    }

    // Pure logic: RSA -> Sha512, everything else -> None.
    fn hash_for(alg: &Algorithm) -> Option<HashAlg> {
        if matches!(alg, Algorithm::Rsa { .. }) { Some(HashAlg::Sha512) } else { None }
    }

    #[test]
    fn rsa_uses_sha512_others_none() {
        assert_eq!(hash_for(&Algorithm::Rsa { hash: None }), Some(HashAlg::Sha512));
        assert_eq!(hash_for(&Algorithm::Ed25519), None);
    }

    const ED25519_PPK: &str = include_str!("fixtures/id_ed25519.ppk");
    const ED25519_ENC_PPK: &str = include_str!("fixtures/id_ed25519_enc.ppk");
    const ED25519_OPENSSH: &str = include_str!("fixtures/id_ed25519_openssh");

    #[test]
    fn parse_key_routes_openssh_key_to_decoder_not_ppk() {
        let key = super::parse_key(ED25519_OPENSSH, None).unwrap();
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
    }

    #[test]
    fn parse_key_routes_putty_header_to_ppk() {
        let key = super::parse_key(ED25519_PPK, None).unwrap();
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
    }

    const ED25519_PUB: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XFSqti user@example.com";

    #[test]
    fn ppk_unencrypted_parses_to_ed25519() {
        let key = super::ppk_to_key(ED25519_PPK, None).unwrap();
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
        assert_eq!(key.public_key().to_openssh().unwrap(), ED25519_PUB);
    }

    #[test]
    fn ppk_encrypted_with_passphrase_yields_same_key() {
        let key = super::ppk_to_key(ED25519_ENC_PPK, Some("123")).unwrap();
        assert_eq!(key.public_key().to_openssh().unwrap(), ED25519_PUB);
    }

    #[test]
    fn ppk_encrypted_without_passphrase_is_passphrase_required() {
        assert!(matches!(
            super::ppk_to_key(ED25519_ENC_PPK, None),
            Err(AppError::PassphraseRequired)
        ));
    }

    #[test]
    fn ppk_encrypted_wrong_passphrase_is_wrong_passphrase() {
        assert!(matches!(
            super::ppk_to_key(ED25519_ENC_PPK, Some("wrong")),
            Err(AppError::WrongPassphrase)
        ));
    }

    struct AnyoneServer;

    impl russh::server::Handler for AnyoneServer {
        type Error = russh::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<russh::server::Auth, Self::Error> {
            Ok(russh::server::Auth::Accept)
        }
    }

    // What `printf request | socat - TCP:...` does behind a remote forward: send, close the
    // sending side, wait for the answer.
    #[tokio::test]
    async fn remote_forward_answers_a_client_that_stopped_sending() {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::time::timeout;

        let limit = Duration::from_secs(10);
        let host_key = russh::keys::decode_secret_key(ED25519_OPENSSH, None).unwrap();
        let fingerprint = host_key.public_key().fingerprint(Default::default()).to_string();
        let config =
            Arc::new(russh::server::Config { keys: vec![host_key], ..Default::default() });
        let (client_side, server_side) = tokio::io::duplex(1 << 16);
        let serving = tokio::spawn(russh::server::run_stream(config, server_side, AnyoneServer));

        let dir = tempfile::tempdir().unwrap();
        let mut known =
            crate::ssh::known_hosts::KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        known.record("server", 22, &fingerprint).unwrap();
        let (known, forwards) = (Arc::new(Mutex::new(known)), super::new_forwards());
        let connecting = super::connect_over(client_side, "server", 22, known, forwards.clone());
        let mut handle = timeout(limit, connecting).await.expect("connect hung").unwrap();
        let login = timeout(limit, handle.authenticate_none("me")).await.expect("login hung");
        assert!(login.unwrap().success());
        let server = timeout(limit, serving).await.expect("the server hung").unwrap().unwrap();

        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        forwards.lock().unwrap().insert(
            ("127.0.0.1".to_string(), 7100),
            super::RemoteTarget {
                target_host: "127.0.0.1".into(),
                target_port: target.local_addr().unwrap().port(),
                up: Arc::default(),
                down: Arc::default(),
            },
        );

        let server = server.handle();
        let opening = server.channel_open_forwarded_tcpip("127.0.0.1", 7100, "127.0.0.1", 1);
        let mut channel = timeout(limit, opening).await.expect("no channel").unwrap();
        channel.data(&b"ping"[..]).await.unwrap();
        channel.eof().await.unwrap();

        let accepting = timeout(limit, target.accept()).await.expect("no connection");
        let (mut stream, _) = accepting.unwrap();
        let mut asked = Vec::new();
        // Returns once the client's end of sending has arrived here too.
        timeout(limit, stream.read_to_end(&mut asked)).await.expect("no end").unwrap();
        assert_eq!(asked, b"ping");
        stream.write_all(b"pong").await.unwrap();
        drop(stream);

        loop {
            match timeout(limit, channel.wait()).await.expect("no answer") {
                Some(russh::ChannelMsg::Data { data }) => {
                    assert_eq!(&data[..], b"pong");
                    break;
                }
                Some(_) => {}
                None => panic!("the channel closed without the answer"),
            }
        }
    }
}
