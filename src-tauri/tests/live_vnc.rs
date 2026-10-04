// Drives the real session against the servers of test-env/, which have to be up:
//   cargo test --manifest-path src-tauri/Cargo.toml --test live_vnc -- --ignored --test-threads=1

use std::collections::HashSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, timeout_at};
use wisp_lib::{AppError, FrameOp, KnownHosts, KnownHostsState, Login, Session, ENCODINGS};

const HOST: &str = "127.0.0.1";
const PASSWORD: &str = "wisptest";
const WAIT: Duration = Duration::from_secs(20);

const ZRLE: &[i32] = &[16, 1, 0];
const HEXTILE: &[i32] = &[5, 1, 0];
const RAW: &[i32] = &[0];

const SHIFT: u32 = 0xFFE1;
const CONTROL: u32 = 0xFFE3;
const RETURN: u32 = 0xFF0D;

#[derive(Clone, Copy)]
struct Server {
    service: &'static str,
    port: u16,
    username: &'static str,
    x509: bool,
    size: (usize, usize),
    name: &'static str,
    colours: usize,
    refusal: &'static str,
}

const VNCAUTH: Server = Server {
    service: "tigervnc-vncauth",
    port: 5902,
    username: "",
    x509: false,
    size: (1280, 800),
    name: "wisp-test-tigervnc-vncauth",
    colours: 9,
    refusal: "Authentication failed",
};
const X509VNC: Server = Server {
    service: "tigervnc-x509",
    port: 5901,
    x509: true,
    name: "wisp-test-tigervnc-x509",
    ..VNCAUTH
};
const X509PLAIN: Server = Server {
    service: "tigervnc-plain",
    port: 5903,
    username: "wisp",
    x509: true,
    name: "wisp-test-tigervnc-plain",
    ..VNCAUTH
};
const DEFAULT: Server = Server {
    service: "tigervnc-default",
    port: 5907,
    name: "wisp-test-tigervnc-default",
    ..VNCAUTH
};
const X11VNC: Server = Server {
    service: "x11vnc",
    port: 5904,
    name: "wisp-test-x11vnc",
    refusal: "password check failed!",
    ..VNCAUTH
};
const TIGHTVNC: Server = Server {
    service: "tightvnc",
    port: 5905,
    name: "root's wisp-test-tightvnc desktop (wisp-test-tightvnc:1)",
    ..VNCAUTH
};
const QEMU: Server = Server {
    service: "qemu",
    port: 5906,
    size: (720, 400),
    name: "QEMU (wisp-test-qemu)",
    colours: 2,
    refusal: "Authentication failed\0",
    ..VNCAUTH
};
const ALL: [Server; 7] = [X509VNC, VNCAUTH, X509PLAIN, X11VNC, TIGHTVNC, QEMU, DEFAULT];

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

// Asks until the answer is the wanted one: what a server does with input shows a moment later.
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

// Presses the keys in turn and lets them go in reverse.
async fn chord(session: &Session, keysyms: &[u32]) {
    for keysym in keysyms {
        session.key(true, *keysym).await.unwrap();
    }
    for keysym in keysyms.iter().rev() {
        session.key(false, *keysym).await.unwrap();
    }
}

// Puts back what a test changed inside a container, however the test ends.
struct Restore {
    server: Server,
    script: String,
}

impl Drop for Restore {
    fn drop(&mut self) {
        let ran = docker(&["exec", &self.server.container(), "sh", "-c", &self.script]);
        // A panic on top of the one that is failing the test would end the whole run.
        if !std::thread::panicking() {
            assert!(ran.ok, "{}: `{}` failed", self.server.service, self.script);
        }
    }
}

type Ops = mpsc::UnboundedReceiver<FrameOp>;

impl Server {
    fn container(&self) -> String {
        format!("wisp-test-{}", self.service)
    }

    // A shell command in the server's container, which has to succeed.
    fn inside(&self, script: &str) -> Vec<u8> {
        let ran = docker(&["exec", &self.container(), "sh", "-c", script]);
        assert!(ran.ok, "{}: `{script}` failed: {}", self.service, text(&ran.err));
        ran.out
    }

