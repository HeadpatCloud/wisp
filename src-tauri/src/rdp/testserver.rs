use std::borrow::Cow;
use std::future::Future;
use std::net::SocketAddr;
use std::num::{NonZeroU16, NonZeroUsize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ironrdp_acceptor::credssp::CredsspSequence;
use ironrdp_acceptor::{Acceptor, BeginResult};
use ironrdp_cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_connector::sspi::credssp::EarlyUserAuthResult;
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::sspi::{AuthIdentity, Username};
use ironrdp_connector::ServerName;
use ironrdp_core::{encode_vec, WriteBuf};
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use ironrdp_pdu::fast_path::{
    EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode,
};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::mcs::{
    DisconnectProviderUltimatum, DisconnectReason, McsMessage, SendDataIndication,
};
use ironrdp_pdu::nego::{ConnectionConfirm, FailureCode, ResponseFlags, SecurityProtocol};
use ironrdp_pdu::rdp::capability_sets::{self, BitmapCodecs, CapabilitySet};
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::finalization_messages::MonitorLayoutPdu;
use ironrdp_pdu::rdp::headers::{
    CompressionFlags, ServerDeactivateAll, ShareControlHeader, ShareControlPdu, ShareDataHeader,
    ShareDataPdu, StreamPriority,
};
use ironrdp_pdu::rdp::server_error_info::{
    ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu,
};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_pdu::surface_commands::{
    ExtendedBitmapDataPdu, FrameAction, FrameMarkerPdu, SurfaceBitsPdu, SurfaceCommand,
};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::Action;
use ironrdp_server::{
    BitmapUpdate, CliprdrServerFactory, ConnectionHandler, Credentials, DesktopSize, DisplayUpdate,
    KeyboardEvent, MouseEvent, PixelFormat, PostConnectionAction, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, ServerEventSender,
};
use ironrdp_svc::StaticChannelSet;
use ironrdp_tokio::{FramedWrite as _, TokioFramed};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle as TaskHandle;
use tokio_rustls::TlsAcceptor;

pub enum Login {
    // TLS; whatever the Client Info PDU carries is accepted.
    Tls,
    // NLA (HYBRID and HYBRID_EX), NTLM against this one account.
    Nla(&'static str, &'static str),
}

pub struct Options {
    pub login: Login,
    pub size: (u16, u16),
    // false: no bitmap codecs on the server, so bitmaps go out uncompressed and arrive exact.
    pub remotefx: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seen {
    Key { code: u8, extended: bool, down: bool },
    Unicode { unit: u16, down: bool },
    Locks(u8),
    Mouse(Mouse),
    // Display Control monitor layout from the client. The server answers it with a resize.
    Layout { width: u32, height: u32, scale: Option<u32> },
    ClipReady,
    ClipFormats(Vec<u32>),
    ClipRequest(u32),
    ClipData(Option<String>),
    Gone(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mouse {
    Move { x: u16, y: u16 },
    Left(bool),
    Right(bool),
    Middle(bool),
    Button4(bool),
    Button5(bool),
    Wheel(i16),
    Scroll { x: i32, y: i32 },
    RelMove { x: i32, y: i32 },
}

pub struct TestServer {
    pub addr: Option<SocketAddr>,
    // ServerEvent::Quit ends the connection; ServerEvent::Clipboard drives the remote clipboard.
    pub events: mpsc::UnboundedSender<ServerEvent>,
    pub display: mpsc::UnboundedSender<DisplayUpdate>,
    pub seen: mpsc::UnboundedReceiver<Seen>,
    // Bitmap updates the server has taken from `display`; a server that cannot write stops taking.
    pub taken: Arc<AtomicUsize>,
    // false: a layout request is reported and nothing is pushed in answer.
    pub answers_layouts: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl TestServer {
    pub async fn listen(options: Options) -> Self {
        let mut server = Self::spawn(options, None::<DuplexStream>).await;
        let (tx, rx) = oneshot::channel();
        server.events.send(ServerEvent::GetLocalAddr(tx)).unwrap();
        server.addr = rx.await.unwrap();
        server
    }

    pub async fn on<S>(options: Options, stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
    {
        Self::spawn(options, Some(stream)).await
    }

    // RdpServer::run is not Send, so it gets a thread and a runtime of its own.
    async fn spawn<S>(options: Options, stream: Option<S>) -> Self
    where
        S: AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
    {
        let (seen_tx, seen) = mpsc::unbounded_channel();
        let (display, updates) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let taken = Arc::new(AtomicUsize::new(0));
        let answers_layouts = Arc::new(AtomicBool::new(true));

        let thread = std::thread::spawn({
            let display = display.clone();
            let stop = Arc::clone(&stop);
            let taken = Arc::clone(&taken);
            let answers_layouts = Arc::clone(&answers_layouts);
            move || {
                let runtime =
                    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(async move {
                    let (acceptor, public_key) = tls();
                    let builder = RdpServer::builder().with_addr(([127, 0, 0, 1], 0));
                    let builder = match options.login {
                        Login::Nla(..) => builder.with_hybrid(acceptor, public_key),
                        Login::Tls => builder.with_tls(acceptor),
                    };
                    let codecs = if options.remotefx {
                        ironrdp_pdu::rdp::capability_sets::server_codecs_capabilities(&[]).unwrap()
                    } else {
                        BitmapCodecs(Vec::new())
                    };
                    let mut server = builder
                        .with_input_handler(Input(seen_tx.clone()))
                        .with_display_handler(Display {
                            size: Arc::new(Mutex::new(DesktopSize {
                                width: options.size.0,
                                height: options.size.1,
                            })),
                            updates: Arc::new(tokio::sync::Mutex::new(updates)),
                            push: display,
                            seen: seen_tx.clone(),
                            taken,
                            answers_layouts,
                        })
                        .with_cliprdr_factory(Some(Box::new(Clipboard(seen_tx.clone()))))
                        .with_connection_handler(Some(Box::new(Connections {
                            stop,
                            seen: seen_tx.clone(),
                        })))
                        .with_bitmap_codecs(codecs)
                        .build();
                    if let Login::Nla(username, password) = options.login {
                        server.set_credentials(Some(Credentials {
                            username: username.to_owned(),
                            password: password.to_owned(),
                            domain: None,
                        }));
                    }
                    ready_tx.send(server.event_sender().clone()).unwrap();

                    let result = match stream {
                        None => server.run().await,
                        Some(stream) => {
                            let result = server.run_connection(stream).await;
                            let ended = result.as_ref().err().map(|e| format!("{e:#}"));
                            seen_tx.send(Seen::Gone(ended)).ok();
                            result
                        }
                    };
                    result.map_err(|e| format!("{e:#}"))
                })
            }
        });

        let events = ready_rx.await.unwrap();
        let thread = Some(thread);
        Self { addr: None, events, display, seen, taken, answers_layouts, stop, thread }
    }

    fn clipboard(&self, message: ClipboardMessage) {
        self.events.send(ServerEvent::Clipboard(message)).unwrap();
    }

    pub fn remote_copies(&self, formats: &[u32]) {
        let format = |id: &u32| ClipboardFormat::new(ClipboardFormatId(*id));
        let formats = formats.iter().map(format).collect();
        self.clipboard(ClipboardMessage::SendInitiateCopy(formats));
    }

    pub fn remote_pastes(&self, format: u32) {
        self.clipboard(ClipboardMessage::SendInitiatePaste(ClipboardFormatId(format)));
    }

    pub fn remote_sends(&self, text: &str) {
        let data = FormatDataResponse::new_unicode_string(text);
        self.clipboard(ClipboardMessage::SendFormatData(data));
    }

    pub fn stop(mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        self.events.send(ServerEvent::Quit("test over".to_owned())).ok();
        self.thread.take().unwrap().join().unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The server may be gone already.
        self.events.send(ServerEvent::Quit("test over".to_owned())).ok();
    }
}

// A bitmap update from R, G, B per pixel. BGRA on the wire: the uncompressed path sends the bytes
// as they are and the client reads them as BGRX, whatever `format` says.
pub fn bitmap(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    pixel: impl Fn(u16, u16) -> [u8; 3],
) -> DisplayUpdate {
    let mut data = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in 0..height {
        for column in 0..width {
            let [r, g, b] = pixel(column, row);
            data.extend([b, g, r, 255]);
        }
    }
    DisplayUpdate::Bitmap(BitmapUpdate {
        x,
        y,
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(height).unwrap(),
        format: PixelFormat::BgrA32,
        data: data.into(),
        stride: NonZeroUsize::new(usize::from(width) * 4).unwrap(),
    })
}

pub fn certificate() -> CertificateDer<'static> {
    CertificateDer::from_pem_slice(include_bytes!("../vnc/fixtures/test-cert.pem")).unwrap()
}

// The acceptor and the subjectPublicKey bytes of its certificate, which CredSSP binds to.
pub fn tls() -> (TlsAcceptor, Vec<u8>) {
    use x509_cert::der::Decode as _;

    let certificate = certificate();
    let key =
        PrivateKeyDer::from_pem_slice(include_bytes!("../vnc/fixtures/test-key.pem")).unwrap();
    let public_key = x509_cert::Certificate::from_der(&certificate)
        .unwrap()
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .unwrap()
        .to_vec();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    // Tickets go out after the handshake; writing them to a client that has already hung up
    // would fail the accept and hide what that client sent.
    config.send_tls13_tickets = 0;
    (TlsAcceptor::from(Arc::new(config)), public_key)
}

struct Input(mpsc::UnboundedSender<Seen>);

impl RdpServerInputHandler for Input {
    fn keyboard(&mut self, event: KeyboardEvent) {
        self.0
            .send(match event {
                KeyboardEvent::Pressed { code, extended } => {
                    Seen::Key { code, extended, down: true }
                }
                KeyboardEvent::Released { code, extended } => {
                    Seen::Key { code, extended, down: false }
                }
                KeyboardEvent::UnicodePressed(unit) => Seen::Unicode { unit, down: true },
                KeyboardEvent::UnicodeReleased(unit) => Seen::Unicode { unit, down: false },
                KeyboardEvent::Synchronize(flags) => Seen::Locks(flags.bits()),
            })
            .ok();
    }

    fn mouse(&mut self, event: MouseEvent) {
        self.0
            .send(Seen::Mouse(match event {
                MouseEvent::Move { x, y } => Mouse::Move { x, y },
                MouseEvent::LeftPressed => Mouse::Left(true),
                MouseEvent::LeftReleased => Mouse::Left(false),
                MouseEvent::RightPressed => Mouse::Right(true),
                MouseEvent::RightReleased => Mouse::Right(false),
                MouseEvent::MiddlePressed => Mouse::Middle(true),
                MouseEvent::MiddleReleased => Mouse::Middle(false),
                MouseEvent::Button4Pressed => Mouse::Button4(true),
                MouseEvent::Button4Released => Mouse::Button4(false),
                MouseEvent::Button5Pressed => Mouse::Button5(true),
                MouseEvent::Button5Released => Mouse::Button5(false),
                MouseEvent::VerticalScroll { value } => Mouse::Wheel(value),
                MouseEvent::Scroll { x, y } => Mouse::Scroll { x, y },
                MouseEvent::RelMove { x, y } => Mouse::RelMove { x, y },
            }))
            .ok();
    }
}

struct Display {
    size: Arc<Mutex<DesktopSize>>,
    updates: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<DisplayUpdate>>>,
    push: mpsc::UnboundedSender<DisplayUpdate>,
    seen: mpsc::UnboundedSender<Seen>,
    taken: Arc<AtomicUsize>,
    answers_layouts: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        *self.size.lock().unwrap()
    }

    // Called again after every reactivation, so the receiver is shared.
    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(Updates {
            size: Arc::clone(&self.size),
            updates: Arc::clone(&self.updates),
            taken: Arc::clone(&self.taken),
        }))
    }

    fn request_layout(&mut self, layout: DisplayControlMonitorLayout) {
        let Some(monitor) = layout.monitors().first() else {
            return;
        };
        let (width, height) = monitor.dimensions();
        let scale = monitor.desktop_scale_factor();
        self.seen.send(Seen::Layout { width, height, scale }).ok();
        if !self.answers_layouts.load(Ordering::SeqCst) {
            return;
        }
        let size = DesktopSize { width: width as u16, height: height as u16 };
        self.push.send(DisplayUpdate::Resize(size)).ok();
        // As Windows and xrdp do, a picture at the new size.
        self.push.send(bitmap(0, 0, 8, 8, |_, _| [0, 0, 0])).ok();
    }
}

struct Updates {
    size: Arc<Mutex<DesktopSize>>,
    updates: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<DisplayUpdate>>>,
    taken: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        let update = self.updates.lock().await.recv().await;
        // The server asks the display for its size again when it reactivates.
        match &update {
            Some(DisplayUpdate::Resize(size)) => *self.size.lock().unwrap() = *size,
            Some(DisplayUpdate::Bitmap(_)) => {
                self.taken.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
        Ok(update)
    }
}

struct Clipboard(mpsc::UnboundedSender<Seen>);

impl CliprdrBackendFactory for Clipboard {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(ClipboardBackend(self.0.clone()))
    }
}

impl ServerEventSender for Clipboard {
    fn set_sender(&mut self, _sender: mpsc::UnboundedSender<ServerEvent>) {}
}

impl CliprdrServerFactory for Clipboard {}

#[derive(Debug)]
struct ClipboardBackend(mpsc::UnboundedSender<Seen>);

ironrdp_core::impl_as_any!(ClipboardBackend);

impl CliprdrBackend for ClipboardBackend {
    fn temporary_directory(&self) -> &str {
        ""
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }
    fn on_ready(&mut self) {
        self.0.send(Seen::ClipReady).ok();
    }
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(
        &mut self,
        _capabilities: ClipboardGeneralCapabilityFlags,
    ) {
    }
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        let ids = available_formats.iter().map(|format| format.id().0).collect();
        self.0.send(Seen::ClipFormats(ids)).ok();
    }
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.0.send(Seen::ClipRequest(request.format.0)).ok();
    }
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let text = if response.is_error() { None } else { response.to_unicode_string().ok() };
        self.0.send(Seen::ClipData(text)).ok();
    }
    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _data_id: LockDataId) {}
    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

