use std::error::Error as _;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ironrdp_connector::credssp::CredsspSequence;
use ironrdp_connector::sspi::credssp::ClientState;
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::{
    custom_err, general_err, ClientConnector, ClientConnectorState, Config, ConnectionResult,
    ConnectorError, ConnectorErrorKind, Credentials, DesktopSize, ServerName,
};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::nego::{FailureCode, NegoRequestData};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_tokio::{FramedWrite as _, TokioFramed};
use rustls::client::Resumption;
use rustls::pki_types;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use x509_cert::der::Decode as _;

use super::err;
use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::tls;
use crate::remote::trust::{self, Scheme};

pub(crate) const LOGIN_LIMIT: Duration = Duration::from_secs(30);

pub(crate) const MAX_PIXELS: u32 = 40_000_000;

pub(crate) fn size_supported(width: u16, height: u16) -> bool {
    width != 0 && height != 0 && u32::from(width) * u32::from(height) <= MAX_PIXELS
}

pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub domain: Option<&'a str>,
    pub password: &'a str,
    pub width: u16,
    pub height: u16,
    pub scale: u32,
    pub keyboard_layout: u32,
}

pub(crate) struct Connected<S> {
    pub framed: TokioFramed<TlsStream<S>>,
    pub result: ConnectionResult,
}

pub(crate) fn config(target: &Target<'_>) -> Config {
    // The connector refuses a domain next to a user name that carries its own.
    let domain = target
        .domain
        .filter(|domain| !domain.is_empty() && !target.username.contains(['\\', '@']))
        .map(str::to_owned);
    Config {
        desktop_size: DesktopSize { width: target.width, height: target.height },
        desktop_scale_factor: if (100..=500).contains(&target.scale) { target.scale } else { 100 },
        enable_tls: true,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword {
            username: target.username.to_owned(),
            password: target.password.to_owned(),
        },
        domain,
        client_build: 0,
        client_name: "wisp".to_owned(),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: target.keyboard_layout,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: if cfg!(target_os = "windows") {
            MajorPlatformType::WINDOWS
        } else if cfg!(target_os = "macos") {
            MajorPlatformType::MACINTOSH
        } else {
            MajorPlatformType::UNIX
        },
        hardware_id: None,
        // The user name goes out only after the certificate has been checked.
        request_data: Some(NegoRequestData::cookie("wisp".to_owned())),
        autologon: !target.password.is_empty(),
        enable_audio_playback: false,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        compression_type: None,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
    }
}

pub(crate) fn io_kind(e: &ConnectorError) -> Option<ErrorKind> {
    let mut source = e.source();
    while let Some(inner) = source {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            return Some(io.kind());
        }
        source = inner.source();
    }
    None
}

fn sign_in_failure(failure: &ironrdp_connector::sspi::Error) -> String {
    let Some(status) = failure.nstatus else {
        return format!("sign-in failed: {}", failure.description);
    };
    match status.0 {
        0xc000006d | 0xc000006a | 0xc0000064 | 0xc007030c => "wrong user name or password".into(),
        0xc0000072 => "this account is disabled".into(),
        0xc0000234 => "this account is locked out".into(),
        0xc0000071 | 0xc0000224 => "the password has expired".into(),
        0xc000006e | 0xc000006f | 0xc0000070 | 0xc000015b => {
            "this account may not sign in here".into()
        }
        other => format!("sign-in failed: {} ({other:#010x})", failure.description),
    }
}

// ironrdp errors print as `[context @ source file:line] message`; the file is a build path.
fn without_location(text: &str) -> &str {
    text.strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .filter(|(location, _)| location.contains(" @ "))
        .map_or(text, |(_, message)| message)
}

fn server_name(host: &str) -> ServerName {
    ServerName::new(crate::net::normalize_host(host))
}

fn is_framing_failure(e: &ConnectorError) -> bool {
    let mut source = e.source();
    while let Some(inner) = source {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            if io.get_ref().is_some_and(|cause| cause.is::<ironrdp_core::DecodeError>()) {
                return true;
            }
        }
        source = inner.source();
    }
    false
}

