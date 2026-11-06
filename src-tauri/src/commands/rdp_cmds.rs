use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::sync::{oneshot, Mutex as TokioMutex};
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::rdp::connect::{size_supported, Target, LOGIN_LIMIT, MAX_PIXELS};
use crate::rdp::gateway::Gateway;
use crate::rdp::input::Input;
use crate::rdp::session::{Session, CLOSE_LIMIT, CONNECT_LIMIT};
use crate::rdp::share::Share;
use crate::remote::{FrameBytes, FrameOp};
use crate::store::model::{RdpGateway, RdpProfile, SharedFolder};
use crate::store::Store;

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpOpened {
    pub id: String,
    pub width: u16,
    pub height: u16,
}

// Where to connect, as one argument: a command takes ten arguments at most.
#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: Option<String>,
    pub secret_id: Option<String>,
    pub profile_id: Option<String>,
    pub keyboard_layout: Option<u32>,
}

#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RdpViewport {
    pub width: u16,
    pub height: u16,
    pub scale: u32,
}

#[derive(Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RdpInput {
    Key { down: bool, code: u8, extended: bool },
    Unicode { down: bool, codepoint: u32 },
    Pointer { buttons: u16, x: u16, y: u16 },
    Locks { caps: bool, num: bool, scroll: bool },
    Clipboard { text: String },
}

enum Entry {
    // Dropping the sender ends the login. `close_all` sends a sender of its own through it instead,
    // which the open holds until it is over.
    Pending { cancel: oneshot::Sender<oneshot::Sender<()>> },
    Open(Arc<Session>),
    // Closed before its open was done. Commands run as tasks of their own, so the close of a tab
    // can even come before its open.
    Closed { at: Instant },
}

// A close marker waits this long for its open, the longest a login can take. Through a gateway
// its open and the login each get LOGIN_LIMIT.
const CLOSED_LIMIT: Duration = {
    let direct = CONNECT_LIMIT.saturating_add(LOGIN_LIMIT);
    let through_gateway = LOGIN_LIMIT.saturating_mul(2);
    if through_gateway.as_nanos() > direct.as_nanos() {
        through_gateway
    } else {
        direct
    }
};

// Ended within moments of its login, a reconnect on Windows leaves the user's disconnected session
// blocked ("blocked by Remote Desktop Configuration") for a minute or more, whether the connection
// is dropped or shut down. A session is kept this long after its login at the least.
const SETTLE: Duration = Duration::from_secs(1);

// At the exit every session still gets its SETTLE and then its CLOSE_LIMIT, all at once; a login
// that is not done by then is dropped.
const EXIT_LIMIT: Duration = SETTLE.saturating_add(CLOSE_LIMIT);

// The view chooses the id, so that it can close a session that is still logging in: on Windows a
// login that is let finish takes the user's session from any other client, so it is dropped as long
// as its credentials are not out.
#[derive(Default)]
pub struct RdpSessions(TokioMutex<HashMap<String, Entry>>);

impl RdpSessions {
    pub async fn open(
        &self,
        id: &str,
        sent: &AtomicBool,
        connecting: impl Future<Output = AppResult<Session>>,
    ) -> AppResult<RdpOpened> {
        let (cancel, mut cancelled) = oneshot::channel();
        {
            let mut entries = self.0.lock().await;
            match entries.get(id) {
                Some(Entry::Closed { .. }) => {
                    entries.remove(id);
                    return Err(AppError::Cancelled);
                }
                Some(_) => return Err(AppError::Internal("rdp: this id is in use".into())),
                None => entries.insert(id.to_owned(), Entry::Pending { cancel }),
            };
        }
        tokio::pin!(connecting);
        // `cancelled` lives to the end, so that a sender `close_all` sends after the login is done
        // is still held until the session has ended.
        let (connected, _done) = tokio::select! {
            connected = &mut connecting => (connected, None),
            done = &mut cancelled => {
                let connected = if sent.load(Ordering::SeqCst) {
                    connecting.await
                } else {
                    Err(AppError::Cancelled)
                };
                (connected, done.ok())
            }
        };
        let mut entries = self.0.lock().await;
        if let Some(Entry::Closed { .. }) = entries.remove(id) {
            drop(entries);
            if let Ok(session) = connected {
                end(&session).await;
            }
            return Err(AppError::Cancelled);
        }
        let session = connected?;
        let opened = RdpOpened { id: id.to_owned(), width: session.width, height: session.height };
        entries.insert(id.to_owned(), Entry::Open(Arc::new(session)));
        Ok(opened)
    }

    // The lock is released before the session is used: its input waits while the queue to a
    // stalled server is full, and that must not hold up `rdp_close` or the other sessions.
    async fn session(&self, id: &str) -> Option<Arc<Session>> {
        match self.0.lock().await.get(id) {
            Some(Entry::Open(session)) => Some(Arc::clone(session)),
            _ => None,
        }
    }

    pub async fn close(&self, id: &str) {
        let mut entries = self.0.lock().await;
        let now = Instant::now();
        // A close can come for an open that has failed already, or one that never reached here.
        entries
            .retain(|_, entry| !matches!(entry, Entry::Closed { at } if now - *at > CLOSED_LIMIT));
        let Some(Entry::Open(session)) = entries.insert(id.to_owned(), Entry::Closed { at: now })
        else {
            return;
        };
        entries.remove(id);
        drop(entries);
        end(&session).await;
    }

    // For the exit: ends every session as `close` does, all at once, within EXIT_LIMIT.
    pub async fn close_all(&self) {
        let mut opened = Vec::new();
        let mut logins = Vec::new();
        let mut entries = self.0.lock().await;
        let now = Instant::now();
        for (id, entry) in std::mem::take(&mut *entries) {
            match entry {
                Entry::Open(session) => opened.push(session),
                Entry::Pending { cancel } => {
                    let (done, over) = oneshot::channel();
                    if cancel.send(done).is_ok() {
                        logins.push(over);
                    }
                    entries.insert(id, Entry::Closed { at: now });
                }
                closed => {
                    entries.insert(id, closed);
                }
            }
        }
        drop(entries);
        let ending = join_all(opened.iter().map(|session| end(session)));
        tokio::select! {
            _ = async { tokio::join!(ending, join_all(logins)) } => {}
            _ = tokio::time::sleep(EXIT_LIMIT) => {}
        }
    }
}