struct Connections {
    stop: Arc<AtomicBool>,
    seen: mpsc::UnboundedSender<Seen>,
}

impl ConnectionHandler for Connections {
    fn on_disconnected(
        &mut self,
        _peer: SocketAddr,
        _duration: Duration,
        error: Option<&anyhow::Error>,
    ) -> PostConnectionAction {
        self.seen.send(Seen::Gone(error.map(|e| format!("{e:#}")))).ok();
        if self.stop.load(Ordering::SeqCst) {
            PostConnectionAction::Stop
        } else {
            PostConnectionAction::Continue
        }
    }
}

pub enum Script {
    Reply(Vec<u8>),
    ReplyThenHangUp(Vec<u8>),
    Silent,
    HangUp,
    NotTls(SecurityProtocol),
    Tls(SecurityProtocol),
    TlsThenHangUp(SecurityProtocol),
    Inject(SecurityProtocol, Vec<u8>, bool),
    TlsThenSend(SecurityProtocol, Vec<u8>),
}

pub struct Heard {
    pub request: Vec<u8>,
    pub after_tls: Vec<u8>,
}

pub fn confirm(protocol: SecurityProtocol) -> Vec<u8> {
    let confirm = ConnectionConfirm::Response { flags: ResponseFlags::empty(), protocol };
    encode_vec(&X224(confirm)).unwrap()
}

