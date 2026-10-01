use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use super::err;
use super::handshake::{Login, Stream, TlsContext, NEEDS_PASSWORD};
use super::proto::vnc_auth_response;
use crate::error::{AppError, AppResult};
use crate::remote::trust::{self, Scheme};

const NONE: u32 = 1;
const VNC_PASSWORD: u32 = 2;
const X509_NONE: u32 = 260;
const X509_VNC: u32 = 261;
const X509_PLAIN: u32 = 262;

fn subtype_name(subtype: u32) -> String {
    match subtype {
        NONE => "None".into(),
        VNC_PASSWORD => "VNC password".into(),
        256 => "VeNCrypt Plain (unencrypted)".into(),
        257 => "VeNCrypt TLSNone (no certificate)".into(),
        258 => "VeNCrypt TLSVnc (no certificate)".into(),
        259 => "VeNCrypt TLSPlain (no certificate)".into(),
        X509_NONE => "VeNCrypt X509None".into(),
        X509_VNC => "VeNCrypt X509Vnc".into(),
        X509_PLAIN => "VeNCrypt X509Plain".into(),
        263 => "VeNCrypt X509SASL".into(),
        264 => "VeNCrypt TLSSASL (no certificate)".into(),
        n => format!("VeNCrypt subtype {n}"),
    }
}

// X509 first. Failing that, subtypes 1 and 2: they are the RFB types of the same number, which
// the client accepts outside VeNCrypt too. Plain, SASL and anonymous TLS are never chosen.
fn choose_subtype(offered: &[u32], login: &Login<'_>) -> Result<u32, String> {
    let preferred: &[u32] = match (login.password.is_empty(), login.username.is_empty()) {
        (true, _) => &[X509_NONE, NONE],
        (false, true) => &[X509_VNC, X509_PLAIN, VNC_PASSWORD],
        (false, false) => &[X509_PLAIN, X509_VNC, VNC_PASSWORD],
    };
    if let Some(subtype) = preferred.iter().find(|subtype| offered.contains(subtype)) {
        return Ok(*subtype);
    }
    let with_password = [VNC_PASSWORD, X509_VNC, X509_PLAIN];
    if login.password.is_empty() && offered.iter().any(|subtype| with_password.contains(subtype)) {
        return Err(NEEDS_PASSWORD.into());
    }
    let mut names: Vec<String> = Vec::new();
    for subtype in offered {
        let name = subtype_name(*subtype);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Err(format!("the server offers only: {}", names.join(", ")))
}

// Accepts any chain and name: trust comes from the pinned fingerprint, checked after the handshake.
#[derive(Debug)]
struct PinnedLater(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for PinnedLater {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

// rustls has only a Debug dump for a certificate it cannot read.
fn tls_failure(e: std::io::Error) -> AppError {
    let unsupported = match e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(CertificateError::Other(other)))
            if other.to_string() == "UnsupportedCertVersion" =>
        {
            Some("not X.509 version 3")
        }
        Some(rustls::Error::InvalidCertificate(
            CertificateError::UnsupportedSignatureAlgorithmContext { .. }
            | CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. },
        )) => Some("signature algorithm"),
        _ => None,
    };
    match unsupported {
        Some(reason) => err(format!("the server's certificate is not supported ({reason})")),
        None => err(format!("TLS handshake failed: {e}")),
    }
}

async fn answer_challenge(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    password: &str,
) -> AppResult<()> {
    let mut challenge = [0u8; 16];
    stream.read_exact(&mut challenge).await?;
    stream.write_all(&vnc_auth_response(password, &challenge)).await?;
    stream.flush().await?;
    Ok(())
}

