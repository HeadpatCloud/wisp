// Drives the app's SSH, SFTP and tunnel code against the OpenSSH server of test-env/, which
// has to be up:
//   cargo test --manifest-path src-tauri/Cargo.toml --test live_ssh -- --ignored \
//       --test-threads=1 --nocapture

use std::future::Future;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tauri::test::MockRuntime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use wisp_lib::live_ssh::{
    auth_key, auth_password, connect, connect_over, download, list, mkdir, new_forwards, open_pty,
    open_sftp, remove, rename, run_dynamic, run_local, run_remote, run_session, stat, upload, Sftp,
    SshHandle, TunnelStatus,
};
use wisp_lib::{AppError, KnownHosts};

const CONTAINER: &str = "wisp-test-openssh";
const HOST: &str = "127.0.0.1";
const PORT: u16 = 2201;
const REKEY_PORT: u16 = 2202;
const USER: &str = "wisp";
const PASSWORD: &str = "wisptest";
const REMOTE: &str = "/home/wisp/live";
const MIB: usize = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(20);
const TRANSFER: Duration = Duration::from_secs(900);

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
        if started.elapsed() > Duration::from_secs(120) {
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

// As the user the tests log in as.
fn inside(script: &str) -> String {
    let ran = docker(&["exec", "-u", USER, CONTAINER, "sh", "-c", script]);
    assert!(ran.ok, "`{script}` failed: {}", text(&ran.err));
    text(&ran.out)
}

// As the app reports it: SHA256 and the digest in base64. Both sshd present the same keys.
fn host_key() -> String {
    let printed = inside("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub");
    printed.split(' ').nth(1).expect("ssh-keygen's output").to_string()
}

fn sha256_inside(path: &str) -> String {
    let printed = inside(&format!("sha256sum {path}"));
    printed.split(' ').next().expect("sha256sum's output").to_string()
}

fn sha256_file(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let (mut hash, mut buf) = (Sha256::new(), vec![0u8; MIB]);
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

// xorshift64*: the same bytes for the same seed on every run.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        bytes.extend_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn rate(bytes: u64, took: Duration) -> f64 {
    bytes as f64 / MIB as f64 / took.as_secs_f64()
}

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

fn median_and_max(mut times: Vec<Duration>) -> (f64, f64) {
    times.sort();
    let middle = times.len() / 2;
    (ms(times[middle - 1] + times[middle]) / 2.0, ms(times[times.len() - 1]))
}

async fn within<T>(limit: Duration, what: &str, work: impl Future<Output = T>) -> T {
    timeout(limit, work).await.unwrap_or_else(|_| panic!("{what}: not done in {limit:?}"))
}

fn known(pins: &[(u16, &str)]) -> (tempfile::TempDir, Arc<Mutex<KnownHosts>>) {
    let dir = tempfile::tempdir().unwrap();
    let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
    for (port, fingerprint) in pins {
        hosts.record(HOST, *port, fingerprint).unwrap();
    }
    (dir, Arc::new(Mutex::new(hosts)))
}

async fn connected(port: u16) -> SshHandle {
    let (_dir, hosts) = known(&[(port, &host_key())]);
    within(WAIT, "connect", connect(HOST, port, hosts, new_forwards())).await.unwrap()
}

async fn logged_in(port: u16) -> SshHandle {
    let mut handle = connected(port).await;
    within(WAIT, "login", auth_password(&mut handle, USER, PASSWORD)).await.unwrap();
    handle
}

// The tunnels report to the app's window; this one has none.
fn app() -> tauri::App<MockRuntime> {
    let app = tauri::test::mock_app();
    tauri_specta::Builder::<MockRuntime>::new()
        .events(tauri_specta::collect_events![TunnelStatus])
        .mount_events(&app);
    app
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((HOST, 0)).unwrap().local_addr().unwrap().port()
}

// A tunnel binds its port in a task of its own, a moment after it was started.
async fn dial(port: u16) -> TcpStream {
    let started = Instant::now();
    loop {
        match TcpStream::connect((HOST, port)).await {
            Ok(stream) => return stream,
            Err(e) => assert!(started.elapsed() < WAIT, "nothing listens on {port}: {e}"),
        }
        sleep(Duration::from_millis(20)).await;
    }
}

// Removes what a test left in the user's home however the test ends.
struct Scratch;

fn scratch() -> Scratch {
    inside(&format!("rm -rf {REMOTE}"));
    Scratch
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let ran = docker(&["exec", "-u", USER, CONTAINER, "rm", "-rf", REMOTE]);
        // A panic on top of the one that is failing the test would end the whole run.
        if !std::thread::panicking() {
            assert!(ran.ok, "{REMOTE} not removed: {}", text(&ran.err));
        }
    }
}