pub fn negotiation_failure(code: FailureCode) -> Vec<u8> {
    encode_vec(&X224(ConnectionConfirm::Failure { code })).unwrap()
}

// A server for what ironrdp-server cannot do: it never runs the connection sequence, so the test
// decides what is on the wire. Ends when the client goes away.
pub fn serve<S>(mut stream: S, script: Script) -> TaskHandle<Heard>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let mut heard = Heard { request: Vec::new(), after_tls: Vec::new() };
        let mut header = [0u8; 4];
        if stream.read_exact(&mut header).await.is_err() {
            return heard;
        }
        heard.request.extend(header);
        let rest = usize::from(u16::from_be_bytes([header[2], header[3]])).saturating_sub(4);
        let mut body = vec![0u8; rest];
        if stream.read_exact(&mut body).await.is_err() {
            return heard;
        }
        heard.request.extend(body);

        let (protocol, hang_up) = match script {
            Script::Reply(bytes) => {
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
                wait_for_end(&mut stream).await;
                return heard;
            }
            Script::ReplyThenHangUp(bytes) => {
                stream.write_all(&bytes).await.unwrap();
                stream.flush().await.unwrap();
                return heard;
            }
            Script::Silent => {
                wait_for_end(&mut stream).await;
                return heard;
            }
            Script::HangUp => return heard,
            Script::NotTls(protocol) => {
                stream.write_all(&confirm(protocol)).await.unwrap();
                stream.read_u8().await.unwrap();
                stream.write_all(b"this is not a TLS record\r\n").await.unwrap();
                wait_for_end(&mut stream).await;
                return heard;
            }
            Script::Inject(protocol, bytes, split) => {
                let confirmed = confirm(protocol);
                if split {
                    stream.write_all(&confirmed).await.ok();
                    stream.flush().await.ok();
                    stream.write_all(&bytes).await.ok();
                } else {
                    stream.write_all(&[confirmed, bytes].concat()).await.ok();
                }
                stream.flush().await.ok();
                if let Ok(mut stream) = tls().0.accept(stream).await {
                    stream.read_to_end(&mut heard.after_tls).await.ok();
                }
                return heard;
            }
            Script::TlsThenSend(protocol, bytes) => {
                stream.write_all(&confirm(protocol)).await.unwrap();
                stream.flush().await.unwrap();
                let mut stream = tls().0.accept(stream).await.unwrap();
                stream.write_all(&bytes).await.ok();
                stream.flush().await.ok();
                stream.read_to_end(&mut heard.after_tls).await.ok();
                return heard;
            }
            Script::Tls(protocol) => (protocol, false),
            Script::TlsThenHangUp(protocol) => (protocol, true),
        };
        stream.write_all(&confirm(protocol)).await.unwrap();
        stream.flush().await.unwrap();
        let mut stream = tls().0.accept(stream).await.unwrap();
        if !hang_up {
            // The client leaves without a close_notify, which reads as an error.
            stream.read_to_end(&mut heard.after_tls).await.ok();
        }
        heard
    })
}