    // Runs a program in the container's background. The guard ends it and removes its files.
    fn start(&self, name: &str, command: &str) -> Restore {
        let script = format!("echo $$ > /tmp/{name}.pid; exec {command}");
        let ran = docker(&["exec", "-d", &self.container(), "sh", "-c", &script]);
        assert!(ran.ok, "{}: `{command}` did not start: {}", self.service, text(&ran.err));
        let files = format!("/tmp/{name}.pid /tmp/{name}.out");
        let script = format!("kill $(cat /tmp/{name}.pid) 2>/dev/null; rm -f {files}");
        Restore { server: *self, script }
    }

    fn shows_window(&self, name: &str) -> bool {
        let script = format!("xwininfo -name '{name}' 2>/dev/null | grep -c IsViewable; true");
        text(&self.inside(&script)) == "1"
    }

    // What the server has logged since the container was started.
    fn log(&self) -> String {
        let container = self.container();
        let started = docker(&["inspect", "-f", "{{.State.StartedAt}}", &container]);
        let logged = docker(&["logs", "--since", &text(&started.out), &container]);
        assert!(logged.ok, "{}: no log: {}", self.service, text(&logged.err));
        text(&[logged.out, logged.err].concat())
    }

    // As the session reports a certificate: SHA256 and the digest in lower-case hex.
    fn fingerprint(&self) -> String {
        let openssl = "openssl x509 -in /etc/wisp/cert.pem -noout -fingerprint -sha256";
        let printed = text(&self.inside(openssl));
        let digest = printed.strip_prefix("sha256 Fingerprint=").expect("openssl's output");
        format!("SHA256:{}", digest.replace(':', "").to_lowercase())
    }

    // Where the pointer is, as the X server inside sees it.
    fn pointer(&self) -> (u16, u16) {
        let tool = if self.service == "tightvnc" { "pointer" } else { "xdotool getmouselocation" };
        let printed = text(&self.inside(tool));
        let mut fields = printed.split([':', ' ']);
        let x = fields.nth(1).and_then(|x| x.parse().ok());
        let y = fields.nth(1).and_then(|y| y.parse().ok());
        x.zip(y).unwrap_or_else(|| panic!("{}: pointer position {printed:?}", self.service))
    }

    // The screen as the X server inside has it, three bytes a pixel.
    fn screenshot(&self) -> Ran {
        let script = "xwd -root -silent | convert xwd:- rgb:-";
        docker(&["exec", &self.container(), "sh", "-c", script])
    }

    // A known-hosts file of the test's own, with the given certificate on record for the server.
    fn known(&self, pin: Option<&str>) -> (tempfile::TempDir, KnownHostsState) {
        let dir = tempfile::tempdir().unwrap();
        let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        if let Some(pin) = pin {
            hosts.record(&format!("vnc/{HOST}"), self.port, pin).unwrap();
        }
        (dir, KnownHostsState(Arc::new(Mutex::new(hosts))))
    }

    // Known hosts that trust the certificate the container holds.
    fn trusted(&self) -> (tempfile::TempDir, KnownHostsState) {
        let pin = self.x509.then(|| self.fingerprint());
        self.known(pin.as_deref())
    }

