// Drives the real RDP session against the xrdp server of test-env/ and the Windows Server VM of
// test-env/windows, which have to be up:
//   cargo test --manifest-path src-tauri/Cargo.toml --test live_rdp -- --ignored --test-threads=1

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, timeout_at};
use wisp_lib::live_rdp::{Input, RdpSessions, Session, Target};
use wisp_lib::{AppError, FrameOp, KnownHosts, KnownHostsState};

const CONTAINER: &str = "wisp-test-xrdp";
const HOST: &str = "127.0.0.1";
const PORT: u16 = 3391;
const USER: &str = "wisp";
const PASSWORD: &str = "wisptest";
const SIZE: (u16, u16) = (1280, 800);
const WAIT: Duration = Duration::from_secs(20);

const SHIFT: u8 = 0x2A;
const CONTROL: u8 = 0x1D;
const RETURN: u8 = 0x1C;
const A: u8 = 0x1E;
const B: u8 = 0x30;
const D: u8 = 0x20;
const ONE: u8 = 0x02;

const LEFT: u16 = 0x01;
const RIGHT: u16 = 0x04;
const WHEEL_UP: u16 = 0x08;
const WHEEL_DOWN: u16 = 0x10;
const WHEEL_LEFT: u16 = 0x20;
const WHEEL_RIGHT: u16 = 0x40;
const X1: u16 = 0x80;
const X2: u16 = 0x100;

struct Ran {
    ok: bool,
    out: Vec<u8>,
    err: Vec<u8>,
}

fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).unwrap();
        bytes
    })
}