async fn wait_for_end(stream: &mut (impl AsyncRead + Unpin)) {
    let mut sink = [0u8; 256];
    while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
}

pub type ScriptedFramed<S> = TokioFramed<tokio_rustls::server::TlsStream<S>>;

// What the frames below need of the connection; the acceptor's own result cannot be held across
// an await in a task.
#[derive(Clone, Copy)]
pub struct Channels {
    io: u16,
    user: u16,
}

// The sets ironrdp-server sends; its own list is private.
fn capabilities(size: DesktopSize, max_request_size: u32) -> Vec<CapabilitySet> {
    use capability_sets::{
        CmdFlags, Input, InputFlags, MultifragmentUpdate, Order, OrderFlags, OrderSupportExFlags,
        Pointer, SurfaceCommands, VirtualChannel, VirtualChannelFlags,
    };

    vec![
        CapabilitySet::General(capability_sets::General {
            extra_flags: capability_sets::GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED,
            ..Default::default()
        }),
        CapabilitySet::Bitmap(capability_sets::Bitmap {
            pref_bits_per_pix: 32,
            desktop_width: size.width,
            desktop_height: size.height,
            desktop_resize_flag: true,
            drawing_flags: capability_sets::BitmapDrawingFlags::empty(),
        }),
        CapabilitySet::Order(Order::new(
            OrderFlags::empty(),
            OrderSupportExFlags::empty(),
            2048,
            224,
        )),
        CapabilitySet::SurfaceCommands(SurfaceCommands { flags: CmdFlags::all() }),
        CapabilitySet::Pointer(Pointer {
            color_pointer_cache_size: 2048,
            pointer_cache_size: 2048,
        }),
        CapabilitySet::Input(Input {
            input_flags: InputFlags::SCANCODES
                | InputFlags::MOUSEX
                | InputFlags::FASTPATH_INPUT
                | InputFlags::UNICODE
                | InputFlags::FASTPATH_INPUT_2,
            keyboard_layout: 0,
            keyboard_type: None,
            keyboard_subtype: 0,
            keyboard_function_key: 128,
            keyboard_ime_filename: String::new(),
        }),
        CapabilitySet::VirtualChannel(VirtualChannel {
            flags: VirtualChannelFlags::NO_COMPRESSION,
            chunk_size: None,
        }),
        CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate { max_request_size }),
        CapabilitySet::BitmapCodecs(BitmapCodecs(Vec::new())),
    ]
}

