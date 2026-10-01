use std::io::ErrorKind;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::proto::vnc_auth_response;
use super::{bounded, err, MAX_TEXT};
use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::trust::{self, Scheme};

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

pub struct Login<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

pub struct TlsContext<'a> {
    pub host: &'a str,
    pub port: u16,
    pub known: &'a KnownHostsState,
}

pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

pub enum Version {
    V3_3,
    V3_7,
    V3_8,
}

pub(super) const NEEDS_PASSWORD: &str = "this server needs a password";
const REFUSED: &str = "the server refused the connection";

pub fn security_name(t: u8) -> String {
    match t {
        1 => "None".into(),
        2 => "VNC password".into(),
        5 | 6 => "RealVNC RA2".into(),
        16 => "Tight".into(),
        18 => "TLS".into(),
        19 => "VeNCrypt".into(),
        30 => "Apple Remote Desktop".into(),
        n => format!("type {n}"),
    }
}

fn offers_only(offered: &[u8]) -> String {
    let mut names: Vec<String> = Vec::new();
    for t in offered {
        let name = security_name(*t);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    format!("the server offers only: {}", names.join(", "))
}

// A password is never given up for type 1: a hostile server could advertise it to get us to
// connect unauthenticated.
pub fn choose_security(offered: &[u8], has_password: bool) -> Result<u8, String> {
    let preferred: &[u8] = if has_password { &[19, 30, 2] } else { &[1, 19] };
    if let Some(t) = preferred.iter().find(|t| offered.contains(t)) {
        return Ok(*t);
    }
    if !has_password && offered.iter().any(|t| matches!(t, 2 | 30)) {
        return Err(NEEDS_PASSWORD.into());
    }
    Err(offers_only(offered))
}

// None when the server gave no reason, or closed the connection before it was through.
async fn read_reason(stream: &mut Box<dyn Stream>) -> AppResult<Option<String>> {
    let cut_off = |e: std::io::Error| match e.kind() {
        ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => {
            Ok(None)
        }
        _ => Err(AppError::from(e)),
    };
    let len = match stream.read_u32().await {
        Ok(len) => bounded(len as usize, MAX_TEXT)?,
        Err(e) => return cut_off(e),
    };
    let mut reason = vec![0u8; len];
    if let Err(e) = stream.read_exact(&mut reason).await {
        return cut_off(e);
    }
    let reason = String::from_utf8_lossy(&reason).into_owned();
    Ok((!reason.is_empty()).then_some(reason))
}

// Runs version + security + ClientInit/ServerInit and returns the stream to keep using.
pub async fn handshake(
    mut stream: Box<dyn Stream>,
    login: &Login<'_>,
    tls: &TlsContext<'_>,
) -> AppResult<(Box<dyn Stream>, ServerInit)> {
    let mut banner = [0u8; 12];
    stream.read_exact(&mut banner).await?;
    let number = |digits: &[u8]| {
        let text = std::str::from_utf8(digits).ok()?;
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse::<u16>().ok()
    };
    let (major, minor) = (number(&banner[4..7]), number(&banner[8..11]));
    let framed = banner.starts_with(b"RFB ") && banner[7] == b'.' && banner[11] == b'\n';
    let version = match (framed, major, minor) {
        (true, Some(3), Some(0..=6)) => Version::V3_3,
        (true, Some(3), Some(7)) => Version::V3_7,
        (true, Some(3..), Some(_)) => Version::V3_8,
        _ => return Err(err("not a VNC server")),
    };
    stream
        .write_all(match version {
            Version::V3_3 => b"RFB 003.003\n",
            Version::V3_7 => b"RFB 003.007\n",
            Version::V3_8 => b"RFB 003.008\n",
        })
        .await?;
    // Every write is flushed before the next read: a stream that buffers would otherwise
    // leave the server waiting.
    stream.flush().await?;

    // A 3.3 server picks the type itself; it is held to the same selection as an offer of one.
    let offered = if matches!(version, Version::V3_3) {
        let kind = stream.read_u32().await?;
        if kind == 0 {
            return Err(err(read_reason(&mut stream).await?.unwrap_or(REFUSED.into())));
        }
        vec![u8::try_from(kind).map_err(|_| err(format!("the server offers only: type {kind}")))?]
    } else {
        let count = stream.read_u8().await?;
        if count == 0 {
            return Err(err(read_reason(&mut stream).await?.unwrap_or(REFUSED.into())));
        }
        let mut types = vec![0u8; count as usize];
        stream.read_exact(&mut types).await?;
        types
    };
    let has_password = !login.password.is_empty();
    let usable: Vec<u8> =
        offered.iter().copied().filter(|t| matches!(t, 1 | 2 | 19 | 30)).collect();
    let security = match choose_security(&usable, has_password) {
        Ok(security) => security,
        Err(reason) if reason == NEEDS_PASSWORD => return Err(err(reason)),
        Err(_) => return Err(err(offers_only(&offered))),
    };
    // VeNCrypt asks this itself once it knows whether its login runs inside TLS.
    if security != 19 {
        trust::check_unencrypted(tls.known, Scheme::Vnc, tls.host, tls.port)?;
    }
    if !matches!(version, Version::V3_3) {
        stream.write_all(&[security]).await?;
        stream.flush().await?;
    }

    if security == 2 {
        let mut challenge = [0u8; 16];
        stream.read_exact(&mut challenge).await?;
        stream.write_all(&vnc_auth_response(login.password, &challenge)).await?;
        stream.flush().await?;
    }
    if security == 19 {
        stream = super::vencrypt::negotiate(stream, login, tls).await?;
    }
    if security == 30 {
        super::ard::login(&mut stream, login).await?;
    }

    // Before 3.8 a server sends no SecurityResult for type 1.
    let result = match (security, &version) {
        (1, Version::V3_3 | Version::V3_7) => 0,
        _ => stream.read_u32().await?,
    };
    if result != 0 {
        let reason = match version {
            Version::V3_8 => read_reason(&mut stream).await?,
            _ => None,
        };
        return Err(err(match (reason, result) {
            (Some(reason), _) => reason,
            (None, 2) => "too many attempts".into(),
            (None, _) if has_password => "wrong password".into(),
            (None, _) => REFUSED.into(),
        }));
    }

    stream.write_all(&[1]).await?; // ClientInit: shared
    stream.flush().await?;

    let width = stream.read_u16().await?;
    let height = stream.read_u16().await?;
    let mut pixel_format = [0u8; 16]; // the server's own format; the session sets ours
    stream.read_exact(&mut pixel_format).await?;
    let mut name = vec![0u8; bounded(stream.read_u32().await? as usize, MAX_TEXT)?];
    stream.read_exact(&mut name).await?;
    let name = String::from_utf8_lossy(&name).into_owned();

    Ok((stream, ServerInit { width, height, name }))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::ReadBuf;

    use super::*;
    use crate::remote::trust::UNENCRYPTED;
    use crate::ssh::known_hosts::{HostKeyVerdict, KnownHosts};
    use Step::{Read, Write};

    // The server's side of a script: bytes it writes, and how many it then reads from the client.
    enum Step {
        Write(Vec<u8>),
        Read(usize),
    }

    // `sent` is everything the client wrote, `left` what the server wrote after the handshake,
    // `pin` what is on record for the host afterwards.
    struct Outcome {
        result: AppResult<ServerInit>,
        sent: Vec<u8>,
        left: Vec<u8>,
        pin: Option<String>,
    }

    async fn play(script: Vec<Step>, password: &str) -> Outcome {
        play_pinned(script, password, None).await
    }

    // `pinned` is on record for the host before the client connects.
    async fn play_pinned(script: Vec<Step>, password: &str, pinned: Option<&str>) -> Outcome {
        let (client, mut server) = tokio::io::duplex(4096);
        let peer = tokio::spawn(async move {
            let mut sent = Vec::new();
            for step in script {
                match step {
                    Write(bytes) => server.write_all(&bytes).await.unwrap(),
                    Read(n) => {
                        let mut buf = vec![0u8; n];
                        server.read_exact(&mut buf).await.unwrap();
                        sent.extend(buf);
                    }
                }
            }
            server.shutdown().await.unwrap();
            server.read_to_end(&mut sent).await.unwrap();
            sent
        });
        let dir = tempfile::tempdir().unwrap();
        let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        if let Some(pinned) = pinned {
            hosts.record("vnc/127.0.0.1", 5900, pinned).unwrap();
        }
        let known = KnownHostsState(Arc::new(Mutex::new(hosts)));
        let tls = TlsContext { host: "127.0.0.1", port: 5900, known: &known };
        let login = Login { username: "", password };
        // Buffered, so a write the handshake does not flush never reaches the server.
        let client = tokio::io::BufWriter::new(client);
        let outcome = tokio::time::timeout(Duration::from_secs(2), async {
            let (result, left) = match handshake(Box::new(client), &login, &tls).await {
                Ok((mut stream, init)) => {
                    let mut left = Vec::new();
                    stream.read_to_end(&mut left).await.unwrap();
                    (Ok(init), left)
                }
                Err(e) => (Err(e), Vec::new()),
            };
            Outcome { result, sent: peer.await.unwrap(), left, pin: None }
        })
        .await
        .expect("handshake script hung");
        let pin = match known.0.lock().unwrap().verify("vnc/127.0.0.1", 5900, "") {
            HostKeyVerdict::Mismatch { stored, .. } => Some(stored),
            _ => None,
        };
        Outcome { pin, ..outcome }
    }

    fn refusal(outcome: &Outcome) -> String {
        match &outcome.result {
            Ok(_) => panic!("handshake should have failed"),
            Err(e) => e.to_string(),
        }
    }

    fn server_init(width: u16, height: u16, name: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(width.to_be_bytes());
        b.extend(height.to_be_bytes());
        b.extend([9u8; 16]);
        b.extend(reason(name));
        b
    }

    fn reason(text: &str) -> Vec<u8> {
        let mut b = (text.len() as u32).to_be_bytes().to_vec();
        b.extend(text.as_bytes());
        b
    }

    // A server offering `types` on the given version, up to the client's choice.
    fn offering(banner: &str, types: &[u8]) -> Vec<Step> {
        let mut list = vec![types.len() as u8];
        list.extend(types);
        vec![Write(banner.into()), Read(12), Write(list)]
    }

    // The same, through the VNC password challenge and the client's response.
    fn challenging(banner: &str) -> Vec<Step> {
        let mut script = offering(banner, &[1, 2]);
        script.extend([Read(1), Write(vec![7; 16]), Read(16)]);
        script
    }

    // Everything a client has sent once it has answered that challenge with "hunter2".
    fn answered(reply: &str) -> Vec<u8> {
        [reply.as_bytes(), &[2], &vnc_auth_response("hunter2", &[7; 16])].concat()
    }

    #[tokio::test]
    async fn replies_with_the_highest_version_both_sides_speak() {
        for (banner, reply) in [
            ("RFB 003.003\n", "RFB 003.003\n"),
            ("RFB 003.005\n", "RFB 003.003\n"),
            ("RFB 003.007\n", "RFB 003.007\n"),
            ("RFB 003.008\n", "RFB 003.008\n"),
            ("RFB 003.889\n", "RFB 003.008\n"),
            ("RFB 004.001\n", "RFB 003.008\n"),
        ] {
            let outcome = play(vec![Write(banner.into()), Read(12)], "").await;
            assert_eq!(outcome.sent, reply.as_bytes(), "{banner:?}");
        }
    }

    #[tokio::test]
    async fn refuses_a_peer_that_is_not_a_vnc_server() {
        for banner in ["HTTP/1.1 400", "RFB abc.def\n", "RFB 003.+08\n", "RFB 002.000\n"] {
            let outcome = play(vec![Write(banner.into())], "").await;
            assert!(refusal(&outcome).contains("not a VNC server"), "{banner:?}");
            assert!(outcome.sent.is_empty(), "{banner:?}");
        }
    }

    #[tokio::test]
    async fn banner_needs_its_dot_and_newline() {
        for banner in ["RFB 003x008\n", "RFB 003.008X"] {
            let outcome = play(vec![Write(banner.into())], "").await;
            assert!(refusal(&outcome).contains("not a VNC server"), "{banner:?}");
            assert!(outcome.sent.is_empty(), "{banner:?}");
        }
    }

    #[test]
    fn password_logins_are_chosen_strongest_first() {
        assert_eq!(choose_security(&[1, 2], true), Ok(2));
        assert_eq!(choose_security(&[2, 19, 30], true), Ok(19));
        assert_eq!(choose_security(&[2, 30], true), Ok(30));
    }

    #[test]
    fn a_password_never_falls_back_to_no_login() {
        assert_eq!(choose_security(&[1], true), Err("the server offers only: None".into()));
        let refused = choose_security(&[5, 16], true).unwrap_err();
        assert!(refused.contains("RealVNC RA2") && refused.contains("Tight"), "{refused}");
        assert_eq!(
            choose_security(&[5, 6, 16, 1], true),
            Err("the server offers only: RealVNC RA2, Tight, None".into()),
        );
    }

    #[test]
    fn without_a_password_none_comes_before_vencrypt() {
        assert_eq!(choose_security(&[1, 2], false), Ok(1));
        assert_eq!(choose_security(&[19, 1], false), Ok(1));
        assert_eq!(choose_security(&[2, 30, 19], false), Ok(19));
        for needs_password in [2, 30] {
            assert_eq!(
                choose_security(&[16, needs_password], false),
                Err("this server needs a password".into()),
            );
        }
        assert_eq!(
            choose_security(&[5, 16], false),
            Err("the server offers only: RealVNC RA2, Tight".into()),
        );
    }

    #[test]
    fn security_types_have_names() {
        let names: Vec<String> =
            [1, 2, 5, 6, 16, 18, 19, 30, 13].into_iter().map(security_name).collect();
        assert_eq!(
            names,
            [
                "None",
                "VNC password",
                "RealVNC RA2",
                "RealVNC RA2",
                "Tight",
                "TLS",
                "VeNCrypt",
                "Apple Remote Desktop",
                "type 13",
            ],
        );
    }

    #[tokio::test]
    async fn v38_password_login_reads_server_init() {
        let mut script = challenging("RFB 003.008\n");
        script.extend([Write(vec![0; 4]), Read(1), Write(server_init(800, 600, "desk"))]);
        let outcome = play(script, "hunter2").await;
        let response = vnc_auth_response("hunter2", &[7; 16]);
        assert_eq!(outcome.sent, [&b"RFB 003.008\n"[..], &[2], &response, &[1]].concat());
        let init = outcome.result.unwrap();
        assert_eq!((init.width, init.height, init.name.as_str()), (800, 600, "desk"));
    }

    #[tokio::test]
    async fn v38_failure_reports_the_servers_reason() {
        let mut script = challenging("RFB 003.008\n");
        script.push(Write([&[0, 0, 0, 1][..], &reason("Too many attempts")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("Too many attempts"));
        assert_eq!(outcome.sent, answered("RFB 003.008\n"));
    }

    #[tokio::test]
    async fn v38_failure_without_a_reason_is_a_wrong_password() {
        let mut script = challenging("RFB 003.008\n");
        script.push(Write([&[0, 0, 0, 1][..], &reason("")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("wrong password"));
        assert_eq!(outcome.sent, answered("RFB 003.008\n"));
    }

    #[tokio::test]
    async fn v38_failure_cut_off_in_the_reason_is_a_wrong_password() {
        for cut in [&[0, 0, 0, 1][..], &[0, 0, 0, 1, 0, 0], &[0, 0, 0, 1, 0, 0, 0, 9, b'T']] {
            let mut script = challenging("RFB 003.008\n");
            script.push(Write(cut.to_vec()));
            let outcome = play(script, "hunter2").await;
            assert!(refusal(&outcome).contains("wrong password"), "{cut:?}");
            assert_eq!(outcome.sent, answered("RFB 003.008\n"), "{cut:?}");
        }
    }

    #[tokio::test]
    async fn v38_result_other_than_one_still_carries_a_reason() {
        let mut script = challenging("RFB 003.008\n");
        script.push(Write([&[0, 0, 0, 5][..], &reason("Locked out")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("Locked out"));
        assert_eq!(outcome.sent, answered("RFB 003.008\n"));

        let mut script = challenging("RFB 003.008\n");
        script.push(Write([&[0, 0, 0, 5][..], &reason("")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("wrong password"));
        assert_eq!(outcome.sent, answered("RFB 003.008\n"));
    }

    #[tokio::test]
    async fn v38_result_two_without_a_reason_is_too_many_attempts() {
        let mut script = challenging("RFB 003.008\n");
        script.push(Write([&[0, 0, 0, 2][..], &reason("")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("too many attempts"));
        assert_eq!(outcome.sent, answered("RFB 003.008\n"));
    }

    #[tokio::test]
    async fn v37_failure_has_no_reason_to_read() {
        let mut script = challenging("RFB 003.007\n");
        script.push(Write(vec![0, 0, 0, 1]));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("wrong password"));
        assert_eq!(outcome.sent, answered("RFB 003.007\n"));
    }

    #[tokio::test]
    async fn v37_result_two_is_too_many_attempts() {
        let mut script = challenging("RFB 003.007\n");
        script.push(Write(vec![0, 0, 0, 2]));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("too many attempts"));
        assert_eq!(outcome.sent, answered("RFB 003.007\n"));
    }

    #[tokio::test]
    async fn v38_none_still_reads_the_security_result() {
        let mut script = offering("RFB 003.008\n", &[1]);
        script.extend([Read(1), Write(vec![0; 4]), Read(1), Write(server_init(640, 480, ""))]);
        let outcome = play(script, "").await;
        assert_eq!(outcome.sent, [&b"RFB 003.008\n"[..], &[1], &[1]].concat());
        let init = outcome.result.unwrap();
        assert_eq!((init.width, init.height, init.name.as_str()), (640, 480, ""));
    }

    #[tokio::test]
    async fn v38_none_refused_without_a_reason() {
        let mut script = offering("RFB 003.008\n", &[1]);
        script.extend([Read(1), Write([&[0, 0, 0, 1][..], &reason("")].concat())]);
        let outcome = play(script, "").await;
        assert!(refusal(&outcome).contains("the server refused the connection"));
    }

    #[tokio::test]
    async fn v38_none_refusal_cut_off_in_the_reason() {
        let mut script = offering("RFB 003.008\n", &[1]);
        script.extend([Read(1), Write(vec![0, 0, 0, 1])]);
        let outcome = play(script, "").await;
        assert!(refusal(&outcome).contains("the server refused the connection"));
        assert_eq!(outcome.sent, [&b"RFB 003.008\n"[..], &[1]].concat());
    }

    // Each server sends what would follow the choice along with its offer, so a client that
    // carried on would be seen answering it.
    fn unencrypted_servers() -> Vec<(Vec<Step>, &'static str)> {
        let offer = |banner: &str, types: &[u8], next: &[u8]| {
            let mut script = offering(banner, types);
            let Some(Write(list)) = script.last_mut() else { unreachable!() };
            list.extend(next);
            script
        };
        let v33 = vec![
            Write(b"RFB 003.003\n".into()),
            Read(12),
            Write([&[0, 0, 0, 2][..], &[7; 16]].concat()),
        ];
        vec![
            (offer("RFB 003.008\n", &[2], &[7; 16]), "hunter2"),
            (offer("RFB 003.008\n", &[1], &[0; 4]), ""),
            (offer("RFB 003.007\n", &[1], &server_init(800, 600, "desk")), ""),
            (offer("RFB 003.889\n", &[30], &APPLE_KEY), "hunter2"),
            (v33, "hunter2"),
        ]
    }

    #[tokio::test]
    async fn unencrypted_login_to_a_host_with_a_certificate_pin_is_refused() {
        for (n, (script, password)) in unencrypted_servers().into_iter().enumerate() {
            let outcome = play_pinned(script, password, Some("SHA256:ab")).await;
            match &outcome.result {
                Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
                    assert_eq!(host, "vnc/127.0.0.1", "{n}");
                    assert_eq!(*port, 5900, "{n}");
                    assert_eq!(stored, "SHA256:ab", "{n}");
                    assert_eq!(offered, UNENCRYPTED, "{n}");
                }
                other => panic!("{n}: expected HostKeyMismatch, got {:?}", other.as_ref().err()),
            }
            assert_eq!(outcome.sent.len(), 12, "{n}");
            assert_eq!(outcome.pin.as_deref(), Some("SHA256:ab"), "{n}");
        }
    }

    #[tokio::test]
    async fn unencrypted_login_goes_ahead_once_accepted_or_without_a_pin() {
        let password = || {
            let mut script = challenging("RFB 003.008\n");
            script.extend([Write(vec![0; 4]), Read(1), Write(server_init(800, 600, "desk"))]);
            script
        };
        let none = || {
            let mut script = offering("RFB 003.008\n", &[1]);
            script.extend([Read(1), Write(vec![0; 4]), Read(1)]);
            script.push(Write(server_init(800, 600, "desk")));
            script
        };
        for pinned in [Some(UNENCRYPTED), None] {
            let outcome = play_pinned(password(), "hunter2", pinned).await;
            assert_eq!(outcome.result.unwrap().name, "desk", "{pinned:?}");
            assert_eq!(outcome.pin.as_deref(), pinned);

            let outcome = play_pinned(none(), "", pinned).await;
            assert_eq!(outcome.result.unwrap().name, "desk", "{pinned:?}");
            assert_eq!(outcome.pin.as_deref(), pinned);
        }
    }

    // Serves `data`, then fails every read with `then`.
    struct Failing {
        data: std::io::Cursor<Vec<u8>>,
        then: ErrorKind,
    }

    impl AsyncRead for Failing {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.data.position() == self.data.get_ref().len() as u64 {
                return Poll::Ready(Err(self.then.into()));
            }
            Pin::new(&mut self.data).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Failing {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn connection_lost_in_the_reason_counts_as_no_reason() {
        let server = [&b"RFB 003.008\n"[..], &[1, 2], &[7; 16], &[0, 0, 0, 1, 0, 0]].concat();
        let dir = tempfile::tempdir().unwrap();
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        let known = KnownHostsState(Arc::new(Mutex::new(hosts)));
        let tls = TlsContext { host: "127.0.0.1", port: 5900, known: &known };
        let login = Login { username: "", password: "hunter2" };
        let lost = [
            (ErrorKind::ConnectionReset, "internal error: vnc: wrong password"),
            (ErrorKind::ConnectionAborted, "internal error: vnc: wrong password"),
            (ErrorKind::TimedOut, "io error: timed out"),
        ];
        for (then, message) in lost {
            let stream = Failing { data: std::io::Cursor::new(server.clone()), then };
            let Err(e) = handshake(Box::new(stream), &login, &tls).await else {
                panic!("{then:?}: handshake should have failed");
            };
            assert_eq!(e.to_string(), message, "{then:?}");
        }
    }

    #[tokio::test]
    async fn v37_none_has_no_security_result() {
        let mut script = offering("RFB 003.007\n", &[1]);
        script.extend([Read(1), Read(1), Write(server_init(800, 600, "desk"))]);
        let outcome = play(script, "").await;
        assert_eq!(outcome.sent, [&b"RFB 003.007\n"[..], &[1], &[1]].concat());
        assert_eq!(outcome.result.unwrap().name, "desk");
    }

    #[tokio::test]
    async fn v33_server_picks_vnc_password() {
        let script = vec![
            Write(b"RFB 003.003\n".into()),
            Read(12),
            Write(vec![0, 0, 0, 2]),
            Write(vec![7; 16]),
            Read(16),
            Write(vec![0; 4]),
            Read(1),
            Write(server_init(800, 600, "desk")),
        ];
        let outcome = play(script, "hunter2").await;
        let response = vnc_auth_response("hunter2", &[7; 16]);
        assert_eq!(outcome.sent, [&b"RFB 003.003\n"[..], &response, &[1]].concat());
        assert_eq!(outcome.result.unwrap().name, "desk");
    }

    #[tokio::test]
    async fn v33_type_zero_fails_with_the_reason() {
        let script = vec![
            Write(b"RFB 003.003\n".into()),
            Read(12),
            Write([&[0, 0, 0, 0][..], &reason("Too many security failures")].concat()),
        ];
        let outcome = play(script, "").await;
        assert!(refusal(&outcome).contains("Too many security failures"));
    }

    #[tokio::test]
    async fn v33_type_zero_without_a_reason_is_a_refusal() {
        let script = vec![
            Write(b"RFB 003.003\n".into()),
            Read(12),
            Write([&[0, 0, 0, 0][..], &reason("")].concat()),
        ];
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).ends_with("vnc: the server refused the connection"));
    }

    #[tokio::test]
    async fn v33_failures_are_named_by_the_result() {
        let named = [(1, "wrong password"), (2, "too many attempts"), (9, "wrong password")];
        for (result, message) in named {
            let script = vec![
                Write(b"RFB 003.003\n".into()),
                Read(12),
                Write(vec![0, 0, 0, 2]),
                Write(vec![7; 16]),
                Read(16),
                Write(vec![0, 0, 0, result]),
            ];
            let outcome = play(script, "hunter2").await;
            assert!(refusal(&outcome).contains(message), "{result}");
            let response = vnc_auth_response("hunter2", &[7; 16]);
            assert_eq!(outcome.sent, [&b"RFB 003.003\n"[..], &response].concat(), "{result}");
        }
    }

    #[tokio::test]
    async fn v33_none_goes_straight_to_client_init() {
        let script = vec![
            Write(b"RFB 003.003\n".into()),
            Read(12),
            Write(vec![0, 0, 0, 1]),
            Read(1),
            Write(server_init(800, 600, "desk")),
        ];
        let outcome = play(script, "").await;
        assert_eq!(outcome.sent, [&b"RFB 003.003\n"[..], &[1]].concat());
        assert_eq!(outcome.result.unwrap().name, "desk");
    }

    #[tokio::test]
    async fn v33_server_choice_goes_through_the_same_selection() {
        let none = vec![Write(b"RFB 003.003\n".into()), Read(12), Write(vec![0, 0, 0, 1])];
        let outcome = play(none, "hunter2").await;
        assert!(refusal(&outcome).contains("the server offers only: None"));
        assert_eq!(outcome.sent, b"RFB 003.003\n");

        let password = vec![Write(b"RFB 003.003\n".into()), Read(12), Write(vec![0, 0, 0, 2])];
        let outcome = play(password, "").await;
        assert!(refusal(&outcome).contains("this server needs a password"));

        let unknown = vec![Write(b"RFB 003.003\n".into()), Read(12), Write(vec![0, 0, 1, 0])];
        let outcome = play(unknown, "").await;
        assert!(refusal(&outcome).contains("the server offers only: type 256"));
    }

    #[tokio::test]
    async fn no_security_types_fails_with_the_reason() {
        let mut script = offering("RFB 003.008\n", &[]);
        script.push(Write(reason("Too many security failures")));
        let outcome = play(script, "").await;
        assert!(refusal(&outcome).contains("Too many security failures"));
        assert_eq!(outcome.sent, b"RFB 003.008\n");
    }

    #[tokio::test]
    async fn no_security_types_without_a_reason_is_a_refusal() {
        for banner in ["RFB 003.007\n", "RFB 003.008\n"] {
            let mut script = offering(banner, &[]);
            script.push(Write(reason("")));
            let outcome = play(script, "hunter2").await;
            let refused = refusal(&outcome);
            assert!(refused.ends_with("vnc: the server refused the connection"), "{banner:?}");
            assert_eq!(outcome.sent, banner.as_bytes());
        }
    }

    #[tokio::test]
    async fn oversized_lengths_are_refused() {
        let mut no_types = offering("RFB 003.008\n", &[]);
        no_types.push(Write(vec![0xff; 4]));
        let mut failure = challenging("RFB 003.008\n");
        failure.push(Write(vec![0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff]));
        let mut name = challenging("RFB 003.008\n");
        name.extend([Write(vec![0; 4]), Read(1), Write([&[9u8; 20][..], &[0xff; 4]].concat())]);
        for script in [no_types, failure, name] {
            let outcome = play(script, "hunter2").await;
            assert!(refusal(&outcome).contains("declared length 4294967295 exceeds 1048576"));
        }
    }

    #[tokio::test]
    async fn only_types_with_a_login_are_chosen() {
        let mut script = offering("RFB 003.008\n", &[18, 16, 2]);
        script.extend([Read(1), Write(vec![7; 16]), Read(16), Write(vec![0, 0, 0, 1, 0, 0, 0, 0])]);
        let outcome = play(script, "hunter2").await;
        assert_eq!(outcome.sent[12], 2);

        let outcome = play(offering("RFB 003.008\n", &[16, 18]), "hunter2").await;
        assert!(refusal(&outcome).contains("the server offers only: Tight, TLS"));
        assert_eq!(outcome.sent, b"RFB 003.008\n");
    }

    // Generator 5, an 8-byte key holding the prime 4294967291, and a server public key.
    const APPLE_KEY: [u8; 20] =
        [0, 5, 0, 8, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xfb, 0, 0, 0, 0, 0x26, 0x20, 0x59, 0x5a];

    #[tokio::test]
    async fn apple_login_is_preferred_to_the_vnc_password() {
        let mut script = offering("RFB 003.889\n", &[2, 30]);
        script.extend([Read(1), Write(APPLE_KEY.into()), Read(128 + 8), Write(vec![0; 4])]);
        script.extend([Read(1), Write(server_init(800, 600, "mac"))]);
        let outcome = play(script, "hunter2").await;
        assert_eq!(outcome.sent.len(), 12 + 1 + 128 + 8 + 1);
        assert_eq!(&outcome.sent[..13], b"RFB 003.008\n\x1e");
        assert_eq!(outcome.sent[149], 1);
        assert_eq!(outcome.result.unwrap().name, "mac");
    }

    #[tokio::test]
    async fn apple_login_refused_is_a_wrong_password() {
        let mut script = offering("RFB 003.889\n", &[30]);
        script.extend([Read(1), Write(APPLE_KEY.into()), Read(128 + 8)]);
        script.push(Write([&[0, 0, 0, 1][..], &reason("")].concat()));
        let outcome = play(script, "hunter2").await;
        assert!(refusal(&outcome).contains("wrong password"));
        assert_eq!(outcome.sent.len(), 12 + 1 + 128 + 8);
    }

    #[tokio::test]
    async fn apple_login_needs_a_password() {
        let outcome = play(offering("RFB 003.889\n", &[30]), "").await;
        assert!(refusal(&outcome).contains("this server needs a password"));
        assert_eq!(outcome.sent, b"RFB 003.008\n");
    }

    #[tokio::test]
    async fn refusals_name_everything_the_server_offered() {
        let outcome = play(offering("RFB 003.008\n", &[1, 16]), "hunter2").await;
        assert!(refusal(&outcome).contains("the server offers only: None, Tight"));
        assert_eq!(outcome.sent, b"RFB 003.008\n");

        let outcome = play(offering("RFB 003.007\n", &[5, 6, 16]), "").await;
        assert!(refusal(&outcome).contains("the server offers only: RealVNC RA2, Tight"));

        let outcome = play(offering("RFB 003.008\n", &[16, 2]), "").await;
        assert!(refusal(&outcome).contains("this server needs a password"));
        assert_eq!(outcome.sent, b"RFB 003.008\n");
    }

    #[tokio::test]
    async fn returned_stream_continues_after_server_init() {
        let mut script = challenging("RFB 003.008\n");
        script.extend([Write(vec![0; 4]), Read(1), Write(server_init(800, 600, "desk"))]);
        script.push(Write(b"next".into()));
        let outcome = play(script, "hunter2").await;
        assert_eq!(outcome.left, b"next");
    }
}