struct Shell {
    input: mpsc::Sender<Vec<u8>>,
    resize: mpsc::Sender<(u32, u32)>,
    output: mpsc::UnboundedReceiver<Vec<u8>>,
    seen: Vec<u8>,
    ended: tokio::task::JoinHandle<Option<u32>>,
}

impl Shell {
    async fn open(handle: &SshHandle) -> Shell {
        let channel = within(WAIT, "shell", open_pty(handle, 80, 24)).await.unwrap();
        let (input, input_rx) = mpsc::channel(64);
        let (resize, resize_rx) = mpsc::channel(8);
        let (sink, output) = mpsc::unbounded_channel();
        // A test that is done with its shell drops the receiver, and the shell may print after.
        let ended = tokio::spawn(run_session(channel, input_rx, resize_rx, move |bytes| {
            sink.send(bytes).ok();
        }));
        let mut shell = Shell { input, resize, output, seen: Vec::new(), ended };
        // Past the login banner and the first prompt, so that later timings are the shell's.
        shell.run("echo ready-$((1+1))", "ready-2").await;
        shell
    }

    // The terminal echoes what is typed, so `wanted` has to be text only the command's output
    // has: the commands here compute it.
    async fn answers(&mut self, command: &str, wanted: &str, limit: Duration) -> Option<Duration> {
        let (from, started) = (self.seen.len(), Instant::now());
        self.input.send(format!("{command}\n").into_bytes()).await.unwrap();
        let wanted = wanted.as_bytes();
        let reading = async {
            while !self.seen[from..].windows(wanted.len()).any(|window| window == wanted) {
                let bytes = self.output.recv().await.expect("the shell ended");
                self.seen.extend(bytes);
            }
        };
        timeout(limit, reading).await.ok().map(|_| started.elapsed())
    }

    async fn run(&mut self, command: &str, wanted: &str) -> Duration {
        let answered = self.answers(command, wanted, WAIT).await;
        answered.unwrap_or_else(|| {
            let seen = String::from_utf8_lossy(&self.seen);
            panic!("`{command}` did not print {wanted:?} in {WAIT:?}; the terminal has {seen:?}")
        })
    }

    async fn echoes(&mut self, count: usize) -> (f64, f64) {
        let mut times = Vec::new();
        for i in 0..count {
            times.push(self.run(&format!("echo m{i}-$((40+2))"), &format!("m{i}-42")).await);
        }
        median_and_max(times)
    }
}

async fn up(sftp: &Sftp, local: &Path, remote: &str) -> Duration {
    let size = std::fs::metadata(local).unwrap().len();
    let (mut last, started) = ((0, 0), Instant::now());
    let sending = upload(sftp, local.to_str().unwrap(), remote, |done, total| last = (done, total));
    within(TRANSFER, "upload", sending).await.unwrap();
    let took = started.elapsed();
    assert_eq!(last, (size, size), "the last progress report of the upload");
    took
}

async fn down(sftp: &Sftp, remote: &str, local: &Path, size: u64) -> Duration {
    let (mut last, started) = ((0, 0), Instant::now());
    let local = local.to_str().unwrap();
    let fetching = download(sftp, remote, local, |done, total| last = (done, total));
    within(TRANSFER, "download", fetching).await.unwrap();
    let took = started.elapsed();
    assert_eq!(last, (size, size), "the last progress report of the download");
    took
}