async fn end(session: &Session) {
    tokio::time::sleep_until(session.opened + SETTLE).await;
    session.close().await;
}

const DEFAULT_LAYOUT: u32 = 0x409;

fn fit(width: u16, height: u16) -> (u16, u16) {
    if width == 0 || height == 0 {
        return (1280, 800);
    }
    let (width, height) = (width.clamp(200, 8192) & !1, height.clamp(200, 8192));
    if size_supported(width, height) {
        return (width, height);
    }
    // Both sides are over 4882 here, so the shape survives and they stay inside the range.
    let scale = (f64::from(MAX_PIXELS) / (f64::from(width) * f64::from(height))).sqrt();
    let height = (f64::from(height) * scale) as u16;
    let width = ((f64::from(width) * scale) as u16).min((MAX_PIXELS / u32::from(height)) as u16);
    (width & !1, height)
}

fn parse_layout(name: &str) -> u32 {
    u32::from_str_radix(name, 16).unwrap_or(DEFAULT_LAYOUT)
}

#[cfg(windows)]
fn local_layout() -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutNameW;

    // KL_NAMELENGTH: eight digits and the terminator.
    let mut name = [0u16; 9];
    if unsafe { GetKeyboardLayoutNameW(name.as_mut_ptr()) } == 0 {
        return DEFAULT_LAYOUT;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    parse_layout(&String::from_utf16_lossy(&name[..len]))
}

#[cfg(not(windows))]
fn local_layout() -> u32 {
    DEFAULT_LAYOUT
}

#[derive(Debug, PartialEq)]
struct Dialed {
    host: String,
    port: u16,
    username: String,
    domain: Option<String>,
    secret_id: Option<String>,
    keyboard_layout: Option<u32>,
    audio: bool,
    shared_folder: Option<SharedFolder>,
    gateway: Option<RdpGateway>,
}

// A saved profile is read at every connect and never copied into the tab: saving a new password
// gives the profile another vault id, and a copy in an open or restored tab would be a dead one.
// The store is read only for a profile, so a quick connect does not wait on its lock.
fn resolve_target(
    saved: impl FnOnce() -> AppResult<Vec<RdpProfile>>,
    given: RdpTarget,
) -> AppResult<Dialed> {
    let Some(id) = given.profile_id else {
        return Ok(Dialed {
            host: given.host,
            port: given.port,
            username: given.username,
            domain: given.domain,
            secret_id: given.secret_id,
            keyboard_layout: given.keyboard_layout,
            audio: true,
            shared_folder: None,
            gateway: None,
        });
    };
    let p = saved()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::NotFound("rdp: this profile no longer exists".into()))?;
    Ok(Dialed {
        host: p.host,
        port: p.port,
        username: p.username,
        domain: p.domain,
        secret_id: p.secret_id,
        keyboard_layout: p.keyboard_layout,
        audio: p.audio,
        shared_folder: p.shared_folder,
        gateway: p.gateway,
    })
}

// The user name and the vault id of the password the gateway gets. Basic has no domain field, so
// the Domain field goes in front of a user name that has no domain of its own.
fn gateway_login(dialed: &Dialed) -> Option<(String, Option<&str>)> {
    let gateway = dialed.gateway.as_ref()?;
    if !gateway.same_login {
        return Some((gateway.username.clone(), gateway.secret_id.as_deref()));
    }
    let username = dialed.username.trim();
    let login = match dialed.domain.as_deref().filter(|domain| !domain.is_empty()) {
        Some(domain) if !username.is_empty() && !username.contains(['\\', '@']) => {
            format!("{domain}\\{username}")
        }
        _ => username.to_owned(),
    };
    Some((login, dialed.secret_id.as_deref()))
}

#[tauri::command]
#[specta::specta]
pub async fn rdp_open(
    app: AppHandle,
    id: String,
    target: RdpTarget,
    viewport: RdpViewport,
    on_frame: Channel<FrameBytes>,
) -> AppResult<RdpOpened> {
    let vault = app.state::<std::sync::Mutex<crate::vault::Vault>>();
    let known = app.state::<KnownHostsState>();
    let rdps = app.state::<RdpSessions>();
    let saved = || -> AppResult<Vec<RdpProfile>> {
        let store = app.state::<std::sync::Mutex<Store>>();
        let store = store.lock().map_err(|_| AppError::Internal("store lock poisoned".into()))?;
        Ok(store.rdp_profiles())
    };
    let dialed = resolve_target(saved, target)?;
    // The password lives in the vault; the caller only ever holds a reference to it.
    let password = match &dialed.secret_id {
        Some(id) => crate::commands::ssh_cmds::secret_string(&vault, id)?,
        None => Zeroizing::new(String::new()),
    };
    let login = gateway_login(&dialed);
    let gateway_password = match login.as_ref().and_then(|(_, secret_id)| *secret_id) {
        Some(id) => crate::commands::ssh_cmds::secret_string(&vault, id)?,
        None => Zeroizing::new(String::new()),
    };
    let RdpViewport { width, height, scale } = viewport;
    let (width, height) = fit(width, height);
    let target = Target {
        host: dialed.host.trim(),
        port: dialed.port,
        username: dialed.username.trim(),
        domain: dialed.domain.as_deref(),
        password: &password,
        width,
        height,
        scale,
        keyboard_layout: dialed.keyboard_layout.unwrap_or_else(local_layout),
        audio: dialed.audio,
        share: dialed
            .shared_folder
            .as_ref()
            .map(|folder| Share { root: Path::new(&folder.path), name: &folder.name }),
        gateway: dialed.gateway.as_ref().zip(login.as_ref()).map(|(gateway, (username, _))| {
            Gateway {
                host: gateway.host.trim(),
                port: gateway.port,
                username,
                password: &gateway_password,
            }
        }),
    };
    // A send fails only once the webview itself is gone, and then nobody is left to tell.
    let sink = move |op: FrameOp| {
        let _ = on_frame.send(op.encode());
    };
    let sent = AtomicBool::new(false);
    rdps.open(&id, &sent, Session::connect(&target, &known, sink, &sent)).await
}

