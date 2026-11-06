use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ironrdp_mstsgu::{GwClient, GwConnectTarget, GwErrorKind};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::Resumption;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{self, CertificateDer, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use super::connect::LOGIN_LIMIT;
use super::err;
use super::session::CONNECT_LIMIT;
use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::trust::{self, Scheme};
use crate::remote::{keep_alive, tls};

pub struct Gateway<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}

// What the system said of the gateway's certificate.
#[derive(Debug)]
enum Said {
    Trusted(Vec<u8>),
    Refused(Vec<u8>),
    Revoked,
}

// The system's word first. A certificate it refuses still has to sign the handshake, and is kept
// for the pin, which is checked before anything is written.
#[derive(Debug)]
struct SystemOrPin {
    system: Arc<dyn ServerCertVerifier>,
    algorithms: WebPkiSupportedAlgorithms,
    said: Mutex<Option<Said>>,
}

impl ServerCertVerifier for SystemOrPin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let system = &self.system;
        let verdict =
            system.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now);
        let said = match verdict {
            Ok(_) => Said::Trusted(end_entity.to_vec()),
            Err(rustls::Error::InvalidCertificate(CertificateError::Revoked)) => Said::Revoked,
            Err(_) => Said::Refused(end_entity.to_vec()),
        };
        *self.said.lock().unwrap_or_else(PoisonError::into_inner) = Some(said);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

pub(crate) async fn open(
    gateway: &Gateway<'_>,
    server: &str,
    server_port: u16,
    known: &KnownHostsState,
) -> AppResult<(GwClient, SocketAddr)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let system = rustls_platform_verifier::Verifier::new(provider).map_err(|e| err(e.to_string()))?;
    let system = Arc::new(system);
    open_within(LOGIN_LIMIT, CONNECT_LIMIT, system, gateway, server, server_port, known).await
}

async fn open_within(
    limit: Duration,
    connect_limit: Duration,
    system: Arc<dyn ServerCertVerifier>,
    gateway: &Gateway<'_>,
    server: &str,
    server_port: u16,
    known: &KnownHostsState,
) -> AppResult<(GwClient, SocketAddr)> {
    let tunnel = tunnel(connect_limit, system, gateway, server, server_port, known);
    timeout(limit, tunnel).await.map_err(|_| err("the gateway did not answer in time"))?
}

async fn tunnel(
    connect_limit: Duration,
    system: Arc<dyn ServerCertVerifier>,
    gateway: &Gateway<'_>,
    server: &str,
    server_port: u16,
    known: &KnownHostsState,
) -> AppResult<(GwClient, SocketAddr)> {
    // A Basic login with an empty password is a failed logon that counts toward a lockout.
    if gateway.username.is_empty() || gateway.password.is_empty() {
        return Err(err("the gateway needs a user name and password"));
    }
    let host = crate::net::normalize_host(gateway.host);
    let tcp = timeout(connect_limit, TcpStream::connect((host.as_str(), gateway.port)))
        .await
        .map_err(|_| err("the gateway connection timed out"))??;
    tcp.set_nodelay(true)?;
    keep_alive(&tcp)?;
    let local = tcp.local_addr()?;

    let name = pki_types::ServerName::try_from(host)
        .map_err(|_| err(format!("invalid host name: {}", gateway.host)))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algorithms = provider.signature_verification_algorithms;
    let verifier = Arc::new(SystemOrPin { system, algorithms, said: Mutex::default() });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("the ring provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as _)
        .with_no_client_auth();
    // A resumed session would skip the verifier, and with it the pin.
    config.resumption = Resumption::disabled();
    let stream = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|e| err(tls::failure(&e)))?;
    let said = verifier.said.lock().unwrap_or_else(PoisonError::into_inner).take();
    match said {
        Some(Said::Trusted(leaf)) => {
            // So that a later certificate the system does not trust reads as a change.
            trust::remember(known, Scheme::Rdg, gateway.host, gateway.port, &leaf)?;
        }
        Some(Said::Refused(leaf)) => {
            trust::check(known, Scheme::Rdg, gateway.host, gateway.port, &leaf)?;
        }
        Some(Said::Revoked) => return Err(err("the gateway's certificate has been revoked")),
        // rustls asks the verifier in every full handshake, and resumption is off.
        None => return Err(err("the gateway sent no certificate")),
    }

    let gw_endpoint = match gateway.port {
        443 => crate::net::url_host(gateway.host),
        port => crate::net::authority(gateway.host, port),
    };
    let target = GwConnectTarget {
        gw_endpoint,
        gw_user: gateway.username.to_owned(),
        gw_pass: gateway.password.to_owned(),
        server: crate::net::normalize_host(server),
        server_port,
    };
    let tunnel = GwClient::connect(stream, &target, "wisp")
        .await
        .map_err(|e| refusal(e.kind(), server, server_port))?;
    Ok((tunnel, local))
}