pub(crate) fn refusal(e: &ConnectorError) -> AppError {
    const ENDED: [ErrorKind; 4] = [
        ErrorKind::UnexpectedEof,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
    ];
    match e.kind() {
        ConnectorErrorKind::Credssp(failure) => err(sign_in_failure(failure)),
        ConnectorErrorKind::AccessDenied => err("this account may not sign in here"),
        ConnectorErrorKind::Negotiation(failure)
            if failure.code() == FailureCode::HYBRID_REQUIRED_BY_SERVER =>
        {
            err("the server requires Network Level Authentication")
        }
        ConnectorErrorKind::Negotiation(failure) => {
            err(format!("the server refused the connection: {failure}"))
        }
        ConnectorErrorKind::Reason(reason) if reason.contains("but server selected") => {
            err("the server only offers the old RDP security layer, which is not supported")
        }
        _ if io_kind(e).is_some_and(|kind| ENDED.contains(&kind)) => {
            err("the server closed the connection during sign-in")
        }
        kind if matches!(kind, ConnectorErrorKind::Decode(_)) || is_framing_failure(e) => {
            err("the server sent data this client does not understand")
        }
        kind => {
            let mut text = kind.to_string();
            let mut source = e.source();
            while let Some(inner) = source {
                text.push_str(&format!(": {}", without_location(&inner.to_string())));
                source = inner.source();
            }
            err(text)
        }
    }
}

// `unread` is what the server sent that never made a frame: a reply that is no TPKT frame, or a
// short one that a close cut off.
fn begin_failure(e: &ConnectorError, unread: &[u8]) -> AppError {
    let garbled = matches!(e.kind(), ConnectorErrorKind::Decode(_))
        || (matches!(e.kind(), ConnectorErrorKind::Custom) && !unread.is_empty());
    if garbled {
        err("this does not look like an RDP server")
    } else {
        refusal(e)
    }
}

pub(crate) async fn login<S>(
    stream: S,
    local: SocketAddr,
    target: &Target<'_>,
    known: &KnownHostsState,
    attach: impl FnOnce(ClientConnector) -> ClientConnector,
    sent: &AtomicBool,
) -> AppResult<Connected<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    login_within(LOGIN_LIMIT, stream, local, target, known, attach, sent).await
}

async fn login_within<S>(
    limit: Duration,
    stream: S,
    local: SocketAddr,
    target: &Target<'_>,
    known: &KnownHostsState,
    attach: impl FnOnce(ClientConnector) -> ClientConnector,
    sent: &AtomicBool,
) -> AppResult<Connected<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    tokio::time::timeout(limit, sequence(stream, local, target, known, attach, sent))
        .await
        .map_err(|_| err("the server did not answer in time"))?
}