// A server for what ironrdp-server cannot send: the connection sequence runs on TLS without NLA,
// then the test owns the stream and writes the frames below itself.
pub async fn accept_scripted<S>(stream: S, size: (u16, u16)) -> (ScriptedFramed<S>, Channels)
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    accept_scripted_with(stream, size, 8 * 1024 * 1024).await
}

// `max_request_size` is the largest fast-path update the server says it reassembles.
pub async fn accept_scripted_with<S>(
    stream: S,
    size: (u16, u16),
    max_request_size: u32,
) -> (ScriptedFramed<S>, Channels)
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let (framed, channels, _) = accept_scripted_keeping(stream, size, max_request_size).await;
    (framed, channels)
}

// The acceptor is what `reactivate_scripted` needs to run the capability exchange again.
pub async fn accept_scripted_keeping<S>(
    stream: S,
    size: (u16, u16),
    max_request_size: u32,
) -> (ScriptedFramed<S>, Channels, Acceptor)
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let size = DesktopSize { width: size.0, height: size.1 };
    let capabilities = capabilities(size, max_request_size);
    let mut acceptor = Acceptor::new(SecurityProtocol::SSL, size, capabilities, None);
    let begun = ironrdp_acceptor::accept_begin(TokioFramed::new(stream), &mut acceptor).await;
    let BeginResult::ShouldUpgrade(stream) = begun.unwrap() else {
        panic!("the client did not ask for TLS");
    };
    let stream = tls().0.accept(stream).await.unwrap();
    acceptor.mark_security_upgrade_as_done();
    let (framed, result) =
        ironrdp_acceptor::accept_finalize(TokioFramed::new(stream), &mut acceptor).await.unwrap();
    (framed, Channels { io: result.io_channel_id, user: result.user_channel_id }, acceptor)
}