    async fn connect(
        &self,
        password: &str,
        known: &KnownHostsState,
        encodings: &[i32],
    ) -> (Result<Session, AppError>, Ops) {
        let (sink, ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let login = Login { username: self.username, password };
        let connecting = Session::connect(HOST, self.port, &login, known, encodings, sink);
        (timeout(WAIT * 2, connecting).await.expect("connect hung"), ops)
    }

    // The error of a login that has to fail, after which nothing may have reached the view.
    async fn refusal(&self, password: &str, known: &KnownHostsState) -> AppError {
        let (session, mut ops) = self.connect(password, known, &ENCODINGS).await;
        let error = session.err().unwrap_or_else(|| panic!("{}: logged in", self.service));
        let told = timeout(WAIT, ops.recv()).await.expect("the sink outlived the refusal");
        assert!(told.is_none(), "{}: a refused login reached the view", self.service);
        error
    }

    async fn open(&self, encodings: &[i32]) -> View {
        let (_dir, known) = self.trusted();
        let (session, ops) = self.connect(PASSWORD, &known, encodings).await;
        let session = session.unwrap_or_else(|e| panic!("{}: {e}", self.service));
        let size = (session.width as usize, session.height as usize);
        View {
            service: self.service,
            session,
            ops,
            size,
            picture: vec![0; size.0 * size.1 * 4],
            drawn: vec![false; size.0 * size.1],
            resizes: Vec::new(),
            cursors: Vec::new(),
            clipboard: Vec::new(),
            closed: None,
        }
    }
}

struct Cursor {
    hot: (u16, u16),
    size: (u16, u16),
    rgba: Vec<u8>,
}

// A session and what a view has made of its operations.
struct View {
    service: &'static str,
    session: Session,
    ops: Ops,
    size: (usize, usize),
    picture: Vec<u8>,
    drawn: Vec<bool>,
    resizes: Vec<(u16, u16)>,
    cursors: Vec<Cursor>,
    clipboard: Vec<String>,
    closed: Option<String>,
}

impl View {
    fn apply(&mut self, op: FrameOp) {
        let stride = self.size.0;
        match op {
            FrameOp::Rect { x, y, w, h, rgba } => {
                let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
                assert_eq!(rgba.len(), w * h * 4, "{}: a rectangle's pixels", self.service);
                for (row, line) in rgba.chunks_exact(w * 4).enumerate() {
                    let at = (y + row) * stride + x;
                    self.picture[at * 4..(at + w) * 4].copy_from_slice(line);
                    self.drawn[at..at + w].fill(true);
                }
            }
            FrameOp::Copy { x, y, w, h, src_x, src_y } => {
                let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
                let (picture, drawn) = (self.picture.clone(), self.drawn.clone());
                for row in 0..h {
                    let from = (src_y as usize + row) * stride + src_x as usize;
                    let to = (y + row) * stride + x;
                    self.picture[to * 4..(to + w) * 4]
                        .copy_from_slice(&picture[from * 4..(from + w) * 4]);
                    self.drawn[to..to + w].copy_from_slice(&drawn[from..from + w]);
                }
            }
            FrameOp::Resize { w, h } => {
                self.size = (w as usize, h as usize);
                self.picture = vec![0; self.size.0 * self.size.1 * 4];
                self.drawn = vec![false; self.size.0 * self.size.1];
                self.resizes.push((w, h));
            }
            FrameOp::Cursor { hot_x, hot_y, w, h, rgba } => {
                assert_eq!(rgba.len(), w as usize * h as usize * 4, "{}: a cursor", self.service);
                self.cursors.push(Cursor { hot: (hot_x, hot_y), size: (w, h), rgba });
            }
            FrameOp::Clipboard(text) => self.clipboard.push(text),
            FrameOp::Closed(reason) => self.closed = Some(reason),
            FrameOp::Sync => self.session.ack(),
        }
    }

    // Applies what the session hands over until `done` holds.
    async fn within(&mut self, limit: Duration, what: &str, done: impl Fn(&View) -> bool) {
        let end = tokio::time::Instant::now() + limit;
        while !done(self) {
            let op = timeout_at(end, self.ops.recv()).await;
            let op = op.unwrap_or_else(|_| panic!("{}: no {what} in {limit:?}", self.service));
            self.apply(op.unwrap_or_else(|| panic!("{}: over before {what}", self.service)));
        }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&View) -> bool) {
        self.within(WAIT, what, done).await
    }

    // Applies what arrives in the given time.
    async fn idle(&mut self, time: Duration) {
        let limit = tokio::time::Instant::now() + time;
        while let Ok(Some(op)) = timeout_at(limit, self.ops.recv()).await {
            self.apply(op);
        }
    }

    fn whole(&self) -> bool {
        self.drawn.iter().all(|drawn| *drawn)
    }

    // Waits for the picture to be the screen as the X server inside has it, and returns how many
    // pixels are not. Those can only be a pointer the server has drawn into the picture, which a
    // screenshot never has, and only where `pointer` allows for one.
    async fn shows_screen(&mut self, server: &Server, pointer: bool) -> usize {
        let started = Instant::now();
        loop {
            let (px, py) = server.pointer();
            let shot = server.screenshot().out;
            let pixels = self.picture.chunks_exact(4).zip(shot.chunks_exact(3)).enumerate();
            let differing = pixels.filter(|(_, (ours, theirs))| ours[..3] != **theirs);
            let places: Vec<_> =
                differing.map(|(at, _)| (at % self.size.0, at / self.size.0)).collect();
            let by_pointer = |(x, y): &(usize, usize)| {
                pointer && x.abs_diff(px as usize) <= 16 && y.abs_diff(py as usize) <= 16
            };
            if shot.len() == self.size.0 * self.size.1 * 3 && places.iter().all(by_pointer) {
                return places.len();
            }
            let off = places.len();
            assert!(started.elapsed() < WAIT, "{}: {off} pixels are not the screen", self.service);
            self.idle(Duration::from_millis(200)).await;
        }
    }