async fn deliver(session: &Session, events: Vec<RdpInput>) -> AppResult<()> {
    let mut run = Vec::new();
    for event in events {
        run.push(match event {
            RdpInput::Key { down, code, extended } => Input::Key { down, code, extended },
            RdpInput::Unicode { down, codepoint } => Input::Unicode { down, codepoint },
            RdpInput::Pointer { buttons, x, y } => Input::Pointer { buttons, x, y },
            RdpInput::Locks { caps, num, scroll } => Input::Locks { caps, num, scroll },
            RdpInput::Clipboard { text } => {
                session.input(std::mem::take(&mut run)).await?;
                session.clipboard(text).await?;
                continue;
            }
        });
    }
    session.input(run).await
}

// The view still sends key releases and a close after the server has ended a session, so the
// rest of a batch for a session that is over is dropped without an error.
async fn send_events(session: &Session, events: Vec<RdpInput>) -> AppResult<()> {
    match deliver(session, events).await {
        Err(AppError::NotFound(_)) => Ok(()),
        sent => sent,
    }
}

// One command for all input: commands run as tasks of their own, so only what is sent in one
// call is certain to arrive in the order it was made.
#[tauri::command]
#[specta::specta]
pub async fn rdp_input(
    rdps: State<'_, RdpSessions>,
    id: String,
    events: Vec<RdpInput>,
) -> AppResult<()> {
    let Some(session) = rdps.session(&id).await else { return Ok(()) };
    send_events(&session, events).await
}