// An NLA (HYBRID_EX) server that holds back its Early User Authorization Result: `heard` fires once
// the client's credentials have come, and the result goes out when `release` does. The rest is as
// `accept_scripted`.
pub async fn accept_nla_holding<S>(
    stream: S,
    size: (u16, u16),
    (username, password): (&str, &str),
    heard: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
) -> (ScriptedFramed<S>, Channels)
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let size = DesktopSize { width: size.0, height: size.1 };
    let account = Credentials {
        username: username.to_owned(),
        password: password.to_owned(),
        domain: None,
    };
    let capabilities = capabilities(size, 8 * 1024 * 1024);
    let mut acceptor =
        Acceptor::new(SecurityProtocol::HYBRID_EX, size, capabilities, Some(account));
    let begun = ironrdp_acceptor::accept_begin(TokioFramed::new(stream), &mut acceptor).await;
    let BeginResult::ShouldUpgrade(stream) = begun.unwrap() else {
        panic!("the client did not ask for TLS");
    };
    let (tls, public_key) = tls();
    let mut framed = TokioFramed::new(tls.accept(stream).await.unwrap());
    acceptor.mark_security_upgrade_as_done();

    let identity = AuthIdentity {
        username: Username::new(username, None).unwrap(),
        password: password.to_owned().into(),
    };
    let name = ServerName::new("rdp-client");
    let mut credssp = CredsspSequence::init(&identity, name, public_key, None).unwrap();
    let mut buf = WriteBuf::new();
    while let Some(hint) = credssp.next_pdu_hint().unwrap() {
        let pdu = framed.read_by_hint(hint).await.unwrap();
        let Some(request) = credssp.decode_client_message(&pdu).unwrap() else { break };
        let GeneratorState::Completed(result) = credssp.process_ts_request(request).start() else {
            panic!("NTLM asked for the network");
        };
        buf.clear();
        if let Some(length) = credssp.handle_process_result(result, &mut buf).unwrap().size() {
            framed.write_all(&buf[..length]).await.unwrap();
        }
    }
    heard.send(()).unwrap();
    release.await.unwrap();
    let mut success = Vec::new();
    EarlyUserAuthResult::Success.to_buffer(&mut success).unwrap();
    framed.write_all(&success).await.unwrap();
    acceptor.mark_credssp_as_done();

    let (framed, result) = ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await.unwrap();
    (framed, Channels { io: result.io_channel_id, user: result.user_channel_id })
}

// What a server does to resize: deactivate, then the capability exchange at the new size.
// `between` goes out after the Deactivate All, before the exchange.
pub async fn reactivate_scripted<S>(
    mut framed: ScriptedFramed<S>,
    channels: &Channels,
    acceptor: Acceptor,
    size: (u16, u16),
    between: &[Vec<u8>],
) -> ScriptedFramed<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    framed.write_all(&deactivate_all(channels)).await.unwrap();
    for frame in between {
        framed.write_all(frame).await.unwrap();
    }
    let size = DesktopSize { width: size.0, height: size.1 };
    let mut acceptor =
        Acceptor::new_deactivation_reactivation(acceptor, StaticChannelSet::new(), size).unwrap();
    ironrdp_acceptor::accept_finalize(framed, &mut acceptor).await.unwrap().0
}

fn io_channel(user_data: Vec<u8>, channels: &Channels) -> Vec<u8> {
    let indication = SendDataIndication {
        initiator_id: channels.user,
        channel_id: channels.io,
        user_data: Cow::Owned(user_data),
    };
    encode_vec(&X224(indication)).unwrap()
}

fn share_data_header(pdu: ShareDataPdu, channels: &Channels) -> Vec<u8> {
    let header = ShareControlHeader {
        share_id: 0,
        pdu_source: channels.io,
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: pdu,
            stream_priority: StreamPriority::Undefined,
            compression_flags: CompressionFlags::empty(),
            compression_type: CompressionType::K8,
        }),
    };
    encode_vec(&header).unwrap()
}

fn share_data(pdu: ShareDataPdu, channels: &Channels) -> Vec<u8> {
    io_channel(share_data_header(pdu, channels), channels)
}

pub fn error_info(info: ErrorInfo, channels: &Channels) -> Vec<u8> {
    share_data(ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(info)), channels)
}

// A code the client's error info list does not have.
pub fn unlisted_error_info(code: u32, channels: &Channels) -> Vec<u8> {
    let none = ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::None);
    let mut frame = error_info(none, channels);
    let end = frame.len();
    frame[end - 4..].copy_from_slice(&code.to_le_bytes());
    frame
}

pub fn deactivate_all(channels: &Channels) -> Vec<u8> {
    let header = ShareControlHeader {
        share_id: 0,
        pdu_source: channels.io,
        share_control_pdu: ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll),
    };
    io_channel(encode_vec(&header).unwrap(), channels)
}

// What xrdp sends: the share control header alone, without the share id and the source.
pub fn short_deactivate_all(channels: &Channels) -> Vec<u8> {
    const DEACTIVATE_ALL: u16 = 0x16;
    let header = [6u16, DEACTIVATE_ALL, channels.user].map(u16::to_le_bytes);
    io_channel(header.concat(), channels)
}

pub fn shutdown_denied(channels: &Channels) -> Vec<u8> {
    share_data(ShareDataPdu::ShutdownDenied, channels)
}