// Runs VeNCrypt 0.2 up to and including the login, inside TLS for the X509 subtypes;
// SecurityResult is the caller's.
pub async fn negotiate(
    mut stream: Box<dyn Stream>,
    login: &Login<'_>,
    tls: &TlsContext<'_>,
) -> AppResult<Box<dyn Stream>> {
    let (major, minor) = (stream.read_u8().await?, stream.read_u8().await?);
    if (major, minor) < (0, 2) {
        return Err(err(format!("unsupported VeNCrypt version {major}.{minor}")));
    }
    stream.write_all(&[0, 2]).await?;
    stream.flush().await?;
    if stream.read_u8().await? != 0 {
        return Err(err("the server does not speak VeNCrypt 0.2"));
    }

    let count = stream.read_u8().await?;
    if count == 0 {
        return Err(err("the server offers no VeNCrypt login"));
    }
    let mut offered = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offered.push(stream.read_u32().await?);
    }
    let subtype = choose_subtype(&offered, login).map_err(err)?;

    // These two run as the RFB types of the same number do: no ack from the server, no TLS.
    if matches!(subtype, NONE | VNC_PASSWORD) {
        trust::check_unencrypted(tls.known, Scheme::Vnc, tls.host, tls.port)?;
        stream.write_all(&subtype.to_be_bytes()).await?;
        stream.flush().await?;
        if subtype == VNC_PASSWORD {
            answer_challenge(&mut stream, login.password).await?;
        }
        return Ok(stream);
    }

    let name = ServerName::try_from(crate::net::normalize_host(tls.host))
        .map_err(|_| err(format!("invalid host name: {}", tls.host)))?;
    stream.write_all(&subtype.to_be_bytes()).await?;
    stream.flush().await?;
    if stream.read_u8().await? != 1 {
        return Err(err(format!("the server refused {}", subtype_name(subtype))));
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinnedLater(provider.signature_verification_algorithms);
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| err(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let mut stream = TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(tls_failure)?;

    // Nothing of the login may be written before this check has passed.
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or_else(|| err("the server sent no certificate"))?;
    trust::check(tls.known, Scheme::Vnc, tls.host, tls.port, certificate)?;

    match subtype {
        X509_VNC => answer_challenge(&mut stream, login.password).await?,
        X509_PLAIN => {
            // One write, so that the TLS record sizes do not give away each length.
            let len = 8 + login.username.len() + login.password.len();
            let mut plain = Zeroizing::new(Vec::with_capacity(len));
            plain.extend((login.username.len() as u32).to_be_bytes());
            plain.extend((login.password.len() as u32).to_be_bytes());
            plain.extend(login.username.bytes());
            plain.extend(login.password.bytes());
            stream.write_all(&plain).await?;
            stream.flush().await?;
        }
        _ => {}
    }

    Ok(Box::new(stream))
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::sync::Mutex;
    use std::time::Duration;

    use rustls::crypto::ring::default_provider;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::PrivateKeyDer;
    use rustls::sign::{CertifiedKey, SingleCertAndKey};
    use rustls::version::{TLS12, TLS13};
    use rustls::{ServerConfig, SupportedProtocolVersion};
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
    use tokio::task::JoinHandle;
    use tokio_rustls::TlsAcceptor;

    use super::*;
    use crate::commands::ssh_cmds::KnownHostsState;
    use crate::error::AppError;
    use crate::remote::trust::{fingerprint, UNENCRYPTED};
    use crate::ssh::known_hosts::{HostKeyVerdict, KnownHosts};
    use crate::vnc::handshake::{handshake, ServerInit};
    use Step::{Read, Write};

    // The server's side of a script: bytes it writes, and how many it then reads from the client.
    enum Step {
        Write(Vec<u8>),
        Read(usize),
    }

    // `plain` runs before TLS. Without `tls` the server never gets to a TLS handshake.
    struct Server {
        plain: Vec<Step>,
        tls: Option<(TlsAcceptor, Vec<Step>)>,
    }

    // What the client sent in the clear and over TLS. `secured` is None without a TLS session.
    struct Seen {
        plain: Vec<u8>,
        secured: Option<Vec<u8>>,
        sni: Option<String>,
    }

    // `left` is what the server wrote after the login, `pin` what is on record for the host
    // afterwards.
    struct Outcome {
        result: AppResult<Option<ServerInit>>,
        seen: Seen,
        left: Vec<u8>,
        pin: Option<String>,
    }

    fn certificate() -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(include_bytes!("fixtures/test-cert.pem")).unwrap()
    }

    // Signs with the fixture key whatever certificate is presented.
    fn acceptor(
        certificate: CertificateDer<'static>,
        versions: &[&'static SupportedProtocolVersion],
    ) -> TlsAcceptor {
        let provider = Arc::new(default_provider());
        let key = PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/test-key.pem")).unwrap();
        let key = provider.key_provider.load_private_key(key).unwrap();
        let resolver = SingleCertAndKey::from(CertifiedKey::new(vec![certificate], key));
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        // Tickets go out after the handshake; writing them to a client that has already hung up
        // would fail the accept and hide what that client sent.
        config.send_tls13_tickets = 0;
        TlsAcceptor::from(Arc::new(config))
    }

    // A VeNCrypt 0.2 server up to its list of subtypes.
    fn offering(subtypes: &[u32]) -> Vec<Step> {
        let mut list = vec![subtypes.len() as u8];
        for subtype in subtypes {
            list.extend(subtype.to_be_bytes());
        }
        vec![Write(vec![0, 2]), Read(2), Write(vec![0]), Write(list)]
    }

    // The same, accepting the client's choice and going on to `secured` over TLS.
    fn x509(subtypes: &[u32], secured: Vec<Step>) -> Server {
        let mut plain = offering(subtypes);
        plain.extend([Read(4), Write(vec![1])]);
        Server { plain, tls: Some((acceptor(certificate(), &[&TLS13, &TLS12]), secured)) }
    }

    async fn run(io: &mut (impl AsyncRead + AsyncWrite + Unpin), steps: Vec<Step>) -> Vec<u8> {
        let mut sent = Vec::new();
        for step in steps {
            match step {
                Write(bytes) => {
                    io.write_all(&bytes).await.unwrap();
                    io.flush().await.unwrap();
                }
                Read(n) => {
                    let mut buf = vec![0u8; n];
                    io.read_exact(&mut buf).await.unwrap();
                    sent.extend(buf);
                }
            }
        }
        sent
    }

    fn serve(mut io: DuplexStream, server: Server) -> JoinHandle<Seen> {
        tokio::spawn(async move {
            let mut plain = run(&mut io, server.plain).await;
            let Some((acceptor, steps)) = server.tls else {
                io.shutdown().await.unwrap();
                io.read_to_end(&mut plain).await.unwrap();
                return Seen { plain, secured: None, sni: None };
            };
            let Ok(mut tls) = acceptor.accept(io).await else {
                return Seen { plain, secured: None, sni: None };
            };
            let sni = tls.get_ref().1.server_name().map(str::to_owned);
            let mut secured = run(&mut tls, steps).await;
            match tls.read_to_end(&mut secured).await {
                Ok(_) => tls.shutdown().await.unwrap(),
                // The client hung up without a close_notify.
                Err(e) => assert_eq!(e.kind(), ErrorKind::UnexpectedEof),
            }
            Seen { plain, secured: Some(secured), sni }
        })
    }

    // `pinned` is the fingerprint on record for the host before the client connects.
    async fn connect(
        server: Server,
        login: Login<'_>,
        host: &str,
        pinned: Option<&str>,
        rfb: bool,
    ) -> Outcome {
        let (client, io) = tokio::io::duplex(4096);
        let peer = serve(io, server);
        let dir = tempfile::tempdir().unwrap();
        let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        if let Some(pinned) = pinned {
            let bare = host.trim_matches(['[', ']']);
            hosts.record(&format!("vnc/{bare}"), 5900, pinned).unwrap();
        }
        let known = KnownHostsState(Arc::new(Mutex::new(hosts)));
        let tls = TlsContext { host, port: 5900, known: &known };
        // Buffered, so a write the login does not flush never reaches the server.
        let client: Box<dyn Stream> = Box::new(tokio::io::BufWriter::new(client));
        let outcome = tokio::time::timeout(Duration::from_secs(5), async {
            let entered = if rfb {
                handshake(client, &login, &tls).await.map(|(stream, init)| (stream, Some(init)))
            } else {
                negotiate(client, &login, &tls).await.map(|stream| (stream, None))
            };
            let (result, left) = match entered {
                Ok((mut stream, init)) => {
                    stream.shutdown().await.unwrap();
                    let mut left = Vec::new();
                    stream.read_to_end(&mut left).await.unwrap();
                    (Ok(init), left)
                }
                Err(e) => (Err(e), Vec::new()),
            };
            Outcome { result, seen: peer.await.unwrap(), left, pin: None }
        })
        .await
        .expect("VeNCrypt script hung");
        let bare = host.trim_matches(['[', ']']);
        let pin = match known.0.lock().unwrap().verify(&format!("vnc/{bare}"), 5900, "") {
            HostKeyVerdict::Mismatch { stored, .. } => Some(stored),
            _ => None,
        };
        Outcome { pin, ..outcome }
    }

    async fn play(server: Server, login: Login<'_>, pinned: Option<&str>) -> Outcome {
        connect(server, login, "127.0.0.1", pinned, false).await
    }

    fn refusal(outcome: &Outcome) -> String {
        match &outcome.result {
            Ok(_) => panic!("login should have failed"),
            Err(e) => e.to_string(),
        }
    }

    const CHOSE_X509VNC: [u8; 6] = [0, 2, 0, 0, 1, 5];
    const CHOSE_X509PLAIN: [u8; 6] = [0, 2, 0, 0, 1, 6];

    #[test]
    fn subtypes_are_chosen_by_what_the_login_has() {
        let both = Login { username: "user", password: "pw" };
        assert_eq!(choose_subtype(&[260, 261, 262], &both), Ok(262));
        assert_eq!(choose_subtype(&[260, 261], &both), Ok(261));

        let password = Login { username: "", password: "pw" };
        assert_eq!(choose_subtype(&[260, 262, 261], &password), Ok(261));
        assert_eq!(choose_subtype(&[260, 262], &password), Ok(262));

        for username in ["", "user"] {
            let none = Login { username, password: "" };
            assert_eq!(choose_subtype(&[262, 261, 1, 260], &none), Ok(260));
        }
    }

    #[test]
    fn base_types_are_chosen_when_no_x509_subtype_fits() {
        for username in ["", "user"] {
            let password = Login { username, password: "pw" };
            assert_eq!(choose_subtype(&[258, 2, 1, 260], &password), Ok(2));
            assert_eq!(choose_subtype(&[2, 256, 261], &password), Ok(261));

            let none = Login { username, password: "" };
            assert_eq!(choose_subtype(&[257, 2, 1, 261], &none), Ok(1));
        }
    }

    #[test]
    fn a_password_is_never_given_up_or_sent_under_anonymous_tls() {
        let login = Login { username: "user", password: "pw" };
        assert_eq!(
            choose_subtype(&[260, 1], &login),
            Err("the server offers only: VeNCrypt X509None, None".into()),
        );
        assert_eq!(
            choose_subtype(&[256, 257, 258, 259, 263, 264, 7], &login),
            Err("the server offers only: VeNCrypt Plain (unencrypted), \
                 VeNCrypt TLSNone (no certificate), VeNCrypt TLSVnc (no certificate), \
                 VeNCrypt TLSPlain (no certificate), VeNCrypt X509SASL, \
                 VeNCrypt TLSSASL (no certificate), VeNCrypt subtype 7"
                .into()),
        );
        assert_eq!(
            choose_subtype(&[258, 256, 258, 258, 256], &login),
            Err("the server offers only: VeNCrypt TLSVnc (no certificate), \
                 VeNCrypt Plain (unencrypted)"
                .into()),
        );
    }

    #[test]
    fn without_a_password_the_password_subtypes_ask_for_one() {
        let none = Login { username: "", password: "" };
        for needs_password in [2, 261, 262] {
            assert_eq!(
                choose_subtype(&[257, needs_password], &none),
                Err("this server needs a password".into()),
            );
        }
        assert_eq!(
            choose_subtype(&[257, 259], &none),
            Err("the server offers only: VeNCrypt TLSNone (no certificate), \
                 VeNCrypt TLSPlain (no certificate)"
                .into()),
        );
    }

    #[test]
    fn unsupported_certificates_are_named_in_words() {
        let failure = |e: rustls::CertificateError| {
            let e = std::io::Error::new(ErrorKind::InvalidData, rustls::Error::from(e));
            tls_failure(e).to_string()
        };
        let algorithm = rustls::CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext {
            signature_algorithm_id: vec![],
            public_key_algorithm_id: vec![],
        };
        assert_eq!(
            failure(algorithm),
            "internal error: vnc: the server's certificate is not supported (signature algorithm)",
        );
        assert_eq!(
            failure(rustls::CertificateError::BadSignature),
            "internal error: vnc: TLS handshake failed: invalid peer certificate: BadSignature",
        );
    }

    #[tokio::test]
    async fn x509_v1_certificate_is_named_as_unsupported() {
        let v1 = CertificateDer::from_pem_slice(include_bytes!("fixtures/test-cert-v1.pem"));
        let v1 = v1.unwrap();
        let pin = fingerprint(&v1);
        let mut server = x509(&[262], vec![]);
        server.tls.as_mut().unwrap().0 = acceptor(v1, &[&TLS13, &TLS12]);
        let login = Login { username: "user", password: "pw" };
        let outcome = play(server, login, Some(&pin)).await;
        assert_eq!(
            refusal(&outcome),
            "internal error: vnc: the server's certificate is not supported (not X.509 version 3)",
        );
        assert!(outcome.seen.secured.is_none());
    }

    #[tokio::test]
    async fn vnc_password_subtype_answers_the_challenge_without_tls() {
        for pinned in [None, Some(UNENCRYPTED)] {
            let mut plain = offering(&[258, 2]);
            plain.extend([Read(4), Write(vec![7; 16]), Read(16)]);
            let login = Login { username: "user", password: "hunter2" };
            let outcome = play(Server { plain, tls: None }, login, pinned).await;
            assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
            let response = vnc_auth_response("hunter2", &[7; 16]);
            assert_eq!(outcome.seen.plain, [&[0, 2, 0, 0, 0, 2][..], &response].concat());
            assert!(outcome.seen.secured.is_none());
            assert_eq!(outcome.pin.as_deref(), pinned);
        }
    }

    #[tokio::test]
    async fn none_subtype_sends_nothing_after_the_choice() {
        for pinned in [None, Some(UNENCRYPTED)] {
            let mut plain = offering(&[257, 1]);
            plain.push(Read(4));
            let login = Login { username: "", password: "" };
            let outcome = play(Server { plain, tls: None }, login, pinned).await;
            assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
            assert_eq!(outcome.seen.plain, [0, 2, 0, 0, 0, 1]);
            assert_eq!(outcome.pin.as_deref(), pinned);
        }
    }

    // The challenge comes along with the list, so a client that carried on would be seen
    // answering it.
    #[tokio::test]
    async fn base_subtypes_are_refused_for_a_host_with_a_certificate_pin() {
        for (subtype, password) in [(2, "hunter2"), (1, "")] {
            let mut plain = offering(&[258, subtype]);
            let Some(Write(list)) = plain.last_mut() else { unreachable!() };
            list.extend([7; 16]);
            let login = Login { username: "", password };
            let outcome = play(Server { plain, tls: None }, login, Some("SHA256:ab")).await;
            match &outcome.result {
                Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
                    assert_eq!(host, "vnc/127.0.0.1");
                    assert_eq!(*port, 5900);
                    assert_eq!(stored, "SHA256:ab");
                    assert_eq!(offered, UNENCRYPTED);
                }
                other => panic!("expected HostKeyMismatch, got {:?}", other.as_ref().err()),
            }
            assert_eq!(outcome.seen.plain, [0, 2], "{subtype}");
            assert_eq!(outcome.pin.as_deref(), Some("SHA256:ab"));
        }
    }

    #[tokio::test]
    async fn x509vnc_answers_the_challenge_over_tls() {
        let secured = vec![Write(vec![7; 16]), Read(16), Write(b"next".into())];
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = play(x509(&[261], secured), login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.plain, CHOSE_X509VNC);
        assert_eq!(outcome.seen.secured.unwrap(), vnc_auth_response("hunter2", &[7; 16]));
        assert_eq!(outcome.left, b"next");
    }

    #[tokio::test]
    async fn x509vnc_works_with_a_tls12_server() {
        let mut server = x509(&[261], vec![Write(vec![7; 16]), Read(16), Write(b"next".into())]);
        server.tls.as_mut().unwrap().0 = acceptor(certificate(), &[&TLS12]);
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = play(server, login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.secured.unwrap(), vnc_auth_response("hunter2", &[7; 16]));
        assert_eq!(outcome.left, b"next");
    }

    #[tokio::test]
    async fn x509plain_sends_the_username_and_password_over_tls() {
        let login = Login { username: "user", password: "pw" };
        let pin = fingerprint(&certificate());
        let server = x509(&[261, 262], vec![Read(14), Write(b"next".into())]);
        let outcome = play(server, login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.plain, CHOSE_X509PLAIN);
        assert_eq!(outcome.seen.secured.unwrap(), b"\0\0\0\x04\0\0\0\x02userpw");
        assert_eq!(outcome.left, b"next");
    }

    #[tokio::test]
    async fn x509plain_without_a_username_sends_an_empty_one() {
        let login = Login { username: "", password: "pw" };
        let pin = fingerprint(&certificate());
        let outcome = play(x509(&[262], vec![Read(10)]), login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.plain, CHOSE_X509PLAIN);
        assert_eq!(outcome.seen.secured.unwrap(), b"\0\0\0\0\0\0\0\x02pw");
    }

    #[tokio::test]
    async fn x509none_logs_in_without_a_password() {
        let login = Login { username: "", password: "" };
        let pin = fingerprint(&certificate());
        let server = x509(&[261, 260], vec![Write(b"next".into())]);
        let outcome = play(server, login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.plain, [0, 2, 0, 0, 1, 4]);
        assert_eq!(outcome.seen.secured.unwrap(), b"");
        assert_eq!(outcome.left, b"next");
    }

    #[tokio::test]
    async fn unknown_certificate_is_reported_before_any_credential() {
        let login = Login { username: "user", password: "pw" };
        let outcome = play(x509(&[262], vec![]), login, None).await;
        match &outcome.result {
            Err(AppError::HostKeyUnknown { host, port, fingerprint: offered }) => {
                assert_eq!(host, "vnc/127.0.0.1");
                assert_eq!(*port, 5900);
                assert_eq!(*offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyUnknown, got {:?}", other.as_ref().err()),
        }
        assert_eq!(outcome.seen.plain, CHOSE_X509PLAIN);
        assert_eq!(outcome.seen.secured.unwrap(), b"");
    }

    #[tokio::test]
    async fn changed_certificate_is_reported_before_any_credential() {
        let login = Login { username: "user", password: "pw" };
        let outcome = play(x509(&[262], vec![]), login, Some("SHA256:00")).await;
        match &outcome.result {
            Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
                assert_eq!(host, "vnc/127.0.0.1");
                assert_eq!(*port, 5900);
                assert_eq!(stored, "SHA256:00");
                assert_eq!(*offered, fingerprint(&certificate()));
            }
            other => panic!("expected HostKeyMismatch, got {:?}", other.as_ref().err()),
        }
        assert_eq!(outcome.seen.secured.unwrap(), b"");
    }

    // The pinned certificate, presented by a server that cannot sign with its key.
    #[tokio::test]
    async fn pinned_certificate_without_its_key_is_refused() {
        let mut forged = certificate().to_vec();
        let point = forged.windows(4).position(|w| w == [0x03, 0x42, 0x00, 0x04]).unwrap();
        forged[point + 4] ^= 1;
        let forged = CertificateDer::from(forged);
        let pin = fingerprint(&forged);
        let mut server = x509(&[262], vec![]);
        server.tls.as_mut().unwrap().0 = acceptor(forged, &[&TLS13, &TLS12]);
        let login = Login { username: "user", password: "pw" };
        let outcome = play(server, login, Some(&pin)).await;
        assert!(refusal(&outcome).contains("TLS handshake failed"), "{}", refusal(&outcome));
        assert!(outcome.seen.secured.is_none());
    }

    #[tokio::test]
    async fn anonymous_and_unencrypted_subtypes_are_refused() {
        let server = Server { plain: offering(&[258, 256]), tls: None };
        let login = Login { username: "", password: "hunter2" };
        let outcome = play(server, login, None).await;
        assert!(refusal(&outcome).ends_with(
            "vnc: the server offers only: VeNCrypt TLSVnc (no certificate), \
             VeNCrypt Plain (unencrypted)"
        ));
        assert_eq!(outcome.seen.plain, [0, 2]);
    }

    #[tokio::test]
    async fn x509_is_chosen_over_anonymous_tls() {
        let secured = vec![Write(vec![7; 16]), Read(16)];
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = play(x509(&[258, 261], secured), login, Some(&pin)).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
        assert_eq!(outcome.seen.plain, CHOSE_X509VNC);
    }

    #[tokio::test]
    async fn rejected_version_is_an_error() {
        let plain = vec![Write(vec![0, 2]), Read(2), Write(vec![1])];
        let login = Login { username: "", password: "hunter2" };
        let outcome = play(Server { plain, tls: None }, login, None).await;
        assert!(refusal(&outcome).contains("the server does not speak VeNCrypt 0.2"));
        assert_eq!(outcome.seen.plain, [0, 2]);
    }

    #[tokio::test]
    async fn version_before_0_2_is_refused() {
        let login = Login { username: "", password: "hunter2" };
        let outcome = play(Server { plain: vec![Write(vec![0, 1])], tls: None }, login, None).await;
        assert!(refusal(&outcome).contains("unsupported VeNCrypt version 0.1"));
        assert!(outcome.seen.plain.is_empty());
    }

    #[tokio::test]
    async fn refused_subtype_is_an_error() {
        let mut plain = offering(&[261]);
        plain.extend([Read(4), Write(vec![0])]);
        let login = Login { username: "", password: "hunter2" };
        let outcome = play(Server { plain, tls: None }, login, None).await;
        assert!(refusal(&outcome).contains("the server refused VeNCrypt X509Vnc"));
        assert_eq!(outcome.seen.plain, CHOSE_X509VNC);
    }

    #[tokio::test]
    async fn no_subtypes_is_an_error() {
        let login = Login { username: "", password: "hunter2" };
        let outcome = play(Server { plain: offering(&[]), tls: None }, login, None).await;
        assert!(refusal(&outcome).contains("the server offers no VeNCrypt login"));
        assert_eq!(outcome.seen.plain, [0, 2]);
    }

    #[tokio::test]
    async fn server_name_is_sent_for_dns_names_only() {
        let pin = fingerprint(&certificate());
        let hosts = [("desk.example", Some("desk.example")), ("127.0.0.1", None), ("[::1]", None)];
        for (host, sni) in hosts {
            let login = Login { username: "user", password: "pw" };
            let server = x509(&[262], vec![Read(14)]);
            let outcome = connect(server, login, host, Some(&pin), false).await;
            assert!(outcome.result.is_ok(), "{host}: {:?}", outcome.result.as_ref().err());
            assert_eq!(outcome.seen.sni.as_deref(), sni, "{host}");
        }
    }

    #[tokio::test]
    async fn host_that_is_no_name_or_address_is_an_error() {
        let pin = fingerprint(&certificate());
        let login = Login { username: "user", password: "pw" };
        let server = Server { plain: offering(&[262]), tls: None };
        let outcome = connect(server, login, "not a host", Some(&pin), false).await;
        assert!(refusal(&outcome).contains("invalid host name: not a host"));
        assert_eq!(outcome.seen.plain, [0, 2]);
    }

    #[tokio::test]
    async fn host_only_has_to_be_nameable_for_tls() {
        let mut plain = offering(&[2]);
        plain.extend([Read(4), Write(vec![7; 16]), Read(16)]);
        let login = Login { username: "", password: "hunter2" };
        let server = Server { plain, tls: None };
        let outcome = connect(server, login, "not a host", None, false).await;
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.as_ref().err());
    }

    // `server` behind an RFB server that offers `types` and reads the client's choice.
    fn rfb(banner: &str, types: &[u8], mut server: Server) -> Server {
        let mut list = vec![types.len() as u8];
        list.extend(types);
        server.plain.splice(0..0, [Write(banner.into()), Read(12), Write(list), Read(1)]);
        server
    }

    // An RFB 3.8 server offering VNC password and VeNCrypt X509Vnc, then `secured` over TLS.
    fn rfb_server(secured: Vec<Step>) -> Server {
        rfb("RFB 003.008\n", &[2, 19], x509(&[261], secured))
    }

    // SecurityResult OK, ClientInit, and the ServerInit of an 800x600 desktop called "desk".
    fn entering() -> Vec<Step> {
        let mut init = vec![3, 32, 2, 88];
        init.extend([9u8; 16]);
        init.extend(b"\0\0\0\x04desk");
        vec![Write(vec![0; 4]), Read(1), Write(init)]
    }

    fn entered(outcome: Outcome) -> Outcome {
        let init = outcome.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!((init.width, init.height, init.name.as_str()), (800, 600, "desk"));
        outcome
    }

    #[tokio::test]
    async fn handshake_logs_in_through_vencrypt() {
        let mut secured = vec![Write(vec![7; 16]), Read(16)];
        secured.extend(entering());
        secured.push(Write(b"next".into()));
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = connect(rfb_server(secured), login, "127.0.0.1", Some(&pin), true).await;
        let outcome = entered(outcome);
        assert_eq!(outcome.seen.plain, [&b"RFB 003.008\n"[..], &[19], &CHOSE_X509VNC].concat());
        let response = vnc_auth_response("hunter2", &[7; 16]);
        assert_eq!(outcome.seen.secured.unwrap(), [&response[..], &[1]].concat());
        assert_eq!(outcome.left, b"next");
        assert_eq!(outcome.pin, Some(pin));
    }

    #[tokio::test]
    async fn handshake_v37_reads_the_security_result_after_vencrypt() {
        let mut secured = vec![Write(vec![7; 16]), Read(16)];
        secured.extend(entering());
        let server = rfb("RFB 003.007\n", &[19], x509(&[261], secured));
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = entered(connect(server, login, "127.0.0.1", Some(&pin), true).await);
        assert_eq!(outcome.seen.plain, [&b"RFB 003.007\n"[..], &[19], &CHOSE_X509VNC].concat());
    }

    #[tokio::test]
    async fn handshake_logs_in_with_x509none() {
        let server = rfb("RFB 003.008\n", &[19], x509(&[260], entering()));
        let login = Login { username: "", password: "" };
        let pin = fingerprint(&certificate());
        let outcome = entered(connect(server, login, "127.0.0.1", Some(&pin), true).await);
        let chose = [&b"RFB 003.008\n"[..], &[19], &[0, 2, 0, 0, 1, 4]].concat();
        assert_eq!(outcome.seen.plain, chose);
        assert_eq!(outcome.seen.secured.unwrap(), [1]);
    }

    #[tokio::test]
    async fn handshake_x509none_refusal_is_not_a_wrong_password() {
        let refusing = vec![Write(vec![0, 0, 0, 1, 0, 0, 0, 0])];
        let server = rfb("RFB 003.008\n", &[19], x509(&[260], refusing));
        let login = Login { username: "", password: "" };
        let pin = fingerprint(&certificate());
        let outcome = connect(server, login, "127.0.0.1", Some(&pin), true).await;
        assert!(refusal(&outcome).ends_with("vnc: the server refused the connection"));
    }

    #[tokio::test]
    async fn handshake_needs_a_password_for_the_vencrypt_password_subtypes() {
        let server = rfb("RFB 003.008\n", &[19], Server { plain: offering(&[261]), tls: None });
        let login = Login { username: "", password: "" };
        let outcome = connect(server, login, "127.0.0.1", None, true).await;
        assert!(refusal(&outcome).ends_with("vnc: this server needs a password"));
        assert_eq!(outcome.seen.plain, [&b"RFB 003.008\n"[..], &[19], &[0, 2]].concat());
    }

    #[tokio::test]
    async fn handshake_uses_the_vnc_password_inside_vencrypt_without_tls() {
        let mut plain = offering(&[258, 2]);
        plain.extend([Read(4), Write(vec![7; 16]), Read(16)]);
        plain.extend(entering());
        let server = rfb("RFB 003.008\n", &[19, 2], Server { plain, tls: None });
        let login = Login { username: "", password: "hunter2" };
        let outcome = entered(connect(server, login, "127.0.0.1", None, true).await);
        let response = vnc_auth_response("hunter2", &[7; 16]);
        let sent = [&b"RFB 003.008\n"[..], &[19], &[0, 2, 0, 0, 0, 2], &response, &[1]].concat();
        assert_eq!(outcome.seen.plain, sent);
        assert!(outcome.seen.secured.is_none());
        assert_eq!(outcome.pin, None);
    }

    #[tokio::test]
    async fn handshake_logs_in_with_the_none_subtype() {
        let mut plain = offering(&[257, 1]);
        plain.push(Read(4));
        plain.extend(entering());
        let server = rfb("RFB 003.008\n", &[19], Server { plain, tls: None });
        let login = Login { username: "", password: "" };
        let outcome = entered(connect(server, login, "127.0.0.1", None, true).await);
        let sent = [&b"RFB 003.008\n"[..], &[19], &[0, 2, 0, 0, 0, 1], &[1]].concat();
        assert_eq!(outcome.seen.plain, sent);
        assert!(outcome.seen.secured.is_none());
    }

    #[tokio::test]
    async fn handshake_reports_a_failed_vencrypt_login() {
        let failure = b"\0\0\0\x01\0\0\0\x15Authentication failed".to_vec();
        let secured = vec![Write(vec![7; 16]), Read(16), Write(failure)];
        let login = Login { username: "", password: "hunter2" };
        let pin = fingerprint(&certificate());
        let outcome = connect(rfb_server(secured), login, "127.0.0.1", Some(&pin), true).await;
        assert!(refusal(&outcome).contains("Authentication failed"));
        assert_eq!(outcome.seen.secured.unwrap(), vnc_auth_response("hunter2", &[7; 16]));
    }

    #[tokio::test]
    async fn handshake_stops_at_an_unknown_certificate() {
        let login = Login { username: "", password: "hunter2" };
        let outcome = connect(rfb_server(vec![]), login, "127.0.0.1", None, true).await;
        assert!(matches!(
            &outcome.result,
            Err(AppError::HostKeyUnknown { host, .. }) if host == "vnc/127.0.0.1"
        ));
        assert_eq!(outcome.seen.secured.unwrap(), b"");
    }
}