#[tauri::command]
#[specta::specta]
pub async fn rdp_resize(
    rdps: State<'_, RdpSessions>,
    id: String,
    width: u16,
    height: u16,
    scale: u32,
) -> AppResult<()> {
    let Some(session) = rdps.session(&id).await else { return Ok(()) };
    match session.resize(width, height, scale).await {
        Err(AppError::NotFound(_)) => Ok(()),
        resized => resized,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn rdp_ack(rdps: State<'_, RdpSessions>, id: String) -> AppResult<()> {
    if let Some(session) = rdps.session(&id).await {
        session.ack();
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn rdp_close(rdps: State<'_, RdpSessions>, id: String) -> AppResult<()> {
    rdps.close(&id).await;
    Ok(())
}

// Before an update is installed: on Windows the updater ends the app with process::exit, which
// skips the close at the exit.
#[tauri::command]
#[specta::specta]
pub async fn rdp_close_all(rdps: State<'_, RdpSessions>) -> AppResult<()> {
    rdps.close_all().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::time::Instant;

    use ironrdp_pdu::nego::SecurityProtocol;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, oneshot};
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::*;
    use crate::rdp::testserver::{
        accept_nla_holding, certificate, confirm, off_thread, read_client, tls, ClientPdu, Login,
        Mouse, Options, Seen, TestServer,
    };
    use crate::remote::trust::fingerprint;
    use crate::ssh::known_hosts::KnownHosts;

    const WAIT: Duration = Duration::from_secs(10);

    fn known(port: u16) -> KnownHostsState {
        let dir = tempfile::tempdir().unwrap();
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        let known = KnownHostsState(Arc::new(std::sync::Mutex::new(hosts)));
        let print = fingerprint(&certificate());
        known.0.lock().unwrap().record("rdp/127.0.0.1", port, &print).unwrap();
        known
    }

    fn target(port: u16) -> Target<'static> {
        Target {
            host: "127.0.0.1",
            port,
            username: "alice",
            domain: None,
            password: "secret",
            width: 1024,
            height: 768,
            scale: 100,
            keyboard_layout: 0x409,
            audio: false,
            share: None,
            gateway: None,
        }
    }

    async fn open() -> (TestServer, Session, mpsc::UnboundedReceiver<FrameOp>) {
        let options = Options { login: Login::Tls, size: (256, 128), remotefx: false };
        let server = TestServer::listen(options).await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let (sink, ops) = mpsc::unbounded_channel();
        let sent = AtomicBool::default();
        let sink = move |op| sink.send(op).unwrap();
        let connecting = Session::connect(&target, &known, sink, &sent);
        let session = timeout(WAIT, connecting).await.expect("connect hung").unwrap();
        (server, session, ops)
    }

    async fn heard_until(server: &mut TestServer, last: Seen) -> Vec<Seen> {
        let mut heard = Vec::new();
        loop {
            let seen = timeout(WAIT, server.seen.recv()).await.expect("timed out").expect("gone");
            let done = seen == last;
            heard.push(seen);
            if done {
                return heard;
            }
        }
    }

    #[tokio::test]
    async fn events_reach_the_server_in_the_order_given() {
        let (mut server, session, _ops) = open().await;
        let events = vec![
            RdpInput::Key { down: true, code: 0x1e, extended: false },
            RdpInput::Pointer { buttons: 0, x: 10, y: 20 },
            RdpInput::Unicode { down: true, codepoint: 0xe9 },
            RdpInput::Clipboard { text: "hi".into() },
            RdpInput::Pointer { buttons: 1, x: 11, y: 20 },
            RdpInput::Locks { caps: true, num: false, scroll: false },
            RdpInput::Key { down: false, code: 0x1e, extended: false },
        ];
        timeout(WAIT, send_events(&session, events)).await.expect("input hung").unwrap();
        let last = Seen::Key { code: 0x1e, extended: false, down: false };
        let mut heard = timeout(WAIT, heard_until(&mut server, last)).await.expect("timed out");
        // The announcement goes over a channel of its own, so it may come after the last key.
        if !heard.contains(&Seen::ClipFormats(vec![13])) {
            let announced = heard_until(&mut server, Seen::ClipFormats(vec![13]));
            heard.extend(timeout(WAIT, announced).await.expect("timed out"));
        }
        let is_input = |seen: &Seen| {
            let keys = matches!(seen, Seen::Key { .. } | Seen::Unicode { .. } | Seen::Locks(_));
            keys || matches!(seen, Seen::Mouse(_))
        };
        let input: Vec<_> = heard.into_iter().filter(is_input).collect();
        assert_eq!(
            input,
            [
                Seen::Key { code: 0x1e, extended: false, down: true },
                Seen::Mouse(Mouse::Move { x: 10, y: 20 }),
                Seen::Unicode { unit: 0xe9, down: true },
                Seen::Mouse(Mouse::Move { x: 11, y: 20 }),
                Seen::Mouse(Mouse::Left(true)),
                Seen::Locks(0x04),
                Seen::Key { code: 0x1e, extended: false, down: false },
            ],
        );
    }

    #[tokio::test]
    async fn a_batch_for_a_session_that_has_ended_is_no_error() {
        let (server, session, _ops) = open().await;
        timeout(WAIT, session.close()).await.expect("close hung");
        let events = vec![
            RdpInput::Key { down: false, code: 0x1e, extended: false },
            RdpInput::Clipboard { text: "late".into() },
            RdpInput::Pointer { buttons: 0, x: 1, y: 2 },
        ];
        timeout(WAIT, send_events(&session, events)).await.expect("input hung").unwrap();
        drop(server);
    }

    // A server that takes the TLS upgrade for NLA and then never answers, so that the login waits
    // in CredSSP. `started` fires on the first CredSSP bytes, and the task ends when the client
    // drops the connection.
    async fn stuck_in_credssp() -> (u16, oneshot::Receiver<()>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (started, heard) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 4];
            tcp.read_exact(&mut header).await.unwrap();
            let mut body = vec![0u8; usize::from(u16::from_be_bytes([header[2], header[3]])) - 4];
            tcp.read_exact(&mut body).await.unwrap();
            tcp.write_all(&confirm(SecurityProtocol::HYBRID_EX)).await.unwrap();
            let mut tls = tls().0.accept(tcp).await.unwrap();
            tls.read_exact(&mut [0u8; 1]).await.unwrap();
            started.send(()).unwrap();
            tls.read_to_end(&mut Vec::new()).await.ok();
        });
        (port, heard, server)
    }

    #[tokio::test]
    async fn closing_an_open_that_waits_in_credssp_ends_it_and_drops_the_connection() {
        let ended = off_thread(WAIT, || async {
            let (port, started, server) = stuck_in_credssp().await;
            let rdps = Arc::new(RdpSessions::default());
            let opening = tokio::spawn({
                let rdps = Arc::clone(&rdps);
                async move {
                    let (known, target) = (known(port), target(port));
                    let sent = AtomicBool::default();
                    let connecting = Session::connect(&target, &known, |_| {}, &sent);
                    rdps.open("tab-1", &sent, connecting).await
                }
            });
            started.await.unwrap();
            let closing = Instant::now();
            rdps.close("tab-1").await;
            let closed_in = closing.elapsed();
            let opened = timeout(Duration::from_secs(1), opening).await.expect("open went on");
            timeout(Duration::from_secs(1), server).await.expect("the connection stayed").unwrap();
            let left = rdps.0.lock().await.len();
            (opened.unwrap().map(|opened| opened.id), closed_in, left)
        })
        .await;
        let (opened, closed_in, left) = ended.expect("the login was not ended");
        assert!(matches!(opened, Err(AppError::Cancelled)), "{opened:?}");
        assert!(closed_in < Duration::from_millis(100), "{closed_in:?}");
        assert_eq!(left, 0);
    }

    async fn listen() -> TestServer {
        TestServer::listen(Options { login: Login::Tls, size: (256, 128), remotefx: false }).await
    }

    async fn gone(server: &mut TestServer) {
        loop {
            let seen = timeout(WAIT, server.seen.recv()).await.expect("timed out").expect("gone");
            if matches!(seen, Seen::Gone(_)) {
                return;
            }
        }
    }

    #[tokio::test]
    async fn an_open_session_is_closed_by_its_id() {
        let mut server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        let opened = timeout(WAIT, rdps.open("tab-1", &sent, connecting)).await.expect("open hung");
        let opened = opened.unwrap();
        assert_eq!((opened.id.as_str(), opened.width, opened.height), ("tab-1", 256, 128));
        assert!(rdps.session("tab-1").await.is_some());

        timeout(WAIT, rdps.close("tab-1")).await.expect("close hung");
        assert!(rdps.session("tab-1").await.is_none());
        timeout(WAIT, gone(&mut server)).await.expect("the server did not see the end");
    }

    #[tokio::test]
    async fn the_close_all_command_ends_every_session_as_the_exit_does() {
        let app = tauri::test::mock_app();
        app.manage(RdpSessions::default());
        let mut servers = Vec::new();
        for id in ["tab-1", "tab-2"] {
            let server = listen().await;
            open_through(&app.state::<RdpSessions>(), id, server.addr.unwrap().port()).await;
            servers.push(server);
        }
        timeout(WAIT, rdp_close_all(app.state())).await.expect("close all hung").unwrap();
        for server in &mut servers {
            let ended = loop {
                let seen = timeout(WAIT, server.seen.recv()).await.expect("timed out");
                if let Seen::Gone(error) = seen.expect("gone") {
                    break error;
                }
            };
            assert_eq!(ended, None, "the session was not shut down");
        }
        let rdps = app.state::<RdpSessions>();
        assert!(rdps.session("tab-1").await.is_none() && rdps.session("tab-2").await.is_none());
    }

    #[tokio::test]
    async fn an_id_in_use_is_refused_and_the_session_holding_it_goes_on() {
        let server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        timeout(WAIT, rdps.open("tab-1", &sent, connecting)).await.expect("open hung").unwrap();

        let again = rdps.open("tab-1", &sent, async { panic!("a second login was started") }).await;
        assert!(matches!(again, Err(AppError::Internal(m)) if m == "rdp: this id is in use"));
        assert!(rdps.session("tab-1").await.is_some());
        timeout(WAIT, rdps.close("tab-1")).await.expect("close hung");
        drop(server);
    }

    // Commands run as tasks of their own, so the close of a tab can come before its open.
    #[tokio::test]
    async fn an_open_whose_id_was_closed_before_it_came_is_cancelled_at_once() {
        let rdps = RdpSessions::default();
        rdps.close("tab-1").await;
        let sent = AtomicBool::default();
        let opened = rdps.open("tab-1", &sent, async { panic!("the login was started") }).await;
        assert!(matches!(opened, Err(AppError::Cancelled)));
        assert!(rdps.0.lock().await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn close_markers_that_no_open_claims_are_dropped_after_the_limit() {
        let rdps = RdpSessions::default();
        for n in 0..100 {
            rdps.close(&format!("gone-{n}")).await;
        }
        tokio::time::advance(CLOSED_LIMIT + Duration::from_millis(1)).await;
        rdps.close("tab-1").await;
        let entries = rdps.0.lock().await;
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries.get("tab-1"), Some(Entry::Closed { .. })));
    }

    #[tokio::test(start_paused = true)]
    async fn a_close_marker_still_cancels_its_open_inside_the_limit() {
        let rdps = RdpSessions::default();
        rdps.close("tab-1").await;
        tokio::time::advance(CLOSED_LIMIT - Duration::from_millis(1)).await;
        rdps.close("tab-2").await;
        let sent = AtomicBool::default();
        let opened = rdps.open("tab-1", &sent, async { panic!("the login was started") }).await;
        assert!(matches!(opened, Err(AppError::Cancelled)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_close_during_a_slow_gateway_open_still_cancels_it() {
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let slow = CONNECT_LIMIT + LOGIN_LIMIT + Duration::from_secs(5);
        assert!(slow <= LOGIN_LIMIT * 2);
        // Stands for a login that has sent the credentials, which a close lets finish.
        let connecting = async {
            sent.store(true, Ordering::SeqCst);
            tokio::time::sleep(slow).await;
            Err(AppError::Internal("rdp: the open went on".into()))
        };
        let closing = async {
            rdps.close("tab-1").await;
            tokio::time::sleep(slow - Duration::from_secs(1)).await;
            // Drops the close markers older than CLOSED_LIMIT.
            rdps.close("tab-2").await;
        };
        let (opened, ()) = tokio::join!(rdps.open("tab-1", &sent, connecting), closing);
        assert!(matches!(opened, Err(AppError::Cancelled)), "{:?}", opened.map(|o| o.id));
    }

    #[tokio::test]
    async fn a_session_closed_as_its_login_finishes_is_ended_and_cancelled() {
        let mut server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let connecting = async {
            let session = Session::connect(&target, &known, |_| {}, &sent).await;
            rdps.close("tab-1").await;
            session
        };
        let opened = timeout(WAIT, rdps.open("tab-1", &sent, connecting)).await.expect("open hung");
        assert!(matches!(opened, Err(AppError::Cancelled)));
        assert!(rdps.0.lock().await.is_empty());
        timeout(WAIT, gone(&mut server)).await.expect("the server did not see the end");
    }

    #[tokio::test]
    async fn a_close_after_the_credentials_went_out_lets_the_login_finish_and_ends_it_later() {
        let mut server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let (marked_tx, marked) = oneshot::channel();
        let (go_tx, go) = oneshot::channel::<()>();
        // Stands for a login that has sent the credentials and is not done yet.
        let connecting = async {
            sent.store(true, Ordering::SeqCst);
            marked_tx.send(()).unwrap();
            go.await.unwrap();
            Session::connect(&target, &known, |_| {}, &AtomicBool::default()).await
        };
        let closing = async {
            marked.await.unwrap();
            rdps.close("tab-1").await;
            assert!(go_tx.send(()).is_ok(), "the login was dropped");
            Instant::now()
        };
        let both = async { tokio::join!(rdps.open("tab-1", &sent, connecting), closing) };
        let (opened, closed) = timeout(WAIT, both).await.expect("open hung");
        assert!(matches!(opened, Err(AppError::Cancelled)), "{:?}", opened.map(|o| o.id));
        assert!(rdps.0.lock().await.is_empty());
        timeout(WAIT, gone(&mut server)).await.expect("the server did not see the end");
        assert!(closed.elapsed() >= SETTLE, "ended {:?} after the close", closed.elapsed());
    }

    #[tokio::test]
    async fn a_session_ends_a_second_after_its_login_at_the_soonest() {
        let mut server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let started = Instant::now();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        timeout(WAIT, rdps.open("tab-1", &sent, connecting)).await.expect("open hung").unwrap();
        timeout(WAIT, rdps.close("tab-1")).await.expect("close hung");
        assert!(started.elapsed() >= SETTLE, "closed {:?} after the open", started.elapsed());
        timeout(WAIT, gone(&mut server)).await.expect("the server did not see the end");

        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        timeout(WAIT, rdps.open("tab-2", &sent, connecting)).await.expect("open hung").unwrap();
        tokio::time::sleep(SETTLE).await;
        let closing = Instant::now();
        timeout(WAIT, rdps.close("tab-2")).await.expect("close hung");
        assert!(closing.elapsed() < SETTLE / 2, "close took {:?}", closing.elapsed());
    }

    // With HYBRID_EX the client reads the server's Early User Authorization Result after the
    // credentials have gone out.
    #[tokio::test]
    async fn a_close_while_the_early_user_authorization_result_is_held_lets_the_login_finish() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (heard_tx, heard) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let account = ("alice", "secret");
            let (mut framed, channels) =
                accept_nla_holding(tcp, (256, 128), account, heard_tx, released).await;
            loop {
                let pdu = read_client(&mut framed, &channels).await.unwrap();
                if matches!(pdu, ClientPdu::ShutdownRequest) {
                    return Instant::now();
                }
            }
        });
        let rdps = Arc::new(RdpSessions::default());
        let opening = tokio::spawn({
            let rdps = Arc::clone(&rdps);
            async move {
                let (known, target) = (known(port), target(port));
                let sent = AtomicBool::default();
                let connecting = Session::connect(&target, &known, |_| {}, &sent);
                rdps.open("tab-1", &sent, connecting).await.map(|opened| opened.id)
            }
        });
        timeout(WAIT, heard).await.expect("the credentials did not come").unwrap();
        let closing = Instant::now();
        rdps.close("tab-1").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!opening.is_finished(), "the login was dropped");
        release.send(()).unwrap();
        let opened = timeout(WAIT, opening).await.expect("open hung").unwrap();
        assert!(matches!(opened, Err(AppError::Cancelled)), "{opened:?}");
        let shut = timeout(WAIT, server).await.expect("no shutdown came").unwrap();
        assert!(shut - closing >= SETTLE, "ended {:?} after the close", shut - closing);
    }

    // Passes the connection to `to` on until `freeze` fires, then holds both ends open and
    // passes nothing more: a server that stops answering.
    async fn freezing(to: u16) -> (u16, Arc<tokio::sync::Notify>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let freeze = Arc::new(tokio::sync::Notify::new());
        tokio::spawn({
            let freeze = Arc::clone(&freeze);
            async move {
                let (mut client, _) = listener.accept().await.unwrap();
                let mut server = tokio::net::TcpStream::connect(("127.0.0.1", to)).await.unwrap();
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                    _ = freeze.notified() => std::future::pending::<()>().await,
                }
            }
        });
        (port, freeze)
    }

    async fn open_through(rdps: &RdpSessions, id: &str, port: u16) {
        let (known, target) = (known(port), target(port));
        let sent = AtomicBool::default();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        timeout(WAIT, rdps.open(id, &sent, connecting)).await.expect("open hung").unwrap();
    }

    #[tokio::test]
    async fn closing_all_ends_every_session_gracefully_and_all_at_once() {
        // The test server takes one connection at a time.
        let mut servers = Vec::new();
        let rdps = Arc::new(RdpSessions::default());
        for id in ["tab-1", "tab-2"] {
            let server = listen().await;
            open_through(&rdps, id, server.addr.unwrap().port()).await;
            servers.push(server);
        }
        let mut frozen = Vec::new();
        for n in 0..3 {
            let server = listen().await;
            let (through, freeze) = freezing(server.addr.unwrap().port()).await;
            open_through(&rdps, &format!("stuck-{n}"), through).await;
            frozen.push((server, freeze));
        }
        let (crossing, started, waiting) = stuck_in_credssp().await;
        let opening = tokio::spawn({
            let rdps = Arc::clone(&rdps);
            async move {
                let (known, target) = (known(crossing), target(crossing));
                let sent = AtomicBool::default();
                let connecting = Session::connect(&target, &known, |_| {}, &sent);
                rdps.open("logging-in", &sent, connecting).await.map(|opened| opened.id)
            }
        });
        started.await.unwrap();
        tokio::time::sleep(SETTLE).await;
        for (_, freeze) in &frozen {
            freeze.notify_one();
        }

        let closing = Instant::now();
        timeout(WAIT, rdps.close_all()).await.expect("close_all hung");
        let took = closing.elapsed();
        // Three servers that do not answer, each given CLOSE_LIMIT: one after the other would
        // take three times that.
        assert!(took >= CLOSE_LIMIT && took < CLOSE_LIMIT + SETTLE / 2, "took {took:?}");
        for server in &mut servers {
            let ended = loop {
                let seen = timeout(Duration::from_millis(200), server.seen.recv()).await;
                let seen = seen.expect("the server did not see the end").expect("gone");
                if let Seen::Gone(error) = seen {
                    break error;
                }
            };
            assert_eq!(ended, None);
        }
        let opened = timeout(Duration::from_secs(1), opening).await.expect("open went on");
        assert!(matches!(opened.unwrap(), Err(AppError::Cancelled)));
        timeout(Duration::from_secs(1), waiting).await.expect("the login stayed").unwrap();
        assert!(rdps.session("tab-1").await.is_none() && rdps.session("stuck-0").await.is_none());
    }

    #[tokio::test]
    async fn closing_all_lets_a_login_past_its_credentials_finish_and_ends_it() {
        let mut server = listen().await;
        let port = server.addr.unwrap().port();
        let (known, target) = (known(port), target(port));
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let (marked_tx, marked) = oneshot::channel();
        let connecting = async {
            sent.store(true, Ordering::SeqCst);
            marked_tx.send(()).unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            Session::connect(&target, &known, |_| {}, &AtomicBool::default()).await
        };
        let closing = async {
            marked.await.unwrap();
            let started = Instant::now();
            rdps.close_all().await;
            started
        };
        let both = async { tokio::join!(rdps.open("tab-1", &sent, connecting), closing) };
        let (opened, started) = timeout(WAIT, both).await.expect("open hung");
        let took = started.elapsed();
        assert!(matches!(opened, Err(AppError::Cancelled)), "{:?}", opened.map(|o| o.id));
        // The login took 200 ms of it, and the session is kept SETTLE after its login.
        assert!(took >= SETTLE && took < EXIT_LIMIT, "took {took:?}");
        let ended = timeout(Duration::from_millis(100), gone(&mut server)).await;
        assert!(ended.is_ok(), "close_all returned before the session had ended");
        assert!(rdps.session("tab-1").await.is_none());
    }

    #[tokio::test]
    async fn a_login_that_never_finishes_holds_up_closing_all_no_longer_than_the_bound() {
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let (marked_tx, marked) = oneshot::channel();
        let connecting = async {
            sent.store(true, Ordering::SeqCst);
            marked_tx.send(()).unwrap();
            std::future::pending::<AppResult<Session>>().await
        };
        let closing = async {
            marked.await.unwrap();
            let started = Instant::now();
            rdps.close_all().await;
            started.elapsed()
        };
        tokio::select! {
            opened = rdps.open("tab-1", &sent, connecting) => {
                panic!("the open ended: {:?}", opened.map(|o| o.id));
            }
            took = timeout(WAIT, closing) => {
                let took = took.expect("close_all hung");
                assert!(took >= EXIT_LIMIT && took < EXIT_LIMIT + SETTLE / 2, "took {took:?}");
            }
        }
    }

    fn office() -> RdpProfile {
        RdpProfile {
            id: "r1".into(),
            name: "office".into(),
            host: "10.0.0.9".into(),
            port: 3390,
            username: "CORP\\faye".into(),
            domain: Some("corp".into()),
            secret_id: Some("vault-new".into()),
            keyboard_layout: Some(0x407),
            audio: false,
            shared_folder: Some(SharedFolder {
                path: "C:\\Users\\faye\\share".into(),
                name: "share".into(),
                extra: Default::default(),
            }),
            gateway: Some(gateway(false)),
            icon: Default::default(),
            order: 0,
            extra: Default::default(),
        }
    }

    fn gateway(same_login: bool) -> RdpGateway {
        RdpGateway {
            host: "gw.corp".into(),
            port: 8443,
            same_login,
            username: "gw-faye".into(),
            secret_id: Some("vault-gw".into()),
            extra: Default::default(),
        }
    }

    fn given(profile_id: Option<&str>) -> RdpTarget {
        RdpTarget {
            host: "shown".into(),
            port: 3389,
            username: "old".into(),
            domain: Some("olddom".into()),
            secret_id: Some("vault-old".into()),
            profile_id: profile_id.map(Into::into),
            keyboard_layout: Some(0x409),
        }
    }

    fn quick() -> Dialed {
        Dialed {
            host: "shown".into(),
            port: 3389,
            username: "old".into(),
            domain: Some("olddom".into()),
            secret_id: Some("vault-old".into()),
            keyboard_layout: Some(0x409),
            audio: true,
            shared_folder: None,
            gateway: None,
        }
    }

    fn unreadable() -> AppResult<Vec<RdpProfile>> {
        Err(AppError::Internal("store lock poisoned".into()))
    }

    #[test]
    fn a_profile_id_connects_with_what_the_profile_holds_now() {
        let target = resolve_target(|| Ok(vec![office()]), given(Some("r1"))).unwrap();
        let held = Dialed {
            host: "10.0.0.9".into(),
            port: 3390,
            username: "CORP\\faye".into(),
            domain: Some("corp".into()),
            secret_id: Some("vault-new".into()),
            keyboard_layout: Some(0x407),
            audio: false,
            shared_folder: Some(SharedFolder {
                path: "C:\\Users\\faye\\share".into(),
                name: "share".into(),
                extra: Default::default(),
            }),
            gateway: Some(RdpGateway {
                host: "gw.corp".into(),
                port: 8443,
                same_login: false,
                username: "gw-faye".into(),
                secret_id: Some("vault-gw".into()),
                extra: Default::default(),
            }),
        };
        assert_eq!(target, held);

        let bare = RdpProfile {
            domain: None,
            secret_id: None,
            keyboard_layout: None,
            audio: true,
            shared_folder: None,
            gateway: None,
            ..office()
        };
        let target = resolve_target(|| Ok(vec![bare]), given(Some("r1"))).unwrap();
        let bare = Dialed {
            domain: None,
            secret_id: None,
            keyboard_layout: None,
            audio: true,
            shared_folder: None,
            gateway: None,
            ..held
        };
        assert_eq!(target, bare);
    }

    #[test]
    fn a_profile_id_that_is_gone_is_not_found() {
        let missing = resolve_target(|| Ok(vec![office()]), given(Some("gone")));
        assert!(matches!(
            missing,
            Err(AppError::NotFound(m)) if m == "rdp: this profile no longer exists"
        ));
    }

    #[test]
    fn a_profile_id_reports_a_store_that_cannot_be_read() {
        let failed = resolve_target(unreadable, given(Some("r1")));
        assert!(matches!(failed, Err(AppError::Internal(m)) if m == "store lock poisoned"));
    }

    #[test]
    fn without_a_profile_id_the_arguments_are_used() {
        assert_eq!(resolve_target(|| Ok(vec![office()]), given(None)).unwrap(), quick());
    }

    #[test]
    fn without_a_profile_id_the_store_is_not_read() {
        let target = resolve_target(|| panic!("the store was read"), given(None));
        assert_eq!(target.unwrap(), quick());
    }

    #[test]
    fn a_quick_connect_plays_sound_without_a_share_or_a_gateway() {
        let target = resolve_target(|| Ok(vec![office()]), given(None)).unwrap();
        assert!(target.audio);
        assert_eq!(target.shared_folder, None);
        assert_eq!(target.gateway, None);
    }

    const LOGINS: [(&str, Option<&str>); 7] = [
        ("alice", None),
        ("alice", Some("CORP")),
        ("alice", Some("")),
        ("CORP\\alice", None),
        ("CORP\\alice", Some("OTHER")),
        ("alice@corp", None),
        ("alice@corp", Some("CORP")),
    ];

    #[test]
    fn the_same_login_puts_the_domain_in_front_of_a_bare_user_name() {
        let sent = [
            "alice",
            "CORP\\alice",
            "alice",
            "CORP\\alice",
            "CORP\\alice",
            "alice@corp",
            "alice@corp",
        ];
        for ((username, domain), sent) in LOGINS.into_iter().zip(sent) {
            let dialed = Dialed {
                username: username.into(),
                domain: domain.map(Into::into),
                gateway: Some(gateway(true)),
                ..quick()
            };
            let login = gateway_login(&dialed);
            assert_eq!(login, Some((sent.to_owned(), Some("vault-old"))), "{username} {domain:?}");
        }
    }

    #[test]
    fn the_same_login_is_the_user_name_the_server_gets() {
        let dialed = Dialed {
            username: " alice ".into(),
            domain: Some("CORP".into()),
            gateway: Some(gateway(true)),
            ..quick()
        };
        assert_eq!(gateway_login(&dialed), Some(("CORP\\alice".to_owned(), Some("vault-old"))));
    }

    #[test]
    fn a_separate_gateway_login_is_used_as_saved() {
        for (username, domain) in LOGINS {
            let dialed = Dialed {
                username: username.into(),
                domain: domain.map(Into::into),
                gateway: Some(gateway(false)),
                ..quick()
            };
            let login = gateway_login(&dialed);
            let saved = Some(("gw-faye".to_owned(), Some("vault-gw")));
            assert_eq!(login, saved, "{username} {domain:?}");
        }
        let without_password =
            Dialed { gateway: Some(RdpGateway { secret_id: None, ..gateway(false) }), ..quick() };
        assert_eq!(gateway_login(&without_password), Some(("gw-faye".to_owned(), None)));
    }

    #[test]
    fn without_a_gateway_there_is_no_gateway_login() {
        assert_eq!(gateway_login(&quick()), None);
    }

    const NO_GATEWAY_LOGIN: &str = "rdp: the gateway needs a user name and password";

    // What a connect fails with when the gateway on a port that listens gets this login. Nothing
    // may reach it: a Basic login with an empty password counts toward a lockout.
    async fn refused_before_the_gateway(username: &str, password: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let gateway = Gateway { host: "127.0.0.1", port, username, password };
        let target = Target { gateway: Some(gateway), ..target(port) };
        let (known, sent) = (known(port), AtomicBool::default());
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        let failed = timeout(WAIT, connecting).await.expect("connect hung");
        let asked = timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(asked.is_err(), "the gateway was asked");
        match failed {
            Err(AppError::Internal(message)) => message,
            other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
        }
    }

    #[tokio::test]
    async fn a_separate_gateway_login_without_a_user_name_or_password_stays_here() {
        // No saved gateway password: the vault is not asked and the password is empty.
        let saved = RdpGateway { secret_id: None, ..gateway(false) };
        let dialed = Dialed { gateway: Some(saved), ..quick() };
        assert_eq!(gateway_login(&dialed), Some(("gw-faye".to_owned(), None)));
        assert_eq!(refused_before_the_gateway("gw-faye", "").await, NO_GATEWAY_LOGIN);
        let saved = RdpGateway { username: String::new(), ..gateway(false) };
        let dialed = Dialed { gateway: Some(saved), ..quick() };
        assert_eq!(gateway_login(&dialed), Some((String::new(), Some("vault-gw"))));
        assert_eq!(refused_before_the_gateway("", "gw-secret").await, NO_GATEWAY_LOGIN);
    }

    #[tokio::test]
    async fn the_same_login_without_a_user_name_or_password_stays_here() {
        let dialed = Dialed { secret_id: None, gateway: Some(gateway(true)), ..quick() };
        assert_eq!(gateway_login(&dialed), Some(("olddom\\old".to_owned(), None)));
        assert_eq!(refused_before_the_gateway("olddom\\old", "").await, NO_GATEWAY_LOGIN);
        // The domain alone is no user name.
        let dialed = Dialed { username: " ".into(), gateway: Some(gateway(true)), ..quick() };
        assert_eq!(gateway_login(&dialed), Some((String::new(), Some("vault-old"))));
        assert_eq!(refused_before_the_gateway("", "secret").await, NO_GATEWAY_LOGIN);
    }

    #[test]
    fn a_viewport_is_brought_into_range() {
        assert_eq!(fit(0, 0), (1280, 800));
        assert_eq!(fit(0, 600), (1280, 800));
        assert_eq!(fit(900, 0), (1280, 800));
        assert_eq!(fit(100, 100), (200, 200));
        assert_eq!(fit(9000, 9000), (6324, 6324));
        assert_eq!(fit(1281, 801), (1280, 801));
        assert_eq!(fit(8191, 8191), (6324, 6324));
        assert_eq!(fit(1920, 1080), (1920, 1080));
        assert_eq!(fit(8192, 4882), (8192, 4882));
        assert_eq!(fit(8192, 4883), (8190, 4882));
        assert_eq!(fit(8192, 200), (8192, 200));
    }

    #[test]
    fn every_viewport_fits_the_area_the_session_accepts() {
        for width in (1..=u16::MAX).step_by(53) {
            for height in (1..=u16::MAX).step_by(59) {
                let (w, h) = fit(width, height);
                assert!((200..=8192).contains(&w) && (200..=8192).contains(&h), "{w}x{h}");
                assert_eq!(w % 2, 0, "{w}x{h}");
                assert!(size_supported(w, h), "{width}x{height} gave {w}x{h}");
                let clamped = (width.clamp(200, 8192) & !1, height.clamp(200, 8192));
                if size_supported(clamped.0, clamped.1) {
                    assert_eq!((w, h), clamped);
                } else {
                    let area = u32::from(w) * u32::from(h);
                    assert!(area > MAX_PIXELS - 3 * 8192, "{width}x{height} gave {w}x{h}");
                }
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_local_layout_is_read_from_windows() {
        assert!(local_layout() > 0);
    }

    #[test]
    fn a_layout_name_is_read_as_hexadecimal() {
        assert_eq!(parse_layout("00000407"), 0x407);
        assert_eq!(parse_layout("00020409"), 0x20409);
        assert_eq!(parse_layout("0000040c"), 0x40c);
        assert_eq!(parse_layout("garbage"), 0x409);
        assert_eq!(parse_layout(""), 0x409);
        assert_eq!(parse_layout("ffffffffff"), 0x409);
    }
}