async fn sequence<S>(
    stream: S,
    local: SocketAddr,
    target: &Target<'_>,
    known: &KnownHostsState,
    attach: impl FnOnce(ClientConnector) -> ClientConnector,
    sent: &AtomicBool,
) -> AppResult<Connected<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let mut connector = attach(ClientConnector::new(config(target), local));
    let mut framed = TokioFramed::new(stream);
    let upgrade = match ironrdp_tokio::connect_begin(&mut framed, &mut connector).await {
        Ok(upgrade) => upgrade,
        Err(e) => return Err(begin_failure(&e, framed.peek())),
    };
    let (stream, leftover) = framed.into_inner();
    if !leftover.is_empty() {
        return Err(err("the server sent data before the secure connection started"));
    }

    let name = pki_types::ServerName::try_from(crate::net::normalize_host(target.host))
        .map_err(|_| err(format!("invalid host name: {}", target.host)))?;
    let mut config = tls::client_config();
    // CredSSP binds to the key of this very TLS session, so it must not run over a resumed one.
    config.resumption = Resumption::disabled();
    let stream = TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|e| err(tls::failure(&e)))?;

    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(|der| der.as_ref().to_vec())
        .ok_or_else(|| err("the server sent no certificate"))?;
    trust::check(known, Scheme::Rdp, target.host, target.port, &certificate)?;

    ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
    if connector.should_perform_credssp()
        && (target.username.is_empty() || target.password.is_empty())
    {
        return Err(err("this server needs a user name and password"));
    }
    let public_key = x509_cert::Certificate::from_der(&certificate)
        .ok()
        .and_then(|cert| {
            let key = cert.tbs_certificate.subject_public_key_info.subject_public_key;
            key.as_bytes().map(<[u8]>::to_vec)
        })
        .ok_or_else(|| err("the server's certificate could not be read"))?;

    // What ironrdp_tokio::connect_finalize does, with a mark once the credentials are out: from
    // then on a cancel lets the login finish (see RdpSessions::open).
    let mut framed = TokioFramed::new(stream);
    let mut buf = WriteBuf::new();
    if let ClientConnectorState::Credssp { selected_protocol } = connector.state {
        let (mut credssp, mut request) = CredsspSequence::init(
            connector.config.credentials.clone(),
            connector.config.domain.as_deref(),
            selected_protocol,
            server_name(target.host),
            public_key,
            None,
        )
        .map_err(|e| refusal(&e))?;
        loop {
            let state = match credssp.process_ts_request(request).start() {
                GeneratorState::Completed(state) => state.map_err(|e| {
                    refusal(&ConnectorError::new("CredSSP", ConnectorErrorKind::Credssp(e)))
                })?,
                GeneratorState::Suspended(_) => {
                    return Err(refusal(&general_err!("no network client (NTLM only)")))
                }
            };
            let (ClientState::ReplyNeeded(out) | ClientState::FinalMessage(out)) = &state;
            if out.auth_info.is_some() {
                sent.store(true, Ordering::SeqCst);
            }
            buf.clear();
            let written = credssp.handle_process_result(state, &mut buf).map_err(|e| refusal(&e))?;
            if let Some(length) = written.size() {
                framed
                    .write_all(&buf[..length])
                    .await
                    .map_err(|e| refusal(&custom_err!("write all", e)))?;
            }
            let Some(hint) = credssp.next_pdu_hint() else { break };
            let pdu = framed
                .read_by_hint(hint)
                .await
                .map_err(|e| refusal(&custom_err!("read frame by hint", e)))?;
            match credssp.decode_server_message(&pdu).map_err(|e| refusal(&e))? {
                Some(next) => request = next,
                None => break,
            }
        }
        connector.mark_credssp_as_done();
    }
    // Without CredSSP the credentials go out in the steps below.
    sent.store(true, Ordering::SeqCst);
    let result = loop {
        ironrdp_tokio::single_sequence_step(&mut framed, &mut connector, &mut buf)
            .await
            .map_err(|e| refusal(&e))?;
        if let ClientConnectorState::Connected { result } = connector.state {
            break result;
        }
    };

    let DesktopSize { width, height } = result.desktop_size;
    if !size_supported(width, height) {
        return Err(err("the server chose an unsupported screen size"));
    }
    Ok(Connected { framed, result })
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::ErrorKind;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use ironrdp_connector::sspi::credssp::NStatusCode;
    use ironrdp_connector::{sspi, Credentials, DesktopSize};
    use ironrdp_pdu::nego::{FailureCode, SecurityProtocol};
    use ironrdp_pdu::Action;
    use tokio::io::duplex;

    use super::*;
    use crate::rdp::testserver::{
        bitmap, certificate, negotiation_failure, off_thread, serve, short_frames, Login, Options,
        Script, TestServer,
    };
    use crate::remote::trust::fingerprint;
    use crate::ssh::known_hosts::KnownHosts;

    const HOST: &str = "desk.lan";
    const PORT: u16 = 3389;

    fn known(dir: &tempfile::TempDir) -> KnownHostsState {
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        KnownHostsState(Arc::new(Mutex::new(hosts)))
    }

    fn pinned(dir: &tempfile::TempDir) -> KnownHostsState {
        let known = known(dir);
        let print = fingerprint(&certificate());
        known.0.lock().unwrap().record(&format!("rdp/{HOST}"), PORT, &print).unwrap();
        known
    }

    fn target<'a>(username: &'a str, password: &'a str) -> Target<'a> {
        Target {
            host: HOST,
            port: PORT,
            username,
            domain: None,
            password,
            width: 1024,
            height: 768,
            scale: 100,
            keyboard_layout: 0x409,
        }
    }

    fn local() -> SocketAddr {
        "127.0.0.1:50000".parse().unwrap()
    }

    async fn within<T>(future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), future).await.expect("timed out")
    }

    fn options(login: Login) -> Options {
        Options { login, size: (800, 600), remotefx: false }
    }

    fn failed<S>(result: AppResult<Connected<S>>) -> AppError {
        match result {
            Ok(_) => panic!("expected the login to fail"),
            Err(e) => e,
        }
    }

    fn text(e: &AppError) -> &str {
        match e {
            AppError::Internal(message) => message,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    async fn login_to(
        client: tokio::io::DuplexStream,
        target: &Target<'_>,
        known: &KnownHostsState,
    ) -> AppResult<Connected<tokio::io::DuplexStream>> {
        within(login(client, local(), target, known, |connector| connector, &AtomicBool::default()))
            .await
    }

    #[tokio::test]
    async fn nla_login_connects_and_reports_the_granted_size() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = TestServer::on(options(Login::Nla("alice", "secret")), stream).await;
        let connected = login_to(client, &target("alice", "secret"), &pinned(&dir)).await;
        let connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
        assert_eq!(connected.result.desktop_size, DesktopSize { width: 800, height: 600 });
    }

    #[tokio::test]
    async fn the_credentials_count_as_sent_once_credssp_is_done() {
        let attempts = [
            (Login::Nla("alice", "secret"), "secret", true),
            (Login::Nla("alice", "secret"), "wrong", false),
            (Login::Tls, "secret", true),
        ];
        for (login_as, password, expected) in attempts {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let _server = TestServer::on(options(login_as), stream).await;
            let target = target("alice", password);
            let sent = AtomicBool::default();
            let known = pinned(&dir);
            let connected =
                within(login(client, local(), &target, &known, |connector| connector, &sent)).await;
            assert_eq!(connected.is_ok(), expected, "{password}");
            assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), expected, "{password}");
        }
    }

    #[tokio::test]
    async fn the_returned_stream_carries_the_session_on() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let server = TestServer::on(options(Login::Nla("alice", "secret")), stream).await;
        let connected = login_to(client, &target("alice", "secret"), &pinned(&dir)).await;
        let mut connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
        server.display.send(bitmap(0, 0, 8, 8, |_, _| [200, 100, 50])).unwrap();
        let (action, frame) = within(connected.framed.read_pdu()).await.unwrap();
        assert_eq!(action, Action::FastPath);
        assert!(frame.len() > 8 * 8 * 4);
    }

    #[tokio::test]
    async fn tls_only_login_connects() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = TestServer::on(options(Login::Tls), stream).await;
        let connected = login_to(client, &target("alice", "secret"), &pinned(&dir)).await;
        let connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
        assert_eq!(connected.result.desktop_size, DesktopSize { width: 800, height: 600 });
    }

    #[tokio::test]
    async fn tls_only_login_does_not_need_a_password() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = TestServer::on(options(Login::Tls), stream).await;
        let connected = login_to(client, &target("", ""), &pinned(&dir)).await;
        assert!(connected.is_ok());
    }

    #[tokio::test]
    async fn unknown_certificate_stops_the_login_before_any_credential() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let server = serve(stream, Script::Tls(SecurityProtocol::HYBRID_EX));
        let error = failed(login_to(client, &target("alice", "secret"), &known(&dir)).await);
        match error {
            AppError::HostKeyUnknown { host, port, fingerprint: offered } => {
                assert_eq!(host, format!("rdp/{HOST}"));
                assert_eq!(port, PORT);
                assert_eq!(offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyUnknown, got {other:?}"),
        }
        let heard = within(server).await.unwrap();
        assert!(heard.after_tls.is_empty());
    }

    #[tokio::test]
    async fn another_recorded_certificate_is_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let other = fingerprint(b"another certificate");
        known.0.lock().unwrap().record(&format!("rdp/{HOST}"), PORT, &other).unwrap();
        let (client, stream) = duplex(1 << 16);
        let server = serve(stream, Script::Tls(SecurityProtocol::HYBRID_EX));
        let error = failed(login_to(client, &target("alice", "secret"), &known).await);
        match error {
            AppError::HostKeyMismatch { host, port, stored, offered } => {
                assert_eq!(host, format!("rdp/{HOST}"));
                assert_eq!(port, PORT);
                assert_eq!(stored, other);
                assert_eq!(offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyMismatch, got {other:?}"),
        }
        assert!(within(server).await.unwrap().after_tls.is_empty());
    }

    #[tokio::test]
    async fn a_vnc_pin_for_the_same_host_and_port_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let known = known(&dir);
        let print = fingerprint(&certificate());
        known.0.lock().unwrap().record(&format!("vnc/{HOST}"), PORT, &print).unwrap();
        let (client, stream) = duplex(1 << 16);
        let server = serve(stream, Script::Tls(SecurityProtocol::HYBRID_EX));
        let error = failed(login_to(client, &target("alice", "secret"), &known).await);
        assert!(matches!(error, AppError::HostKeyUnknown { host, .. } if host == "rdp/desk.lan"));
        assert!(within(server).await.unwrap().after_tls.is_empty());
    }

    #[tokio::test]
    async fn a_wrong_password_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = TestServer::on(options(Login::Nla("alice", "secret")), stream).await;
        let error = failed(login_to(client, &target("alice", "guess"), &pinned(&dir)).await);
        assert_eq!(text(&error), "rdp: wrong user name or password");
    }

    fn credssp(status: Option<u32>) -> ConnectorError {
        ConnectorError::new(
            "CredSSP",
            ConnectorErrorKind::Credssp(sspi::Error {
                error_type: sspi::ErrorKind::InvalidToken,
                description: "CredSSP server returned an error status".into(),
                nstatus: status.map(NStatusCode),
            }),
        )
    }

    #[test]
    fn refusal_words_follow_the_status_the_server_sent() {
        let table: [(&[u32], &str); 5] = [
            (&[0xc000006d, 0xc000006a, 0xc0000064, 0xc007030c], "wrong user name or password"),
            (&[0xc0000072], "this account is disabled"),
            (&[0xc0000234], "this account is locked out"),
            (&[0xc0000071, 0xc0000224], "the password has expired"),
            (
                &[0xc000006e, 0xc000006f, 0xc0000070, 0xc000015b],
                "this account may not sign in here",
            ),
        ];
        for (statuses, words) in table {
            for status in statuses {
                let error = refusal(&credssp(Some(*status)));
                assert_eq!(text(&error), format!("rdp: {words}"), "{status:#x}");
            }
        }
        let access_denied = ConnectorError::new("CredSSP", ConnectorErrorKind::AccessDenied);
        assert_eq!(text(&refusal(&access_denied)), "rdp: this account may not sign in here");
    }

    #[test]
    fn an_unknown_status_is_shown_with_its_value() {
        let error = refusal(&credssp(Some(0xc0000133)));
        assert_eq!(
            text(&error),
            "rdp: sign-in failed: CredSSP server returned an error status (0xc0000133)"
        );
        let error = refusal(&credssp(None));
        assert_eq!(text(&error), "rdp: sign-in failed: CredSSP server returned an error status");
    }

    #[test]
    fn negotiation_and_protocol_refusals_are_named() {
        let required =
            ConnectorErrorKind::Negotiation(FailureCode::HYBRID_REQUIRED_BY_SERVER.into());
        let error = refusal(&ConnectorError::new("Initiation", required));
        assert_eq!(text(&error), "rdp: the server requires Network Level Authentication");

        let other = ConnectorErrorKind::Negotiation(FailureCode::SSL_CERT_NOT_ON_SERVER.into());
        let error = refusal(&ConnectorError::new("Initiation", other));
        assert_eq!(
            text(&error),
            "rdp: the server refused the connection: server lacks valid authentication certificate"
        );

        let reason = ConnectorErrorKind::Reason(
            "client advertised PROTOCOL_SSL, but server selected (empty)".into(),
        );
        let error = refusal(&ConnectorError::new("Initiation", reason));
        assert_eq!(
            text(&error),
            "rdp: the server only offers the old RDP security layer, which is not supported"
        );

        let other = ConnectorErrorKind::Reason("unsupported color depth".into());
        let error = refusal(&ConnectorError::new("Settings", other));
        assert_eq!(text(&error), "rdp: reason: unsupported color depth");
    }

    #[test]
    fn a_connection_that_ended_is_told_as_such() {
        for kind in [ErrorKind::UnexpectedEof, ErrorKind::ConnectionReset, ErrorKind::BrokenPipe] {
            let error = ConnectorError::new("read frame by hint", ConnectorErrorKind::Custom)
                .with_source(std::io::Error::from(kind));
            assert_eq!(
                text(&refusal(&error)),
                "rdp: the server closed the connection during sign-in",
                "{kind:?}"
            );
        }
        let error = ConnectorError::new("write all", ConnectorErrorKind::Custom)
            .with_source(std::io::Error::new(ErrorKind::TimedOut, "no luck"));
        assert_eq!(text(&refusal(&error)), "rdp: custom error: no luck");
    }

    #[tokio::test]
    async fn nla_without_a_password_or_a_user_name_stops_before_credssp() {
        for (username, password) in [("alice", ""), ("", "secret"), ("", "")] {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let server = serve(stream, Script::Tls(SecurityProtocol::HYBRID_EX));
            let error = failed(login_to(client, &target(username, password), &pinned(&dir)).await);
            assert_eq!(text(&error), "rdp: this server needs a user name and password");
            let heard = within(server).await.unwrap();
            assert!(heard.after_tls.is_empty(), "{username:?} {password:?}");
        }
    }

    #[tokio::test]
    async fn the_request_carries_a_fixed_cookie_and_not_the_user_name() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let refusal = negotiation_failure(FailureCode::HYBRID_REQUIRED_BY_SERVER);
        let server = serve(stream, Script::Reply(refusal));
        let target = target("alice.secret", "hunter2");
        failed(login_to(client, &target, &pinned(&dir)).await);
        let request = within(server).await.unwrap().request;
        let contains = |needle: &[u8]| request.windows(needle.len()).any(|part| part == needle);
        assert!(contains(b"Cookie: mstshash=wisp"));
        assert!(!contains(b"alice.secret"));
        assert!(!contains(b"hunter2"));
    }

    fn config_for(username: &str, domain: Option<&str>, password: &str, scale: u32) -> Config {
        let target = Target { domain, scale, ..target(username, password) };
        config(&target)
    }

    #[test]
    fn the_domain_is_only_passed_with_a_plain_user_name() {
        assert_eq!(config_for("alice", Some("CORP"), "pw", 100).domain.as_deref(), Some("CORP"));
        assert_eq!(config_for("CORP\\alice", Some("X"), "pw", 100).domain, None);
        assert_eq!(config_for("alice@corp.example", Some("X"), "pw", 100).domain, None);
        assert_eq!(config_for("alice", Some(""), "pw", 100).domain, None);
        assert_eq!(config_for("alice", None, "pw", 100).domain, None);
        let config = config_for("CORP\\alice", None, "pw", 100);
        match config.credentials {
            Credentials::UsernamePassword { username, password } => {
                assert_eq!((username.as_str(), password.as_str()), ("CORP\\alice", "pw"));
            }
            other => panic!("expected a password login, got {other:?}"),
        }
    }

    #[test]
    fn config_follows_the_target() {
        let config = config_for("alice", None, "pw", 100);
        assert!(config.autologon);
        assert!(!config_for("alice", None, "", 100).autologon);
        assert_eq!(config_for("alice", None, "pw", 90).desktop_scale_factor, 100);
        assert_eq!(config_for("alice", None, "pw", 501).desktop_scale_factor, 100);
        assert_eq!(config_for("alice", None, "pw", 150).desktop_scale_factor, 150);
        assert_eq!(config_for("alice", None, "pw", 500).desktop_scale_factor, 500);
        assert_eq!(config.desktop_size, DesktopSize { width: 1024, height: 768 });
        assert_eq!(config.keyboard_layout, 0x409);
        assert!(config.enable_tls && config.enable_credssp);
        assert!(config.bitmap.is_none() && config.compression_type.is_none());
        assert!(config.enable_server_pointer && !config.pointer_software_rendering);
        assert!(!config.enable_audio_playback);
        assert_eq!(config.client_name, "wisp");
    }

    #[tokio::test]
    async fn a_server_that_requires_nla_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let refusal = negotiation_failure(FailureCode::HYBRID_REQUIRED_BY_SERVER);
        let _server = serve(stream, Script::Reply(refusal));
        let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
        assert_eq!(text(&error), "rdp: the server requires Network Level Authentication");
    }

    #[tokio::test]
    async fn a_port_that_speaks_something_else_is_not_an_rdp_server() {
        let scripts = [
            Script::Reply(b"SSH-2.0-OpenSSH_9.9\r\n".to_vec()),
            Script::ReplyThenHangUp(b"HTTP/1.1 400 Bad Request\r\n\r\n".to_vec()),
        ];
        for script in scripts {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let _server = serve(stream, script);
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            assert_eq!(text(&error), "rdp: this does not look like an RDP server");
        }
    }

    fn injected() -> Vec<u8> {
        [&[3, 0, 0, 22][..], b"INJECTED-PLAINTEXT"].concat()
    }

    #[tokio::test]
    async fn plaintext_sent_with_the_confirmation_is_refused() {
        for protocol in [SecurityProtocol::HYBRID_EX, SecurityProtocol::SSL] {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let server = serve(stream, Script::Inject(protocol, injected(), false));
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            assert_eq!(
                text(&error),
                "rdp: the server sent data before the secure connection started",
                "{protocol:?}"
            );
            assert!(within(server).await.unwrap().after_tls.is_empty());
        }
    }

    #[tokio::test]
    async fn plaintext_sent_right_behind_the_confirmation_never_reaches_the_connector() {
        for _ in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let script = Script::Inject(SecurityProtocol::HYBRID_EX, injected(), true);
            let server = serve(stream, script);
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            let message = text(&error);
            assert!(
                message == "rdp: the server sent data before the secure connection started"
                    || message.starts_with("rdp: TLS handshake failed: "),
                "{message}"
            );
            assert!(within(server).await.unwrap().after_tls.is_empty());
        }
    }

    const UNREADABLE: &str = "rdp: the server sent data this client does not understand";

    fn clean(message: &str) {
        for internal in ["@", ".rs", "\\registry", "index.crates.io"] {
            assert!(!message.contains(internal), "{internal:?} in {message:?}");
        }
    }

    #[test]
    fn the_location_prefix_of_a_library_error_is_dropped() {
        let measured = "[ironrdp_pdu::ber::read_application_tag @ \
            C:/Users/domin/.cargo\\registry\\src\\\
            index.crates.io-1949cf8c6b5b557f\\ironrdp-core-0.2.1\\src\\error.rs:101] \
            invalid identifier: expected 0x60";
        assert_eq!(without_location(measured), "invalid identifier: expected 0x60");
        assert_eq!(without_location("invalid identifier"), "invalid identifier");
        assert_eq!(without_location("[1] not a location"), "[1] not a location");
        assert_eq!(without_location("[no end @ x.rs:1"), "[no end @ x.rs:1");
    }

    #[test]
    fn nested_library_errors_are_shown_without_their_locations() {
        let inner = ConnectorError::new("inner", ConnectorErrorKind::Reason("boom".into()));
        let error = ConnectorError::new("outer", ConnectorErrorKind::Custom).with_source(inner);
        assert_eq!(text(&refusal(&error)), "rdp: custom error: reason: boom");
    }

    #[tokio::test]
    async fn data_the_client_cannot_read_after_tls_is_named_without_internals() {
        let tpkt_short = vec![3, 0, 0, 3];
        let bad_type = vec![9, 0, 0, 12, 2, 0xf0, 0x80, 0, 0, 0, 0, 0];
        for bytes in [bad_type, injected(), tpkt_short] {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let _server = serve(stream, Script::TlsThenSend(SecurityProtocol::SSL, bytes.clone()));
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            assert_eq!(text(&error), UNREADABLE, "{bytes:?}");
        }
    }

    #[tokio::test]
    async fn empty_and_cut_short_frames_end_the_login_or_wait_for_more() {
        for bytes in short_frames() {
            let dir = tempfile::tempdir().unwrap();
            let known = pinned(&dir);
            let (client, stream) = duplex(1 << 16);
            let _server = serve(stream, Script::TlsThenSend(SecurityProtocol::SSL, bytes.clone()));
            let login = off_thread(Duration::from_secs(3), move || async move {
                let target = target("alice", "secret");
                let limit = Duration::from_millis(300);
                let sent = AtomicBool::default();
                login_within(limit, client, local(), &target, &known, |connector| connector, &sent)
                    .await
                    .err()
            });
            let error = login.await.unwrap_or_else(|| panic!("the login spins on {bytes:?}"));
            let message = text(&error.expect("the login worked")).to_owned();
            assert!(
                message == UNREADABLE || message == "rdp: the server did not answer in time",
                "{bytes:?}: {message}"
            );
        }
    }

    #[tokio::test]
    async fn a_frame_of_length_zero_is_data_the_client_does_not_understand() {
        let dir = tempfile::tempdir().unwrap();
        let known = pinned(&dir);
        let (client, stream) = duplex(1 << 16);
        let _server = serve(stream, Script::TlsThenSend(SecurityProtocol::SSL, vec![0; 40]));
        let login = off_thread(Duration::from_secs(3), move || async move {
            let target = target("alice", "secret");
            let sent = AtomicBool::default();
            login(client, local(), &target, &known, |connector| connector, &sent).await.err()
        });
        let error = login.await.expect("the login spins");
        assert_eq!(text(&error.expect("the login worked")), UNREADABLE);
    }

    #[tokio::test]
    async fn no_message_shows_where_a_library_failed() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = serve(stream, Script::NotTls(SecurityProtocol::SSL));
        let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
        clean(text(&error));

        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = TestServer::on(options(Login::Nla("alice", "secret")), stream).await;
        let error = failed(login_to(client, &target("a\\b\\c", "secret"), &pinned(&dir)).await);
        clean(text(&error));
        assert!(text(&error).starts_with("rdp: "), "{error:?}");
    }

    #[test]
    fn the_server_name_is_the_bare_host() {
        assert_eq!(server_name("[fe80::1]").as_str(), "fe80::1");
        assert_eq!(server_name("fe80::1").as_str(), "fe80::1");
        assert_eq!(server_name("desk.lan").as_str(), "desk.lan");
    }

    #[tokio::test]
    async fn a_silent_server_runs_into_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = serve(stream, Script::Silent);
        let started = Instant::now();
        let target = target("alice", "secret");
        let known = pinned(&dir);
        let result = within(login_within(
            Duration::from_millis(200),
            client,
            local(),
            &target,
            &known,
            |connector| connector,
            &AtomicBool::default(),
        ))
        .await;
        assert_eq!(text(&failed(result)), "rdp: the server did not answer in time");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_after_the_handshake_is_named() {
        for protocol in [SecurityProtocol::HYBRID_EX, SecurityProtocol::SSL] {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let _server = serve(stream, Script::TlsThenHangUp(protocol));
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            assert_eq!(
                text(&error),
                "rdp: the server closed the connection during sign-in",
                "{protocol:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_server_that_closes_before_answering_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = serve(stream, Script::HangUp);
        let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
        assert_eq!(text(&error), "rdp: the server closed the connection during sign-in");
    }

    #[tokio::test]
    async fn a_failed_tls_handshake_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (client, stream) = duplex(1 << 16);
        let _server = serve(stream, Script::NotTls(SecurityProtocol::HYBRID_EX));
        let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
        assert!(text(&error).starts_with("rdp: TLS handshake failed: "), "{error:?}");
    }

    #[tokio::test]
    async fn a_desktop_the_server_chose_too_large_or_empty_is_refused() {
        for size in [(8192, 8192), (0, 480), (800, 0)] {
            let dir = tempfile::tempdir().unwrap();
            let (client, stream) = duplex(1 << 16);
            let options = Options { login: Login::Tls, size, remotefx: false };
            let _server = TestServer::on(options, stream).await;
            let error = failed(login_to(client, &target("alice", "secret"), &pinned(&dir)).await);
            assert_eq!(
                text(&error),
                "rdp: the server chose an unsupported screen size",
                "{size:?}"
            );
        }
    }
}