// With a limit of its own, so that a command that hangs fails the test instead of holding it.
fn docker(args: &[&str]) -> Ran {
    let mut child = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("docker did not start");
    let out = drain(child.stdout.take().unwrap());
    let err = drain(child.stderr.take().unwrap());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60) {
            child.kill().unwrap();
            panic!("docker {args:?} did not return");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ran { ok: status.success(), out: out.join().unwrap(), err: err.join().unwrap() }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

// As root, for the server's own files.
fn root(script: &str) -> Vec<u8> {
    let ran = docker(&["exec", CONTAINER, "sh", "-c", script]);
    assert!(ran.ok, "`{script}` failed: {}", text(&ran.err));
    ran.out
}

const AS_USER: [&str; 6] = ["exec", "-u", USER, "-e", "DISPLAY=:10", CONTAINER];

// As the user, in the X display of the session.
fn inside(script: &str) -> Vec<u8> {
    let ran = docker(&[&AS_USER[..], &["sh", "-c", script]].concat());
    assert!(ran.ok, "`{script}` failed: {}", text(&ran.err));
    ran.out
}

// What a server does with input shows inside a moment later.
async fn settles<T: PartialEq + std::fmt::Debug>(what: &str, wanted: T, ask: impl Fn() -> T) {
    let started = Instant::now();
    loop {
        let answer = ask();
        if answer == wanted {
            return;
        }
        assert!(started.elapsed() < WAIT, "{what}: {answer:?} instead of {wanted:?}");
        sleep(Duration::from_millis(100)).await;
    }
}

// A guard, so that a test that fails leaves the session as it found it too.
struct Restore(String);

impl Drop for Restore {
    fn drop(&mut self) {
        let ran = docker(&[&AS_USER[..], &["sh", "-c", &self.0]].concat());
        // A panic on top of the one that is failing the test would end the whole run.
        if !std::thread::panicking() {
            assert!(ran.ok, "`{}` failed", self.0);
        }
    }
}

fn start(name: &str, command: &str) -> Restore {
    // exec, so that the pid the guard ends is the program's.
    let script = format!("echo $$ > /tmp/{name}.pid; exec {command}");
    let ran = docker(&[&AS_USER[..1], &["-d"], &AS_USER[1..], &["sh", "-c", &script]].concat());
    assert!(ran.ok, "`{command}` did not start: {}", text(&ran.err));
    let files = format!("/tmp/{name}.pid /tmp/{name}.out");
    Restore(format!("kill $(cat /tmp/{name}.pid) 2>/dev/null; rm -f {files}"))
}

fn shows_window(name: &str) -> bool {
    let script = format!("xwininfo -name '{name}' 2>/dev/null | grep -c IsViewable; true");
    text(&inside(&script)) == "1"
}

fn pointer() -> (u16, u16) {
    let printed = text(&inside("xdotool getmouselocation"));
    let mut fields = printed.split([':', ' ']);
    let x = fields.nth(1).and_then(|x| x.parse().ok());
    let y = fields.nth(1).and_then(|y| y.parse().ok());
    x.zip(y).unwrap_or_else(|| panic!("pointer position {printed:?}"))
}

fn screen_size() -> String {
    let printed = text(&inside("xrandr"));
    let first = printed.lines().next().unwrap_or_default();
    let current = first.split(", ").find_map(|part| part.strip_prefix("current "));
    current.unwrap_or(first).to_owned()
}

fn screenshot() -> Vec<u8> {
    inside("xwd -root -silent | convert xwd:- rgb:-")
}

// The log file outlives restarts of the container, so only what follows `from` counts.
fn log_lines() -> usize {
    text(&root("wc -l < /var/log/xrdp.log")).parse().unwrap()
}

fn log_since(from: usize) -> String {
    text(&root(&format!("tail -n +{} /var/log/xrdp.log", from + 1)))
}

// As the session reports a certificate: SHA256 and the digest in lower-case hex.
fn fingerprint() -> String {
    let printed = text(&root("openssl x509 -in /etc/xrdp/cert.pem -noout -fingerprint -sha256"));
    let digest = printed.strip_prefix("sha256 Fingerprint=").expect("openssl's output");
    format!("SHA256:{}", digest.replace(':', "").to_lowercase())
}

fn known(pin: Option<&str>) -> (tempfile::TempDir, KnownHostsState) {
    known_at(HOST, PORT, pin)
}

fn known_at(host: &str, port: u16, pin: Option<&str>) -> (tempfile::TempDir, KnownHostsState) {
    let dir = tempfile::tempdir().unwrap();
    let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
    if let Some(pin) = pin {
        hosts.record(&format!("rdp/{host}"), port, pin).unwrap();
    }
    (dir, KnownHostsState(Arc::new(Mutex::new(hosts))))
}

// xrdp answers a connection request once it listens; Docker takes connections before that.
fn answers() -> bool {
    const REQUEST: [u8; 19] = [3, 0, 0, 19, 14, 0xE0, 0, 0, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0];
    let mut reply = [0u8; 19];
    TcpStream::connect((HOST, PORT))
        .and_then(|mut stream| {
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.write_all(&REQUEST)?;
            stream.read_exact(&mut reply)
        })
        .is_ok()
}

// Brings a stopped server back however the test ends. A later run could not: it fails at its
// first connect.
struct Up;

impl Drop for Up {
    fn drop(&mut self) {
        let started = docker(&["start", CONTAINER]).ok;
        let waiting = Instant::now();
        while !answers() && waiting.elapsed() < WAIT * 3 {
            std::thread::sleep(Duration::from_millis(100));
        }
        if !std::thread::panicking() {
            assert!(started, "not started");
            assert!(answers(), "xrdp does not answer after the start");
        }
    }
}

type Ops = mpsc::UnboundedReceiver<FrameOp>;

async fn connect(
    password: &str,
    known: &KnownHostsState,
    size: (u16, u16),
) -> (Result<Session, AppError>, Ops) {
    let target = Target {
        host: HOST,
        port: PORT,
        username: USER,
        domain: None,
        password,
        width: size.0,
        height: size.1,
        scale: 100,
        keyboard_layout: 0x409,
    };
    connect_to(&target, known).await
}

async fn connect_to(
    target: &Target<'_>,
    known: &KnownHostsState,
) -> (Result<Session, AppError>, Ops) {
    let (sink, ops) = mpsc::unbounded_channel();
    let sink = move |op| sink.send(op).unwrap();
    let sent = AtomicBool::default();
    let connecting = Session::connect(target, known, sink, &sent);
    (timeout(WAIT * 2, connecting).await.expect("connect hung"), ops)
}

async fn open_with(password: &str) -> View {
    let (_dir, known) = known(Some(&fingerprint()));
    let (session, ops) = connect(password, &known, SIZE).await;
    View::new(session.unwrap_or_else(|e| panic!("{e}")), ops)
}

async fn open() -> View {
    open_with(PASSWORD).await
}

struct View {
    session: Session,
    ops: Ops,
    size: (usize, usize),
    picture: Vec<u8>,
    drawn: Vec<bool>,
    resizes: Vec<(u16, u16)>,
    clipboard: Vec<String>,
    closed: Option<String>,
}

impl View {
    fn new(session: Session, ops: Ops) -> View {
        let size = (session.width as usize, session.height as usize);
        View {
            session,
            ops,
            size,
            picture: vec![0; size.0 * size.1 * 4],
            drawn: vec![false; size.0 * size.1],
            resizes: Vec::new(),
            clipboard: Vec::new(),
            closed: None,
        }
    }

    fn apply(&mut self, op: FrameOp) {
        let stride = self.size.0;
        match op {
            FrameOp::Rect { x, y, w, h, rgba } => {
                let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
                assert_eq!(rgba.len(), w * h * 4, "a rectangle's pixels");
                for (row, line) in rgba.chunks_exact(w * 4).enumerate() {
                    let at = (y + row) * stride + x;
                    self.picture[at * 4..(at + w) * 4].copy_from_slice(line);
                    self.drawn[at..at + w].fill(true);
                }
            }
            FrameOp::Resize { w, h } => {
                self.size = (w as usize, h as usize);
                self.picture = vec![0; self.size.0 * self.size.1 * 4];
                self.drawn = vec![false; self.size.0 * self.size.1];
                self.resizes.push((w, h));
            }
            FrameOp::Cursor { .. } | FrameOp::CursorDefault => {}
            FrameOp::Clipboard(text) => self.clipboard.push(text),
            FrameOp::Closed(reason) => self.closed = Some(reason),
            FrameOp::Sync => self.session.ack(),
            FrameOp::Copy { .. } => panic!("a copy, which RDP does not have"),
        }
    }

    async fn within(&mut self, limit: Duration, what: &str, done: impl Fn(&View) -> bool) {
        let end = tokio::time::Instant::now() + limit;
        while !done(self) {
            let op = timeout_at(end, self.ops.recv()).await;
            let op = op.unwrap_or_else(|_| panic!("no {what} in {limit:?}"));
            self.apply(op.unwrap_or_else(|| panic!("over before {what}")));
        }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&View) -> bool) {
        self.within(WAIT, what, done).await
    }

    async fn idle(&mut self, time: Duration) {
        let limit = tokio::time::Instant::now() + time;
        while let Ok(Some(op)) = timeout_at(limit, self.ops.recv()).await {
            self.apply(op);
        }
    }

    fn whole(&self) -> bool {
        self.drawn.iter().all(|drawn| *drawn)
    }

    async fn keys(&self, codes: &[u8]) {
        let key = |down, code| Input::Key { down, code, extended: false };
        let mut events: Vec<_> = codes.iter().map(|code| key(true, *code)).collect();
        events.extend(codes.iter().rev().map(|code| key(false, *code)));
        self.session.input(events).await.unwrap();
    }

    async fn point(&self, buttons: u16, x: u16, y: u16) {
        self.session.input(vec![Input::Pointer { buttons, x, y }]).await.unwrap();
    }

    async fn close(mut self) -> Duration {
        let started = Instant::now();
        timeout(WAIT, self.session.close()).await.expect("close hung");
        let took = started.elapsed();
        let rest = async {
            while let Some(op) = self.ops.recv().await {
                self.apply(op);
            }
        };
        timeout(WAIT, rest).await.expect("the sink outlived close");
        assert!(self.closed.is_none(), "close told the view {:?}", self.closed);
        let skipped = self.session.skipped();
        assert!(skipped.is_empty(), "PDUs skipped as unhandled: {skipped:?}");
        took
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_certificate_is_unknown_until_trusted_and_a_mismatch_once_it_changes() {
    let shown = fingerprint();
    let (_dir, known) = known(None);
    match connect(PASSWORD, &known, SIZE).await.0 {
        Err(AppError::HostKeyUnknown { host, port, fingerprint }) => {
            assert_eq!((host.as_str(), port), ("rdp/127.0.0.1", PORT));
            assert_eq!(fingerprint, shown);
            // What accepting the certificate in the dialog does.
            known.0.lock().unwrap().record(&host, port, &fingerprint).unwrap();
        }
        other => panic!("expected HostKeyUnknown, got {:?}", other.err()),
    }
    let (session, _ops) = connect(PASSWORD, &known, SIZE).await;
    let session = session.unwrap_or_else(|e| panic!("{e}"));
    timeout(WAIT, session.close()).await.expect("close hung");

    let other = format!("SHA256:{}", "0".repeat(64));
    let (_dir, known) = self::known(Some(&other));
    match connect(PASSWORD, &known, SIZE).await.0 {
        Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
            assert_eq!((host.as_str(), port), ("rdp/127.0.0.1", PORT));
            assert_eq!((stored, offered), (other, shown));
        }
        other => panic!("expected HostKeyMismatch, got {:?}", other.err()),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_picture_is_the_screen_inside() {
    let started = Instant::now();
    let mut view = open().await;
    assert_eq!(view.size, (SIZE.0 as usize, SIZE.1 as usize));
    view.until("whole screen", View::whole).await;
    println!("whole screen after {:?}", started.elapsed());
    assert_eq!(screen_size(), "1280 x 800");

    let started = Instant::now();
    loop {
        let shot = screenshot();
        assert_eq!(shot.len(), view.size.0 * view.size.1 * 3);
        let theirs: HashSet<_> = shot.chunks_exact(3).collect();
        let ours: HashSet<_> = view.picture.chunks_exact(4).map(|px| &px[..3]).collect();
        let width = view.size.0;
        let differing: Vec<_> = view
            .picture
            .chunks_exact(4)
            .zip(shot.chunks_exact(3))
            .enumerate()
            .filter(|(_, (ours, theirs))| ours[..3] != **theirs)
            .map(|(at, (ours, theirs))| {
                let off = ours.iter().zip(theirs).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                (at % width, at / width, off)
            })
            .collect();
        // RemoteFX is lossy: around the edges in the pattern's window some pixels come out a
        // little off.
        let near_window = |(x, y, _): &&(usize, usize, u8)| {
            (32..=852).contains(x) && (32..=551).contains(y)
        };
        let most = differing.iter().map(|(_, _, off)| *off).max().unwrap_or(0);
        let outside = differing.len() - differing.iter().filter(near_window).count();
        if theirs.len() == 9 && theirs.is_subset(&ours) && outside == 0 && most <= 32 {
            println!("{} pixels differ, by {most} at most", differing.len());
            break;
        }
        assert!(
            started.elapsed() < WAIT,
            "{} pixels differ, {outside} away from the window, by {most} at most; {} of {} colours",
            differing.len(),
            theirs.intersection(&ours).count(),
            theirs.len()
        );
        view.idle(Duration::from_millis(200)).await;
    }
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn keys_arrive_in_an_xterm_as_they_were_typed() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    let _pointer = Restore("xdotool mousemove 640 400".into());
    let xterm = "xterm -T typed -geometry 80x4+40+600 -fn 10x20";
    let _xterm = start("typed", &format!("{xterm} -e sh -c 'cat > /tmp/typed.out'"));
    settles("the xterm", true, || shows_window("typed")).await;
    // Keys go to the window under the pointer.
    view.point(0, 300, 660).await;
    settles("the pointer", (300, 660), pointer).await;

    for keys in [&[A][..], &[SHIFT, B], &[ONE], &[SHIFT, ONE], &[RETURN]] {
        view.keys(keys).await;
    }
    let typed = || inside("cat /tmp/typed.out 2>/dev/null; true");
    settles("the typed line", b"aB1!\n".to_vec(), typed).await;

    // xrdp types a Unicode key event only when the keyboard layout has a key for it.
    let unicode = |down, codepoint| Input::Unicode { down, codepoint };
    for codepoint in [0x7A, 0xE9] {
        let events = vec![unicode(true, codepoint), unicode(false, codepoint)];
        view.session.input(events).await.unwrap();
    }
    view.keys(&[RETURN]).await;
    settles("the second line", b"aB1!\nz\n".to_vec(), typed).await;

    // Ctrl+D ends the cat and with it the xterm.
    view.keys(&[CONTROL, D]).await;
    settles("the xterm leaving", false, || shows_window("typed")).await;
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_pointer_and_its_buttons_arrive() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    let _pointer = Restore("xdotool mousemove 640 400".into());
    for (x, y) in [(1000, 700), (640, 400)] {
        view.point(0, x, y).await;
        settles("the pointer", (x, y), pointer).await;
    }

    let _xev = start("xev", "xev -geometry 200x200+1000+100 > /tmp/xev.out 2>&1");
    settles("xev's window", true, || shows_window("Event Tester")).await;
    view.point(0, 1100, 200).await;
    settles("the pointer", (1100, 200), pointer).await;
    for buttons in [LEFT, RIGHT, WHEEL_UP, WHEEL_DOWN, WHEEL_LEFT, WHEEL_RIGHT, X1, X2] {
        view.point(buttons, 1100, 200).await;
        view.point(0, 1100, 200).await;
    }
    view.point(0, 1101, 200).await;
    settles("the last move", (1101, 200), pointer).await;
    let seen = "grep -A2 '^Button' /tmp/xev.out | grep -o '^Button[A-Za-z]*\\|button [0-9]*'";
    let buttons = || text(&inside(&format!("{seen}; true"))).replace('\n', " ");
    let wanted: Vec<_> = [1, 3, 4, 5, 6, 7, 8, 9]
        .map(|button| format!("ButtonPress button {button} ButtonRelease button {button}"))
        .into();
    settles("the buttons", wanted.join(" "), buttons).await;
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_clipboard_goes_both_ways() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    let run = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();

    // xclip owns the selection until xrdp takes it, and the container has no pkill.
    let owner = "[ \"$(tr '\\0' ' ' < $p/cmdline)\" = 'xclip -selection clipboard ' ]";
    let kill = format!("for p in /proc/[0-9]*; do {owner} && kill ${{p#/proc/}}; done; true");
    let _owner = Restore(kill);
    let started = Instant::now();
    let copy = format!("printf 'inside {run} \\303\\251' | xclip -selection clipboard");
    inside(&format!("{copy} >/dev/null 2>&1"));
    let wanted = format!("inside {run} é");
    view.until("clipboard text from inside", |view| view.clipboard.contains(&wanted)).await;
    println!("from inside after {:?}", started.elapsed());

    let started = Instant::now();
    view.session.clipboard(format!("outside {run} é")).await.unwrap();
    let paste = [&AS_USER[..], &["xclip", "-o", "-selection", "clipboard"]].concat();
    let wanted = format!("outside {run} é").into_bytes();
    settles("the clipboard inside", wanted, || docker(&paste).out).await;
    println!("from outside after {:?}", started.elapsed());
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn resize_asks_xrdp_through_display_control() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    for (w, h) in [(1024, 768), (1280, 800)] {
        let started = Instant::now();
        view.session.resize(w, h, 100).await.unwrap();
        let resized = |view: &View| view.size == (w as usize, h as usize) && view.whole();
        view.until("whole screen at the new size", resized).await;
        println!("{w}x{h} after {:?}", started.elapsed());
        assert_eq!(view.resizes.last(), Some(&(w, h)));
        settles("the size inside", format!("{w} x {h}"), screen_size).await;
    }
    assert_eq!(view.resizes, [(1024, 768), (1280, 800)]);
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_wrong_password_gets_xrdps_connection_log() {
    let before = log_lines();
    let mut view = open_with("wisptext").await;
    // xrdp draws its connection log window and nothing else.
    let width = view.size.0;
    let window = |at: usize| (10..410).contains(&(at % width)) && (10..410).contains(&(at / width));
    let drawn = |view: &View| -> Vec<usize> {
        view.drawn.iter().enumerate().filter(|(_, drawn)| **drawn).map(|(at, _)| at).collect()
    };
    let whole = |view: &View| drawn(view).into_iter().filter(|at| window(*at)).count() == 400 * 400;
    view.until("the connection log", whole).await;
    view.idle(Duration::from_secs(3)).await;
    assert!(drawn(&view).into_iter().all(window));
    let pixel = |at: usize| &view.picture[at * 4..at * 4 + 3];
    let colours: HashSet<_> = drawn(&view).into_iter().map(pixel).collect();
    let xrdp = [[0xe1, 0xe1, 0xe1], [0x00, 0x78, 0xd7], [0xff; 3], [0xb4; 3], [0x00; 3]];
    assert_eq!(colours, xrdp.iter().map(|rgb| &rgb[..]).collect());
    assert!(view.closed.is_none());
    assert!(log_since(before).contains("User does not exist, or could not be authenticated"));
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_stopped_server_ends_the_session() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    let up = Up;
    let stopping = Instant::now();
    assert!(docker(&["stop", CONTAINER]).ok, "not stopped");
    let ended = |view: &View| view.closed.is_some();
    view.within(Duration::from_secs(60), "end of the session", ended).await;
    let took = stopping.elapsed();
    assert_eq!(view.closed.as_deref(), Some("network connection lost"));
    println!("closed {took:?} after the stop began");
    drop(up);
    let skipped = view.session.skipped();
    assert!(skipped.is_empty(), "PDUs skipped as unhandled: {skipped:?}");
    timeout(WAIT, view.session.close()).await.expect("close hung");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn close_ends_within_a_second_and_xrdp_logs_the_disconnect() {
    let mut view = open().await;
    view.until("whole screen", View::whole).await;
    let before = log_lines();
    let took = view.close().await;
    println!("close took {took:?}");
    assert!(took < Duration::from_millis(1100), "close took {took:?}");
    let left = || log_since(before);
    let started = Instant::now();
    while !left().contains("xrdp_process_data_in: xrdp_process_loop failed") {
        assert!(started.elapsed() < WAIT, "xrdp logged {:?}", left());
        sleep(Duration::from_millis(100)).await;
    }
    // A connection that is cut instead fails in SSL_read first.
    let logged = left();
    let first = logged.lines().next().unwrap_or_default();
    assert!(first.ends_with("[ERROR] xrdp_sec_recv: xrdp_mcs_recv failed"), "{logged}");
    assert!(!logged.contains("SSL_read"), "{logged}");
}

// The Windows Server VM of test-env/windows, which has to be running in Hyper-V.

const VM: &str = "wisp-test-rdp";
const WINDOWS_PORT: u16 = 3389;
const WINDOWS_PASSWORD: &str = "Wisp-Test-2026";
// A logon that makes a new session starts the probe only after Windows has set it up.
const LOGON: Duration = Duration::from_secs(120);
const US: u32 = 0x409;
const GERMAN: u32 = 0x407;
const PROBE: [u8; 3] = [0x30, 0x60, 0x90];
const BARS: [[u8; 3]; 4] = [[0xff, 0, 0], [0, 0xff, 0], [0, 0, 0xff], [0xff, 0xff, 0]];

const BACKSPACE: u8 = 0x0E;
const ALT: u8 = 0x38;
const Q: u8 = 0x10;
// The keys labelled Y and Z on a US keyboard.
const Y: u8 = 0x15;
const Z: u8 = 0x2C;
const MIDDLE: u16 = 0x02;

// Every line a complete statement: `-Command -` runs each line as it comes.
const PRELUDE: &str = "\
$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'
$password = ConvertTo-SecureString 'Wisp-Test-2026' -AsPlainText -Force
$credential = New-Object System.Management.Automation.PSCredential('wisp', $password)
function Decode($code) { [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($code)) }
function Reply($out) { \
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($out | Out-String))) }
function OnHost($code) { & ([scriptblock]::Create($code)) }
function InGuest($code) { if (-not $guest -or $guest.State -ne 'Opened') { \
    $global:guest = New-PSSession -VMName wisp-test-rdp -Credential $credential }; \
    Invoke-Command -Session $guest -ScriptBlock ([scriptblock]::Create($code)) }
";

struct Shell {
    input: ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    sent: u64,
}

// One PowerShell for the whole run, as opening a PowerShell Direct session takes seconds.
fn shell() -> Option<MutexGuard<'static, Shell>> {
    static SHELL: OnceLock<Option<Mutex<Shell>>> = OnceLock::new();
    let shell = SHELL.get_or_init(|| {
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let (lines, received) = std::sync::mpsc::channel();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let mut input = child.stdin.take().unwrap();
        input.write_all(PRELUDE.as_bytes()).unwrap();
        Some(Mutex::new(Shell { input, lines: received, sent: 0 }))
    });
    shell.as_ref().map(|shell| shell.lock().unwrap_or_else(PoisonError::into_inner))
}

// The script's output as text. Both ways it travels as base64, as the console's code page
// would mangle anything outside it.
fn powershell(function: &str, script: &str) -> Result<String, String> {
    let mut shell = shell().ok_or("powershell.exe did not start")?;
    // A reply that comes after its script timed out must not pass for the next one's.
    shell.sent += 1;
    let (ok, failed) = (format!("ok {} ", shell.sent), format!("failed {} ", shell.sent));
    let code = BASE64.encode(script);
    let reply = format!("'{ok}' + (Reply ({function} (Decode '{code}')))");
    let line = format!("try {{ {reply} }} catch {{ '{failed}' + (Reply $_) }}\n");
    shell.input.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let decode = |text: &str| {
        let bytes = BASE64.decode(text).unwrap();
        String::from_utf8(bytes).unwrap().trim_end().to_owned()
    };
    loop {
        let reply = shell.lines.recv_timeout(Duration::from_secs(120));
        let reply = reply.unwrap_or_else(|_| panic!("powershell did not answer `{script}`"));
        if let Some(out) = reply.strip_prefix(&ok) {
            return Ok(decode(out));
        }
        if let Some(failure) = reply.strip_prefix(&failed) {
            return Err(decode(failure));
        }
    }
}

fn on_host(script: &str) -> String {
    powershell("OnHost", script).unwrap_or_else(|e| panic!("`{script}` failed: {e}"))
}

// Through PowerShell Direct, as wisp, outside the RDP session.
fn in_guest(script: &str) -> String {
    powershell("InGuest", script).unwrap_or_else(|e| panic!("`{script}` failed in the VM: {e}"))
}

// The VM's address, or None when it is not running, which skips the test.
fn vm() -> Option<String> {
    let state = powershell("OnHost", &format!("(Get-VM -Name {VM}).State"));
    if state.as_deref() != Ok("Running") {
        println!("skipped: the Hyper-V VM {VM} is not running here ({state:?})");
        return None;
    }
    Some(address())
}

const ADDRESS: &str = "(Get-VMNetworkAdapter -VMName wisp-test-rdp).IPAddresses | \
    Where-Object { $_ -match '^\\d+\\.' }";

fn address() -> String {
    let address = on_host(ADDRESS);
    assert!(!address.is_empty() && !address.contains('\n'), "the VM's address: {address:?}");
    address
}

// A file of the probe in C:\wisp as it is now, empty when there is none.
fn probe_file(name: &str) -> Vec<u8> {
    // Opened for others to write, as the probe may be writing it.
    let script = format!(
        "$path = 'C:\\wisp\\{name}'; if (Test-Path $path) {{ \
         $file = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite, Delete'); \
         $bytes = New-Object IO.MemoryStream; $file.CopyTo($bytes); $file.Close(); \
         [Convert]::ToBase64String($bytes.ToArray()) }}"
    );
    BASE64.decode(in_guest(&script)).unwrap()
}

fn probe_text(name: &str) -> String {
    String::from_utf8(probe_file(name)).unwrap()
}

fn probe_lines(name: &str) -> Vec<String> {
    probe_text(name).lines().map(str::to_owned).collect()
}

fn set_probe_file(name: &str, text: &str) {
    let code = BASE64.encode(text);
    in_guest(&format!(
        "[IO.File]::WriteAllBytes('C:\\wisp\\{name}', [Convert]::FromBase64String('{code}'))"
    ));
}

// The certificate the RDP listener uses, as the session reports it.
fn windows_fingerprint() -> String {
    let script = "$setting = Get-CimInstance -Namespace root\\cimv2\\TerminalServices \
        -ClassName Win32_TSGeneralSetting -Filter \"TerminalName='RDP-Tcp'\"; \
        $hash = $setting.SSLCertificateSHA1Hash; \
        $der = (Get-Item \"Cert:\\LocalMachine\\Remote Desktop\\$hash\").RawData; \
        -join ([Security.Cryptography.SHA256]::Create().ComputeHash($der) | \
        ForEach-Object { $_.ToString('x2') })";
    format!("SHA256:{}", in_guest(script))
}

fn windows<'a>(
    address: &'a str,
    username: &'a str,
    domain: Option<&'a str>,
    password: &'a str,
    keyboard_layout: u32,
) -> Target<'a> {
    Target {
        host: address,
        port: WINDOWS_PORT,
        username,
        domain,
        password,
        width: SIZE.0,
        height: SIZE.1,
        scale: 100,
        keyboard_layout,
    }
}