    // Ends the session the way closing the tab does, and returns how long that took.
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
        assert!(self.closed.is_none(), "{}: close told the view {:?}", self.service, self.closed);
        took
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn every_server_logs_in_and_sends_its_whole_screen() {
    for server in ALL {
        let started = Instant::now();
        let mut view = server.open(&ENCODINGS).await;
        assert_eq!(view.size, server.size, "{}", server.service);
        assert_eq!(view.session.name, server.name);
        view.until("whole screen", View::whole).await;
        let took = started.elapsed();
        let colours = view.picture.chunks_exact(4).collect::<HashSet<_>>().len();
        assert_eq!(colours, server.colours, "{}: distinct colours", server.service);
        println!("{}: {:?}, {colours} colours after {took:?}", server.service, view.size);
        view.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_wrong_password_is_refused_with_the_servers_reason() {
    for server in ALL {
        let (_dir, known) = server.trusted();
        let error = server.refusal("wisptext", &known).await;
        assert_eq!(error.to_string(), format!("internal error: vnc: {}", server.refusal));
        // TightVNC locks everybody out after five in a row; a login that works starts the count
        // again.
        server.open(&ENCODINGS).await.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_login_without_a_password_is_refused() {
    for server in ALL {
        let (_dir, known) = server.known(None);
        let error = server.refusal("", &known).await;
        let wanted = "internal error: vnc: this server needs a password";
        assert_eq!(error.to_string(), wanted, "{}", server.service);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_certificate_is_unknown_until_trusted_and_a_mismatch_once_it_changes() {
    for server in [X509VNC, X509PLAIN] {
        let shown = server.fingerprint();
        let (_dir, known) = server.known(None);
        match server.refusal(PASSWORD, &known).await {
            AppError::HostKeyUnknown { host, port, fingerprint } => {
                assert_eq!((host.as_str(), port), ("vnc/127.0.0.1", server.port));
                assert_eq!(fingerprint, shown, "{}", server.service);
                // What accepting the certificate in the dialog does.
                known.0.lock().unwrap().record(&host, port, &fingerprint).unwrap();
            }
            other => panic!("{}: expected HostKeyUnknown, got {other:?}", server.service),
        }
        let (session, _ops) = server.connect(PASSWORD, &known, &ENCODINGS).await;
        let session = session.unwrap_or_else(|e| panic!("{}: {e}", server.service));
        timeout(WAIT, session.close()).await.expect("close hung");

        let other = format!("SHA256:{}", "0".repeat(64));
        let (_dir, known) = server.known(Some(&other));
        match server.refusal(PASSWORD, &known).await {
            AppError::HostKeyMismatch { host, port, stored, offered } => {
                assert_eq!((host.as_str(), port), ("vnc/127.0.0.1", server.port));
                assert_eq!((stored, offered), (other, shown), "{}", server.service);
            }
            other => panic!("{}: expected HostKeyMismatch, got {other:?}", server.service),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn every_encoding_draws_the_picture_the_server_has() {
    let all = [ZRLE, HEXTILE, RAW];
    for (server, offers) in
        [(VNCAUTH, &all[..]), (X11VNC, &all[..]), (TIGHTVNC, &all[1..]), (QEMU, &all[..])]
    {
        let mut first = None;
        for encodings in offers {
            let mut view = server.open(encodings).await;
            view.until("whole screen", View::whole).await;
            match &first {
                None => {
                    // Nothing can look at QEMU's screen from inside.
                    if server.service != "qemu" {
                        let pointer = view.shows_screen(&server, true).await;
                        println!("{}: {pointer} pixels of pointer in the picture", server.service);
                    }
                    first = Some(view.picture.clone());
                }
                // QEMU's text cursor blinks, so there the pictures are the same every other moment.
                Some(first) => {
                    let what = format!("picture of {:?} with {encodings:?}", offers[0]);
                    view.until(&what, |view| &view.picture == first).await;
                }
            }
            view.close().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn keys_arrive_in_an_xterm_as_they_were_typed() {
    for server in [X11VNC, VNCAUTH, X509PLAIN] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        let xterm = "xterm -T typed -geometry 80x4+40+600 -fn 10x20";
        let _xterm = server.start("typed", &format!("{xterm} -e sh -c 'cat > /tmp/typed.out'"));
        settles("the xterm", true, || server.shows_window("typed")).await;
        // Keys go to the window under the pointer.
        view.session.pointer(0, 300, 660).await.unwrap();
        settles("the pointer", (300, 660), || server.pointer()).await;

        for keys in [&[0x61][..], &[SHIFT, 0x42], &[0x31], &[SHIFT, 0x21], &[0xE9], &[RETURN]] {
            chord(&view.session, keys).await;
        }
        let typed = || server.inside("cat /tmp/typed.out 2>/dev/null; true");
        settles("the typed line", b"aB1!\xe9\n".to_vec(), typed).await;

        // Ctrl+D ends the cat and with it the xterm.
        chord(&view.session, &[CONTROL, 0x64]).await;
        view.session.pointer(0, 640, 400).await.unwrap();
        view.close().await;
        settles("the xterm leaving", false, || server.shows_window("typed")).await;
        settles("the pointer", (640, 400), || server.pointer()).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_pointer_goes_where_it_is_sent() {
    for server in [X11VNC, VNCAUTH, TIGHTVNC] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        for (x, y) in [(1000, 700), (640, 400)] {
            view.session.pointer(0, x, y).await.unwrap();
            settles("the pointer", (x, y), || server.pointer()).await;
        }
        view.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_wheel_turns_as_buttons_four_and_five() {
    for server in [X11VNC, VNCAUTH] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        let _xev = server.start("xev", "xev -geometry 200x200+1000+100 > /tmp/xev.out 2>&1");
        settles("xev's window", true, || server.shows_window("Event Tester")).await;

        for buttons in [0, 8, 0, 16, 0] {
            view.session.pointer(buttons, 1100, 200).await.unwrap();
        }
        let seen = "grep -A2 '^Button' /tmp/xev.out | grep -o '^Button[A-Za-z]*\\|button [0-9]'";
        let wanted = "ButtonPress button 4 ButtonRelease button 4 \
                      ButtonPress button 5 ButtonRelease button 5";
        let buttons = || text(&server.inside(&format!("{seen}; true"))).replace('\n', " ");
        settles("the wheel", wanted.to_string(), buttons).await;

        view.session.pointer(0, 640, 400).await.unwrap();
        settles("the pointer", (640, 400), || server.pointer()).await;
        view.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_clipboard_goes_both_ways() {
    // On the wire text is Latin-1. TigerVNC's X side has it as UTF-8, x11vnc passes the bytes on.
    for (server, e_acute) in [(X509VNC, &[0xC3, 0xA9][..]), (X11VNC, &[0xE9][..])] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        let copy = |text: &str| {
            server.inside(&format!("printf '{text}' | xclip -selection clipboard >/dev/null 2>&1"))
        };
        // x11vnc's clipboard does nothing until it has logged this, 9 to 19 s into the first
        // connection after its start, and the first text takes another while to arrive.
        if server.service == "x11vnc" {
            let started = Instant::now();
            while !server.log().contains("created selwin") {
                assert!(started.elapsed() < WAIT * 3, "x11vnc: no selection window");
                view.idle(Duration::from_millis(500)).await;
            }
            copy("first");
            let first = |view: &View| view.clipboard.iter().any(|text| text == "first");
            view.within(WAIT * 3, "first clipboard text", first).await;
            println!("x11vnc: clipboard awake after {:?}", started.elapsed());
        }

        let run = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let octal: String = e_acute.iter().map(|byte| format!("\\{byte:03o}")).collect();
        copy(&format!("inside {run} {octal}"));
        let wanted = format!("inside {run} é");
        view.until("clipboard text from inside", |view| view.clipboard.contains(&wanted)).await;

        view.session.clipboard(&format!("outside {run} é")).await.unwrap();
        let wanted = [format!("outside {run} ").as_bytes(), e_acute].concat();
        let paste = ["exec", &server.container(), "xclip", "-o", "-selection", "clipboard"];
        settles("the clipboard inside", wanted, || docker(&paste).out).await;
        // The server owns the selection now, which ends the xclip that held the text from inside.
        let programs = || text(&server.inside("cat /proc/[0-9]*/comm 2>/dev/null; true"));
        settles("xclip leaving", false, || programs().lines().any(|name| name == "xclip")).await;
        view.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_resized_screen_is_drawn_at_its_new_size() {
    let server = VNCAUTH;
    let mut view = server.open(&ENCODINGS).await;
    view.until("whole screen", View::whole).await;
    let _back = Restore { server, script: "xrandr -s 1280x800".into() };

    let sizes = [(1024, 768), (1280, 800)];
    for (step, (w, h)) in sizes.into_iter().enumerate() {
        server.inside(&format!("xrandr -s {w}x{h}"));
        let resized = |view: &View| view.size == (w as usize, h as usize) && view.whole();
        view.until("whole screen at the new size", resized).await;
        assert_eq!(view.resizes, sizes[..=step]);
        let pointer = view.shows_screen(&server, true).await;
        println!("{w}x{h}: {pointer} pixels of pointer in the picture");
    }
    view.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn the_cursor_shape_arrives_apart_from_the_picture() {
    for server in [X11VNC, VNCAUTH] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        // The X of the root window, then the I-beam of the xterm: where, size, hotspot and how
        // many of its pixels are opaque.
        let shapes = [((1000, 700), (16, 16), (7, 7), 176), ((640, 400), (9, 16), (4, 8), 86)];
        for ((x, y), size, hot, opaque) in shapes {
            view.session.pointer(0, x, y).await.unwrap();
            let shaped = |view: &View| view.cursors.last().is_some_and(|last| last.size == size);
            view.until("cursor of the window under the pointer", shaped).await;
            let cursor = view.cursors.last().unwrap();
            assert_eq!(cursor.hot, hot, "{}", server.service);
            let alpha = |value| cursor.rgba.chunks_exact(4).filter(|px| px[3] == value).count();
            let clear = cursor.rgba.len() / 4 - opaque;
            assert_eq!((alpha(255), alpha(0)), (opaque, clear), "{}", server.service);
            assert_eq!(view.shows_screen(&server, false).await, 0);
        }
        view.close().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_stopped_server_ends_the_session() {
    for server in [TIGHTVNC, X509VNC] {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        view.session.pointer(0, 640, 400).await.unwrap();
        settles("the pointer", (640, 400), || server.pointer()).await;
        let before = server.screenshot();
        assert!(before.ok, "{}: no screenshot", server.service);

        let stopping = Instant::now();
        assert!(docker(&["stop", &server.container()]).ok, "{}: not stopped", server.service);
        let ending = async {
            while view.closed.is_none() {
                match view.ops.recv().await {
                    Some(op) => view.apply(op),
                    None => break,
                }
            }
        };
        let ended = timeout(Duration::from_secs(60), ending).await;
        let took = stopping.elapsed();

        assert!(docker(&["start", &server.container()]).ok, "{}: not started", server.service);
        // Docker takes connections before the server does, and closes them without a banner.
        let (starting, mut banner) = (Instant::now(), [0u8; 12]);
        loop {
            let greeting = async {
                let mut stream = TcpStream::connect((HOST, server.port)).await?;
                stream.read_exact(&mut banner).await
            };
            if matches!(timeout(Duration::from_secs(2), greeting).await, Ok(Ok(_))) {
                break;
            }
            assert!(starting.elapsed() < WAIT * 3, "{}: no banner", server.service);
            sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(&banner, b"RFB 003.008\n", "{}", server.service);
        let as_before = || {
            let shot = server.screenshot();
            shot.ok && shot.out == before.out
        };
        settles("the screen as it was", true, as_before).await;

        assert!(ended.is_ok(), "{}: the session outlived its server", server.service);
        assert_eq!(view.closed.as_deref(), Some("network connection lost"), "{}", server.service);
        println!("{}: closed {took:?} after the stop began", server.service);
        timeout(WAIT, view.session.close()).await.expect("close hung");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn close_returns_within_about_a_second_and_the_server_sees_the_client_leave() {
    // What each server logs when a client leaves; QEMU logs nothing of its clients.
    let leaving = [
        (VNCAUTH, Some("Clean disconnection")),
        (X509VNC, Some("Clean disconnection")),
        (X11VNC, Some(" gone")),
        (TIGHTVNC, Some(" gone")),
        (QEMU, None),
    ];
    for (server, line) in leaving {
        let mut view = server.open(&ENCODINGS).await;
        view.until("whole screen", View::whole).await;
        let left = || line.map(|line| server.log().matches(line).count());
        let before = left();
        let took = view.close().await;
        println!("{}: close took {took:?}", server.service);
        assert!(took < Duration::from_millis(1500), "{}: close took {took:?}", server.service);
        let after = before.map(|count| count + 1);
        settles("the server's word of the client leaving", after, left).await;
    }
}