// 64 MiB up, hashed inside, and down again: the rates, and the key exchanges the server
// logged during each transfer.
async fn round_trip(port: u16, seed: u64) -> (f64, f64, usize, usize) {
    let _scratch = scratch();
    let dir = tempfile::tempdir().unwrap();
    let (sent, back) = (dir.path().join("sent.bin"), dir.path().join("back.bin"));
    std::fs::write(&sent, noise(seed, 64 * MIB)).unwrap();
    let (hash, size) = (sha256_file(&sent), 64 * MIB as u64);
    let remote = format!("{REMOTE}/round.bin");

    let handle = logged_in(port).await;
    let sftp = within(WAIT, "sftp", open_sftp(&handle)).await.unwrap();
    within(WAIT, "mkdir", mkdir(&sftp, REMOTE)).await.unwrap();

    // What the sshd on 2202 logs for each new set of keys it starts to send with.
    let exchanges = || {
        let logged = docker(&["logs", CONTAINER]);
        assert!(logged.ok, "no log: {}", text(&logged.err));
        text(&[logged.out, logged.err].concat()).matches("rekeying out").count()
    };
    let before = exchanges();
    let up_rate = rate(size, up(&sftp, &sent, &remote).await);
    assert_eq!(sha256_inside(&remote), hash, "the uploaded file inside");
    let between = exchanges();
    let down_rate = rate(size, down(&sftp, &remote, &back, size).await);
    assert_eq!(sha256_file(&back), hash, "the downloaded file");
    let after = exchanges();
    (up_rate, down_rate, between - before, after - between)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn host_key_unknown_then_trusted_then_mismatch() {
    let shown = host_key();
    let (_dir, hosts) = known(&[]);
    let unknown = within(WAIT, "connect", connect(HOST, PORT, hosts.clone(), new_forwards())).await;
    match unknown.err().expect("connected to a server whose key nobody trusted") {
        AppError::HostKeyUnknown { host, port, fingerprint } => {
            assert_eq!((host.as_str(), port), (HOST, PORT));
            assert_eq!(fingerprint, shown);
            // What accepting the key in the dialog does.
            hosts.lock().unwrap().record(&host, port, &fingerprint).unwrap();
        }
        other => panic!("expected HostKeyUnknown, got {other:?}"),
    }
    let trusted = within(WAIT, "connect", connect(HOST, PORT, hosts, new_forwards())).await;
    let mut handle = trusted.unwrap_or_else(|e| panic!("the trusted key was refused: {e}"));
    within(WAIT, "login", auth_password(&mut handle, USER, PASSWORD)).await.unwrap();

    let other = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let (_dir, hosts) = known(&[(PORT, other)]);
    let changed = within(WAIT, "connect", connect(HOST, PORT, hosts, new_forwards())).await;
    match changed.err().expect("connected to a server with another key than the trusted one") {
        AppError::HostKeyMismatch { host, port, stored, offered } => {
            assert_eq!((host.as_str(), port), (HOST, PORT));
            assert_eq!((stored.as_str(), offered), (other, shown));
        }
        other => panic!("expected HostKeyMismatch, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn password_login() {
    let (_dir, hosts) = known(&[(PORT, &host_key())]);
    let started = Instant::now();
    let connecting = connect(HOST, PORT, hosts, new_forwards());
    let mut handle = within(WAIT, "connect", connecting).await.unwrap();
    let connect_ms = ms(started.elapsed());

    let started = Instant::now();
    let refused = within(WAIT, "login", auth_password(&mut handle, USER, "wisptext")).await;
    let refuse_ms = ms(started.elapsed());
    let refused = refused.expect_err("logged in with a wrong password");
    assert_eq!(refused.to_string(), "authentication failed: password rejected");

    // The same connection takes the right one after that.
    let started = Instant::now();
    within(WAIT, "login", auth_password(&mut handle, USER, PASSWORD)).await.unwrap();
    let login_ms = ms(started.elapsed());
    Shell::open(&handle).await;
    println!(
        "MEASURE password_login connect_ms={connect_ms:.1} refuse_ms={refuse_ms:.1} \
         login_ms={login_ms:.1}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn key_login() {
    let dir = tempfile::tempdir().unwrap();
    let key = |name: &str| {
        let ran = docker(&["exec", CONTAINER, "cat", &format!("/etc/wisp/keys/{name}")]);
        assert!(ran.ok, "no key {name}: {}", text(&ran.err));
        let path = dir.path().join(name);
        std::fs::write(&path, ran.out).unwrap();
        path.to_str().unwrap().to_string()
    };

    let mut times = Vec::new();
    for name in ["id_ed25519", "id_rsa"] {
        let (path, mut handle) = (key(name), connected(PORT).await);
        let started = Instant::now();
        let login = within(WAIT, "login", auth_key(&mut handle, USER, &path, None)).await;
        times.push(ms(started.elapsed()));
        login.unwrap_or_else(|e| panic!("{name}: {e}"));
        Shell::open(&handle).await;
    }

    let (path, mut handle) = (key("id_ed25519_enc"), connected(PORT).await);
    let locked = within(WAIT, "login", auth_key(&mut handle, USER, &path, None)).await;
    let locked = locked.expect_err("logged in with a locked key");
    assert!(matches!(locked, AppError::PassphraseRequired), "{locked:?}");
    assert_eq!(locked.to_string(), "key is encrypted - passphrase required");
    let wrong = within(WAIT, "login", auth_key(&mut handle, USER, &path, Some("wisptext"))).await;
    let wrong = wrong.expect_err("logged in with a wrong passphrase");
    assert!(matches!(wrong, AppError::WrongPassphrase), "{wrong:?}");
    assert_eq!(wrong.to_string(), "wrong passphrase");
    let started = Instant::now();
    within(WAIT, "login", auth_key(&mut handle, USER, &path, Some(PASSWORD))).await.unwrap();
    times.push(ms(started.elapsed()));
    Shell::open(&handle).await;
    println!(
        "MEASURE key_login ed25519_ms={:.1} rsa_ms={:.1} ed25519_passphrase_ms={:.1}",
        times[0], times[1], times[2]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn shell_echo_and_resize() {
    let handle = logged_in(PORT).await;
    let mut shell = Shell::open(&handle).await;
    let echo_ms = ms(shell.run("echo wisp-$((6*7))", "wisp-42").await);
    shell.run("stty size | tr ' ' x", "24x80").await;

    shell.resize.send((132, 43)).await.unwrap();
    // Typed text and a new size travel apart, so a command can run before the size is there.
    let started = Instant::now();
    while shell.answers("stty size | tr ' ' x", "43x132", Duration::from_secs(1)).await.is_none() {
        assert!(started.elapsed() < WAIT, "the terminal inside never became 132x43");
    }

    shell.input.send(b"exit 3\n".to_vec()).await.unwrap();
    let exit = within(WAIT, "the end of the shell", shell.ended).await.unwrap();
    assert_eq!(exit, Some(3));
    println!("MEASURE shell_echo_and_resize echo_ms={echo_ms:.1}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn sftp_round_trip() {
    let (up_rate, down_rate, ..) = round_trip(PORT, 5).await;
    println!("MEASURE sftp_round_trip up_mib_s={up_rate:.1} down_mib_s={down_rate:.1}");

    let _scratch = scratch();
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.bin");
    std::fs::write(&small, noise(6, 3 * MIB + 17)).unwrap();
    let size = 3 * MIB as u64 + 17;
    let (file, sub) = (format!("{REMOTE}/small.bin"), format!("{REMOTE}/sub"));
    let moved = format!("{sub}/moved.bin");

    let handle = logged_in(PORT).await;
    let sftp = within(WAIT, "sftp", open_sftp(&handle)).await.unwrap();
    within(WAIT, "mkdir", mkdir(&sftp, REMOTE)).await.unwrap();
    up(&sftp, &small, &file).await;
    within(WAIT, "mkdir", mkdir(&sftp, &sub)).await.unwrap();

    let listed = within(WAIT, "list", list(&sftp, REMOTE)).await.unwrap();
    let seen: Vec<_> = listed.iter().map(|e| (e.name.as_str(), e.is_dir, e.size)).collect();
    assert_eq!(seen[1], ("small.bin", false, size));
    assert_eq!((seen.len(), seen[0].0, seen[0].1), (2, "sub", true));
    assert_eq!(listed[1].path, file);

    let entry = within(WAIT, "stat", stat(&sftp, &file)).await.unwrap();
    assert_eq!((entry.name.as_str(), entry.is_dir, entry.size), ("small.bin", false, size));
    let written: u64 = inside(&format!("stat -c %Y {file}")).parse().unwrap();
    assert_eq!(entry.modified, Some(written));

    within(WAIT, "rename", rename(&sftp, &file, &moved)).await.unwrap();
    let gone = within(WAIT, "stat", stat(&sftp, &file)).await;
    assert!(matches!(gone, Err(AppError::Sftp(_))), "the old name is still there");
    let listed = within(WAIT, "list", list(&sftp, &sub)).await.unwrap();
    let seen: Vec<_> = listed.iter().map(|e| (e.path.as_str(), e.is_dir, e.size)).collect();
    assert_eq!(seen, [(moved.as_str(), false, size)]);
    assert_eq!(sha256_inside(&moved), sha256_file(&small));

    within(WAIT, "remove", remove(&sftp, &moved, false)).await.unwrap();
    within(WAIT, "remove", remove(&sftp, &sub, true)).await.unwrap();
    assert!(within(WAIT, "list", list(&sftp, REMOTE)).await.unwrap().is_empty());
    within(WAIT, "remove", remove(&sftp, REMOTE, true)).await.unwrap();
    assert_eq!(inside(&format!("ls -d {REMOTE} 2>/dev/null; true")), "");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn sftp_through_forced_rekeys() {
    let (up_rate, down_rate, up_rekeys, down_rekeys) = round_trip(REKEY_PORT, 6).await;
    println!(
        "MEASURE sftp_through_forced_rekeys up_mib_s={up_rate:.1} down_mib_s={down_rate:.1} \
         up_rekeys={up_rekeys} down_rekeys={down_rekeys}"
    );
    // One exchange per 789,196 bytes sshd sent, but it had received 1.7 to 3.1 MB of an upload
    // between two of them: 25 to 32 were counted for these 64 MiB.
    assert!(up_rekeys >= 15, "{up_rekeys} key exchanges during the upload");
    assert!(down_rekeys >= 60, "{down_rekeys} key exchanges during the download");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn sftp_past_one_gibibyte() {
    let _scratch = scratch();
    let size = 1200 * MIB as u64;
    let (made, sent) = (format!("{REMOTE}/made.bin"), format!("{REMOTE}/sent.bin"));
    inside(&format!("mkdir {REMOTE} && head -c {size} /dev/urandom > {made}"));
    let hash = sha256_inside(&made);
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("big.bin");

    let handle = logged_in(PORT).await;
    let sftp = within(WAIT, "sftp", open_sftp(&handle)).await.unwrap();
    let down_time = down(&sftp, &made, &local, size).await;
    assert_eq!(sha256_file(&local), hash, "the downloaded file");
    let up_time = up(&sftp, &local, &sent).await;
    assert_eq!(sha256_inside(&sent), hash, "the uploaded file inside");
    // The same connection is still good for a small request.
    let entry = within(WAIT, "stat", stat(&sftp, &sent)).await.unwrap();
    assert_eq!(entry.size, size);
    println!(
        "MEASURE sftp_past_one_gibibyte down_s={:.1} up_s={:.1} down_mib_s={:.1} up_mib_s={:.1}",
        down_time.as_secs_f64(),
        up_time.as_secs_f64(),
        rate(size, down_time),
        rate(size, up_time)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn local_forward_carries_bulk_both_ways() {
    let app = app();
    let handle = Arc::new(logged_in(PORT).await);
    let port = free_port();
    let (app_handle, bind) = (app.handle().clone(), format!("{HOST}:{port}"));
    let tunnel =
        run_local(app_handle, "live".into(), "live".into(), handle, bind, HOST.into(), 7000)
            .unwrap();

    let sent = noise(8, 8 * MIB);
    let (mut reading, mut writing) = dial(port).await.into_split();
    let started = Instant::now();
    let outgoing = sent.clone();
    let writer = tokio::spawn(async move {
        writing.write_all(&outgoing).await.unwrap();
        writing
    });
    let mut back = vec![0u8; sent.len()];
    within(Duration::from_secs(120), "the echo", reading.read_exact(&mut back)).await.unwrap();
    let took = started.elapsed();
    let _writing = within(WAIT, "the writer", writer).await.unwrap();
    assert!(back == sent, "the echo is not what was sent");
    let mib_s = rate(8 * MIB as u64, took);
    println!("MEASURE local_forward_carries_bulk_both_ways mib_s={mib_s:.1}");
    tunnel.abort.abort();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn dynamic_forward() {
    let app = app();
    let handle = Arc::new(logged_in(PORT).await);
    let port = free_port();
    let bind = format!("{HOST}:{port}");
    let tunnel =
        run_dynamic(app.handle().clone(), "live".into(), "live".into(), handle, bind).unwrap();

    let mut stream = dial(port).await;
    let started = Instant::now();
    let talk = async {
        stream.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        stream.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        // CONNECT to 127.0.0.1:7000, the echo.
        stream.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0x1B, 0x58]).await.unwrap();
        let mut reply = [0u8; 10];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        let open_ms = ms(started.elapsed());

        let sent = noise(9, MIB);
        let (mut reading, mut writing) = stream.split();
        let mut back = vec![0u8; sent.len()];
        let (wrote, read) = tokio::join!(writing.write_all(&sent), reading.read_exact(&mut back));
        wrote.unwrap();
        read.unwrap();
        assert!(back == sent, "the echo is not what was sent");
        open_ms
    };
    let open_ms = within(Duration::from_secs(60), "the SOCKS5 exchange", talk).await;
    println!("MEASURE dynamic_forward open_ms={open_ms:.1}");

    // A port nothing listens on inside is refused in SOCKS5's own words.
    let mut stream = dial(port).await;
    let refusal = async {
        stream.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        stream.read_exact(&mut method).await.unwrap();
        stream.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0x1B, 0x63]).await.unwrap();
        let mut reply = [0u8; 10];
        stream.read_exact(&mut reply).await.unwrap();
        reply
    };
    let reply = within(WAIT, "the SOCKS5 refusal", refusal).await;
    assert_eq!(reply, [5, 5, 0, 1, 0, 0, 0, 0, 0, 0]);
    tunnel.abort.abort();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn remote_forward() {
    let app = app();
    let (_dir, hosts) = known(&[(PORT, &host_key())]);
    let forwards = new_forwards();
    let connecting = connect(HOST, PORT, hosts, forwards.clone());
    let mut handle = within(WAIT, "connect", connecting).await.unwrap();
    within(WAIT, "login", auth_password(&mut handle, USER, PASSWORD)).await.unwrap();

    let listener = TcpListener::bind((HOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut asked = [0u8; 5];
        stream.read_exact(&mut asked).await.unwrap();
        stream.write_all(b"pong\n").await.unwrap();
        asked
    });

    let starting = run_remote(
        app.handle().clone(),
        "live".into(),
        "live".into(),
        Arc::new(handle),
        forwards,
        HOST.into(),
        7100,
        HOST.into(),
        port,
    );
    let tunnel = within(WAIT, "the remote forward", starting).await.unwrap();
    let cleanup = tunnel.remote.as_ref().expect("a remote tunnel");
    assert_eq!((cleanup.bind_host.as_str(), cleanup.bound_port), (HOST, 7100));

    let started = Instant::now();
    // socat closes its sending side once `ping` is out and then waits for the answer.
    let answer = inside("printf 'ping\\n' | socat -t 10 - TCP:127.0.0.1:7100");
    let took = started.elapsed();
    assert_eq!(answer, "pong");
    assert_eq!(&within(WAIT, "the listener here", served).await.unwrap(), b"ping\n");
    println!("MEASURE remote_forward exchange_ms={:.1}", ms(took));
    tunnel.abort.abort();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn jump_host() {
    let outer = logged_in(PORT).await;
    let started = Instant::now();
    let opening = outer.channel_open_direct_tcpip(HOST, 22, HOST, 0);
    let channel = within(WAIT, "the channel to the second hop", opening).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
    hosts.record(HOST, 22, &host_key()).unwrap();
    let hosts = Arc::new(Mutex::new(hosts));
    let connecting = connect_over(channel.into_stream(), HOST, 22, hosts, new_forwards());
    let mut inner = within(WAIT, "connect over the first hop", connecting).await.unwrap();
    within(WAIT, "login", auth_password(&mut inner, USER, PASSWORD)).await.unwrap();
    let login_ms = ms(started.elapsed());

    let mut shell = Shell::open(&inner).await;
    let echo_ms = ms(shell.run("echo jump-$((6*7))", "jump-42").await);
    // sshd saw this session arrive from itself, not from the Docker gateway.
    shell.run("echo from-${SSH_CONNECTION%% *}-", "from-127.0.0.1-").await;
    println!("MEASURE jump_host login_ms={login_ms:.1} echo_ms={echo_ms:.1}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn shell_stays_responsive_while_a_tunnel_is_blocked() {
    let app = app();
    let handle = Arc::new(logged_in(PORT).await);
    let mut shell = Shell::open(&handle).await;
    let (quiet_median, quiet_max) = shell.echoes(20).await;

    let port = free_port();
    let bind = format!("{HOST}:{port}");
    let app_handle = app.handle().clone();
    let tunnel =
        run_local(app_handle, "live".into(), "live".into(), handle.clone(), bind, HOST.into(), 7001)
            .unwrap();
    let mut stream = dial(port).await;
    let chunk = vec![0x55u8; 64 * 1024];
    let mut taken = 0;
    // The sink inside reads nothing, so every buffer on the way fills and the write stops.
    while let Ok(wrote) = timeout(Duration::from_secs(3), stream.write(&chunk)).await {
        taken += wrote.unwrap();
        assert!(taken < 512 * MIB, "the sink took {taken} bytes and the write never blocked");
    }

    let (median, max) = shell.echoes(20).await;
    let still = timeout(Duration::from_millis(500), stream.write(&chunk)).await;
    assert!(still.is_err(), "the tunnel was no longer blocked after the echoes");
    println!(
        "MEASURE shell_stays_responsive_while_a_tunnel_is_blocked median_ms={median:.1} \
         max_ms={max:.1} quiet_median_ms={quiet_median:.1} quiet_max_ms={quiet_max:.1} \
         taken_kib={}",
        taken / 1024
    );
    tunnel.abort.abort();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn shell_stays_responsive_during_a_transfer() {
    let _scratch = scratch();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("load.bin");
    std::fs::write(&local, noise(13, 256 * MIB)).unwrap();
    let (hash, size) = (sha256_file(&local), 256 * MIB as u64);
    let remote = format!("{REMOTE}/load.bin");

    let handle = logged_in(PORT).await;
    let mut shell = Shell::open(&handle).await;
    let sftp = Arc::new(within(WAIT, "sftp", open_sftp(&handle)).await.unwrap());
    within(WAIT, "mkdir", mkdir(&sftp, REMOTE)).await.unwrap();
    let (quiet_median, quiet_max) = shell.echoes(20).await;

    let done = Arc::new(AtomicU64::new(0));
    let (progress, session, to) = (done.clone(), sftp.clone(), remote.clone());
    let started = Instant::now();
    let sending = tokio::spawn(async move {
        let report = |sent, _| progress.store(sent, Ordering::Relaxed);
        upload(&session, local.to_str().unwrap(), &to, report).await
    });
    while done.load(Ordering::Relaxed) < 16 * MIB as u64 {
        assert!(started.elapsed() < WAIT * 3, "the upload does not get going");
        sleep(Duration::from_millis(10)).await;
    }

    let from = done.load(Ordering::Relaxed);
    let (median, max) = shell.echoes(20).await;
    let until = done.load(Ordering::Relaxed);
    assert!(until < size, "the upload was over before the echoes were");
    within(TRANSFER, "upload", sending).await.unwrap().unwrap();
    let up_rate = rate(size, started.elapsed());
    assert_eq!(sha256_inside(&remote), hash, "the uploaded file inside");
    println!(
        "MEASURE shell_stays_responsive_during_a_transfer median_ms={median:.1} max_ms={max:.1} \
         quiet_median_ms={quiet_median:.1} quiet_max_ms={quiet_max:.1} up_mib_s={up_rate:.1} \
         echoes_from_mib={} echoes_until_mib={}",
        from / MIB as u64,
        until / MIB as u64
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn idle_session_survives() {
    let handle = logged_in(PORT).await;
    let mut shell = Shell::open(&handle).await;
    sleep(Duration::from_secs(90)).await;
    let echo_ms = ms(shell.run("echo idle-$((6*7))", "idle-42").await);
    let sftp = within(WAIT, "sftp", open_sftp(&handle)).await.unwrap();
    within(WAIT, "stat", stat(&sftp, "/home/wisp")).await.unwrap();
    println!("MEASURE idle_session_survives echo_ms={echo_ms:.1}");
}