fn near(ours: &[u8], wanted: [u8; 3]) -> bool {
    ours.iter().zip(wanted).all(|(a, b)| a.abs_diff(b) <= 8)
}

impl View {
    fn pixel(&self, x: usize, y: usize) -> &[u8] {
        let at = (y * self.size.0 + x) * 4;
        &self.picture[at..at + 3]
    }

    // The probe's window: its background, and the four bars under the text box.
    fn probe_shown(&self) -> bool {
        let bar = |(i, colour): (usize, &[u8; 3])| near(self.pixel(100 + i * 160, 200), *colour);
        BARS.iter().enumerate().all(bar)
            && near(self.pixel(self.size.0 - 100, self.size.1 - 100), PROBE)
    }
}

async fn open_windows(address: &str, keyboard_layout: u32) -> View {
    if probe_text("layout.txt") != format!("{keyboard_layout:04x}") {
        log_off().await;
    }
    let (_dir, known) = known_at(address, WINDOWS_PORT, Some(&windows_fingerprint()));
    let target = windows(address, "wisp", None, WINDOWS_PASSWORD, keyboard_layout);
    let (session, ops) = connect_to(&target, &known).await;
    let mut view = View::new(session.unwrap_or_else(|e| panic!("{e}")), ops);
    // As the app does before the first key: the session keeps them from the last connection.
    view.locks(false).await;
    view.within(LOGON, "the probe's window", View::probe_shown).await;
    view
}