// ShareDataPdu cannot encode a sound, so the header of an empty PDU gets the sound's type and body.
pub fn play_sound(channels: &Channels) -> Vec<u8> {
    const TOTAL_LENGTH: usize = 0;
    const UNCOMPRESSED_LENGTH: usize = 12;
    const PDU_TYPE: usize = 14;

    let body = [0u8; 8];
    let mut data = share_data_header(ShareDataPdu::ShutdownDenied, channels);
    data[PDU_TYPE] = 0x22;
    data[UNCOMPRESSED_LENGTH..UNCOMPRESSED_LENGTH + 2].copy_from_slice(&8u16.to_le_bytes());
    let total = u16::try_from(data.len() + body.len()).unwrap();
    data[TOTAL_LENGTH..TOTAL_LENGTH + 2].copy_from_slice(&total.to_le_bytes());
    data.extend(body);
    io_channel(data, channels)
}

pub fn monitor_layout(channels: &Channels) -> Vec<u8> {
    share_data(ShareDataPdu::MonitorLayout(MonitorLayoutPdu { monitors: Vec::new() }), channels)
}

pub fn garbage(channels: &Channels) -> Vec<u8> {
    io_channel(vec![0xff; 12], channels)
}

// One chunk of a static channel message. The channels are numbered from the first one the client
// asked for.
pub fn channel_chunk(
    channels: &Channels,
    channel: u16,
    flags: ChannelControlFlags,
    length: u32,
    data: &[u8],
) -> Vec<u8> {
    let user_data = [encode_vec(&ChannelPduHeader { length, flags }).unwrap(), data.to_vec()];
    let indication = SendDataIndication {
        initiator_id: channels.user,
        channel_id: channels.io + 1 + channel,
        user_data: Cow::Owned(user_data.concat()),
    };
    encode_vec(&X224(indication)).unwrap()
}

pub fn mcs_disconnect(reason: DisconnectReason) -> Vec<u8> {
    let ultimatum = DisconnectProviderUltimatum::from_reason(reason);
    encode_vec(&X224(McsMessage::DisconnectProviderUltimatum(ultimatum))).unwrap()
}

// An uncompressed Set Surface Bits update in one fast-path frame, so at most 16 KB of pixels. The
// rows go bottom-up, as the server encoder writes them.
pub fn surface_bits(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    pixel: impl Fn(u16, u16) -> [u8; 3],
) -> Vec<u8> {
    let command = surface_command(x, y, width, height, pixel);
    fast_path_update(Fragmentation::Single, UpdateCode::SurfaceCommands, &command)
}

pub fn surface_command(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    pixel: impl Fn(u16, u16) -> [u8; 3],
) -> Vec<u8> {
    let mut data = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in (0..height).rev() {
        for column in 0..width {
            let [r, g, b] = pixel(column, row);
            data.extend([b, g, r, 255]);
        }
    }
    let command = SurfaceCommand::SetSurfaceBits(SurfaceBitsPdu {
        destination: ExclusiveRectangle { left: x, top: y, right: x + width, bottom: y + height },
        extended_bitmap_data: ExtendedBitmapDataPdu {
            bpp: 32,
            codec_id: 0,
            width,
            height,
            header: None,
            data: &data,
        },
    });
    encode_vec(&command).unwrap()
}

pub fn frame_marker(frame_action: FrameAction, frame_id: u32) -> Vec<u8> {
    let marker = FrameMarkerPdu { frame_action, frame_id: Some(frame_id) };
    let command = encode_vec(&SurfaceCommand::FrameMarker(marker)).unwrap();
    fast_path_update(Fragmentation::Single, UpdateCode::SurfaceCommands, &command)
}

// One fast-path frame carrying one update, or one fragment of it.
pub fn fast_path_update(fragmentation: Fragmentation, code: UpdateCode, data: &[u8]) -> Vec<u8> {
    let update = FastPathUpdatePdu {
        fragmentation,
        update_code: code,
        compression_flags: None,
        compression_type: None,
        data,
    };
    let update = encode_vec(&update).unwrap();
    let header = FastPathHeader::new(EncryptionFlags::empty(), update.len());
    [encode_vec(&header).unwrap(), update].concat()
}

// An update cut into fast-path frames of `piece` bytes of it each.
pub fn fragments(code: UpdateCode, data: &[u8], piece: usize) -> Vec<Vec<u8>> {
    let count = data.len().div_ceil(piece);
    let fragmentation = |index| match index {
        0 => Fragmentation::First,
        last if last + 1 == count => Fragmentation::Last,
        _ => Fragmentation::Next,
    };
    let pieces = data.chunks(piece).enumerate();
    pieces.map(|(index, chunk)| fast_path_update(fragmentation(index), code, chunk)).collect()
}