fn refusal(kind: &GwErrorKind, server: &str, port: u16) -> AppError {
    match kind {
        GwErrorKind::Unauthorized { basic: false } => {
            err("the gateway requires NTLM, which isn't supported")
        }
        GwErrorKind::Unauthorized { basic: true } => err("wrong gateway user name or password"),
        // A page or a redirect is what a web server answers; only an error status is a refusal.
        GwErrorKind::HttpStatus(status) if *status >= 400 => {
            err(format!("the gateway refused the connection (HTTP {status})"))
        }
        GwErrorKind::TunnelRefused(code)
        | GwErrorKind::TunnelAuthRefused(code)
        | GwErrorKind::ChannelRefused(code) => {
            let server = crate::net::authority(server, port);
            err(format!("the gateway refused the connection to {server} ({code:#010x})"))
        }
        GwErrorKind::UnsupportedFeature => {
            err("the gateway shows a message that must be accepted, which isn't supported")
        }
        GwErrorKind::Closed => err("the gateway closed the connection"),
        GwErrorKind::ClosedAtChannel => {
            let server = crate::net::authority(server, port);
            err(format!("the gateway closed the connection before connecting to {server}"))
        }
        _ => err("this does not look like an RD Gateway"),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use ironrdp_pdu::mcs::DisconnectReason;
    use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode};
    use ironrdp_pdu::surface_commands::FrameAction;
    use ironrdp_tokio::FramedWrite as _;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
    use tokio::io::{duplex, AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
    use tokio::net::{TcpSocket, TcpStream};
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::error::AppError;
    use crate::rdp::connect::Target;
    use crate::rdp::gatewaytest::{
        channel_response, consent_response, data_packet, handshake_response, packet, stand_in,
        tunnel_auth_response, tunnel_response, Answer, Hangup, Heard, Script, StandIn,
        CHANNEL_RESPONSE, HANDSHAKE_RESPONSE, TUNNEL_RESPONSE,
    };
    use crate::rdp::session::Session;
    use crate::rdp::testserver::{
        accept_scripted, bitmap, certificate, error_info, frame_marker, read_client,
        shutdown_denied, Channels, ClientPdu, Login, Options, ScriptedFramed, TestServer,
    };
    use crate::remote::trust::fingerprint;
    use crate::remote::FrameOp;
    use crate::ssh::known_hosts::{HostKeyVerdict, KnownHosts};

    const WAIT: Duration = Duration::from_secs(10);
    const GATEWAY: &str = "127.0.0.1";
    const SERVER: &str = "desk.lan";
    const SERVER_PORT: u16 = 3390;
    const NOT_A_GATEWAY: &str = "rdp: this does not look like an RD Gateway";

    // What the system says of every certificate: trusted, or this error. The handshake signatures
    // are not its to check.
    #[derive(Debug)]
    struct System(Result<(), CertificateError>);

    impl ServerCertVerifier for System {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            let said = self.0.clone().map_err(rustls::Error::InvalidCertificate);
            said.map(|()| ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("the system was asked for a signature".into()))
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Err(rustls::Error::General("the system was asked for a signature".into()))
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            Vec::new()
        }
    }

    fn known(dir: &tempfile::TempDir) -> KnownHostsState {
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        KnownHostsState(Arc::new(Mutex::new(hosts)))
    }

    fn pin(known: &KnownHostsState, host: &str, port: u16, print: &str) {
        known.0.lock().unwrap().record(host, port, print).unwrap();
    }

    fn gateway(port: u16) -> Gateway<'static> {
        Gateway { host: GATEWAY, port, username: "gw-user", password: "gw-pass" }
    }

    async fn within<T>(future: impl Future<Output = T>) -> T {
        timeout(WAIT, future).await.expect("timed out")
    }

    // An open through `stand_in` whose certificate the system trusts if `trusted`.
    async fn open_to(
        stand_in: &StandIn,
        trusted: bool,
        known: &KnownHostsState,
    ) -> AppResult<(GwClient, SocketAddr)> {
        let said = if trusted { Ok(()) } else { Err(CertificateError::UnknownIssuer) };
        open_said(stand_in, said, known).await
    }

    // An open through `stand_in` where the system says `said` of its certificate.
    async fn open_said(
        stand_in: &StandIn,
        said: Result<(), CertificateError>,
        known: &KnownHostsState,
    ) -> AppResult<(GwClient, SocketAddr)> {
        let system = Arc::new(System(said));
        let gateway = gateway(stand_in.port);
        let opening =
            open_within(LOGIN_LIMIT, CONNECT_LIMIT, system, &gateway, SERVER, SERVER_PORT, known);
        within(opening).await
    }

    async fn heard(stand_in: StandIn) -> Heard {
        within(stand_in.heard).await.unwrap()
    }

    fn failed<T>(result: AppResult<T>) -> AppError {
        match result {
            Ok(_) => panic!("expected the open to fail"),
            Err(e) => e,
        }
    }

    fn text(e: &AppError) -> &str {
        match e {
            AppError::Internal(message) => message,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    // What a system-trusted open through a stand-in playing `script` fails with.
    async fn refusal(script: Script) -> String {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(script, None).await;
        let error = failed(open_to(&stand_in, true, &known(&dir)).await);
        text(&error).to_owned()
    }

    fn reply(response: &[u8]) -> Script {
        Script { answer: Answer::Reply(response.to_vec()), ..Script::default() }
    }

    fn shown(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[tokio::test]
    async fn an_untrusted_gateway_certificate_stops_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(Script::default(), None).await;
        let port = stand_in.port;
        let opened = open_to(&stand_in, false, &known(&dir)).await.map(drop);
        let heard = heard(stand_in).await;
        assert!(heard.bytes.is_empty(), "the stand-in heard {:?}", shown(&heard.bytes));
        match opened {
            Err(AppError::HostKeyUnknown { host, port: asked, fingerprint: offered }) => {
                assert_eq!(host, "rdg/127.0.0.1");
                assert_eq!(asked, port);
                assert_eq!(offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyUnknown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_changed_gateway_certificate_is_a_mismatch_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let stand_in = stand_in(Script::default(), None).await;
        let port = stand_in.port;
        let other = fingerprint(b"another certificate");
        pin(&known, "rdg/127.0.0.1", port, &other);
        let opened = open_to(&stand_in, false, &known).await.map(drop);
        let heard = heard(stand_in).await;
        assert!(heard.bytes.is_empty(), "the stand-in heard {:?}", shown(&heard.bytes));
        match opened {
            Err(AppError::HostKeyMismatch { host, port: asked, stored, offered }) => {
                assert_eq!(host, "rdg/127.0.0.1");
                assert_eq!(asked, port);
                assert_eq!(stored, other);
                assert_eq!(offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_system_trusted_gateway_needs_no_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        // Neither a missing pin nor one for another certificate holds the open up.
        for other in [None, Some(fingerprint(b"another certificate"))] {
            let stand_in = stand_in(Script::default(), None).await;
            if let Some(other) = &other {
                pin(&known, "rdg/127.0.0.1", stand_in.port, other);
            }
            let opened = open_to(&stand_in, true, &known).await;
            assert!(opened.is_ok(), "{other:?}: {:?}", opened.err());
        }
    }

    #[tokio::test]
    async fn a_revoked_gateway_certificate_is_refused_and_never_offered_as_a_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        // Not even a pin for this very certificate lets it through.
        for pinned in [false, true] {
            let stand_in = stand_in(Script::default(), None).await;
            if pinned {
                pin(&known, "rdg/127.0.0.1", stand_in.port, &fingerprint(&certificate()));
            }
            let opened = open_said(&stand_in, Err(CertificateError::Revoked), &known).await;
            let error = failed(opened);
            let revoked = "rdp: the gateway's certificate has been revoked";
            assert_eq!(text(&error), revoked, "pinned: {pinned}");
            let heard = heard(stand_in).await;
            assert!(heard.bytes.is_empty(), "the stand-in heard {:?}", shown(&heard.bytes));
        }
    }

    #[tokio::test]
    async fn a_system_trusted_gateway_certificate_becomes_its_pin() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let print = fingerprint(&certificate());
        for before in [None, Some(fingerprint(b"another certificate"))] {
            let stand_in = stand_in(Script::default(), None).await;
            let port = stand_in.port;
            if let Some(before) = &before {
                pin(&known, "rdg/127.0.0.1", port, before);
            }
            open_to(&stand_in, true, &known).await.unwrap();
            let verdict = known.0.lock().unwrap().verify("rdg/127.0.0.1", port, &print);
            assert!(matches!(verdict, HostKeyVerdict::Trusted), "before: {before:?}");
            // A certificate the system does not trust is now a change, not a first connection.
            let forged = trust::check(&known, Scheme::Rdg, GATEWAY, port, b"a forged certificate");
            assert!(matches!(forged, Err(AppError::HostKeyMismatch { .. })), "before: {before:?}");
        }
    }

    #[tokio::test]
    async fn a_pin_that_holds_the_trusted_certificate_already_is_not_written_again() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let stand_in = stand_in(Script::default(), None).await;
        pin(&known, "rdg/127.0.0.1", stand_in.port, &fingerprint(&certificate()));
        let file = dir.path().join("known_hosts.json");
        std::fs::remove_file(&file).unwrap();
        open_to(&stand_in, true, &known).await.unwrap();
        assert!(!file.exists(), "the pin was written again");
    }

    #[tokio::test]
    async fn the_system_does_not_trust_the_test_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(Script::default(), None).await;
        let opened = within(open(&gateway(stand_in.port), SERVER, SERVER_PORT, &known(&dir))).await;
        match opened {
            Err(AppError::HostKeyUnknown { host, .. }) => assert_eq!(host, "rdg/127.0.0.1"),
            other => panic!("expected HostKeyUnknown, got {:?}", other.map(drop)),
        }
        assert!(heard(stand_in).await.bytes.is_empty());
    }

    type Ops = mpsc::UnboundedReceiver<FrameOp>;

    // A connect through `stand_in`, with both certificates pinned.
    async fn connect_through(
        stand_in: &StandIn,
        known: &KnownHostsState,
    ) -> AppResult<(Session, Ops)> {
        let print = fingerprint(&certificate());
        pin(known, "rdg/127.0.0.1", stand_in.port, &print);
        pin(known, "rdp/desk.lan", SERVER_PORT, &print);
        let target = Target {
            host: SERVER,
            port: SERVER_PORT,
            username: "alice",
            domain: None,
            password: "secret",
            width: 1024,
            height: 768,
            scale: 100,
            keyboard_layout: 0x409,
            audio: false,
            share: None,
            gateway: Some(gateway(stand_in.port)),
        };
        let (sink, ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let sent = AtomicBool::default();
        let session = within(Session::connect(&target, known, sink, &sent)).await?;
        Ok((session, ops))
    }

    // A session through a stand-in for the test server, with both certificates pinned.
    async fn session_through(
        server: &TestServer,
        stand_in: &StandIn,
        known: &KnownHostsState,
    ) -> (Session, Ops) {
        let connected = connect_through(stand_in, known).await;
        let (session, mut ops) = connected.unwrap_or_else(|e| panic!("connect failed: {e:?}"));
        server.display.send(bitmap(0, 0, 8, 8, |_, _| [200, 100, 50])).unwrap();
        loop {
            match within(ops.recv()).await.expect("the session is gone") {
                FrameOp::Rect { .. } => break,
                FrameOp::Closed(text) => panic!("the session closed: {text}"),
                _ => {}
            }
        }
        (session, ops)
    }

    fn rdp_server(stream: tokio::io::DuplexStream) -> impl Future<Output = TestServer> {
        let login = Login::Nla("alice", "secret");
        TestServer::on(Options { login, size: (256, 128), remotefx: false }, stream)
    }

    #[tokio::test]
    async fn a_trusted_gateway_carries_the_rdp_login() {
        let dir = tempfile::tempdir().unwrap();
        let (near, far) = duplex(1 << 16);
        let server = rdp_server(far).await;
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let port = stand_in.port;
        let (session, _ops) = session_through(&server, &stand_in, &known(&dir)).await;
        within(session.close()).await;
        drop(session);
        let heard = heard(stand_in).await;
        assert_eq!(heard.request_line, "RDG_OUT_DATA /remoteDesktopGateway/ HTTP/1.1");
        // The stand-in's port is not 443, so the Host header carries it.
        let host = format!("127.0.0.1:{port}");
        assert_eq!(heard.header("host"), Some(host.as_str()));
        let login = format!("Basic {}", BASE64.encode("gw-user:gw-pass"));
        assert_eq!(heard.header("authorization"), Some(login.as_str()));
        assert_eq!(heard.channel, Some(("desk.lan".to_owned(), 3390)));
    }

    #[tokio::test]
    async fn a_server_name_outside_ascii_is_asked_for_with_the_right_length() {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(Script::default(), None).await;
        let (gateway, known) = (gateway(stand_in.port), known(&dir));
        let system = Arc::new(System(Ok(())));
        let opening =
            open_within(LOGIN_LIMIT, CONNECT_LIMIT, system, &gateway, "bürö.lan", 3390, &known);
        let (tunnel, _) = within(opening).await.unwrap();
        drop(tunnel);
        assert_eq!(heard(stand_in).await.channel, Some(("bürö.lan".to_owned(), 3390)));
    }

    #[tokio::test]
    async fn a_write_larger_than_one_packet_arrives_whole() {
        let dir = tempfile::tempdir().unwrap();
        let (near, mut far) = duplex(1 << 20);
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        let payload: Vec<u8> = (0..100_020u32).map(|n| (n % 251) as u8).collect();
        within(tunnel.write_all(&payload)).await.unwrap();
        let mut arrived = vec![0; payload.len()];
        within(far.read_exact(&mut arrived)).await.unwrap();
        assert!(arrived == payload, "the data arrived changed");
        drop(tunnel);
        let packets = heard(stand_in).await.packets;
        assert!(packets.iter().all(|&size| size <= 8192), "{packets:?}");
        let carried: usize = packets.iter().map(|size| size - 10).sum();
        assert_eq!(carried, payload.len());
    }

    // The session reads nothing while it writes.
    #[tokio::test]
    async fn a_large_write_goes_on_while_incoming_data_waits_unread() {
        let dir = tempfile::tempdir().unwrap();
        let incoming: Vec<Vec<u8>> = (0..8u8).map(|n| vec![n; 1000]).collect();
        let after = incoming.iter().map(|data| Message::binary(data_packet(data))).collect();
        let (near, mut far) = duplex(1 << 20);
        let stand_in = stand_in(Script { after, ..Script::default() }, Some(near)).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let payload: Vec<u8> = (0..100_020u32).map(|n| (n % 251) as u8).collect();
        let written = timeout(Duration::from_secs(5), tunnel.write_all(&payload)).await;
        assert!(written.expect("the write is stuck").is_ok());
        let mut arrived = vec![0; payload.len()];
        within(far.read_exact(&mut arrived)).await.unwrap();
        assert!(arrived == payload, "the data arrived changed");
        let mut read = vec![0; incoming.concat().len()];
        within(tunnel.read_exact(&mut read)).await.unwrap();
        assert_eq!(read, incoming.concat());
    }

    // A session through a stand-in for a server whose frames `script` writes once the connection
    // sequence is over.
    async fn scripted_through<T, Fut>(
        known: &KnownHostsState,
        script: impl FnOnce(ScriptedFramed<DuplexStream>, Channels) -> Fut + Send + 'static,
    ) -> (JoinHandle<T>, StandIn, Session, Ops)
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (near, far) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (framed, channels) = accept_scripted(far, (256, 128)).await;
            script(framed, channels).await
        });
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let connected = connect_through(&stand_in, known).await;
        let (session, ops) = connected.unwrap_or_else(|e| panic!("connect failed: {e:?}"));
        (server, stand_in, session, ops)
    }

    // Everything the sink is still given until the session has ended and dropped it.
    async fn rest(ops: &mut Ops) -> Vec<FrameOp> {
        let mut left = Vec::new();
        while let Some(op) = within(ops.recv()).await {
            left.push(op);
        }
        left
    }

    #[tokio::test]
    async fn a_logoff_through_the_gateway_ends_the_session_as_it_does_directly() {
        for round in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let (server, _stand_in, _session, mut ops) =
                scripted_through(&known(&dir), |mut framed, channels| async move {
                    let logoff = ProtocolIndependentCode::LogoffByUser;
                    let info = error_info(ErrorInfo::ProtocolIndependentCode(logoff), &channels);
                    framed.write_all(&info).await.unwrap();
                    framed.into_inner().0.shutdown().await.ok();
                })
                .await;
            let closed = [FrameOp::Closed("logged off".into())];
            assert_eq!(rest(&mut ops).await, closed, "round {round}");
            within(server).await.unwrap();
        }
    }

    // The frame end makes the session write its acknowledgement after the gateway has closed and
    // before it reads the logoff.
    #[tokio::test]
    async fn a_write_after_the_gateway_closed_does_not_hide_the_logoff() {
        for round in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let (server, _stand_in, _session, mut ops) =
                scripted_through(&known(&dir), |mut framed, channels| async move {
                    framed.write_all(&frame_marker(FrameAction::End, 7)).await.unwrap();
                    let logoff = ProtocolIndependentCode::LogoffByUser;
                    let info = error_info(ErrorInfo::ProtocolIndependentCode(logoff), &channels);
                    framed.write_all(&info).await.unwrap();
                    framed.into_inner().0.shutdown().await.ok();
                })
                .await;
            let ended = rest(&mut ops).await;
            let closed: Vec<_> =
                ended.iter().filter(|op| matches!(op, FrameOp::Closed(_))).collect();
            assert_eq!(closed, [&FrameOp::Closed("logged off".into())], "round {round}");
            within(server).await.unwrap();
        }
    }

    // As a socket after the peer's FIN: writes go nowhere until a read has met the end.
    #[tokio::test]
    async fn writes_after_the_gateway_closed_are_taken_until_a_read_meets_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let after = vec![Message::binary(data_packet(b"last words"))];
        let stand_in = stand_in(Script { after, ..Script::default() }, None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        stand_in.cut.send(()).unwrap();
        within(stand_in.heard).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        within(tunnel.write_all(b"an answer")).await.unwrap();
        within(tunnel.flush()).await.unwrap();
        let mut read = [0; 10];
        within(tunnel.read_exact(&mut read)).await.unwrap();
        assert_eq!(&read, b"last words");
        within(tunnel.write_all(b"another answer")).await.unwrap();
        let end = within(tunnel.read(&mut [0; 16])).await.expect_err("the tunnel read on");
        assert_eq!(end.kind(), std::io::ErrorKind::UnexpectedEof, "{end}");
        let write = within(tunnel.write_all(b"too late")).await.expect_err("a write went on");
        assert_eq!(write.kind(), std::io::ErrorKind::UnexpectedEof, "{write}");
        let flush = within(tunnel.flush()).await.expect_err("a flush went on");
        assert_eq!(flush.kind(), std::io::ErrorKind::UnexpectedEof, "{flush}");
    }

    // Flushes made while the task ends at the gateway's close, its last data unread: the task can
    // drop a flush unanswered, or end with one left in its channel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_flush_as_the_gateway_closes_ends_well_and_in_time() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let mut failed = Vec::new();
        for round in 0..200 {
            let after = vec![Message::binary(data_packet(b"last words"))];
            let stand_in = stand_in(Script { after, ..Script::default() }, None).await;
            let (mut tunnel, _) = open_to(&stand_in, true, &known).await.unwrap();
            let cutting = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(round % 7)).await;
                stand_in.cut.send(()).unwrap();
                stand_in.heard.await.unwrap();
            });
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(30) {
                if let Err(e) = within(tunnel.flush()).await {
                    failed.push((round, e.kind(), e.to_string()));
                    break;
                }
                tokio::task::yield_now().await;
            }
            within(cutting).await.unwrap();
        }
        assert!(failed.is_empty(), "{} of 200 failed: {failed:?}", failed.len());
    }

    #[tokio::test]
    async fn a_tunnel_that_broke_refuses_writes_with_its_data_unread() {
        let dir = tempfile::tempdir().unwrap();
        let mut broken = data_packet(b"hello");
        broken[4..8].copy_from_slice(&100u32.to_le_bytes());
        let after = vec![Message::binary(data_packet(b"before")), Message::binary(broken)];
        let stand_in = stand_in(Script { after, ..Script::default() }, None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let write = within(tunnel.write_all(b"an answer")).await.expect_err("a write went on");
        assert_ne!(write.kind(), std::io::ErrorKind::UnexpectedEof, "{write}");
        let mut read = [0; 6];
        within(tunnel.read_exact(&mut read)).await.unwrap();
        assert_eq!(&read, b"before");
    }

    #[tokio::test]
    async fn data_the_gateway_sent_before_it_closed_is_all_read() {
        let dir = tempfile::tempdir().unwrap();
        let incoming: Vec<Vec<u8>> = (0..8u8).map(|n| vec![n; 1000]).collect();
        let after = incoming.iter().map(|data| Message::binary(data_packet(data))).collect();
        let stand_in = stand_in(Script { after, ..Script::default() }, None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        stand_in.cut.send(()).unwrap();
        within(stand_in.heard).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut read = vec![0; incoming.concat().len()];
        within(tunnel.read_exact(&mut read)).await.unwrap();
        assert_eq!(read, incoming.concat());
        let end = within(tunnel.read(&mut [0; 16])).await.expect_err("the tunnel read on");
        assert_eq!(end.kind(), std::io::ErrorKind::UnexpectedEof, "{end}");
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_during_sign_in_through_the_gateway_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (near, far) = duplex(1 << 16);
        drop(far);
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let error = failed(connect_through(&stand_in, &known(&dir)).await);
        assert_eq!(text(&error), "rdp: the server closed the connection during sign-in");
    }

    #[tokio::test]
    async fn closing_a_session_through_the_gateway_delivers_the_disconnect() {
        for round in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let (server, stand_in, session, mut ops) =
                scripted_through(&known(&dir), |mut framed, channels| async move {
                    loop {
                        match read_client(&mut framed, &channels).await {
                            Ok(ClientPdu::ShutdownRequest) => {
                                framed.write_all(&shutdown_denied(&channels)).await.unwrap();
                            }
                            Ok(ClientPdu::Disconnect(reason)) => return Some(reason),
                            Ok(_) => {}
                            Err(_) => return None,
                        }
                    }
                })
                .await;
            within(session.close()).await;
            drop(session);
            let reason = within(server).await.unwrap();
            assert_eq!(reason, Some(DisconnectReason::UserRequested), "round {round}");
            assert!(heard(stand_in).await.closed, "round {round}: no WebSocket Close");
            assert_eq!(rest(&mut ops).await, [], "round {round}");
        }
    }

    #[tokio::test]
    async fn a_dropped_tunnel_sends_what_it_was_given_and_a_close() {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(Script::default(), None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        let payload: Vec<u8> = (0..30_000u32).map(|n| (n % 251) as u8).collect();
        within(tunnel.write_all(&payload)).await.unwrap();
        drop(tunnel);
        let heard = heard(stand_in).await;
        let carried: usize = heard.packets.iter().map(|size| size - 10).sum();
        assert_eq!(carried, payload.len());
        assert!(heard.closed, "no WebSocket Close");
    }

    #[tokio::test]
    async fn a_shut_down_tunnel_has_sent_what_it_was_given_and_a_close() {
        let dir = tempfile::tempdir().unwrap();
        let (near, mut far) = duplex(1 << 20);
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        let payload: Vec<u8> = (0..30_000u32).map(|n| (n % 251) as u8).collect();
        within(tunnel.write_all(&payload)).await.unwrap();
        within(tunnel.shutdown()).await.unwrap();
        let mut arrived = vec![0; payload.len()];
        within(far.read_exact(&mut arrived)).await.unwrap();
        assert!(arrived == payload, "the data arrived changed");
        assert!(within(tunnel.write_all(b"late")).await.is_err(), "a write after the shutdown");
        drop(tunnel);
        assert!(heard(stand_in).await.closed, "no WebSocket Close");
    }

    #[tokio::test]
    async fn a_ping_from_the_gateway_is_skipped_and_the_data_after_it_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let after = vec![
            Message::Ping(b"are you there".to_vec().into()),
            Message::Pong(Vec::new().into()),
            Message::binary(data_packet(b"hello")),
        ];
        let stand_in = stand_in(Script { after, ..Script::default() }, None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        let mut read = [0; 5];
        within(tunnel.read_exact(&mut read)).await.unwrap();
        assert_eq!(&read, b"hello");
        within(tunnel.write_all(b"still there")).await.unwrap();
        within(tunnel.flush()).await.unwrap();
        drop(tunnel);
        assert_eq!(heard(stand_in).await.packets, [10 + 11], "the write after the ping");
    }

    #[tokio::test]
    async fn a_401_without_basic_means_ntlm_is_required() {
        let responses: [&[u8]; 4] = [
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Negotiate\r\n\
              WWW-Authenticate: NTLM\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Negotiate, NTLM\r\n\
              Content-Length: 0\r\n\r\n",
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"a, Basic b\", \
              qop=\"auth\"\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n",
        ];
        for response in responses {
            assert_eq!(
                refusal(reply(response)).await,
                "rdp: the gateway requires NTLM, which isn't supported",
                "{}",
                shown(response)
            );
        }
    }

    #[tokio::test]
    async fn a_401_with_basic_means_a_wrong_login() {
        // What Windows Server 2025 answers a wrong password with.
        let windows = b"HTTP/1.1 401 Access Denied\r\nServer: Microsoft-HTTPAPI/2.0\r\n\
            RDG-Auth-Scheme: SMARTCARD\r\nWWW-Authenticate: Negotiate\r\n\
            WWW-Authenticate: NTLM\r\nWWW-Authenticate: Digest qop=\"auth\",\
            algorithm=MD5-sess,nonce=\"+Upgraded+v1b57bdaa367d0321b336f259b5aa060440b8f84ca15\",\
            charset=utf-8,realm=\"Digest\"\r\nWWW-Authenticate: Basic\r\n\
            Content-Length: 0\r\n\r\n";
        let responses: [&[u8]; 3] = [
            windows,
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Negotiate, Basic realm=\"gate\"\r\n\
              Content-Length: 0\r\n\r\n",
            b"HTTP/1.1 401 Unauthorized\r\nwww-authenticate: basic realm=\"restricted\"\r\n\
              Content-Length: 0\r\n\r\n",
        ];
        for response in responses {
            assert_eq!(
                refusal(reply(response)).await,
                "rdp: wrong gateway user name or password",
                "{}",
                shown(response)
            );
        }
    }

    #[tokio::test]
    async fn a_web_server_is_no_gateway() {
        for status in [404, 500] {
            let response = format!("HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\n\r\n");
            assert_eq!(
                refusal(reply(response.as_bytes())).await,
                format!("rdp: the gateway refused the connection (HTTP {status})")
            );
        }
        let page = "<html><body><form>Sign in</form></body></html>";
        let html = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
            page.len()
        );
        let moved = b"HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\n\r\n";
        let mut state = 0x2545_f491u32;
        let random: Vec<u8> = (0..512)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        for response in [html.as_bytes(), moved, &random] {
            assert_eq!(refusal(reply(response)).await, NOT_A_GATEWAY, "{}", shown(response));
        }
    }

    #[tokio::test]
    async fn a_silent_gateway_times_out() {
        let silent = Script { answer: Answer::Silent, ..Script::default() };
        let silent_after_the_upgrade = Script { handshake: Vec::new(), ..Script::default() };
        for script in [silent, silent_after_the_upgrade] {
            let dir = tempfile::tempdir().unwrap();
            let stand_in = stand_in(script, None).await;
            let system = Arc::new(System(Ok(())));
            let (gateway, known) = (gateway(stand_in.port), known(&dir));
            let limit = Duration::from_millis(300);
            let started = Instant::now();
            let opening =
                open_within(limit, CONNECT_LIMIT, system, &gateway, SERVER, SERVER_PORT, &known);
            let error = failed(within(opening).await);
            assert_eq!(text(&error), "rdp: the gateway did not answer in time");
            assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        }
    }

    #[tokio::test]
    async fn a_gateway_that_never_takes_the_connection_times_out() {
        // With the queue full the system drops further connection attempts.
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(1).unwrap();
        let addr = listener.local_addr().unwrap();
        let mut queued = Vec::new();
        for _ in 0..16 {
            let attempt = timeout(Duration::from_millis(100), TcpStream::connect(addr));
            if let Ok(Ok(stream)) = attempt.await {
                queued.push(stream);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let (gateway, known) = (gateway(addr.port()), known(&dir));
        let system = Arc::new(System(Ok(())));
        let limit = Duration::from_millis(200);
        let opening =
            open_within(LOGIN_LIMIT, limit, system, &gateway, SERVER, SERVER_PORT, &known);
        let error = failed(within(opening).await);
        assert_eq!(text(&error), "rdp: the gateway connection timed out");
        drop((listener, queued));
    }

    #[tokio::test]
    async fn malformed_tsgu_packets_end_with_a_message() {
        let handshake = handshake_response();
        let mut answers = Vec::new();
        // Cut short at every length, the header telling the length that is there.
        for length in 0..10 {
            answers.push(packet(HANDSHAKE_RESPONSE, &handshake[8..8 + length]));
        }
        // Shorter than a header, and a header that says less than its own size.
        answers.push(handshake[..3].to_vec());
        for length in [0u32, 4, 7] {
            let mut short = handshake.clone();
            short[4..8].copy_from_slice(&length.to_le_bytes());
            answers.push(short);
        }
        // A length beyond the packet.
        let mut long = handshake.clone();
        long[4..8].copy_from_slice(&118u32.to_le_bytes());
        answers.push(long);
        // A packet type MS-TSGU does not have.
        answers.push(packet(0x77, &handshake[8..]));
        let mut scripts: Vec<Script> = answers
            .into_iter()
            .map(|handshake| Script { handshake, ..Script::default() })
            .collect();
        // A tunnel response where the channel response belongs.
        let channel = packet(TUNNEL_RESPONSE, &channel_response(0)[8..]);
        scripts.push(Script { channel, ..Script::default() });
        // Bytes past the fields a tunnel or a channel response has.
        let tunnel = packet(TUNNEL_RESPONSE, &[&tunnel_response(0)[8..], &[0, 0]].concat());
        scripts.push(Script { tunnel, ..Script::default() });
        let channel = packet(CHANNEL_RESPONSE, &[&channel_response(0)[8..], &[0, 0]].concat());
        scripts.push(Script { channel, ..Script::default() });
        for (n, script) in scripts.into_iter().enumerate() {
            assert_eq!(refusal(script).await, NOT_A_GATEWAY, "script {n}");
        }
    }

    // Request 0 is the handshake, 1 the tunnel, 2 the tunnel authorization and 3 the channel.
    async fn hung_up_at(at: usize) -> Vec<String> {
        let mut said = Vec::new();
        for how in [Hangup::Close, Hangup::Reset] {
            said.push(refusal(Script { hangup: Some((at, how)), ..Script::default() }).await);
        }
        said
    }

    #[tokio::test]
    async fn a_gateway_that_hangs_up_on_the_channel_request_closed_before_connecting() {
        let closed = "rdp: the gateway closed the connection before connecting to desk.lan:3390";
        assert_eq!(hung_up_at(3).await, [closed, closed]);
    }

    #[tokio::test]
    async fn a_gateway_that_hangs_up_after_its_handshake_closed_the_connection() {
        let closed = "rdp: the gateway closed the connection";
        for at in [1, 2] {
            assert_eq!(hung_up_at(at).await, [closed, closed], "request {at}");
        }
    }

    #[tokio::test]
    async fn a_hang_up_before_the_handshake_answer_is_no_gateway() {
        assert_eq!(hung_up_at(0).await, [NOT_A_GATEWAY, NOT_A_GATEWAY]);
    }

    #[tokio::test]
    async fn a_refused_channel_names_the_server() {
        let script = Script { channel: channel_response(0x800759db), ..Script::default() };
        assert_eq!(
            refusal(script).await,
            "rdp: the gateway refused the connection to desk.lan:3390 (0x800759db)"
        );
    }

    #[tokio::test]
    async fn a_refused_tunnel_or_tunnel_login_names_the_server_and_the_code() {
        let tunnel = Script { tunnel: tunnel_response(0x800759f8), ..Script::default() };
        let login = Script { tunnel_auth: tunnel_auth_response(0x59dd), ..Script::default() };
        assert_eq!(
            refusal(tunnel).await,
            "rdp: the gateway refused the connection to desk.lan:3390 (0x800759f8)"
        );
        assert_eq!(
            refusal(login).await,
            "rdp: the gateway refused the connection to desk.lan:3390 (0x000059dd)"
        );
    }

    #[tokio::test]
    async fn a_message_the_gateway_wants_accepted_is_not_supported() {
        let script = Script { tunnel: consent_response("Rules of use"), ..Script::default() };
        assert_eq!(
            refusal(script).await,
            "rdp: the gateway shows a message that must be accepted, which isn't supported"
        );
    }

    #[tokio::test]
    async fn a_data_packet_longer_than_its_message_ends_the_tunnel_without_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        for length in [100u32, 4] {
            let mut packet = data_packet(b"hello");
            packet[4..8].copy_from_slice(&length.to_le_bytes());
            let script = Script { after: vec![Message::binary(packet)], ..Script::default() };
            let stand_in = stand_in(script, None).await;
            let (mut tunnel, _) = open_to(&stand_in, true, &known).await.unwrap();
            let error = within(tunnel.read(&mut [0; 16])).await.expect_err("the tunnel read on");
            let inner = error.get_ref().expect("no cause");
            assert!(!inner.is::<tokio::task::JoinError>(), "{length}: {error}");
        }
    }

    #[tokio::test]
    async fn a_tunnel_that_has_ended_fails_every_later_read_and_write() {
        let dir = tempfile::tempdir().unwrap();
        let stand_in = stand_in(Script::default(), None).await;
        let (mut tunnel, _) = open_to(&stand_in, true, &known(&dir)).await.unwrap();
        stand_in.cut.send(()).unwrap();
        within(stand_in.heard).await.unwrap();
        for _ in 0..2 {
            assert!(within(tunnel.read(&mut [0; 16])).await.is_err());
        }
        assert!(within(tunnel.write_all(b"late")).await.is_err());
        assert!(within(tunnel.read(&mut [0; 16])).await.is_err());
    }

    #[tokio::test]
    async fn a_lost_tunnel_ends_the_session_as_a_lost_link() {
        let dir = tempfile::tempdir().unwrap();
        let (near, far) = duplex(1 << 16);
        let server = rdp_server(far).await;
        let stand_in = stand_in(Script::default(), Some(near)).await;
        let (session, mut ops) = session_through(&server, &stand_in, &known(&dir)).await;
        stand_in.cut.send(()).unwrap();
        loop {
            match within(ops.recv()).await.expect("the session ended without a word") {
                FrameOp::Closed(text) => {
                    assert_eq!(text, "network connection lost");
                    break;
                }
                _ => {}
            }
        }
        drop(session);
    }
}