// A guard that runs a script in the VM however the test ends.
struct Undo(String);

impl Drop for Undo {
    fn drop(&mut self) {
        let ran = powershell("InGuest", &self.0);
        if !std::thread::panicking() {
            ran.unwrap_or_else(|e| panic!("`{}` failed in the VM: {e}", self.0));
        }
    }
}

fn answers_at(address: &str) -> bool {
    let address = (address, WINDOWS_PORT);
    std::net::ToSocketAddrs::to_socket_addrs(&address)
        .ok()
        .and_then(|mut all| all.next())
        .is_some_and(|at| TcpStream::connect_timeout(&at, Duration::from_secs(2)).is_ok())
}

// Starts a VM that a test turned off, however the test ends: the next test would find it off.
struct Started;

impl Drop for Started {
    fn drop(&mut self) {
        let started = powershell("OnHost", &format!("Start-VM -Name {VM}"));
        let waiting = Instant::now();
        let up = || {
            let address = powershell("OnHost", ADDRESS);
            address.is_ok_and(|address| !address.is_empty() && answers_at(&address))
                && powershell("InGuest", "$true").is_ok()
        };
        while !up() && waiting.elapsed() < Duration::from_secs(300) {
            std::thread::sleep(Duration::from_secs(1));
        }
        if !std::thread::panicking() {
            started.unwrap_or_else(|e| panic!("the VM did not start: {e}"));
            assert!(up(), "the VM did not come back");
        }
    }
}