// Empty and cut-short frames, as a fast-path or a TPKT header announces them.
pub fn short_frames() -> Vec<Vec<u8>> {
    let mut frames = vec![vec![0; 40]];
    frames.extend((0..=2).map(|length| vec![0x00, length]));
    frames.extend((0..=2).map(|length| vec![0x00, 0x80, length]));
    frames.extend((0..=4).map(|length| vec![0x03, 0x00, 0x00, length]));
    frames
}

// A client that never yields cannot be timed out from inside, so it gets a thread and a runtime of
// its own and the test waits for it with a deadline. A thread that spins stays until the process
// ends.
pub async fn off_thread<T, Fut>(
    deadline: Duration,
    work: impl FnOnce() -> Fut + Send + 'static,
) -> Option<T>
where
    T: Send + 'static,
    Fut: Future<Output = T>,
{
    let (done, result) = oneshot::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        done.send(runtime.block_on(work())).ok();
    });
    tokio::time::timeout(deadline, result).await.ok().map(|result| result.unwrap())
}

#[derive(Debug, PartialEq)]
pub enum ClientPdu {
    ShutdownRequest,
    Disconnect(DisconnectReason),
    Input(Vec<FastPathInputEvent>),
    FrameAcknowledge(u32),
    Layout,
    Other,
}

pub async fn read_client<S>(
    framed: &mut ScriptedFramed<S>,
    channels: &Channels,
) -> std::io::Result<ClientPdu>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let (action, frame) = framed.read_pdu().await?;
    if action == Action::FastPath {
        return Ok(match ironrdp_core::decode::<FastPathInput>(&frame) {
            Ok(input) => ClientPdu::Input(input.input_events().to_vec()),
            Err(_) => ClientPdu::Other,
        });
    }
    if action != Action::X224 {
        return Ok(ClientPdu::Other);
    }
    let message = ironrdp_core::decode::<X224<McsMessage<'_>>>(&frame).unwrap().0;
    Ok(match message {
        McsMessage::DisconnectProviderUltimatum(ultimatum) => {
            ClientPdu::Disconnect(ultimatum.reason)
        }
        McsMessage::SendDataRequest(data) if data.channel_id == channels.io => {
            let header: ShareControlHeader = ironrdp_core::decode(data.user_data.as_ref()).unwrap();
            match header.share_control_pdu {
                ShareControlPdu::Data(data) => match data.share_data_pdu {
                    ShareDataPdu::ShutdownRequest => ClientPdu::ShutdownRequest,
                    ShareDataPdu::FrameAcknowledge(ack) => {
                        ClientPdu::FrameAcknowledge(ack.frame_id)
                    }
                    _ => ClientPdu::Other,
                },
                _ => ClientPdu::Other,
            }
        }
        // A Display Control monitor layout, which goes out as one dynamic channel data PDU.
        McsMessage::SendDataRequest(data) => {
            const MONITOR_LAYOUT: [u8; 4] = [2, 0, 0, 0];
            let pdu = data.user_data.get(8..).map(ironrdp_core::decode::<DrdynvcClientPdu>);
            match pdu {
                Some(Ok(DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(pdu))))
                    if pdu.data().starts_with(&MONITOR_LAYOUT) =>
                {
                    ClientPdu::Layout
                }
                _ => ClientPdu::Other,
            }
        }
        _ => ClientPdu::Other,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{ServerName, UnixTime};
    use tokio::net::TcpStream;

    use super::*;

    #[derive(Debug)]
    struct AcceptAll;

    impl ServerCertVerifier for AcceptAll {
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
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    #[tokio::test]
    async fn server_accepts_tls_on_loopback() {
        let mut server =
            TestServer::listen(Options { login: Login::Tls, size: (320, 240), remotefx: false })
                .await;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAll))
        .with_no_client_auth();

        let mut tcp = TcpStream::connect(server.addr.unwrap()).await.unwrap();
        // A Connection Request asking for TLS: cookie-less, 19 bytes.
        let request = [3, 0, 0, 19, 14, 0xe0, 0, 0, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0];
        tcp.write_all(&request).await.unwrap();
        let mut confirm = [0u8; 19];
        tokio::time::timeout(Duration::from_secs(10), tcp.read_exact(&mut confirm))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&confirm[..2], &[3, 0]);

        let name = ServerName::try_from("rdp-test.local").unwrap();
        let tls = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp),
        )
        .await
        .unwrap()
        .unwrap();
        let chain = tls.get_ref().1.peer_certificates().unwrap();
        assert_eq!(chain[0].as_ref(), certificate().as_ref());
        drop(tls);
        let ended = tokio::time::timeout(Duration::from_secs(10), server.seen.recv()).await;
        assert!(matches!(ended, Ok(Some(Seen::Gone(_)))), "{ended:?}");
        tokio::task::spawn_blocking(move || server.stop()).await.unwrap().ok();
    }
}