impl View {
    async fn press(&self, keys: &[(u8, bool)]) {
        let key = |down, (code, extended): (u8, bool)| Input::Key { down, code, extended };
        let mut events: Vec<_> = keys.iter().map(|k| key(true, *k)).collect();
        events.extend(keys.iter().rev().map(|k| key(false, *k)));
        self.session.input(events).await.unwrap();
    }

    async fn locks(&self, caps: bool) {
        let locks = Input::Locks { caps, num: false, scroll: false };
        self.session.input(vec![locks]).await.unwrap();
    }

    // Into the probe's text box, which also gives it the focus back.
    async fn click_box(&self) {
        self.point(LEFT, 300, 80).await;
        self.point(0, 300, 80).await;
    }
}

fn sessions() -> String {
    in_guest("@(Get-Process explorer -ErrorAction SilentlyContinue).Count")
}

// Windows sets the keyboard layout at logon, and a connection to the session that is there
// keeps it, so another layout needs a logon of its own.
async fn log_off() {
    // From outside: logoff.exe run by the probe in a disconnected session does nothing.
    in_guest(
        "Get-Process explorer -ErrorAction SilentlyContinue | \
         ForEach-Object { logoff $_.SessionId }",
    );
    settles("the logoff", "0".to_owned(), sessions).await;
    // Until the probe of the next session writes it again.
    in_guest("Remove-Item -ErrorAction SilentlyContinue C:\\wisp\\layout.txt");
}

fn typed() -> String {
    probe_text("typed.txt")
}

// The key events in keys.txt after the first `from`: down or up, scancode, extended.
fn keys_since(from: usize) -> Vec<(String, String, String, String)> {
    let lines = probe_lines("keys.txt");
    let fields = |line: &String| {
        let mut parts = line.split(' ').map(str::to_owned);
        let mut next = || parts.next().unwrap_or_default();
        (next(), next(), next(), next())
    };
    lines.iter().skip(from).map(fields).collect()
}

fn every_key_released(events: &[(String, String, String, String)]) {
    for (at, (what, _, scancode, extended)) in events.iter().enumerate() {
        if what == "down" {
            let released = events[at..]
                .iter()
                .any(|(w, _, s, e)| w == "up" && s == scancode && e == extended);
            assert!(released, "{scancode} {extended} pressed, never released: {events:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_certificate_is_unknown_until_trusted_and_a_mismatch_once_it_changes() {
    let Some(address) = vm() else { return };
    let shown = windows_fingerprint();
    let target = windows(&address, "wisp", None, WINDOWS_PASSWORD, US);
    let (_dir, known) = known_at(&address, WINDOWS_PORT, None);
    match connect_to(&target, &known).await.0 {
        Err(AppError::HostKeyUnknown { host, port, fingerprint }) => {
            assert_eq!((host.clone(), port), (format!("rdp/{address}"), WINDOWS_PORT));
            assert_eq!(fingerprint, shown);
            known.0.lock().unwrap().record(&host, port, &fingerprint).unwrap();
        }
        other => panic!("expected HostKeyUnknown, got {:?}", other.err()),
    }
    let (session, ops) = connect_to(&target, &known).await;
    let mut view = View::new(session.unwrap_or_else(|e| panic!("{e}")), ops);
    // A session closed the moment it opened left Windows' clipboard process (rdpclip) stuck:
    // no later connection to that session got a clipboard.
    view.within(LOGON, "the probe's window", View::probe_shown).await;
    view.close().await;

    let other = format!("SHA256:{}", "0".repeat(64));
    let (_dir, known) = known_at(&address, WINDOWS_PORT, Some(&other));
    match connect_to(&target, &known).await.0 {
        Err(AppError::HostKeyMismatch { host, port, stored, offered }) => {
            assert_eq!((host, port), (format!("rdp/{address}"), WINDOWS_PORT));
            assert_eq!((stored, offered), (other, shown));
        }
        other => panic!("expected HostKeyMismatch, got {:?}", other.err()),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_nla_opens_at_the_size_asked_and_names_each_refusal() {
    let Some(address) = vm() else { return };
    let (_dir, known) = known_at(&address, WINDOWS_PORT, Some(&windows_fingerprint()));
    let target = windows(&address, "wisp", None, WINDOWS_PASSWORD, US);
    let target = Target { width: 1440, height: 900, ..target };
    let (session, ops) = connect_to(&target, &known).await;
    let mut view = View::new(session.unwrap_or_else(|e| panic!("{e}")), ops);
    assert_eq!(view.size, (1440, 900));
    view.within(LOGON, "the probe's window", View::probe_shown).await;
    settles("the size inside", "1440 900".to_owned(), || probe_text("screen.txt")).await;
    view.close().await;

    // Windows answers a wrong password, an unknown user and an empty password alike, with
    // STATUS_LOGON_FAILURE; the empty one does not get that far.
    let refusals = [
        ("wisp", "Wisp-Test-2025", "rdp: wrong user name or password"),
        ("nobody", WINDOWS_PASSWORD, "rdp: wrong user name or password"),
        ("wisp", "", "rdp: this server needs a user name and password"),
    ];
    for (user, password, words) in refusals {
        let target = windows(&address, user, None, password, US);
        match connect_to(&target, &known).await.0 {
            Err(AppError::Internal(message)) => assert_eq!(message, words, "{user} {password:?}"),
            other => panic!("{user} {password:?}: expected a refusal, got {:?}", other.err()),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_takes_the_computer_name_as_a_domain_but_not_as_a_upn_suffix() {
    let Some(address) = vm() else { return };
    let computer = in_guest("$env:COMPUTERNAME");
    let (_dir, known) = known_at(&address, WINDOWS_PORT, Some(&windows_fingerprint()));
    let backslash = format!("{computer}\\wisp");
    for (user, domain) in [(backslash.as_str(), None), ("wisp", Some(computer.as_str()))] {
        let target = windows(&address, user, domain, WINDOWS_PASSWORD, US);
        let (session, ops) = connect_to(&target, &known).await;
        let session = session.unwrap_or_else(|e| panic!("{user} {domain:?}: {e}"));
        let mut view = View::new(session, ops);
        view.within(LOGON, "the probe's window", View::probe_shown).await;
        view.close().await;
    }
    // NTLM carries a UPN whole in the user name, and a local account has none.
    let upn = format!("wisp@{computer}");
    let target = windows(&address, &upn, None, WINDOWS_PASSWORD, US);
    match connect_to(&target, &known).await.0 {
        Err(AppError::Internal(message)) => {
            assert_eq!(message, "rdp: wrong user name or password");
        }
        other => panic!("{upn}: expected a refusal, got {:?}", other.err()),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_picture_shows_the_probe_and_a_full_screen_change() {
    let Some(address) = vm() else { return };
    let mut view = open_windows(&address, US).await;
    let white = view.pixel(500, 120).to_vec();
    assert!(near(&white, [0xff; 3]), "the text box is {white:?}");
    let _colour = Undo("[IO.File]::WriteAllText('C:\\wisp\\colour.txt', '306090')".into());
    set_probe_file("colour.txt", "a05030");
    // Everything but the text box and the bars.
    let outside = |at: usize, width: usize| at % width >= 680 || at / width >= 260;
    let changed = |view: &View| {
        let width = view.size.0;
        view.picture.chunks_exact(4).enumerate().all(|(at, px)| {
            !outside(at, width) || near(px, [0xa0, 0x50, 0x30])
        })
    };
    let started = Instant::now();
    view.until("the new colour everywhere", changed).await;
    println!("the new colour after {:?}", started.elapsed());
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_keys_arrive_in_the_probe_as_typed() {
    let Some(address) = vm() else { return };
    let view = open_windows(&address, US).await;
    view.click_box().await;
    view.keys(&[CONTROL, A]).await;
    view.keys(&[BACKSPACE]).await;
    settles("an empty text box", String::new(), typed).await;
    let from = probe_lines("keys.txt").len();
    for keys in [&[A][..], &[SHIFT, B], &[ONE], &[SHIFT, ONE], &[RETURN]] {
        view.keys(keys).await;
    }
    settles("the typed text", "aB1!\n".to_owned(), typed).await;
    // Ctrl+A selects everything, so the next key replaces it.
    view.keys(&[CONTROL, A]).await;
    view.keys(&[Y]).await;
    settles("the text replaced", "y".to_owned(), typed).await;
    let events = keys_since(from);
    let downs: Vec<_> = events.iter().filter(|e| e.0 == "down").map(|e| e.2.as_str()).collect();
    assert_eq!(downs, ["1e", "2a", "30", "02", "2a", "02", "1c", "1d", "1e", "15"]);
    every_key_released(&events);
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_german_layout_swaps_y_and_z_and_has_altgr() {
    let Some(address) = vm() else { return };
    let view = open_windows(&address, GERMAN).await;
    view.click_box().await;
    view.keys(&[CONTROL, A]).await;
    view.keys(&[BACKSPACE]).await;
    settles("an empty text box", String::new(), typed).await;
    let from = probe_lines("keys.txt").len();
    view.keys(&[Y]).await;
    view.keys(&[Z]).await;
    view.press(&[(ALT, true), (Q, false)]).await;
    settles("the typed text", "zy@".to_owned(), typed).await;
    let events = keys_since(from);
    let vk = |scancode: &str| {
        events.iter().find(|e| e.0 == "down" && e.2 == scancode).map(|e| e.1.clone())
    };
    // VK_Z on the key that is Y on a US keyboard, and the other way round.
    assert_eq!((vk("15"), vk("2c")), (Some("5a".to_owned()), Some("59".to_owned())));
    every_key_released(&events);
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_caps_lock_on_locally_types_capitals() {
    let Some(address) = vm() else { return };
    let view = open_windows(&address, US).await;
    view.click_box().await;
    view.keys(&[CONTROL, A]).await;
    view.keys(&[BACKSPACE]).await;
    settles("an empty text box", String::new(), typed).await;
    view.locks(true).await;
    view.keys(&[A]).await;
    view.keys(&[B]).await;
    settles("capitals", "AB".to_owned(), typed).await;
    view.locks(false).await;
    view.keys(&[A]).await;
    settles("a small letter again", "ABa".to_owned(), typed).await;
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_pointer_buttons_and_wheels_arrive() {
    let Some(address) = vm() else { return };
    let view = open_windows(&address, US).await;
    let from = probe_lines("pointer.txt").len();
    let last = || probe_lines("pointer.txt").into_iter().skip(from).last();
    // Windows keeps only the last of the moves that wait in the queue, as it does with wheels.
    for (x, y) in [(1000, 600), (900, 500)] {
        view.point(0, x, y).await;
        settles("the move", Some(format!("move {x} {y} 0")), last).await;
    }
    let from = probe_lines("pointer.txt").len();
    let seen = || {
        let lines = probe_lines("pointer.txt");
        lines.into_iter().skip(from).filter(|line| !line.starts_with("move ")).collect::<Vec<_>>()
    };
    let buttons = [
        (LEFT, "left"),
        (RIGHT, "right"),
        (MIDDLE, "middle"),
        (X1, "x1"),
        (X2, "x2"),
        (WHEEL_UP, "wheel 120"),
        (WHEEL_DOWN, "wheel -120"),
        (WHEEL_LEFT, "hwheel -120"),
        (WHEEL_RIGHT, "hwheel 120"),
    ];
    let mut wanted = Vec::new();
    for (buttons, name) in buttons {
        view.point(buttons, 900, 500).await;
        view.point(0, 900, 500).await;
        match name.split_once(' ') {
            Some((wheel, delta)) => wanted.push(format!("{wheel} 900 500 {delta}")),
            None => wanted.extend(["down", "up"].map(|way| format!("{name}-{way} 900 500"))),
        }
        // Windows adds up the wheel messages that wait in the queue: up and down make one of 0.
        settles(name, wanted.clone(), seen).await;
    }
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_clipboard_goes_both_ways() {
    let Some(address) = vm() else { return };
    let mut view = open_windows(&address, US).await;
    let run = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();

    let started = Instant::now();
    set_probe_file("set-clipboard.txt", &format!("inside {run} é 😀\r\nsecond line"));
    let wanted = format!("inside {run} é 😀\nsecond line");
    view.until("clipboard text from inside", |view| view.clipboard.contains(&wanted)).await;
    println!("from inside after {:?}", started.elapsed());

    let started = Instant::now();
    view.session.clipboard(format!("outside {run} é 😀\nsecond line")).await.unwrap();
    let wanted = format!("outside {run} é 😀\r\nsecond line");
    settles("the clipboard inside", wanted, || probe_text("clipboard.txt")).await;
    println!("from outside after {:?}", started.elapsed());
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_resizes_through_display_control() {
    let Some(address) = vm() else { return };
    let mut view = open_windows(&address, US).await;
    let started = Instant::now();
    view.session.resize(1600, 900, 100).await.unwrap();
    let resized = |view: &View| view.size == (1600, 900) && view.probe_shown();
    view.until("the probe at the new size", resized).await;
    println!("1600x900 after {:?}", started.elapsed());
    assert_eq!(view.resizes, [(1600, 900)]);
    settles("the size inside", "1600 900".to_owned(), || probe_text("screen.txt")).await;

    let sizes: Vec<_> = (0..10).map(|i| (1200 + 40 * i, 700 + 20 * i)).collect();
    for (w, h) in &sizes {
        view.session.resize(*w, *h, 100).await.unwrap();
    }
    let last = sizes[9];
    let size = (last.0 as usize, last.1 as usize);
    let resized = |view: &View| view.size == size && view.probe_shown();
    view.until("the probe at the last size", resized).await;
    settles("the last size inside", format!("{} {}", last.0, last.1), || probe_text("screen.txt"))
        .await;
    view.idle(Duration::from_secs(2)).await;
    assert_eq!(view.resizes.last(), Some(&last));
    println!("sizes on the way: {:?}", view.resizes);
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_logoff_inside_ends_the_session() {
    let Some(address) = vm() else { return };
    let mut view = open_windows(&address, US).await;
    set_probe_file("logoff.txt", "");
    view.within(Duration::from_secs(60), "the end", |view| view.closed.is_some()).await;
    assert_eq!(view.closed.as_deref(), Some("logged off"));
    let skipped = view.session.skipped();
    assert!(skipped.is_empty(), "PDUs skipped as unhandled: {skipped:?}");
    timeout(WAIT, view.session.close()).await.expect("close hung");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_second_connection_takes_the_session_over() {
    let Some(address) = vm() else { return };
    let mut first = open_windows(&address, US).await;
    let second = open_windows(&address, US).await;
    first.within(WAIT, "the end", |view| view.closed.is_some()).await;
    assert_eq!(first.closed.as_deref(), Some("another user connected to this session"));
    let skipped = first.session.skipped();
    assert!(skipped.is_empty(), "PDUs skipped as unhandled: {skipped:?}");
    timeout(WAIT, first.session.close()).await.expect("close hung");
    second.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_turned_off_ends_the_session() {
    let Some(address) = vm() else { return };
    let mut view = open_windows(&address, US).await;
    let started = Started;
    // Files written just before, such as the colour.txt of the picture test's Undo, could
    // otherwise be lost or torn by the turn-off.
    in_guest("Write-VolumeCache C");
    let stopping = Instant::now();
    on_host(&format!("Stop-VM -Name {VM} -TurnOff -Force"));
    let ended = |view: &View| view.closed.is_some();
    view.within(Duration::from_secs(60), "the end of the session", ended).await;
    let took = stopping.elapsed();
    assert_eq!(view.closed.as_deref(), Some("network connection lost"));
    println!("closed {took:?} after the turn-off began");
    let skipped = view.session.skipped();
    assert!(skipped.is_empty(), "PDUs skipped as unhandled: {skipped:?}");
    timeout(WAIT, view.session.close()).await.expect("close hung");
    let restarting = Instant::now();
    drop(started);
    println!("back after {:?}", restarting.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_close_ends_within_a_second() {
    let Some(address) = vm() else { return };
    let view = open_windows(&address, US).await;
    let took = view.close().await;
    println!("close took {took:?}");
    assert!(took < Duration::from_millis(1100), "close took {took:?}");
}

// What the tab does when it is closed while connecting: the close comes 20-60 ms after the open,
// which on this VM is at the end of a reconnect to the disconnected session. The next tab, 1.5 s
// later, opens twice as the dev build does, the first one dropped at once. Ended at once, between 2
// and 10 in 20 such closes left the session blocked ("blocked by Remote Desktop Configuration") for
// a minute or more, and the next connection showed Windows' sign-in prompt instead of the session.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_a_login_closed_as_it_finishes_leaves_the_disconnected_session_free() {
    let Some(address) = vm() else { return };
    let _logged_off = Undo(
        "Get-Process explorer -ErrorAction SilentlyContinue | \
         ForEach-Object { logoff $_.SessionId }"
            .into(),
    );
    let (_dir, known) = known_at(&address, WINDOWS_PORT, Some(&windows_fingerprint()));
    let target = windows(&address, "wisp", None, WINDOWS_PASSWORD, US);
    open_windows(&address, US).await.close().await;
    for round in 0..20u64 {
        settles("a disconnected session", true, || in_guest("quser 2>&1").contains("Disc")).await;
        sleep(Duration::from_secs(1)).await;
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        let closing = async {
            sleep(Duration::from_millis(20 + round * 7 % 41)).await;
            rdps.close("closed").await;
        };
        // A login done before the close is open, and the close ends it.
        let (opened, ()) = tokio::join!(rdps.open("closed", &sent, connecting), closing);
        println!("round {round}: {:?}", opened.map(|o| o.id));
        sleep(Duration::from_millis(1500)).await;
        let dropped = Session::connect(&target, &known, |_| {}, &sent);
        assert!(timeout(Duration::from_millis(1), dropped).await.is_err());
        let (session, ops) = connect_to(&target, &known).await;
        let mut view = View::new(session.unwrap_or_else(|e| panic!("round {round}: {e}")), ops);
        let end = tokio::time::Instant::now() + Duration::from_secs(15);
        while !view.probe_shown() {
            match timeout_at(end, view.ops.recv()).await {
                Ok(Some(op)) => view.apply(op),
                _ => {
                    let sessions = in_guest("qwinsta 2>&1 | Out-String");
                    panic!("round {round}: the probe never showed; {sessions}")
                }
            }
        }
        view.close().await;
    }
}

// What the app does at its exit, with the tab opened at times from the middle of the reconnect to
// the disconnected session to well after it.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn windows_closing_every_session_at_the_exit_leaves_the_disconnected_session_free() {
    let Some(address) = vm() else { return };
    let _logged_off = Undo(
        "Get-Process explorer -ErrorAction SilentlyContinue | \
         ForEach-Object { logoff $_.SessionId }"
            .into(),
    );
    let (_dir, known) = known_at(&address, WINDOWS_PORT, Some(&windows_fingerprint()));
    let target = windows(&address, "wisp", None, WINDOWS_PASSWORD, US);
    open_windows(&address, US).await.close().await;
    for (round, delay) in [20, 30, 40, 50, 60, 100, 300, 600, 1500, 3000].into_iter().enumerate() {
        settles("a disconnected session", true, || in_guest("quser 2>&1").contains("Disc")).await;
        sleep(Duration::from_secs(1)).await;
        let rdps = RdpSessions::default();
        let sent = AtomicBool::default();
        let connecting = Session::connect(&target, &known, |_| {}, &sent);
        let exiting = async {
            sleep(Duration::from_millis(delay)).await;
            let started = Instant::now();
            rdps.close_all().await;
            started.elapsed()
        };
        let (opened, took) = tokio::join!(rdps.open("tab", &sent, connecting), exiting);
        println!("round {round}, exit at {delay} ms: {:?}, took {took:?}", opened.map(|o| o.id));
        assert!(took < Duration::from_millis(2100), "round {round}: the exit took {took:?}");
        sleep(Duration::from_millis(1500)).await;
        let (session, ops) = connect_to(&target, &known).await;
        let mut view = View::new(session.unwrap_or_else(|e| panic!("round {round}: {e}")), ops);
        let end = tokio::time::Instant::now() + Duration::from_secs(15);
        while !view.probe_shown() {
            match timeout_at(end, view.ops.recv()).await {
                Ok(Some(op)) => view.apply(op),
                _ => {
                    let sessions = in_guest("qwinsta 2>&1 | Out-String");
                    panic!("round {round}: the probe never showed; {sessions}")
                }
            }
        }
        view.close().await;
    }
}
