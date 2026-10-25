use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use futures_util::FutureExt as _;
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationState,
};
use ironrdp_connector::{ClientConnector, DesktopSize, Sequence as _};
use ironrdp_core::{DecodeErrorKind, ReadCursor, WriteBuf};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_dvc::pdu::{DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_dvc::DrdynvcClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::fast_path::{
    FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode,
};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::mcs::McsMessage;
use ironrdp_pdu::rdp::headers::{IoChannelPdu, ShareDataPdu};
use ironrdp_pdu::rdp::server_error_info::{
    ErrorInfo, ProtocolIndependentCode, ServerSetErrorInfoPdu,
};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::surface_commands::{FrameAction, SurfaceCommand};
use ironrdp_pdu::x224::X224;
use ironrdp_pdu::Action;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{
    fast_path, ActiveStage, ActiveStageBuilder, ActiveStageOutput, SessionError, SessionErrorKind,
};
use ironrdp_tokio::{split_tokio_framed, FramedWrite as _, TokioFramed};
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{
    interval, interval_at, sleep, sleep_until, timeout, Instant, MissedTickBehavior,
};
use tokio_rustls::client::TlsStream;

use super::clipboard::{self, Clipboard, Step, PASTE_LIMIT};
use super::connect::{login, size_supported, Connected, Target};
use super::err;
use super::input::{Input, Translator};
use crate::commands::ssh_cmds::KnownHostsState;
use crate::error::{AppError, AppResult};
use crate::remote::FrameOp;

pub(crate) const CONNECT_LIMIT: Duration = Duration::from_secs(10);
const REACTIVATE_LIMIT: Duration = Duration::from_secs(10);
pub(crate) const CLOSE_LIMIT: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(16);
const MAX_POINTER: u16 = 128;
const MAX_PENDING: usize = 8;
const MAX_EVENTS: usize = 255;
const CLIPBOARD_TICK: Duration = Duration::from_secs(5);
const RESIZE_LIMIT: Duration = Duration::from_secs(5);
// Windows and xrdp refuse a monitor layout while their login goes on, which on the test servers
// ends about 0.5 s and 0.3 s after the open, so the first request is asked again this soon.
const FIRST_RESIZE_LIMIT: Duration = Duration::from_secs(1);
const MAX_CHANNEL_MESSAGE: usize = 16 << 20;
const MAX_DYNAMIC_MESSAGE: usize = 64 << 10;
const HEADER_SIZE: usize = 8;
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

pub struct Session {
    pub width: u16,
    pub height: u16,
    pub(crate) opened: Instant,
    commands: mpsc::Sender<Command>,
    acks: Arc<Semaphore>,
    buttons: AtomicU16,
    task: Mutex<JoinSet<()>>,
    skipped: Arc<std::sync::Mutex<BTreeMap<String, usize>>>,
}

pub(crate) enum Command {
    Close,
    Input(Vec<Input>),
    Clipboard(Option<String>),
    Resize(u16, u16, u32),
}

enum Ending {
    Lost,
    Ended,
    Garbled,
    Resized,
}

enum ServerEnd {
    Info(ErrorInfo),
    Disconnect,
}

// IronRDP keeps the parts of a dynamic channel message until the length the first part declares,
// whatever that length is, so what each may still take is counted here. Every PDU is a static
// channel message of its own; `current` is the dynamic channel of the one being read.
#[derive(Default)]
struct Dynamic {
    channel: Option<u16>,
    current: Option<u32>,
    reading: bool,
    left: HashMap<u32, usize>,
}

impl Session {
    pub async fn connect(
        target: &Target<'_>,
        known: &KnownHostsState,
        sink: impl FnMut(FrameOp) + Send + 'static,
        sent: &AtomicBool,
    ) -> AppResult<Session> {
        Self::connect_within(target, known, sink, CONNECT_LIMIT, sent).await
    }

    async fn connect_within(
        target: &Target<'_>,
        known: &KnownHostsState,
        sink: impl FnMut(FrameOp) + Send + 'static,
        limit: Duration,
        sent: &AtomicBool,
    ) -> AppResult<Session> {
        let host = crate::net::normalize_host(target.host);
        let tcp = timeout(limit, TcpStream::connect((host.as_str(), target.port)))
            .await
            .map_err(|_| err("connection timed out"))??;
        tcp.set_nodelay(true)?;
        keep_alive(&tcp)?;
        let local = tcp.local_addr()?;
        let connected = login(tcp, local, target, known, attach, sent).await?;
        Ok(Self::from_connected(connected, sink))
    }

    pub(crate) fn from_connected<S>(
        connected: Connected<S>,
        sink: impl FnMut(FrameOp) + Send + 'static,
    ) -> Session
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        let (first, tick) = (Some(FIRST_RESIZE_LIMIT), CLIPBOARD_TICK);
        Self::from_connected_within(connected, sink, RESIZE_LIMIT, first, tick, PASTE_LIMIT)
    }

    fn from_connected_within<S>(
        connected: Connected<S>,
        sink: impl FnMut(FrameOp) + Send + 'static,
        resize_limit: Duration,
        first_resize_limit: Option<Duration>,
        clipboard_tick: Duration,
        paste_limit: Duration,
    ) -> Session
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    {
        let Connected { framed, result } = connected;
        let DesktopSize { width, height } = result.desktop_size;
        let unfinished = result.static_channels.channel_ids().map(|id| (id, 0)).collect();
        let max_update = update_limit(result.max_request_size, width, height);
        let dynamic = Dynamic {
            channel: result.static_channels.get_channel_id_by_type::<DrdynvcClient>(),
            ..Dynamic::default()
        };
        let (reader, writer) = split_tokio_framed(framed);
        let mut stage = ActiveStageBuilder {
            static_channels: result.static_channels,
            user_channel_id: result.user_channel_id,
            io_channel_id: result.io_channel_id,
            message_channel_id: result.message_channel_id,
            share_id: result.share_id,
            compression_type: result.compression_type,
            enable_server_pointer: result.enable_server_pointer,
            pointer_software_rendering: result.pointer_software_rendering,
        }
        .build();
        let clipboard = Clipboard::take(&mut stage, paste_limit);
        let skipped = Arc::default();
        let running = Task {
            reader,
            writer,
            stage,
            image: DecodedImage::new(PixelFormat::RgbA32, width, height),
            factory: result.activation_factory,
            io_channel_id: result.io_channel_id,
            pending: Vec::new(),
            unfinished,
            dynamic,
            fragmented: None,
            max_update,
            pointer: None,
            size: None,
            unacked: 0,
            info: None,
            closing: false,
            translator: Translator::new(),
            clipboard,
            text: None,
            held: Vec::new(),
            drawn: false,
            framing: false,
            hold: None,
            wanted: None,
            asked: None,
            again: None,
            resize_limit,
            first_resize_limit,
            first_again: false,
            clipboard_tick,
            skipped: Arc::clone(&skipped),
            sink,
        };

        let (commands, messages) = mpsc::channel(256);
        let acks = Arc::new(Semaphore::new(0));
        let mut task = JoinSet::new();
        task.spawn(running.run(messages, acks.clone()));
        let buttons = AtomicU16::new(0);
        let task = Mutex::new(task);
        Session { width, height, opened: Instant::now(), commands, acks, buttons, task, skipped }
    }

    // For the live tests: how often each PDU that IronRDP has no use for was skipped.
    pub fn skipped(&self) -> BTreeMap<String, usize> {
        self.skipped.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    // The view has applied everything up to one more `Sync`.
    pub fn ack(&self) {
        self.acks.add_permits(1);
    }

    pub async fn input(&self, events: Vec<Input>) -> AppResult<()> {
        let mut buttons = self.buttons.load(Ordering::Relaxed);
        let mut moves = true;
        for event in &events {
            moves &= match event {
                Input::Pointer { buttons: next, .. } => {
                    let same = *next == buttons;
                    buttons = *next;
                    same
                }
                _ => false,
            };
        }
        if events.is_empty() {
            return Ok(());
        }
        let command = Command::Input(events);
        if moves {
            // Only moves: the next one says where the pointer is, so it does not wait for room.
            return match self.commands.try_send(command) {
                Err(TrySendError::Closed(_)) => Err(closed()),
                _ => Ok(()),
            };
        }
        self.commands.send(command).await.map_err(|_| closed())?;
        // Only now: a press that was never queued must not count as held.
        self.buttons.store(buttons, Ordering::Relaxed);
        Ok(())
    }

    pub async fn clipboard(&self, text: String) -> AppResult<()> {
        let text = clipboard::to_remote(&text);
        self.commands.send(Command::Clipboard(text)).await.map_err(|_| closed())
    }

    pub async fn resize(&self, width: u16, height: u16, scale: u32) -> AppResult<()> {
        self.commands.send(Command::Resize(width, height, scale)).await.map_err(|_| closed())
    }

    pub async fn close(&self) {
        let mut task = self.task.lock().await;
        // The task writes the shutdown request, then reads until the server ends the connection.
        let finished = async {
            if self.commands.send(Command::Close).await.is_ok() {
                task.join_next().await;
            }
        };
        tokio::select! {
            _ = finished => {}
            _ = sleep(CLOSE_LIMIT) => {}
        }
        task.shutdown().await;
    }
}

fn keep_alive(tcp: &TcpStream) -> std::io::Result<()> {
    // Windows sends 10 probes unless told otherwise: 130 s for a link that is gone.
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(2);
    SockRef::from(tcp).set_tcp_keepalive(&keepalive)
}

struct Task<S, F> {
    reader: TokioFramed<ReadHalf<TlsStream<S>>>,
    writer: TokioFramed<WriteHalf<TlsStream<S>>>,
    stage: ActiveStage,
    image: DecodedImage,
    factory: ConnectionActivationFactory,
    io_channel_id: u16,
    pending: Vec<InclusiveRectangle>,
    unfinished: HashMap<u16, usize>,
    dynamic: Dynamic,
    fragmented: Option<usize>,
    max_update: usize,
    pointer: Option<FrameOp>,
    size: Option<(u16, u16)>,
    unacked: u32,
    info: Option<ErrorInfo>,
    closing: bool,
    translator: Translator,
    clipboard: Clipboard,
    text: Option<String>,
    held: Vec<Vec<u8>>,
    drawn: bool,
    framing: bool,
    hold: Option<Instant>,
    wanted: Option<(u32, u32, Option<u32>)>,
    asked: Option<Instant>,
    again: Option<(u32, u32, Option<u32>)>,
    resize_limit: Duration,
    first_resize_limit: Option<Duration>,
    first_again: bool,
    clipboard_tick: Duration,
    skipped: Arc<std::sync::Mutex<BTreeMap<String, usize>>>,
    sink: F,
}

impl<S, F> Task<S, F>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
    F: FnMut(FrameOp),
{
    async fn run(mut self, mut commands: mpsc::Receiver<Command>, acks: Arc<Semaphore>) {
        // IronRDP panics on some rectangles a server can send.
        let ending = match AssertUnwindSafe(self.drive(&mut commands, &acks)).catch_unwind().await {
            Ok(Err(ending)) => ending,
            Err(_) => Ending::Garbled,
        };
        // `close` asked for the end, and the view is told nothing.
        if !self.closing {
            let text = words(&ending, self.info);
            (self.sink)(FrameOp::Closed(text));
        }
    }

    async fn drive(
        &mut self,
        commands: &mut mpsc::Receiver<Command>,
        acks: &Semaphore,
    ) -> Result<Infallible, Ending> {
        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut timeouts = interval_at(Instant::now() + self.clipboard_tick, self.clipboard_tick);
        timeouts.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let asked = self.asked;
            let held = self.hold.map(|since| since + self.resize_limit);
            // With two flushes unacknowledged nothing is sent, but reading goes on and what
            // arrives merges into the pending changes.
            let changed = !self.pending.is_empty()
                || self.pointer.is_some()
                || self.size.is_some()
                || self.text.is_some();
            let flushable = !self.closing && self.unacked < 2 && changed;
            tokio::select! {
                frame = self.reader.read_pdu() => {
                    let (action, frame) = frame.map_err(|e| read_ending(&e))?;
                    self.frame(action, &frame).await?;
                    for frame in std::mem::take(&mut self.held) {
                        self.frame(Action::X224, &frame).await?;
                    }
                    self.clipboard(Step::Events).await?;
                    self.resize().await?;
                }
                Some(command) = commands.recv() => self.command(command).await?,
                Ok(ack) = acks.acquire() => {
                    ack.forget();
                    self.unacked = self.unacked.saturating_sub(1);
                }
                _ = tick.tick(), if flushable => self.flush(),
                _ = timeouts.tick() => self.clipboard(Step::Timeouts).await?,
                _ = sleep_until(asked.unwrap_or_else(Instant::now)), if asked.is_some() => {
                    self.asked = None;
                    self.wanted = self.wanted.or(self.again);
                    self.resize().await?;
                }
                _ = sleep_until(held.unwrap_or_else(Instant::now)), if held.is_some() => {
                    self.resize().await?;
                }
            }
        }
    }

    async fn command(&mut self, command: Command) -> Result<(), Ending> {
        match command {
            Command::Close => {
                self.closing = true;
                self.pending.clear();
                let shutdown = self.stage.graceful_shutdown().map_err(|_| Ending::Garbled)?;
                self.outputs(shutdown).await
            }
            Command::Clipboard(text) => self.clipboard(Step::Copy(text)).await,
            Command::Resize(width, height, scale) => {
                let (width, height) =
                    MonitorLayoutEntry::adjust_display_size(width.into(), height.into());
                // Adjusted sides are 8192 at most, so they fit.
                self.wanted = size_supported(width as u16, height as u16)
                    .then(|| (width, height, Some(scale).filter(|s| (100..=500).contains(s))));
                self.resize().await
            }
            Command::Input(events) => {
                if self.closing {
                    return Ok(());
                }
                let (right, bottom) = (self.image.width() - 1, self.image.height() - 1);
                let events: Vec<_> = events
                    .into_iter()
                    .map(|event| match event {
                        Input::Pointer { buttons, x, y } => {
                            Input::Pointer { buttons, x: x.min(right), y: y.min(bottom) }
                        }
                        other => other,
                    })
                    .flat_map(|event| self.translator.apply(event))
                    .collect();
                for chunk in events.chunks(MAX_EVENTS) {
                    let sent = self.stage.process_fastpath_input(&mut self.image, chunk);
                    self.outputs(sent.map_err(|_| Ending::Garbled)?).await?;
                }
                Ok(())
            }
        }
    }

    async fn clipboard(&mut self, step: Step) -> Result<(), Ending> {
        let step = if self.closing { Step::Discard } else { step };
        let frames = self.clipboard.step(&mut self.stage, step, &mut self.text);
        let frames = frames.map_err(|_| Ending::Garbled)?;
        if !frames.is_empty() {
            self.writer.write_all(&frames).await.map_err(|_| Ending::Lost)?;
        }
        Ok(())
    }

    // One request at a time: what is wanted waits for the answer, or for the limit. It also waits
    // for the first picture and for the end of a frame the server has begun: xrdp 0.10.1 crashes on
    // a frame acknowledgement that reaches it after a layout it has begun to apply. A server that
    // sends no picture, or a frame end this client cannot see, would hold it for good, so the hold
    // ends at the limit.
    async fn resize(&mut self) -> Result<(), Ending> {
        let Some((width, height, scale)) = self.wanted else {
            self.hold = None;
            return Ok(());
        };
        let ready = self
            .stage
            .get_dvc::<DisplayControlClient>()
            .filter(|dvc| dvc.is_open())
            .and_then(|dvc| dvc.channel_processor_downcast_ref::<DisplayControlClient>())
            .is_some_and(DisplayControlClient::ready);
        if self.closing || self.asked.is_some() || !ready {
            self.hold = None;
            return Ok(());
        }
        if (width, height) == (self.image.width().into(), self.image.height().into()) {
            self.hold = None;
            self.wanted = None;
            self.first_resize_limit = None;
            return Ok(());
        }
        if !self.drawn || self.framing {
            let since = *self.hold.get_or_insert_with(Instant::now);
            if Instant::now() < since + self.resize_limit {
                return Ok(());
            }
        }
        self.hold = None;
        self.wanted = None;
        if let Some(frame) = self.stage.encode_resize(width, height, scale, None) {
            let frame = frame.map_err(|_| Ending::Garbled)?;
            self.writer.write_all(&frame).await.map_err(|_| Ending::Lost)?;
            let first = self.first_resize_limit.take();
            self.asked = Some(Instant::now() + first.unwrap_or(self.resize_limit));
            // The same request twice in a row is the repeat: a server that ignores it twice is
            // left at the size it keeps. The first request's early ask again is not that repeat.
            let request = Some((width, height, scale));
            let early = std::mem::replace(&mut self.first_again, first.is_some());
            if !(early && self.again == request) {
                self.again = request.filter(|_| self.again != request);
            }
        }
        Ok(())
    }

    async fn frame(&mut self, action: Action, frame: &[u8]) -> Result<(), Ending> {
        note(peek(action, frame, self.io_channel_id), &mut self.info)?;
        count_chunk(&mut self.unfinished, &mut self.dynamic, action, frame)?;
        count_fragments(&mut self.fragmented, self.max_update, action, frame)?;
        if short_deactivate_all(action, frame, self.io_channel_id) {
            return self.reactivate().await;
        }
        if let Some(open) = frame_open(action, frame) {
            self.framing = open;
        }
        match self.stage.process(&mut self.image, action, frame) {
            Ok(outputs) => self.outputs(outputs).await,
            Err(e) if unhandled(&e) => {
                if let SessionErrorKind::Reason(reason) = e.kind() {
                    let mut counts = self.skipped.lock().unwrap_or_else(PoisonError::into_inner);
                    *counts.entry(reason.clone()).or_default() += 1;
                }
                Ok(())
            }
            Err(_) => Err(Ending::Garbled),
        }
    }

    async fn outputs(&mut self, outputs: Vec<ActiveStageOutput>) -> Result<(), Ending> {
        for output in outputs {
            match output {
                ActiveStageOutput::ResponseFrame(frame) => {
                    if !frame.is_empty() {
                        self.writer.write_all(&frame).await.map_err(|_| Ending::Lost)?;
                    }
                }
                ActiveStageOutput::GraphicsUpdate(rect) => {
                    self.drawn = true;
                    merge(&mut self.pending, rect, self.image.width(), self.image.height());
                }
                ActiveStageOutput::DeactivateAll => self.reactivate().await?,
                ActiveStageOutput::Terminate(_) => return Err(Ending::Ended),
                ActiveStageOutput::MultitransportRequest(_) | ActiveStageOutput::AutoDetect(_) => {}
                pointer => {
                    if let Some(op) = pointer_op(&pointer) {
                        self.pointer = Some(op);
                    }
                }
            }
        }
        Ok(())
    }

    fn flush(&mut self) {
        if let Some((w, h)) = self.size.take() {
            (self.sink)(FrameOp::Resize { w, h });
        }
        if let Some(op) = self.pointer.take() {
            (self.sink)(op);
        }
        for rect in std::mem::take(&mut self.pending) {
            let (x, y, w, h) = (rect.left, rect.top, rect.width(), rect.height());
            let rgba = copy_region(&self.image, &rect);
            (self.sink)(FrameOp::Rect { x, y, w, h, rgba });
        }
        if let Some(text) = self.text.take() {
            (self.sink)(FrameOp::Clipboard(text));
        }
        (self.sink)(FrameOp::Sync);
        self.unacked += 1;
    }

    async fn reactivate(&mut self) -> Result<(), Ending> {
        let mut sequence = self.factory.create();
        let mut buf = WriteBuf::new();
        let (reader, writer, info) = (&mut self.reader, &mut self.writer, &mut self.info);
        let held = &mut self.held;
        let io_channel_id = self.io_channel_id;
        let stepping = async {
            let mut held_bytes = 0;
            loop {
                buf.clear();
                // Read here and not by `single_sequence_step_read`, so that `peek` sees the frame.
                let written = match sequence.next_pdu_hint() {
                    Some(hint) => {
                        let frame = reader.read_by_hint(hint).await.map_err(|e| read_ending(&e))?;
                        let first = frame.first();
                        if let Some(Ok(action)) = first.map(|b| Action::from_fp_output_header(*b)) {
                            note(peek(action, &frame, io_channel_id), info)?;
                        }
                        // The static channels go on while the server reactivates, and what they
                        // carry is handled once it is done.
                        let indication = ironrdp_pdu::mcs::decode_send_data_indication(&frame);
                        if indication.is_ok_and(|context| context.channel_id != io_channel_id) {
                            held_bytes += frame.len();
                            if held_bytes > MAX_CHANNEL_MESSAGE {
                                return Err(Ending::Garbled);
                            }
                            held.push(frame.to_vec());
                            continue;
                        }
                        sequence.step(&frame, &mut buf)
                    }
                    None => sequence.step_no_input(&mut buf),
                };
                let written = written.map_err(|_| Ending::Garbled)?;
                if written.size().is_some() {
                    writer.write_all(buf.filled()).await.map_err(|_| Ending::Lost)?;
                }
                if let ConnectionActivationState::Finalized {
                    desktop_size,
                    share_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                } = sequence.connection_activation_state()
                {
                    return Ok::<_, Ending>((
                        desktop_size,
                        share_id,
                        enable_server_pointer,
                        pointer_software_rendering,
                    ));
                }
            }
        };
        let (size, share_id, enable_server_pointer, pointer_software_rendering) =
            timeout(REACTIVATE_LIMIT, stepping).await.map_err(|_| Ending::Lost)??;

        if !size_supported(size.width, size.height) {
            return Err(Ending::Resized);
        }
        // The old picture goes first, so that both are never in memory together.
        self.image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        self.image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
        let processor = fast_path::ProcessorBuilder {
            io_channel_id: sequence.io_channel_id(),
            user_channel_id: sequence.user_channel_id(),
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            bulk_decompressor: None,
        };
        self.stage.set_fastpath_processor(processor.build());
        self.fragmented = None;
        self.max_update = update_limit(sequence.max_request_size(), size.width, size.height);
        self.stage.set_share_id(share_id);
        self.stage.set_enable_server_pointer(enable_server_pointer);
        self.pending.clear();
        self.size = Some((size.width, size.height));
        self.asked = None;
        self.again = None;
        self.first_again = false;
        self.drawn = false;
        self.framing = false;
        self.hold = None;
        Ok(())
    }
}

fn attach(connector: ClientConnector) -> ClientConnector {
    let display = DisplayControlClient::new(|_| Ok(Vec::new()));
    let channels = DrdynvcClient::new().with_dynamic_channel(display);
    clipboard::attach(connector).with_static_channel(channels)
}

fn closed() -> AppError {
    AppError::NotFound("rdp session closed".into())
}

// IronRDP appends the chunks of a static channel message until the last one and never holds them to
// the declared length, so the bytes of each unfinished message are counted here. A first chunk does
// not start the count again, because IronRDP keeps what it has.
fn count_chunk(
    unfinished: &mut HashMap<u16, usize>,
    dynamic: &mut Dynamic,
    action: Action,
    frame: &[u8],
) -> Result<(), Ending> {
    if action != Action::X224 {
        return Ok(());
    }
    let Ok(context) = ironrdp_pdu::mcs::decode_send_data_indication(frame) else {
        return Ok(());
    };
    let Some(total) = unfinished.get_mut(&context.channel_id) else {
        return Ok(());
    };
    let Ok(header) = ironrdp_core::decode::<ChannelPduHeader>(context.user_data) else {
        return Ok(());
    };
    *total = total.saturating_add(context.user_data.len().saturating_sub(HEADER_SIZE));
    if *total > MAX_CHANNEL_MESSAGE {
        return Err(Ending::Garbled);
    }
    let last = header.flags.contains(ChannelControlFlags::FLAG_LAST);
    if last {
        *total = 0;
    }
    if dynamic.channel == Some(context.channel_id) {
        count_dynamic(dynamic, &context.user_data[HEADER_SIZE..], last)?;
    }
    Ok(())
}

// The connector advertises back whatever the server advertised, so a server could allow itself any
// size. One uncompressed 32-bit frame of the desktop and 1 MiB for the headers around it is the
// most an update needs.
fn update_limit(advertised: u32, width: u16, height: u16) -> usize {
    (advertised as usize).min(usize::from(width) * usize::from(height) * 4 + (1 << 20))
}

// IronRDP keeps the fragments of a fast-path update until the last one, however many come, so their
// bytes are counted here against the size the client advertised. Like IronRDP, a first or a single
// update drops what was kept, and a fragment with nothing before it is not kept. Compression is not
// negotiated, so what IronRDP keeps is what was read.
fn count_fragments(
    fragmented: &mut Option<usize>,
    limit: usize,
    action: Action,
    frame: &[u8],
) -> Result<(), Ending> {
    if action != Action::FastPath {
        return Ok(());
    }
    let mut cursor = ReadCursor::new(frame);
    if ironrdp_core::decode_cursor::<FastPathHeader>(&mut cursor).is_err() {
        return Ok(());
    }
    while !cursor.is_empty() {
        let Ok(update) = ironrdp_core::decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor) else {
            return Ok(());
        };
        let size = update.data.len();
        match update.fragmentation {
            Fragmentation::Single => *fragmented = None,
            Fragmentation::First => *fragmented = Some(size),
            Fragmentation::Next | Fragmentation::Last => {
                if let Some(total) = fragmented {
                    *total += size;
                }
            }
        }
        if fragmented.is_some_and(|total| total > limit) {
            return Err(Ending::Garbled);
        }
        if update.fragmentation == Fragmentation::Last {
            *fragmented = None;
        }
    }
    Ok(())
}

// Whether the server's frame is open after this one, when it carries a frame marker.
fn frame_open(action: Action, frame: &[u8]) -> Option<bool> {
    if action != Action::FastPath {
        return None;
    }
    let mut cursor = ReadCursor::new(frame);
    ironrdp_core::decode_cursor::<FastPathHeader>(&mut cursor).ok()?;
    let mut open = None;
    while !cursor.is_empty() {
        let Ok(update) = ironrdp_core::decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor) else {
            break;
        };
        if update.fragmentation != Fragmentation::Single
            || update.update_code != UpdateCode::SurfaceCommands
        {
            continue;
        }
        let Ok(FastPathUpdate::SurfaceCommands(commands)) =
            FastPathUpdate::decode_with_code(update.data, update.update_code)
        else {
            continue;
        };
        for command in commands {
            if let SurfaceCommand::FrameMarker(marker) = command {
                open = Some(marker.frame_action == FrameAction::Begin);
            }
        }
    }
    open
}

// IronRDP reads a static channel message from the chunk after a last one, whatever its flags say.
fn count_dynamic(dynamic: &mut Dynamic, chunk: &[u8], last: bool) -> Result<(), Ending> {
    let mut taken = chunk.len();
    if !dynamic.reading {
        dynamic.current = None;
        match ironrdp_core::decode::<DrdynvcServerPdu>(chunk) {
            Ok(DrdynvcServerPdu::Data(pdu)) => {
                let id = pdu.channel_id();
                match &pdu {
                    DrdynvcDataPdu::DataFirst(first) => {
                        let length = first.length() as usize;
                        if length > MAX_DYNAMIC_MESSAGE {
                            return Err(Ending::Garbled);
                        }
                        dynamic.left.insert(id, length);
                        taken = first.data().len();
                    }
                    DrdynvcDataPdu::Data(data) => taken = data.data().len(),
                }
                dynamic.current = Some(id).filter(|id| dynamic.left.contains_key(id));
            }
            Ok(DrdynvcServerPdu::Close(close)) => {
                dynamic.left.remove(&close.channel_id());
            }
            // The length could be in the part of the start that has not come yet.
            Err(e) if !last && matches!(e.kind(), DecodeErrorKind::NotEnoughBytes { .. }) => {
                return Err(Ending::Garbled);
            }
            Ok(_) | Err(_) => {}
        }
    }
    dynamic.reading = !last;
    let Some(id) = dynamic.current else {
        return Ok(());
    };
    let left = dynamic.left.get_mut(&id).ok_or(Ending::Garbled)?;
    *left = left.checked_sub(taken).ok_or(Ending::Garbled)?;
    if last && *left == 0 {
        dynamic.left.remove(&id);
    }
    Ok(())
}

// ActiveStage only gives the error info as text, and turns a server MCS disconnect into a decode
// error, so both are read from the frame first.
fn peek(action: Action, frame: &[u8], io_channel_id: u16) -> Option<ServerEnd> {
    if action != Action::X224 {
        return None;
    }
    if let Ok(X224(McsMessage::DisconnectProviderUltimatum(_))) =
        ironrdp_core::decode::<X224<McsMessage<'_>>>(frame)
    {
        return Some(ServerEnd::Disconnect);
    }
    let context = ironrdp_pdu::mcs::decode_send_data_indication(frame).ok()?;
    if context.channel_id != io_channel_id {
        return None;
    }
    let user_data = context.user_data;
    match ironrdp_pdu::rdp::headers::decode_io_channel(context) {
        Ok(IoChannelPdu::Data(data)) => match data.pdu {
            ShareDataPdu::ServerSetErrorInfo(ServerSetErrorInfoPdu(info))
                if info != ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::None) =>
            {
                Some(ServerEnd::Info(info))
            }
            _ => None,
        },
        Ok(_) => None,
        // Servers send codes that IronRDP does not list, and it fails to decode those.
        Err(_) if unlisted_error_info(user_data) => Some(ServerEnd::Disconnect),
        Err(_) => None,
    }
}

fn unlisted_error_info(user_data: &[u8]) -> bool {
    // The share control header, the share id and the rest of the share data header come first.
    const CODE_AT: usize = 18;
    const SET_ERROR_INFO: u8 = 0x2f;
    const DATA_PDU: u8 = 7;

    let data = user_data.get(2).is_some_and(|kind| kind & 0xf == DATA_PDU);
    let info = user_data.get(14) == Some(&SET_ERROR_INFO);
    let code = user_data.get(CODE_AT..CODE_AT + 4);
    data && info && code.is_some_and(|code| code.iter().any(|byte| *byte != 0))
}

// xrdp sends the Deactivate All PDU as the share control header alone, without the share id that
// IronRDP needs to decode it.
fn short_deactivate_all(action: Action, frame: &[u8], io_channel_id: u16) -> bool {
    const TOTAL_LENGTH: u8 = 6;
    const DEACTIVATE_ALL: u8 = 0x16;

    if action != Action::X224 {
        return false;
    }
    let Ok(context) = ironrdp_pdu::mcs::decode_send_data_indication(frame) else {
        return false;
    };
    let user_data = context.user_data;
    context.channel_id == io_channel_id
        && user_data.len() == 6
        && user_data[..4] == [TOTAL_LENGTH, 0, DEACTIVATE_ALL, 0]
}

// A frame header that makes no sense is the server's fault, not a lost connection.
fn read_ending(e: &std::io::Error) -> Ending {
    if e.get_ref().is_some_and(|cause| cause.is::<ironrdp_core::DecodeError>()) {
        Ending::Garbled
    } else {
        Ending::Lost
    }
}

fn note(end: Option<ServerEnd>, info: &mut Option<ErrorInfo>) -> Result<(), Ending> {
    match end {
        Some(ServerEnd::Disconnect) => Err(Ending::Ended),
        Some(ServerEnd::Info(found)) => {
            *info = Some(found);
            Ok(())
        }
        None => Ok(()),
    }
}

// IronRDP ends the session on PDUs it has no use for, such as a monitor layout or a sound.
fn unhandled(e: &SessionError) -> bool {
    matches!(e.kind(), SessionErrorKind::Reason(reason) if reason.starts_with("unhandled PDU"))
}

fn pointer_op(output: &ActiveStageOutput) -> Option<FrameOp> {
    let hidden = || FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
    match output {
        ActiveStageOutput::PointerBitmap(pointer) => {
            let (w, h) = (pointer.width, pointer.height);
            if w == 0 || h == 0 {
                return Some(hidden());
            }
            if w > MAX_POINTER || h > MAX_POINTER {
                return Some(FrameOp::CursorDefault);
            }
            Some(FrameOp::Cursor {
                hot_x: pointer.hotspot_x.min(w - 1),
                hot_y: pointer.hotspot_y.min(h - 1),
                w,
                h,
                rgba: pointer.bitmap_data.clone(),
            })
        }
        ActiveStageOutput::PointerHidden => Some(hidden()),
        ActiveStageOutput::PointerDefault => Some(FrameOp::CursorDefault),
        _ => None,
    }
}

fn words(ending: &Ending, info: Option<ErrorInfo>) -> String {
    if let Some(info) = info {
        return info_words(info);
    }
    match ending {
        Ending::Lost => "network connection lost",
        Ending::Ended => "the server ended the session",
        Ending::Garbled => "the server sent data this client does not understand",
        Ending::Resized => "the server chose an unsupported screen size",
    }
    .to_owned()
}

fn info_words(info: ErrorInfo) -> String {
    use ProtocolIndependentCode as Code;

    let words = match info {
        ErrorInfo::ProtocolIndependentCode(code) => match code {
            Code::LogoffByUser | Code::RpcInitiatedLogoff => "logged off",
            Code::DisconnectedByOtherconnection => "another user connected to this session",
            Code::IdleTimeout => "the session was idle for too long",
            Code::LogonTimeout => "the session time limit was reached",
            Code::RpcInitiatedDisconnect | Code::RpcInitiatedDisconnectByuser => {
                "disconnected by an administrator"
            }
            Code::ServerDeniedConnection => "the server denied the connection",
            Code::ServerInsufficientPrivileges => "this account may not sign in here",
            Code::ServerFreshCredentialsRequired => "the server asks for the password again",
            Code::OutOfMemory => "the server ran out of memory",
            _ => return described(info),
        },
        ErrorInfo::ProtocolIndependentLicensingCode(_) => {
            "the server has no licence for this connection"
        }
        _ => return described(info),
    };
    words.to_owned()
}

// IronRDP starts its description with the kind of code in brackets.
fn described(info: ErrorInfo) -> String {
    let text = info.description();
    match text.split_once(']') {
        Some((_, rest)) => rest.trim_start_matches(':').trim().to_owned(),
        None => text,
    }
}

// The list stays short and never adds up to more than the image.
fn merge(pending: &mut Vec<InclusiveRectangle>, rect: InclusiveRectangle, width: u16, height: u16) {
    let image = InclusiveRectangle { left: 0, top: 0, right: width - 1, bottom: height - 1 };
    let Some(rect) = rect.intersect(&image) else {
        return;
    };
    pending.push(rect);
    let area = |rect: &InclusiveRectangle| u64::from(rect.width()) * u64::from(rect.height());
    let total: u64 = pending.iter().map(area).sum();
    if pending.len() > MAX_PENDING || total > u64::from(width) * u64::from(height) {
        *pending = vec![InclusiveRectangle::union_all(pending)];
    }
}

// Only the rows of the rectangle, not the span of the image between them. The image does not
// keep a reliable alpha, and the view draws what it is given.
fn copy_region(image: &DecodedImage, rect: &InclusiveRectangle) -> Vec<u8> {
    let bytes = image.bytes_per_pixel();
    let row = usize::from(rect.width()) * bytes;
    let mut rgba = Vec::with_capacity(row * usize::from(rect.height()));
    for y in usize::from(rect.top)..=usize::from(rect.bottom) {
        let start = y * image.stride() + usize::from(rect.left) * bytes;
        rgba.extend_from_slice(&image.data()[start..start + row]);
    }
    for pixel in rgba.chunks_exact_mut(bytes) {
        pixel[3] = 255;
    }
    rgba
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::future::Future;
    use std::net::SocketAddr;
    use std::sync::atomic::Ordering;
    use std::sync::OnceLock;
    use std::time::Instant;

    use ironrdp_cliprdr::CliprdrClient;
    use ironrdp_connector::connection_activation::DEFAULT_MAX_REQUEST_SIZE;
    use ironrdp_dvc::pdu::{ClosePdu, CreateRequestPdu, DataFirstPdu, DataPdu};
    use ironrdp_graphics::pointer::DecodedPointer;
    use ironrdp_pdu::fast_path::{Fragmentation, UpdateCode};
    use ironrdp_pdu::geometry::Rectangle as _;
    use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
    use ironrdp_pdu::input::mouse::PointerFlags;
    use ironrdp_pdu::input::mouse_x::PointerXFlags;
    use ironrdp_pdu::input::{MousePdu, MouseXPdu};
    use ironrdp_pdu::mcs::DisconnectReason;
    use ironrdp_pdu::rdp::server_error_info::{
        ProtocolIndependentCode as Code, ProtocolIndependentConnectionBrokerCode as Broker,
        ProtocolIndependentLicensingCode as Licence, RdpSpecificCode,
    };
    use ironrdp_pdu::rdp::vc::ChannelControlFlags;
    use ironrdp_server::{DisplayUpdate, RGBAPointer, ServerEvent};
    use ironrdp_tokio::FramedWrite as _;
    use tokio::io::{duplex, AsyncWriteExt as _, DuplexStream};
    use tokio::net::{TcpListener, TcpSocket};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::*;
    use crate::rdp::testserver::{
        accept_scripted, accept_scripted_keeping, accept_scripted_with, bitmap, certificate,
        channel_chunk, deactivate_all, error_info, fast_path_update, fragments, frame_marker,
        garbage, mcs_disconnect, monitor_layout, off_thread, play_sound, reactivate_scripted,
        read_client, short_deactivate_all, short_frames, shutdown_denied, surface_bits,
        surface_command, unlisted_error_info, Channels, ClientPdu, Login, Mouse, Options,
        ScriptedFramed, Seen, TestServer,
    };
    use crate::error::AppError;
    use crate::rdp::clipboard::MAX_TEXT;
    use crate::remote::trust::fingerprint;
    use crate::ssh::known_hosts::KnownHosts;

    const WAIT: Duration = Duration::from_secs(5);
    const SIZE: (u16, u16) = (256, 128);
    const HOST: &str = "desk.lan";
    const PORT: u16 = 3389;
    const LOST: &str = "network connection lost";
    const BAD_SIZE: &str = "the server chose an unsupported screen size";
    const NOT_UNDERSTOOD: &str = "the server sent data this client does not understand";

    type Ops = mpsc::UnboundedReceiver<FrameOp>;

    fn pinned(host: &str, port: u16) -> KnownHostsState {
        let dir = tempfile::tempdir().unwrap();
        let hosts = KnownHosts::load(dir.path().join("known_hosts.json")).unwrap();
        let known = KnownHostsState(Arc::new(std::sync::Mutex::new(hosts)));
        let print = fingerprint(&certificate());
        known.0.lock().unwrap().record(&format!("rdp/{host}"), port, &print).unwrap();
        known
    }

    fn target(host: &str, port: u16) -> Target<'_> {
        Target {
            host,
            port,
            username: "alice",
            domain: None,
            password: "secret",
            width: 1024,
            height: 768,
            scale: 100,
            keyboard_layout: 0x409,
        }
    }

    fn local() -> SocketAddr {
        "127.0.0.1:50000".parse().unwrap()
    }

    fn text(e: &AppError) -> &str {
        match e {
            AppError::Internal(message) => message,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    async fn within<T>(future: impl Future<Output = T>) -> T {
        timeout(WAIT, future).await.expect("timed out")
    }

    async fn next(ops: &mut Ops) -> FrameOp {
        timeout(WAIT, ops.recv()).await.expect("no operation arrived").expect("the session is gone")
    }

    // Everything the sink is still given until the session has ended and dropped it.
    async fn rest(ops: &mut Ops) -> Vec<FrameOp> {
        let mut left = Vec::new();
        while let Some(op) = timeout(WAIT, ops.recv()).await.expect("the session did not end") {
            left.push(op);
        }
        left
    }

    async fn quiet(ops: &mut Ops) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(ops.try_recv().is_err(), "an operation arrived");
    }

    // The picture the view keeps: opaque black, then whatever the operations paint.
    struct Screen {
        width: usize,
        rgba: Vec<u8>,
    }

    impl Screen {
        fn new(width: u16, height: u16) -> Self {
            let rgba = [0, 0, 0, 255].repeat(usize::from(width) * usize::from(height));
            Screen { width: usize::from(width), rgba }
        }

        fn paint(&mut self, x: u16, y: u16, w: u16, h: u16, pixel: impl Fn(u16, u16) -> [u8; 3]) {
            for row in 0..h {
                for column in 0..w {
                    let [r, g, b] = pixel(column, row);
                    let at = ((usize::from(y + row)) * self.width + usize::from(x + column)) * 4;
                    self.rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }

        fn apply(&mut self, op: &FrameOp) -> usize {
            let FrameOp::Rect { x, y, w, h, rgba } = op else {
                panic!("expected a rectangle, got {op:?}");
            };
            let (x, y, w, h) = (usize::from(*x), usize::from(*y), usize::from(*w), usize::from(*h));
            assert_eq!(rgba.len(), w * h * 4);
            assert!(rgba.chunks(4).all(|pixel| pixel[3] == 255), "a pixel is not opaque");
            for row in 0..h {
                let at = ((y + row) * self.width + x) * 4;
                self.rgba[at..at + w * 4].copy_from_slice(&rgba[row * w * 4..(row + 1) * w * 4]);
            }
            w * h
        }
    }

    // Applies the operations up to the next Sync; returns how many pixels they carried.
    async fn flush(ops: &mut Ops, screen: &mut Screen) -> usize {
        let mut pixels = 0;
        loop {
            match next(ops).await {
                FrameOp::Sync => return pixels,
                op => pixels += screen.apply(&op),
            }
        }
    }

    fn shade(n: u8) -> impl Fn(u16, u16) -> [u8; 3] {
        move |column, row| [n, (row * 3) as u8, (column * 2) as u8]
    }

    async fn rig(size: (u16, u16)) -> (TestServer, Session, Ops) {
        let (client, stream) = duplex(1 << 16);
        let server =
            TestServer::on(Options { login: Login::Tls, size, remotefx: false }, stream).await;
        let (session, ops) = join(client, &pinned(HOST, PORT)).await;
        (server, session, ops)
    }

    type Attach = fn(ClientConnector) -> ClientConnector;

    // The tick of the clipboard timeouts and how long a paste request may stay unanswered.
    type Clipboard = (Duration, Duration);

    const CLIPBOARD: Clipboard = (CLIPBOARD_TICK, PASTE_LIMIT);

    async fn logged_in(client: DuplexStream, known: &KnownHostsState) -> Connected<DuplexStream> {
        logged_in_with(client, known, |connector| connector).await
    }

    async fn logged_in_with(
        client: DuplexStream,
        known: &KnownHostsState,
        attach: Attach,
    ) -> Connected<DuplexStream> {
        let target = target(HOST, PORT);
        let sent = AtomicBool::default();
        let connected = within(login(client, local(), &target, known, attach, &sent)).await;
        connected.unwrap_or_else(|e| panic!("login failed: {e:?}"))
    }

    async fn join(client: DuplexStream, known: &KnownHostsState) -> (Session, Ops) {
        join_with(client, known, |connector| connector, CLIPBOARD).await
    }

    async fn join_with(
        client: DuplexStream,
        known: &KnownHostsState,
        attach: Attach,
        (tick, limit): Clipboard,
    ) -> (Session, Ops) {
        let connected = logged_in_with(client, known, attach).await;
        let (sink, ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        (Session::from_connected_within(connected, sink, RESIZE_LIMIT, None, tick, limit), ops)
    }

    // A session against a server whose frames the test writes; `script` gets the stream once the
    // connection sequence is over.
    async fn scripted<T, Fut>(
        size: (u16, u16),
        script: impl FnOnce(ScriptedFramed<DuplexStream>, Channels) -> Fut + Send + 'static,
    ) -> (JoinHandle<T>, Session, Ops)
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        scripted_with(size, |connector| connector, CLIPBOARD, script).await
    }

    async fn scripted_with<T, Fut>(
        size: (u16, u16),
        attach: Attach,
        clipboard: Clipboard,
        script: impl FnOnce(ScriptedFramed<DuplexStream>, Channels) -> Fut + Send + 'static,
    ) -> (JoinHandle<T>, Session, Ops)
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (framed, result) = accept_scripted(stream, size).await;
            script(framed, result).await
        });
        let (session, ops) = join_with(client, &pinned(HOST, PORT), attach, clipboard).await;
        (server, session, ops)
    }

    // The client may be gone already, which the script does not care about.
    async fn write(framed: &mut ScriptedFramed<DuplexStream>, frame: Vec<u8>) {
        framed.write_all(&frame).await.ok();
    }

    #[tokio::test]
    async fn a_pushed_bitmap_arrives_as_rects_and_one_sync() {
        let (server, _session, mut ops) = rig(SIZE).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        let gradient = |column: u16, row: u16| [200, (row * 6) as u8, (column * 5) as u8];
        expected.paint(16, 8, 48, 40, gradient);
        server.display.send(bitmap(16, 8, 48, 40, gradient)).unwrap();
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        flush(&mut ops, &mut screen).await;
        assert_eq!(screen.rgba, expected.rgba);
        quiet(&mut ops).await;
    }

    #[tokio::test]
    async fn without_acknowledgements_the_session_keeps_reading_and_merges() {
        let (server, session, mut ops) = rig(SIZE).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        let push = |n: u8, expected: &mut Screen| {
            let (x, y) = (u16::from(n) * 37 % 192, u16::from(n) * 13 % 96);
            expected.paint(x, y, 64, 32, shade(n));
            server.display.send(bitmap(x, y, 64, 32, shade(n))).unwrap();
        };
        for n in 0..2 {
            push(n, &mut expected);
            flush(&mut ops, &mut screen).await;
        }
        for n in 2..52 {
            push(n, &mut expected);
        }
        // 50 updates of 8 KiB do not fit the pipe, so the server only takes them all when the
        // session keeps reading.
        let started = Instant::now();
        while server.taken.load(Ordering::SeqCst) < 52 {
            assert!(started.elapsed() < Duration::from_secs(2), "the server was blocked");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        quiet(&mut ops).await;

        session.ack();
        session.ack();
        let image = usize::from(SIZE.0) * usize::from(SIZE.1);
        while screen.rgba != expected.rgba {
            let pixels = flush(&mut ops, &mut screen).await;
            assert!(pixels <= image, "a flush carried {pixels} pixels of {image}");
        }
        quiet(&mut ops).await;
    }

    #[tokio::test]
    async fn an_acknowledgement_given_inside_the_sink_is_not_lost() {
        let (client, stream) = duplex(1 << 16);
        let options = Options { login: Login::Tls, size: SIZE, remotefx: false };
        let server = TestServer::on(options, stream).await;
        let connected = logged_in(client, &pinned(HOST, PORT)).await;
        let slot = Arc::new(OnceLock::<Session>::new());
        let acking = slot.clone();
        let (sink, mut ops) = mpsc::unbounded_channel();
        let sink = move |op: FrameOp| {
            if op == FrameOp::Sync {
                acking.get().unwrap().ack();
            }
            sink.send(op).unwrap();
        };
        assert!(slot.set(Session::from_connected(connected, sink)).is_ok());

        let mut expected = Screen::new(SIZE.0, SIZE.1);
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        for n in 0..6 {
            expected.paint(n * 20, 0, 16, 16, shade(n as u8));
            server.display.send(bitmap(n * 20, 0, 16, 16, shade(n as u8))).unwrap();
            flush(&mut ops, &mut screen).await;
        }
        assert_eq!(screen.rgba, expected.rgba);
    }

    fn pointer_data() -> Vec<u8> {
        let mut data = [10, 20, 30, 255].repeat(8);
        data.extend([40, 50, 60, 128].repeat(56));
        data
    }

    #[tokio::test]
    async fn a_server_pointer_arrives_with_its_hotspot() {
        let (server, _session, mut ops) = rig(SIZE).await;
        let pointer = RGBAPointer {
            cache_index: 0,
            width: 8,
            height: 8,
            hot_x: 2,
            hot_y: 3,
            data: pointer_data(),
        };
        server.display.send(DisplayUpdate::RGBAPointer(pointer)).unwrap();
        let shape = match next(&mut ops).await {
            FrameOp::Cursor { hot_x: 2, hot_y: 3, w: 8, h: 8, rgba } => rgba,
            other => panic!("expected the pointer, got {other:?}"),
        };
        assert_eq!(shape.len(), 256);
        // The server sends BGRA bottom-up.
        assert_eq!(shape[..4], [60, 50, 40, 128]);
        assert_eq!(shape[252..], [30, 20, 10, 255]);
        assert_eq!(next(&mut ops).await, FrameOp::Sync);

        // The cache hit hides the cursor and shows the shape; a flush may take both or the last.
        server.display.send(DisplayUpdate::CachedPointer(0)).unwrap();
        let hidden = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
        let again = FrameOp::Cursor { hot_x: 2, hot_y: 3, w: 8, h: 8, rgba: shape };
        loop {
            match next(&mut ops).await {
                FrameOp::Sync => {}
                op if op == hidden => {}
                op => {
                    assert_eq!(op, again);
                    break;
                }
            }
        }
    }

    fn rgba_pointer(n: u8) -> DisplayUpdate {
        DisplayUpdate::RGBAPointer(RGBAPointer {
            cache_index: 0,
            width: 8,
            height: 8,
            hot_x: 2,
            hot_y: 3,
            data: [n, n, n, 255].repeat(64),
        })
    }

    // Whatever has arrived, grouped by flush; the last group has no Sync if it is unfinished.
    fn flushes(ops: &mut Ops) -> Vec<Vec<FrameOp>> {
        let mut groups = vec![Vec::new()];
        while let Ok(op) = ops.try_recv() {
            let end = op == FrameOp::Sync;
            groups.last_mut().unwrap().push(op);
            if end {
                groups.push(Vec::new());
            }
        }
        groups.retain(|group| !group.is_empty());
        groups
    }

    fn count(groups: &[Vec<FrameOp>], wanted: impl Fn(&FrameOp) -> bool) -> Vec<usize> {
        groups.iter().map(|group| group.iter().filter(|op| wanted(op)).count()).collect()
    }

    #[tokio::test]
    async fn pointer_updates_without_acknowledgements_come_once_per_flush() {
        let (server, session, mut ops) = rig(SIZE).await;
        for n in 0..200 {
            server.display.send(rgba_pointer(n)).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut groups = flushes(&mut ops);
        let is_cursor = |op: &FrameOp| matches!(op, FrameOp::Cursor { .. });
        assert!(groups.len() <= 2, "{} flushes", groups.len());
        assert!(count(&groups, is_cursor).iter().all(|n| *n <= 1), "{groups:?}");

        // Updates the server has not sent yet come after these flushes, and need more of them.
        let deadline = Instant::now() + WAIT;
        loop {
            session.ack();
            groups.extend(flushes(&mut ops));
            let cursors: Vec<_> = groups.iter().flatten().filter(|op| is_cursor(op)).collect();
            if let Some(FrameOp::Cursor { rgba, .. }) = cursors.last() {
                if rgba[..4] == [199, 199, 199, 255] {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the last pointer did not arrive");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn resizes_without_acknowledgements_come_at_most_twice_and_the_last_wins() {
        let (server, session, mut ops) = rig(SIZE).await;
        for n in 0..30u16 {
            let size = ironrdp_server::DesktopSize { width: 100 + n * 2, height: 64 };
            server.display.send(DisplayUpdate::Resize(size)).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(600)).await;
        let mut groups = flushes(&mut ops);
        let is_resize = |op: &FrameOp| matches!(op, FrameOp::Resize { .. });
        assert!(groups.len() <= 2, "{} flushes", groups.len());
        assert!(count(&groups, is_resize).iter().sum::<usize>() <= 2, "{groups:?}");

        // Updates the server has not sent yet come after these flushes, and need more of them.
        let last = FrameOp::Resize { w: 158, h: 64 };
        let deadline = Instant::now() + WAIT;
        loop {
            session.ack();
            groups.extend(flushes(&mut ops));
            let resizes: Vec<_> = groups.iter().flatten().filter(|op| is_resize(op)).collect();
            if resizes.last() == Some(&&last) {
                break;
            }
            assert!(Instant::now() < deadline, "the last size did not arrive: {resizes:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn a_pointer_waiting_for_an_acknowledgement_survives_a_resize() {
        let (server, session, mut ops) = rig(SIZE).await;
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        for n in 0..2 {
            server.display.send(bitmap(0, 0, 16, 16, shade(n))).unwrap();
            flush(&mut ops, &mut screen).await;
        }
        server.display.send(rgba_pointer(9)).unwrap();
        let size = ironrdp_server::DesktopSize { width: 128, height: 64 };
        server.display.send(DisplayUpdate::Resize(size)).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        quiet(&mut ops).await;
        session.ack();

        assert_eq!(next(&mut ops).await, FrameOp::Resize { w: 128, h: 64 });
        assert!(matches!(next(&mut ops).await, FrameOp::Cursor { w: 8, h: 8, .. }));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
    }

    #[tokio::test]
    async fn a_pointer_change_alone_is_flushed() {
        let (server, _session, mut ops) = rig(SIZE).await;
        server.display.send(rgba_pointer(5)).unwrap();
        assert!(matches!(next(&mut ops).await, FrameOp::Cursor { w: 8, h: 8, .. }));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        quiet(&mut ops).await;
    }

    #[tokio::test]
    async fn a_flush_carries_the_size_then_the_pointer_then_the_rectangles() {
        let (server, session, mut ops) = rig(SIZE).await;
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        for n in 0..2 {
            server.display.send(bitmap(0, 0, 16, 16, shade(n))).unwrap();
            flush(&mut ops, &mut screen).await;
        }
        // Two flushes are unacknowledged, so all three changes wait for the next one.
        let size = ironrdp_server::DesktopSize { width: 128, height: 64 };
        server.display.send(DisplayUpdate::Resize(size)).unwrap();
        server.display.send(rgba_pointer(9)).unwrap();
        server.display.send(bitmap(8, 8, 16, 16, shade(9))).unwrap();
        while server.taken.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        quiet(&mut ops).await;
        session.ack();

        assert_eq!(next(&mut ops).await, FrameOp::Resize { w: 128, h: 64 });
        assert!(matches!(next(&mut ops).await, FrameOp::Cursor { w: 8, h: 8, .. }));
        assert!(matches!(next(&mut ops).await, FrameOp::Rect { x: 8, y: 8, w: 16, h: 16, .. }));
        assert_eq!(next(&mut ops).await, FrameOp::Sync);
        quiet(&mut ops).await;
    }

    fn pointer(width: u16, height: u16, hot: (u16, u16)) -> ActiveStageOutput {
        let bitmap_data = vec![7; usize::from(width) * usize::from(height) * 4];
        let (hotspot_x, hotspot_y) = hot;
        let pointer = DecodedPointer { width, height, hotspot_x, hotspot_y, bitmap_data };
        ActiveStageOutput::PointerBitmap(Arc::new(pointer))
    }

    #[test]
    fn pointer_outputs_map_to_cursor_operations() {
        let hidden = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
        assert_eq!(pointer_op(&pointer(0, 0, (0, 0))), Some(hidden));
        let hidden = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
        assert_eq!(pointer_op(&pointer(0, 8, (3, 3))), Some(hidden));
        let hidden = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
        assert_eq!(pointer_op(&ActiveStageOutput::PointerHidden), Some(hidden));
        assert_eq!(pointer_op(&ActiveStageOutput::PointerDefault), Some(FrameOp::CursorDefault));

        assert_eq!(pointer_op(&pointer(129, 128, (0, 0))), Some(FrameOp::CursorDefault));
        assert_eq!(pointer_op(&pointer(128, 129, (0, 0))), Some(FrameOp::CursorDefault));
        assert!(matches!(pointer_op(&pointer(128, 128, (0, 0))), Some(FrameOp::Cursor { .. })));

        let clamped = FrameOp::Cursor { hot_x: 7, hot_y: 7, w: 8, h: 8, rgba: vec![7; 256] };
        assert_eq!(pointer_op(&pointer(8, 8, (20, 20))), Some(clamped));
        let kept = FrameOp::Cursor { hot_x: 2, hot_y: 3, w: 8, h: 8, rgba: vec![7; 256] };
        assert_eq!(pointer_op(&pointer(8, 8, (2, 3))), Some(kept));

        let moved = ActiveStageOutput::PointerPosition { x: 3, y: 4 };
        assert_eq!(pointer_op(&moved), None);
    }

    #[tokio::test]
    async fn a_server_resize_is_announced_and_later_pixels_land_in_the_new_picture() {
        let (server, _session, mut ops) = rig(SIZE).await;
        let size = ironrdp_server::DesktopSize { width: 400, height: 300 };
        server.display.send(DisplayUpdate::Resize(size)).unwrap();
        assert_eq!(next(&mut ops).await, FrameOp::Resize { w: 400, h: 300 });
        assert_eq!(next(&mut ops).await, FrameOp::Sync);

        let mut expected = Screen::new(400, 300);
        expected.paint(336, 236, 64, 64, shade(9));
        server.display.send(bitmap(336, 236, 64, 64, shade(9))).unwrap();
        let mut screen = Screen::new(400, 300);
        flush(&mut ops, &mut screen).await;
        assert_eq!(screen.rgba, expected.rgba);
    }

    #[tokio::test]
    async fn changes_waiting_for_an_acknowledgement_do_not_outlive_a_resize() {
        let (server, session, mut ops) = rig(SIZE).await;
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        for n in 0..2 {
            server.display.send(bitmap(0, 0, 16, 16, shade(n))).unwrap();
            flush(&mut ops, &mut screen).await;
        }
        // Past the bottom of the smaller picture, and held back by the missing acknowledgements.
        server.display.send(bitmap(192, 96, 64, 32, shade(7))).unwrap();
        while server.taken.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let size = ironrdp_server::DesktopSize { width: 128, height: 64 };
        server.display.send(DisplayUpdate::Resize(size)).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        quiet(&mut ops).await;
        session.ack();
        session.ack();
        assert_eq!(next(&mut ops).await, FrameOp::Resize { w: 128, h: 64 });
        assert_eq!(next(&mut ops).await, FrameOp::Sync);

        let mut expected = Screen::new(128, 64);
        expected.paint(8, 8, 16, 16, shade(9));
        server.display.send(bitmap(8, 8, 16, 16, shade(9))).unwrap();
        let mut screen = Screen::new(128, 64);
        flush(&mut ops, &mut screen).await;
        assert_eq!(screen.rgba, expected.rgba);
    }

    #[tokio::test]
    async fn a_resize_to_an_unsupported_size_ends_the_session() {
        for (width, height) in [(8192, 8192), (0, 600), (800, 0)] {
            let (server, _session, mut ops) = rig(SIZE).await;
            let size = ironrdp_server::DesktopSize { width, height };
            server.display.send(DisplayUpdate::Resize(size)).unwrap();
            let closed = FrameOp::Closed(BAD_SIZE.into());
            assert_eq!(rest(&mut ops).await, [closed], "{width}x{height}");
        }
    }

    #[test]
    fn the_size_check_refuses_empty_and_oversized_desktops() {
        for (width, height) in [(0, 600), (800, 0), (0, 0), (8192, 8192), (u16::MAX, u16::MAX)] {
            assert!(!size_supported(width, height), "{width}x{height}");
        }
        for (width, height) in [(1920, 1080), (7680, 4320), (1, 1)] {
            assert!(size_supported(width, height), "{width}x{height}");
        }
    }

    #[tokio::test]
    async fn a_logoff_error_info_gives_one_closed() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            let info = ErrorInfo::ProtocolIndependentCode(Code::LogoffByUser);
            write(&mut framed, error_info(info, &result)).await;
            framed.into_inner().0.shutdown().await.ok();
        })
        .await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed("logged off".into())]);
        within(server).await.unwrap();
    }

    #[tokio::test]
    async fn error_info_followed_by_an_mcs_ultimatum_gives_one_closed() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            let info = ErrorInfo::ProtocolIndependentCode(Code::DisconnectedByOtherconnection);
            write(&mut framed, error_info(info, &result)).await;
            write(&mut framed, mcs_disconnect(DisconnectReason::ProviderInitiated)).await;
        })
        .await;
        let closed = FrameOp::Closed("another user connected to this session".into());
        assert_eq!(rest(&mut ops).await, [closed]);
        within(server).await.unwrap();
    }

    #[tokio::test]
    async fn an_mcs_ultimatum_alone_is_the_server_ending_the_session() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, _result| async move {
            write(&mut framed, mcs_disconnect(DisconnectReason::ProviderInitiated)).await;
        })
        .await;
        let closed = FrameOp::Closed("the server ended the session".into());
        assert_eq!(rest(&mut ops).await, [closed]);
        within(server).await.unwrap();
    }

    #[tokio::test]
    async fn the_server_closing_the_connection_is_a_lost_connection() {
        let (server, _session, mut ops) = rig(SIZE).await;
        server.events.send(ServerEvent::Quit("bye".into())).unwrap();
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(LOST.into())]);
    }

    #[tokio::test]
    async fn a_connection_cut_without_a_tls_close_is_a_lost_connection() {
        let (server, _session, mut ops) = scripted(SIZE, |framed, _result| async move {
            drop(framed);
        })
        .await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(LOST.into())]);
        within(server).await.unwrap();
    }

    #[test]
    fn every_ending_has_its_words() {
        let code = |code| Some(ErrorInfo::ProtocolIndependentCode(code));
        let table = [
            (Ending::Ended, code(Code::LogoffByUser), "logged off"),
            (Ending::Ended, code(Code::RpcInitiatedLogoff), "logged off"),
            (
                Ending::Ended,
                code(Code::DisconnectedByOtherconnection),
                "another user connected to this session",
            ),
            (Ending::Ended, code(Code::IdleTimeout), "the session was idle for too long"),
            (Ending::Ended, code(Code::LogonTimeout), "the session time limit was reached"),
            (Ending::Ended, code(Code::RpcInitiatedDisconnect), "disconnected by an administrator"),
            (
                Ending::Ended,
                code(Code::RpcInitiatedDisconnectByuser),
                "disconnected by an administrator",
            ),
            (Ending::Ended, code(Code::ServerDeniedConnection), "the server denied the connection"),
            (
                Ending::Ended,
                code(Code::ServerInsufficientPrivileges),
                "this account may not sign in here",
            ),
            (
                Ending::Ended,
                code(Code::ServerFreshCredentialsRequired),
                "the server asks for the password again",
            ),
            (Ending::Ended, code(Code::OutOfMemory), "the server ran out of memory"),
            (Ending::Ended, None, "the server ended the session"),
            (Ending::Lost, None, LOST),
            (Ending::Garbled, None, NOT_UNDERSTOOD),
            (Ending::Resized, None, BAD_SIZE),
        ];
        for (ending, info, expected) in table {
            assert_eq!(words(&ending, info), expected, "{info:?}");
        }
        // The error info wins over the way the connection ended.
        let info = code(Code::LogoffByUser);
        assert_eq!(words(&Ending::Lost, info), "logged off");
        assert_eq!(words(&Ending::Garbled, info), "logged off");
    }

    #[test]
    fn licensing_errors_say_the_licence_is_missing() {
        let licensing = |code| Some(ErrorInfo::ProtocolIndependentLicensingCode(code));
        let codes = [
            Licence::Internal,
            Licence::NoLicenseServer,
            Licence::NoLicense,
            Licence::BadClientMsg,
            Licence::HwidDoesntMatchLicense,
            Licence::BadClientLicense,
            Licence::CantFinishProtocol,
            Licence::ClientEndedProtocol,
            Licence::BadClientEncryption,
            Licence::CantUpgradeLicense,
            Licence::NoRemoteConnections,
        ];
        for code in codes {
            let expected = "the server has no licence for this connection";
            assert_eq!(words(&Ending::Ended, licensing(code)), expected, "{code:?}");
        }
    }

    #[test]
    fn other_error_info_keeps_the_description_without_its_prefix() {
        let other = Some(ErrorInfo::ProtocolIndependentCode(Code::ServerDwmCrash));
        let text = words(&Ending::Ended, other);
        assert!(!text.contains('['), "{text}");
        assert!(text.len() > 10 && !text.starts_with(' '), "{text}");

        let broker =
            Some(ErrorInfo::ProtocolIndependentConnectionBrokerCode(Broker::DestinationNotFound));
        assert_eq!(words(&Ending::Ended, broker), "The target endpoint could not be found");

        let specific = Some(ErrorInfo::RdpSpecificCode(RdpSpecificCode::UnknownPduType));
        let text = words(&Ending::Ended, specific);
        assert!(!text.contains('[') && !text.starts_with([':', ' ']), "{text}");
    }

    #[tokio::test]
    async fn pdus_ironrdp_does_not_handle_are_skipped() {
        let first = |column: u16, row: u16| [90, (row * 8) as u8, (column * 8) as u8];
        let second = |column: u16, row: u16| [(column * 8) as u8, 120, (row * 8) as u8];
        let (server, session, mut ops) = scripted(SIZE, move |mut framed, result| async move {
            write(&mut framed, play_sound(&result)).await;
            write(&mut framed, surface_bits(16, 8, 16, 16, first)).await;
            write(&mut framed, monitor_layout(&result)).await;
            write(&mut framed, surface_bits(100, 60, 16, 16, second)).await;
            framed
        })
        .await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(16, 8, 16, 16, first);
        expected.paint(100, 60, 16, 16, second);
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        let skipped = [
            ("unhandled PDU: \"Play Sound PDU\"".to_owned(), 1),
            ("unhandled PDU: \"Monitor Layout PDU\"".to_owned(), 1),
        ];
        assert_eq!(session.skipped(), BTreeMap::from(skipped));
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_bitmap_outside_the_picture_is_ignored() {
        let (server, _session, mut ops) = scripted(SIZE, move |mut framed, _result| async move {
            write(&mut framed, surface_bits(250, 120, 16, 16, shade(5))).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(6))).await;
            framed
        })
        .await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(8, 8, 16, 16, shade(6));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_frame_that_is_no_pdu_ends_the_session_once() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            write(&mut framed, garbage(&result)).await;
            write(&mut framed, garbage(&result)).await;
        })
        .await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        within(server).await.unwrap();
    }

    #[tokio::test]
    async fn a_frame_that_panics_inside_ironrdp_ends_the_session_once() {
        // The destination ends at 0, and starts after it ends.
        for (x, width) in [(0, 0), (10, 0)] {
            let (server, session, mut ops) = scripted(SIZE, move |mut framed, _result| async move {
                write(&mut framed, surface_bits(x, 0, width, 4, shade(1))).await;
                write(&mut framed, surface_bits(8, 8, 16, 16, shade(2))).await;
                framed
            })
            .await;
            if cfg!(debug_assertions) || x > 0 {
                assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())], "{x}");
            } else {
                // A release build clips the first destination to one pixel and goes on.
                let mut expected = Screen::new(SIZE.0, SIZE.1);
                expected.paint(8, 8, 16, 16, shade(2));
                let mut screen = Screen::new(SIZE.0, SIZE.1);
                let square = |screen: &Screen| -> Vec<u8> {
                    let row = |y: usize| screen.rgba[(y * 256 + 8) * 4..(y * 256 + 24) * 4].to_vec();
                    (8..24).flat_map(row).collect()
                };
                while square(&screen) != square(&expected) {
                    flush(&mut ops, &mut screen).await;
                }
                quiet(&mut ops).await;
            }
            drop(within(server).await.unwrap());
            let started = Instant::now();
            within(session.close()).await;
            assert!(started.elapsed() < Duration::from_millis(250));
        }
    }

    #[tokio::test]
    async fn a_panic_outside_a_frame_also_ends_the_session_with_a_reason() {
        let (client, stream) = duplex(1 << 16);
        let options = Options { login: Login::Tls, size: SIZE, remotefx: false };
        let server = TestServer::on(options, stream).await;
        let connected = logged_in(client, &pinned(HOST, PORT)).await;
        let (sink, mut ops) = mpsc::unbounded_channel();
        let sink = move |op: FrameOp| match op {
            FrameOp::Rect { .. } => panic!("the sink broke"),
            op => sink.send(op).unwrap(),
        };
        let session = Session::from_connected(connected, sink);
        server.display.send(bitmap(0, 0, 8, 8, shade(1))).unwrap();
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        within(session.close()).await;
    }

    // The client lists its static channels by type id, and the server numbers them in that order.
    fn clipboard_channel() -> u16 {
        u16::from(TypeId::of::<CliprdrClient>() > TypeId::of::<DrdynvcClient>())
    }

    fn dynamic_channel() -> u16 {
        1 - clipboard_channel()
    }

    // A Format Data Response whose channel message is `size` bytes long.
    fn data_response(size: usize) -> Vec<u8> {
        let mut pdu = vec![5, 0, 1, 0];
        pdu.extend(((size - 8) as u32).to_le_bytes());
        pdu.resize(size, b'x');
        pdu
    }

    async fn message(
        framed: &mut ScriptedFramed<DuplexStream>,
        channels: &Channels,
        channel: u16,
        data: &[u8],
        last: bool,
    ) {
        let pieces = data.chunks(16_000).len();
        for (n, piece) in data.chunks(16_000).enumerate() {
            let mut flags = ChannelControlFlags::empty();
            if n == 0 {
                flags |= ChannelControlFlags::FLAG_FIRST;
            }
            if last && n + 1 == pieces {
                flags |= ChannelControlFlags::FLAG_LAST;
            }
            let length = data.len() as u32;
            write(framed, channel_chunk(channels, channel, flags, length, piece)).await;
        }
    }

    // 3000 chunks of 8000 bytes that say the message is 1000 bytes long and never end it.
    async fn flood(
        framed: &mut ScriptedFramed<DuplexStream>,
        channels: &Channels,
        channel: u16,
        every_chunk_is_first: bool,
    ) {
        let piece = [0u8; 8000];
        for n in 0..3000 {
            let first = n == 0 || every_chunk_is_first;
            let flags =
                if first { ChannelControlFlags::FLAG_FIRST } else { ChannelControlFlags::empty() };
            write(framed, channel_chunk(channels, channel, flags, 1000, &piece)).await;
        }
    }

    #[tokio::test]
    async fn a_static_channel_message_without_an_end_closes_the_session() {
        for channel in [clipboard_channel(), dynamic_channel()] {
            for every_chunk_is_first in [false, true] {
                let script = move |mut framed, channels| async move {
                    flood(&mut framed, &channels, channel, every_chunk_is_first).await;
                    framed
                };
                let rig = scripted_with(SIZE, attach, CLIPBOARD, script).await;
                let (server, _session, mut ops) = rig;
                let closed = [FrameOp::Closed(NOT_UNDERSTOOD.into())];
                assert_eq!(rest(&mut ops).await, closed, "{channel} {every_chunk_is_first}");
                drop(within(server).await.unwrap());
            }
        }
    }

    #[tokio::test]
    async fn messages_up_to_the_cap_pass_and_the_count_starts_again_after_each() {
        let script = |mut framed, channels| async move {
            let data = data_response(MAX_CHANNEL_MESSAGE);
            message(&mut framed, &channels, clipboard_channel(), &data, true).await;
            message(&mut framed, &channels, clipboard_channel(), &data, true).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(8, 8, 16, 16, shade(4));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    #[test]
    fn the_bytes_of_an_unfinished_message_are_counted_up_to_the_cap() {
        let frame = |channel, flags| {
            let header = ironrdp_core::encode_vec(&ChannelPduHeader { length: 1000, flags });
            let indication = ironrdp_pdu::mcs::SendDataIndication {
                initiator_id: 1002,
                channel_id: channel,
                user_data: std::borrow::Cow::Owned([header.unwrap(), vec![0; 8000]].concat()),
            };
            ironrdp_core::encode_vec(&X224(indication)).unwrap()
        };
        let mut unfinished = HashMap::from([(7, 0)]);
        let first = frame(7, ChannelControlFlags::FLAG_FIRST);
        let more = frame(7, ChannelControlFlags::empty());
        let mut taken = 0;
        let mut next = &first;
        let mut dynamic = Dynamic::default();
        while count_chunk(&mut unfinished, &mut dynamic, Action::X224, next).is_ok() {
            taken += 1;
            next = &more;
            assert!(taken < 3000, "no end");
        }
        assert_eq!(taken, MAX_CHANNEL_MESSAGE / 8000);
        assert_eq!(unfinished[&7], (taken + 1) * 8000);

        let other = frame(9, ChannelControlFlags::FLAG_FIRST);
        assert!(count_chunk(&mut unfinished, &mut dynamic, Action::X224, &other).is_ok());
        assert!(count_chunk(&mut unfinished, &mut dynamic, Action::FastPath, &more).is_ok());
        assert_eq!(unfinished.len(), 1);
    }

    // Dynamic channel PDUs given to `count_chunk` as messages on static channel 7.
    struct Feed {
        unfinished: HashMap<u16, usize>,
        dynamic: Dynamic,
    }

    impl Feed {
        fn new() -> Self {
            let dynamic = Dynamic { channel: Some(7), ..Dynamic::default() };
            Feed { unfinished: HashMap::from([(7, 0)]), dynamic }
        }

        // The PDU in chunks that end at `cuts`; false once the count refuses one.
        fn chunked(&mut self, pdu: &[u8], cuts: &[usize]) -> bool {
            let bounds = [&[0][..], cuts, &[pdu.len()]].concat();
            for (n, piece) in bounds.windows(2).enumerate() {
                let mut flags = ChannelControlFlags::empty();
                if n == 0 {
                    flags |= ChannelControlFlags::FLAG_FIRST;
                }
                if n + 2 == bounds.len() {
                    flags |= ChannelControlFlags::FLAG_LAST;
                }
                let header = ChannelPduHeader { length: pdu.len() as u32, flags };
                let header = ironrdp_core::encode_vec(&header).unwrap();
                let data = [header, pdu[piece[0]..piece[1]].to_vec()];
                let indication = ironrdp_pdu::mcs::SendDataIndication {
                    initiator_id: 1002,
                    channel_id: 7,
                    user_data: std::borrow::Cow::Owned(data.concat()),
                };
                let frame = ironrdp_core::encode_vec(&X224(indication)).unwrap();
                let (unfinished, dynamic) = (&mut self.unfinished, &mut self.dynamic);
                if count_chunk(unfinished, dynamic, Action::X224, &frame).is_err() {
                    return false;
                }
            }
            true
        }

        fn pdu(&mut self, pdu: &[u8]) -> bool {
            self.chunked(pdu, &[])
        }
    }

    #[test]
    fn a_dynamic_message_may_declare_the_cap_and_no_more() {
        let cap = MAX_DYNAMIC_MESSAGE as u32;
        assert!(Feed::new().pdu(&dvc_first(1, cap, &[0; 100])));
        assert!(!Feed::new().pdu(&dvc_first(1, cap + 1, &[0; 100])));
        assert!(!Feed::new().pdu(&dvc_first(1, u32::MAX, &[])));
    }

    #[test]
    fn dynamic_channels_are_counted_apart() {
        let cap = MAX_DYNAMIC_MESSAGE as u32;
        let piece = [0; 1024];
        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, cap, &piece)));
        assert!(feed.pdu(&dvc_first(2, cap, &piece)));
        for _ in 1..MAX_DYNAMIC_MESSAGE / 1024 {
            assert!(feed.pdu(&dvc_data(1, &piece)));
            assert!(feed.pdu(&dvc_data(2, &piece)));
        }
        assert!(feed.dynamic.left.is_empty());

        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, 3000, &[0; 1000])));
        assert!(feed.pdu(&dvc_first(2, 3000, &[0; 1000])));
        assert!(feed.pdu(&dvc_data(1, &[0; 2000])));
        assert!(!feed.pdu(&dvc_data(2, &[0; 2001])));
    }

    #[test]
    fn a_dynamic_message_longer_than_it_declared_is_refused() {
        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, 3000, &[0; 1000])));
        assert!(feed.pdu(&dvc_data(1, &[0; 1000])));
        assert!(!feed.pdu(&dvc_data(1, &[0; 1001])));
        assert!(!Feed::new().pdu(&dvc_first(1, 10, &[0; 11])));
    }

    #[test]
    fn a_finished_dynamic_message_leaves_the_next_data_alone() {
        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, 3000, &[0; 1000])));
        assert!(feed.pdu(&dvc_data(1, &[0; 2000])));
        assert!(feed.pdu(&dvc_data(1, &[0; 5000])));
        assert!(feed.dynamic.left.is_empty());
    }

    #[test]
    fn a_new_first_part_replaces_the_unfinished_message() {
        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, 100, &[0; 10])));
        assert!(feed.pdu(&dvc_first(1, 3000, &[0; 10])));
        assert!(feed.pdu(&dvc_data(1, &[0; 2990])));
        assert!(feed.dynamic.left.is_empty());
    }

    #[test]
    fn the_chunks_of_one_dynamic_pdu_count_together() {
        let mut feed = Feed::new();
        assert!(feed.chunked(&dvc_first(1, 3000, &[0; 1000]), &[10, 500]));
        assert!(feed.chunked(&dvc_data(1, &[0; 2000]), &[10, 900]));
        assert!(feed.dynamic.left.is_empty());

        let mut feed = Feed::new();
        assert!(feed.chunked(&dvc_first(1, 3000, &[0; 1000]), &[500]));
        assert!(!feed.chunked(&dvc_data(1, &[0; 2001]), &[900]));
    }

    #[test]
    fn a_dynamic_pdu_whose_start_is_cut_short_is_refused() {
        assert!(!Feed::new().chunked(&dvc_first(1, u32::MAX, &[0; 1000]), &[1]));
        // A whole message that short, or a start no part can mend, is IronRDP's to refuse.
        assert!(Feed::new().pdu(&[0x30]));
        assert!(Feed::new().chunked(&[0; 1000], &[1]));
    }

    #[test]
    fn a_closed_dynamic_channel_is_counted_no_more() {
        let mut feed = Feed::new();
        assert!(feed.pdu(&dvc_first(1, 100, &[0; 10])));
        assert!(feed.pdu(&dvc(DrdynvcServerPdu::Close(ClosePdu::new(1)))));
        assert!(feed.pdu(&dvc_data(1, &[0; 200])));
    }

    #[tokio::test]
    async fn a_message_one_byte_over_the_cap_closes_the_session() {
        let script = |mut framed, channels| async move {
            let data = data_response(MAX_CHANNEL_MESSAGE + 1);
            message(&mut framed, &channels, clipboard_channel(), &data, true).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn the_channels_are_counted_apart() {
        let script = |mut framed, channels| async move {
            let data = vec![0u8; MAX_CHANNEL_MESSAGE * 3 / 4];
            message(&mut framed, &channels, clipboard_channel(), &data, false).await;
            message(&mut framed, &channels, dynamic_channel(), &data, false).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(8, 8, 16, 16, shade(4));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    // What the reviewer sent: 1 First and 26,214 Next fragments of 16,000 bytes, 419 MB in all.
    #[tokio::test]
    async fn a_fast_path_update_that_never_ends_closes_the_session() {
        let script = |mut framed: ScriptedFramed<DuplexStream>, _channels| async move {
            let piece = vec![0u8; 16_000];
            let mut written = 0;
            for n in 0..=26_214 {
                let fragmentation = if n == 0 { Fragmentation::First } else { Fragmentation::Next };
                let frame = fast_path_update(fragmentation, UpdateCode::SurfaceCommands, &piece);
                if framed.write_all(&frame).await.is_err() {
                    break;
                }
                written += 1;
            }
            written
        };
        let started = Instant::now();
        let (server, _session, mut ops) = scripted(SIZE, script).await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        assert!(started.elapsed() < WAIT, "{:?}", started.elapsed());
        let written = within(server).await.unwrap();
        assert!(written < 600, "{written}");
    }

    fn fragment(fragmentation: Fragmentation, size: usize) -> Vec<u8> {
        fast_path_update(fragmentation, UpdateCode::SurfaceCommands, &vec![0; size])
    }

    #[test]
    fn the_fragments_of_an_update_are_counted_up_to_the_limit() {
        let limit = DEFAULT_MAX_REQUEST_SIZE as usize;
        let mut fragmented = None;
        let first = fragment(Fragmentation::First, 16_000);
        let more = fragment(Fragmentation::Next, 16_000);
        let mut taken = 0;
        let mut next = &first;
        while count_fragments(&mut fragmented, limit, Action::FastPath, next).is_ok() {
            taken += 1;
            next = &more;
            assert!(taken <= 26_214, "no end");
        }
        assert_eq!(taken, limit / 16_000);
        assert_eq!(fragmented, Some((taken + 1) * 16_000));
        assert!(fragmented.unwrap() <= limit + 16_000);
    }

    #[test]
    fn an_update_of_exactly_the_limit_passes_and_the_count_starts_again_after_the_last() {
        let mut fragmented = None;
        let mut count = |fragmentation, size| {
            count_fragments(&mut fragmented, 40_000, Action::FastPath, &fragment(fragmentation, size))
                .is_ok()
        };
        assert!(count(Fragmentation::First, 16_000));
        assert!(count(Fragmentation::Next, 16_000));
        assert!(count(Fragmentation::Last, 8000));
        assert!(count(Fragmentation::First, 16_000));
        assert!(count(Fragmentation::Next, 16_000));
        assert!(!count(Fragmentation::Last, 8001));
    }

    #[test]
    fn a_single_or_a_first_update_drops_the_fragments_kept_before_it() {
        let mut fragmented = None;
        let mut count = |fragmentation, size| {
            let frame = fragment(fragmentation, size);
            let counted = count_fragments(&mut fragmented, 40_000, Action::FastPath, &frame);
            (counted.is_ok(), fragmented)
        };
        assert_eq!(count(Fragmentation::First, 30_000), (true, Some(30_000)));
        assert_eq!(count(Fragmentation::Single, 100), (true, None));
        // IronRDP keeps nothing for fragments that follow no first one.
        assert_eq!(count(Fragmentation::Next, 30_000), (true, None));
        assert_eq!(count(Fragmentation::Next, 30_000), (true, None));
        assert_eq!(count(Fragmentation::Last, 30_000), (true, None));
        assert_eq!(count(Fragmentation::First, 30_000), (true, Some(30_000)));
        assert_eq!(count(Fragmentation::First, 30_000), (true, Some(30_000)));
        assert_eq!(count(Fragmentation::Next, 10_001), (false, Some(40_001)));
    }

    #[test]
    fn every_update_in_a_frame_is_counted_and_other_frames_are_not() {
        use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu};

        let update = |fragmentation, size| FastPathUpdatePdu {
            fragmentation,
            update_code: UpdateCode::SurfaceCommands,
            compression_flags: None,
            compression_type: None,
            data: &[0; 20_000][..size],
        };
        let updates = [
            ironrdp_core::encode_vec(&update(Fragmentation::First, 20_000)).unwrap(),
            ironrdp_core::encode_vec(&update(Fragmentation::Next, 20_000)).unwrap(),
        ]
        .concat();
        let header = FastPathHeader::new(EncryptionFlags::empty(), updates.len());
        let frame = [ironrdp_core::encode_vec(&header).unwrap(), updates].concat();
        let mut fragmented = None;
        assert!(count_fragments(&mut fragmented, 40_000, Action::FastPath, &frame).is_ok());
        assert_eq!(fragmented, Some(40_000));
        assert!(count_fragments(&mut fragmented, 40_000, Action::X224, &frame).is_ok());
        assert_eq!(fragmented, Some(40_000));
        let mut fragmented = None;
        assert!(count_fragments(&mut fragmented, 39_999, Action::FastPath, &frame).is_err());
    }

    #[tokio::test]
    async fn a_fragmented_update_just_under_the_limit_is_drawn_and_the_count_starts_again() {
        const BIG: (u16, u16) = (2048, 1024);
        let big = surface_command(0, 0, 2047, 1024, shade(3));
        let limit = DEFAULT_MAX_REQUEST_SIZE as usize;
        assert!(big.len() <= limit && big.len() > limit - 5000, "{}", big.len());
        let small = surface_command(8, 8, 64, 64, shade(4));
        let (server, _session, mut ops) = scripted(BIG, move |mut framed, _result| async move {
            for frame in fragments(UpdateCode::SurfaceCommands, &big, 16_000) {
                write(&mut framed, frame).await;
            }
            for frame in fragments(UpdateCode::SurfaceCommands, &small, 6000) {
                write(&mut framed, frame).await;
            }
            framed
        })
        .await;
        let mut expected = Screen::new(BIG.0, BIG.1);
        expected.paint(0, 0, 2047, 1024, shade(3));
        expected.paint(8, 8, 64, 64, shade(4));
        let mut screen = Screen::new(BIG.0, BIG.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    #[test]
    fn the_update_limit_is_one_frame_of_the_current_desktop() {
        let frame = |width: usize, height: usize| width * height * 4 + (1 << 20);
        assert_eq!(update_limit(u32::MAX, 256, 128), frame(256, 128));
        assert_eq!(update_limit(u32::MAX, 1024, 768), frame(1024, 768));
        assert_eq!(update_limit(u32::MAX, 8192, 4096), frame(8192, 4096));
        // What xrdp advertises, a frame and 16 KiB, and what Windows advertises, at the sizes the
        // live tests use.
        for (width, height) in [(1024, 768), (1280, 800), (1440, 900)] {
            let xrdp = u32::from(width) * u32::from(height) * 4 + (16 << 10);
            assert_eq!(update_limit(xrdp, width, height), xrdp as usize);
            assert_eq!(update_limit(608_299, width, height), 608_299);
        }
    }

    #[test]
    fn a_server_advertising_four_gigabytes_is_held_to_one_frame_of_the_desktop() {
        let mut fragmented = None;
        let limit = update_limit(u32::MAX, SIZE.0, SIZE.1);
        let first = fragment(Fragmentation::First, 16_000);
        let more = fragment(Fragmentation::Next, 16_000);
        let mut taken = 0;
        let mut next = &first;
        while count_fragments(&mut fragmented, limit, Action::FastPath, next).is_ok() {
            taken += 1;
            next = &more;
            assert!(taken <= 300_000, "no end");
        }
        assert_eq!(taken, limit / 16_000);
        assert!(fragmented.unwrap() <= limit + 16_000);
    }

    #[tokio::test]
    async fn a_server_advertising_four_gigabytes_is_cut_at_one_frame_of_the_desktop() {
        let limit = usize::from(SIZE.0) * usize::from(SIZE.1) * 4 + (1 << 20);
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (mut framed, _channels) = accept_scripted_with(stream, SIZE, u32::MAX).await;
            let piece = vec![0u8; 16_000];
            let mut written = 0;
            for n in 0..(limit / 16_000 + 1000) {
                let fragmentation = if n == 0 { Fragmentation::First } else { Fragmentation::Next };
                let frame = fast_path_update(fragmentation, UpdateCode::SurfaceCommands, &piece);
                if framed.write_all(&frame).await.is_err() {
                    break;
                }
                written += 1;
            }
            written
        });
        let started = Instant::now();
        let (_session, mut ops) = join(client, &pinned(HOST, PORT)).await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
        let written = within(server).await.unwrap();
        assert!(written < limit / 16_000 + 100, "{written}");
    }

    #[tokio::test]
    async fn after_a_reactivation_the_update_limit_is_one_frame_of_the_new_desktop() {
        const BIGGER: (u16, u16) = (512, 256);
        let frame = |(width, height): (u16, u16)| usize::from(width) * usize::from(height) * 4;
        let (before, after) = (frame(SIZE) + (1 << 20), frame(BIGGER) + (1 << 20));
        let pieces = after / 16_000;
        assert!(pieces * 16_000 > before + 16_000, "{before} {after}");
        let (client, stream) = duplex(1 << 16);
        let (sent_tx, sent) = oneshot::channel();
        let (more_tx, more) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (framed, channels, acceptor) =
                accept_scripted_keeping(stream, SIZE, u32::MAX).await;
            let mut framed = reactivate_scripted(framed, &channels, acceptor, BIGGER, &[]).await;
            let piece = vec![0u8; 16_000];
            let frame = |fragmentation| {
                fast_path_update(fragmentation, UpdateCode::SurfaceCommands, &piece)
            };
            write(&mut framed, frame(Fragmentation::First)).await;
            for _ in 1..pieces {
                write(&mut framed, frame(Fragmentation::Next)).await;
            }
            sent_tx.send(()).unwrap();
            more.await.unwrap();
            for _ in 0..2 {
                write(&mut framed, frame(Fragmentation::Next)).await;
            }
            framed
        });
        let (_session, mut ops) = join(client, &pinned(HOST, PORT)).await;
        within(sent).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        while let Ok(op) = ops.try_recv() {
            assert!(!matches!(op, FrameOp::Closed(_)), "{op:?}");
        }
        more_tx.send(()).unwrap();
        let ended = rest(&mut ops).await;
        assert_eq!(ended.last(), Some(&FrameOp::Closed(NOT_UNDERSTOOD.into())), "{ended:?}");
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn the_limit_is_the_one_the_server_advertised() {
        const LIMIT: u32 = 64 << 10;
        let under = surface_command(0, 0, 128, 127, shade(5));
        let over = surface_command(0, 0, 128, 128, shade(6));
        assert!(under.len() <= LIMIT as usize && over.len() > LIMIT as usize);
        let (client, stream) = duplex(1 << 16);
        let (go, wait) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut framed, _channels) = accept_scripted_with(stream, SIZE, LIMIT).await;
            for frame in fragments(UpdateCode::SurfaceCommands, &under, 16_000) {
                write(&mut framed, frame).await;
            }
            wait.await.unwrap();
            for frame in fragments(UpdateCode::SurfaceCommands, &over, 16_000) {
                write(&mut framed, frame).await;
            }
            framed
        });
        let (_session, mut ops) = join(client, &pinned(HOST, PORT)).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(0, 0, 128, 127, shade(5));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        go.send(()).unwrap();
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        drop(within(server).await.unwrap());
    }

    const DISPLAY: u32 = 3;

    fn dvc(pdu: DrdynvcServerPdu) -> Vec<u8> {
        ironrdp_core::encode_vec(&pdu).unwrap()
    }

    fn dvc_data(id: u32, data: &[u8]) -> Vec<u8> {
        dvc(DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(id, data.to_vec()))))
    }

    fn dvc_first(id: u32, length: u32, data: &[u8]) -> Vec<u8> {
        let first = DataFirstPdu::new(id, length, data.to_vec());
        dvc(DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirst(first)))
    }

    async fn open_display(framed: &mut ScriptedFramed<DuplexStream>, channels: &Channels) {
        let create = CreateRequestPdu::new(DISPLAY, ironrdp_displaycontrol::CHANNEL_NAME.into());
        let create = dvc(DrdynvcServerPdu::Create(create));
        message(framed, channels, dynamic_channel(), &create, true).await;
    }

    // Display Control capabilities padded to `size` bytes; the client reads only the start.
    fn display_caps(size: usize) -> Vec<u8> {
        let mut caps: Vec<u8> = [1u32, 8192, 8192].iter().flat_map(|n| n.to_le_bytes()).collect();
        caps.resize(size, 0);
        caps
    }

    // A dynamic channel message as a server splits it, each PDU a static channel message.
    async fn dynamic_message(
        framed: &mut ScriptedFramed<DuplexStream>,
        channels: &Channels,
        data: &[u8],
    ) {
        let mut pieces = data.chunks(DrdynvcDataPdu::MAX_DATA_SIZE);
        let first = pieces.next().unwrap();
        let first = if first.len() == data.len() {
            dvc_data(DISPLAY, first)
        } else {
            dvc_first(DISPLAY, data.len() as u32, first)
        };
        message(framed, channels, dynamic_channel(), &first, true).await;
        for piece in pieces {
            message(framed, channels, dynamic_channel(), &dvc_data(DISPLAY, piece), true).await;
        }
    }

    // The pixels sent after the message still arrive.
    async fn a_dynamic_message_passes(size: usize) {
        let script = move |mut framed, channels| async move {
            open_display(&mut framed, &channels).await;
            dynamic_message(&mut framed, &channels, &display_caps(size)).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(8, 8, 16, 16, shade(4));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_small_dynamic_message_passes() {
        a_dynamic_message_passes(12).await;
    }

    #[tokio::test]
    async fn a_dynamic_message_of_the_cap_passes() {
        a_dynamic_message_passes(MAX_DYNAMIC_MESSAGE).await;
    }

    #[tokio::test]
    async fn a_dynamic_message_declared_past_the_cap_closes_the_session() {
        bounded(|| async {
            let script = |mut framed: ScriptedFramed<DuplexStream>, channels| async move {
                open_display(&mut framed, &channels).await;
                let first = dvc_first(DISPLAY, 0xFFFF_FF00, &[0; 1000]);
                message(&mut framed, &channels, dynamic_channel(), &first, true).await;
                let data = dvc_data(DISPLAY, &vec![0; 1 << 20]);
                let pieces = data.chunks(16_000).len();
                'endless: for _ in 0..256 {
                    for (n, piece) in data.chunks(16_000).enumerate() {
                        let mut flags = ChannelControlFlags::empty();
                        if n == 0 {
                            flags |= ChannelControlFlags::FLAG_FIRST;
                        }
                        if n + 1 == pieces {
                            flags |= ChannelControlFlags::FLAG_LAST;
                        }
                        let length = data.len() as u32;
                        let channel = dynamic_channel();
                        let chunk = channel_chunk(&channels, channel, flags, length, piece);
                        if framed.write_all(&chunk).await.is_err() {
                            break 'endless;
                        }
                    }
                }
                framed
            };
            let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
            assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
            drop(within(server).await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn an_error_info_code_the_list_lacks_is_the_server_ending_the_session() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            write(&mut framed, unlisted_error_info(0xee, &result)).await;
            framed
        })
        .await;
        let closed = FrameOp::Closed("the server ended the session".into());
        assert_eq!(rest(&mut ops).await, [closed]);
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn the_server_ending_the_session_during_reactivation_is_not_garbled_data() {
        type Frame = fn(&Channels) -> Vec<u8>;
        let logoff: Frame = |channels| {
            error_info(ErrorInfo::ProtocolIndependentCode(Code::LogoffByUser), channels)
        };
        let unlisted: Frame = |channels| unlisted_error_info(0xee, channels);
        let ultimatum: Frame = |_| mcs_disconnect(DisconnectReason::ProviderInitiated);
        let table = [
            (logoff, "logged off"),
            (unlisted, "the server ended the session"),
            (ultimatum, "the server ended the session"),
        ];
        for (frame, words) in table {
            let (server, _session, mut ops) = scripted(SIZE, move |mut framed, result| async move {
                write(&mut framed, deactivate_all(&result)).await;
                write(&mut framed, frame(&result)).await;
                framed
            })
            .await;
            assert_eq!(rest(&mut ops).await, [FrameOp::Closed(words.into())], "{words}");
            drop(within(server).await.unwrap());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_reactivation_that_never_finishes_ends_the_session_after_ten_seconds() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            write(&mut framed, deactivate_all(&result)).await;
            framed
        })
        .await;
        let started = tokio::time::Instant::now();
        let closed = timeout(Duration::from_secs(60), ops.recv()).await.expect("no ending");
        assert_eq!(closed, Some(FrameOp::Closed(LOST.into())));
        let took = started.elapsed();
        assert!(took >= REACTIVATE_LIMIT && took < REACTIVATE_LIMIT + Duration::from_secs(1));
        drop(within(server).await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn a_deactivate_all_of_the_header_alone_starts_a_reactivation() {
        let (server, _session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            write(&mut framed, short_deactivate_all(&result)).await;
            framed
        })
        .await;
        let started = tokio::time::Instant::now();
        let closed = timeout(Duration::from_secs(60), ops.recv()).await.expect("no ending");
        assert_eq!(closed, Some(FrameOp::Closed(LOST.into())));
        let took = started.elapsed();
        assert!(took >= REACTIVATE_LIMIT && took < REACTIVATE_LIMIT + Duration::from_secs(1));
        drop(within(server).await.unwrap());
    }

    // xrdp answers the clipboard's format list while it reactivates for an early resize.
    #[tokio::test]
    async fn clipboard_messages_during_a_reactivation_are_handled_after_it() {
        const BIGGER: (u16, u16) = (512, 256);
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (mut framed, channels, acceptor) =
                accept_scripted_keeping(stream, SIZE, u32::MAX).await;
            clip_start(&mut framed, &channels).await;
            let copied: Vec<u8> = "copied\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
            let between = [
                clip_frame(&channels, clip_pdu(3, RESPONSE_OK, &[])),
                clip_frame(&channels, clip_pdu(2, 0, &format_entry(13, ""))),
                clip_frame(&channels, clip_pdu(5, RESPONSE_OK, &copied)),
            ];
            reactivate_scripted(framed, &channels, acceptor, BIGGER, &between).await
        });
        let (session, mut ops) = join_with(client, &pinned(HOST, PORT), attach, CLIPBOARD).await;
        let mut seen = Vec::new();
        while !seen.contains(&FrameOp::Clipboard("copied".into())) {
            let op = next(&mut ops).await;
            assert!(!matches!(op, FrameOp::Closed(_)), "{op:?} after {seen:?}");
            if op == FrameOp::Sync {
                session.ack();
            }
            seen.push(op);
        }
        assert!(seen.contains(&FrameOp::Resize { w: 512, h: 256 }), "{seen:?}");
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn channel_data_held_during_a_reactivation_is_capped() {
        let script = |mut framed, channels| async move {
            write(&mut framed, deactivate_all(&channels)).await;
            let data = vec![b'x'; MAX_CHANNEL_MESSAGE];
            message(&mut framed, &channels, clipboard_channel(), &data, true).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_header_alone_that_is_not_exactly_deactivate_all_is_garbled_data() {
        use ironrdp_pdu::mcs::{decode_send_data_indication, SendDataIndication};
        use ironrdp_pdu::x224::X224;

        type Frame = fn(&Channels) -> Vec<u8>;
        // The user data is the last six bytes of the frame.
        let no_version: Frame = |channels| {
            let mut frame = short_deactivate_all(channels);
            let start = frame.len() - 6;
            frame[start..start + 4].copy_from_slice(&[6, 0, 6, 0]);
            frame
        };
        let wrong_version: Frame = |channels| {
            let mut frame = short_deactivate_all(channels);
            let start = frame.len() - 6;
            frame[start..start + 4].copy_from_slice(&[6, 0, 0x26, 0xff]);
            frame
        };
        let wrong_length: Frame = |channels| {
            let mut frame = short_deactivate_all(channels);
            let start = frame.len() - 6;
            frame[start] = 8;
            frame
        };
        let other_channel: Frame = |channels| {
            let frame = short_deactivate_all(channels);
            let context = decode_send_data_indication(&frame).unwrap();
            let indication = SendDataIndication {
                initiator_id: context.initiator_id,
                channel_id: context.channel_id + 1,
                user_data: context.user_data.into(),
            };
            ironrdp_core::encode_vec(&X224(indication)).unwrap()
        };
        let table = [
            (no_version, "no version"),
            (wrong_version, "wrong version"),
            (wrong_length, "wrong length"),
            (other_channel, "other channel"),
        ];
        for (frame, case) in table {
            let (server, _session, mut ops) = scripted(SIZE, move |mut framed, result| async move {
                write(&mut framed, frame(&result)).await;
                framed
            })
            .await;
            assert_eq!(rest(&mut ops).await, [FrameOp::Closed(NOT_UNDERSTOOD.into())], "{case}");
            drop(within(server).await.unwrap());
        }
    }

    #[tokio::test]
    async fn acknowledgements_with_nothing_to_acknowledge_do_not_raise_the_limit() {
        let (server, session, mut ops) = rig(SIZE).await;
        for _ in 0..5 {
            session.ack();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        for n in 0..2 {
            server.display.send(bitmap(0, 0, 16, 16, shade(n))).unwrap();
            flush(&mut ops, &mut screen).await;
        }
        server.display.send(bitmap(32, 0, 16, 16, shade(7))).unwrap();
        quiet(&mut ops).await;
        session.ack();
        flush(&mut ops, &mut screen).await;
    }

    #[tokio::test]
    async fn nothing_is_flushed_once_close_has_started() {
        let (server, session, mut ops) = scripted(SIZE, |mut framed, result| async move {
            loop {
                match read_client(&mut framed, &result).await {
                    Ok(ClientPdu::ShutdownRequest) => {
                        write(&mut framed, surface_bits(8, 8, 16, 16, shade(3))).await;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        })
        .await;
        within(session.close()).await;
        assert_eq!(rest(&mut ops).await, []);
        within(server).await.unwrap();
    }

    #[tokio::test]
    async fn close_gives_a_stuck_writer_its_limit_and_no_more() {
        let (client, stream) = duplex(16);
        let server = tokio::spawn(async move { accept_scripted(stream, SIZE).await });
        let (session, mut ops) = join(client, &pinned(HOST, PORT)).await;
        let started = Instant::now();
        within(session.close()).await;
        let took = started.elapsed();
        assert!(took >= CLOSE_LIMIT && took < CLOSE_LIMIT * 2, "close took {took:?}");
        assert_eq!(rest(&mut ops).await, []);
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_connect_that_never_completes_times_out() {
        // With the queue full the system drops further connection attempts.
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let listener = socket.listen(1).unwrap();
        let addr = listener.local_addr().unwrap();
        let mut queued = Vec::new();
        for _ in 0..16 {
            let attempt = timeout(Duration::from_millis(100), TcpStream::connect(addr)).await;
            if let Ok(Ok(stream)) = attempt {
                queued.push(stream);
            }
        }
        let target = target("127.0.0.1", addr.port());
        let known = pinned("127.0.0.1", addr.port());
        let (sink, mut ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let limit = Duration::from_millis(200);
        let started = Instant::now();
        let sent = AtomicBool::default();
        let connecting = Session::connect_within(&target, &known, sink, limit, &sent);
        let session = timeout(WAIT, connecting).await.expect("connect hung");
        assert_eq!(text(&session.err().expect("the connect worked")), "rdp: connection timed out");
        assert!(started.elapsed() >= limit);
        assert_eq!(rest(&mut ops).await, []);
        drop((listener, queued));
    }

    #[tokio::test]
    async fn a_closed_port_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (sink, mut ops) = mpsc::unbounded_channel();
        let target = target("127.0.0.1", port);
        let known = pinned("127.0.0.1", port);
        let sent = AtomicBool::default();
        let sink = move |op| sink.send(op).unwrap();
        let connecting = Session::connect(&target, &known, sink, &sent);
        let session = timeout(WAIT, connecting).await.expect("connect hung");
        let refused = session.err().expect("the connect worked");
        assert!(matches!(&refused, AppError::Io(os) if !os.is_empty()), "{refused:?}");
        assert_eq!(rest(&mut ops).await, []);
    }

    // A server that is gone without a word (a VM turned off) leaves only the keepalive to notice.
    #[tokio::test]
    async fn the_keepalive_gives_up_on_a_dead_link_within_a_minute() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        keep_alive(&tcp).unwrap();
        // socket2 has no getter for the other two on Windows.
        let probes = SockRef::from(&tcp).tcp_keepalive_retries().unwrap();
        let limit = KEEPALIVE_IDLE + KEEPALIVE_INTERVAL * probes;
        assert!(limit < Duration::from_secs(60), "{probes} probes: {limit:?}");
    }

    async fn after_login(bytes: Vec<u8>, hang_up: bool) -> Option<Vec<FrameOp>> {
        after_login_in(bytes, hang_up, false).await
    }

    async fn after_login_in(
        bytes: Vec<u8>,
        hang_up: bool,
        reactivating: bool,
    ) -> Option<Vec<FrameOp>> {
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (mut framed, result) = accept_scripted(stream, SIZE).await;
            if reactivating {
                write(&mut framed, deactivate_all(&result)).await;
            }
            write(&mut framed, bytes).await;
            if !hang_up {
                std::future::pending::<()>().await;
            }
        });
        let known = pinned(HOST, PORT);
        let ops = off_thread(Duration::from_secs(3), move || async move {
            let connected = logged_in(client, &known).await;
            let (sink, mut ops) = mpsc::unbounded_channel();
            let session = Session::from_connected(connected, move |op| sink.send(op).unwrap());
            let mut left = Vec::new();
            let wait = Duration::from_millis(500);
            while let Some(op) = timeout(wait, ops.recv()).await.unwrap_or(None) {
                left.push(op);
            }
            drop(session);
            left
        })
        .await;
        server.abort();
        ops
    }

    #[tokio::test]
    async fn a_frame_of_length_zero_ends_the_session_once() {
        let ops = after_login(vec![0; 40], false).await.expect("the session spins");
        assert_eq!(ops, [FrameOp::Closed(NOT_UNDERSTOOD.into())]);
    }

    #[tokio::test]
    async fn empty_and_cut_short_frames_during_reactivation_do_not_make_the_session_spin() {
        for bytes in [vec![0; 40], vec![0x00, 0x00], vec![0x00, 0x80, 0x00]] {
            let ops = after_login_in(bytes.clone(), false, true).await;
            let ops = ops.unwrap_or_else(|| panic!("the session spins on {bytes:?}"));
            assert_eq!(ops, [FrameOp::Closed(NOT_UNDERSTOOD.into())], "{bytes:?}");
        }
    }

    #[tokio::test]
    async fn empty_and_cut_short_frames_do_not_make_the_session_spin() {
        for bytes in short_frames() {
            let ops = after_login(bytes.clone(), true).await;
            let ops = ops.unwrap_or_else(|| panic!("the session spins on {bytes:?}"));
            let closed = [FrameOp::Closed(NOT_UNDERSTOOD.into())];
            let lost = [FrameOp::Closed(LOST.into())];
            assert!(ops == closed || ops == lost, "{bytes:?}: {ops:?}");
        }
    }

    // Answers a Shutdown Request with Shutdown Denied if `answer`; returns the MCS disconnect
    // reason it read, if the client got that far.
    async fn shutting_down(
        mut framed: ScriptedFramed<DuplexStream>,
        result: Channels,
        answer: bool,
    ) -> Option<DisconnectReason> {
        loop {
            match read_client(&mut framed, &result).await {
                Ok(ClientPdu::ShutdownRequest) if answer => {
                    write(&mut framed, shutdown_denied(&result)).await;
                }
                Ok(ClientPdu::Disconnect(reason)) => return Some(reason),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    #[tokio::test]
    async fn close_completes_the_shutdown_the_server_denies() {
        let (server, session, mut ops) =
            scripted(SIZE, |framed, result| shutting_down(framed, result, true)).await;
        let started = Instant::now();
        within(session.close()).await;
        let took = started.elapsed();
        assert!(took < Duration::from_millis(250), "close took {took:?}");
        assert_eq!(within(server).await.unwrap(), Some(DisconnectReason::UserRequested));
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn close_gives_a_silent_server_a_second() {
        let (server, session, mut ops) =
            scripted(SIZE, |framed, result| shutting_down(framed, result, false)).await;
        let started = Instant::now();
        within(session.close()).await;
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(900) && took < Duration::from_millis(1500),
            "{took:?}"
        );
        assert_eq!(within(server).await.unwrap(), None);
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn a_dropped_session_ends_the_connection() {
        let (server, session, mut ops) =
            scripted(SIZE, |framed, result| shutting_down(framed, result, false)).await;
        drop(session);
        let ended = timeout(Duration::from_secs(1), server).await.expect("the connection stayed");
        assert_eq!(ended.unwrap(), None);
        assert_eq!(rest(&mut ops).await, []);
    }

    #[tokio::test]
    async fn ten_sessions_in_a_row_leave_nothing_behind() {
        let server =
            TestServer::listen(Options { login: Login::Tls, size: SIZE, remotefx: false }).await;
        let port = server.addr.unwrap().port();
        let known = pinned("127.0.0.1", port);
        for round in 0..11 {
            let target = target("127.0.0.1", port);
            let (sink, mut ops) = mpsc::unbounded_channel();
            let sent = AtomicBool::default();
            let sink = move |op| sink.send(op).unwrap();
            let connecting = Session::connect(&target, &known, sink, &sent);
            let session = timeout(Duration::from_secs(60), connecting).await.expect("connect hung");
            let session = session.unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            assert_eq!((session.width, session.height), SIZE);
            within(session.close()).await;
            assert_eq!(rest(&mut ops).await, [], "round {round}");
        }
    }

    #[test]
    fn rectangles_collapse_past_eight_or_past_the_image() {
        let rect = |left, top, right, bottom| InclusiveRectangle { left, top, right, bottom };
        let mut pending = Vec::new();
        for n in 0..8 {
            merge(&mut pending, rect(n * 10, 0, n * 10 + 1, 1), 256, 128);
        }
        assert_eq!(pending.len(), 8);
        merge(&mut pending, rect(100, 50, 101, 51), 256, 128);
        assert_eq!(pending, [rect(0, 0, 101, 51)]);

        let mut pending = Vec::new();
        merge(&mut pending, rect(0, 0, 255, 70), 256, 128);
        assert_eq!(pending.len(), 1);
        merge(&mut pending, rect(0, 60, 255, 127), 256, 128);
        assert_eq!(pending, [rect(0, 0, 255, 127)]);
    }

    #[test]
    fn the_single_pixel_rectangle_is_taken_as_it_is() {
        let origin = InclusiveRectangle::empty();
        let mut pending = Vec::new();
        merge(&mut pending, origin.clone(), 256, 128);
        assert_eq!(pending, [origin]);
    }

    #[test]
    fn rectangles_are_kept_inside_the_image() {
        let rect = |left, top, right, bottom| InclusiveRectangle { left, top, right, bottom };
        let mut pending = Vec::new();
        merge(&mut pending, rect(250, 120, 300, 200), 256, 128);
        assert_eq!(pending, [rect(250, 120, 255, 127)]);
        pending.clear();
        merge(&mut pending, rect(256, 0, 260, 4), 256, 128);
        merge(&mut pending, rect(0, 128, 4, 130), 256, 128);
        merge(&mut pending, rect(5, 5, 4, 4), 256, 128);
        assert!(pending.is_empty());
    }

    #[test]
    fn a_copied_region_is_tight_and_opaque() {
        let image =
            DecodedImage::new(ironrdp_graphics::image_processing::PixelFormat::RgbA32, 8, 4);
        let rect = InclusiveRectangle { left: 2, top: 1, right: 4, bottom: 2 };
        let copy = copy_region(&image, &rect);
        assert_eq!(copy, [0, 0, 0, 255].repeat(6));
    }

    async fn seen_input(server: &mut TestServer, count: usize) -> Vec<Seen> {
        let mut seen = Vec::new();
        while seen.len() < count {
            match within(server.seen.recv()).await.expect("the server is gone") {
                input @ (Seen::Key { .. }
                | Seen::Unicode { .. }
                | Seen::Locks(_)
                | Seen::Mouse(_)) => seen.push(input),
                _ => {}
            }
        }
        seen
    }

    fn key(down: bool, code: u8, extended: bool) -> Input {
        Input::Key { down, code, extended }
    }

    fn mouse_at(buttons: u16, x: u16, y: u16) -> Input {
        Input::Pointer { buttons, x, y }
    }

    async fn collecting(
        mut framed: ScriptedFramed<DuplexStream>,
        result: Channels,
        want: usize,
    ) -> Vec<Vec<FastPathInputEvent>> {
        let mut frames = Vec::new();
        let mut count = 0;
        while count < want {
            match read_client(&mut framed, &result).await {
                Ok(ClientPdu::Input(events)) => {
                    count += events.len();
                    frames.push(events);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        frames
    }

    #[tokio::test]
    async fn keys_locks_and_unicode_reach_the_real_server() {
        let (mut server, session, _ops) = rig(SIZE).await;
        let events = vec![
            key(true, 0x1e, false),
            key(false, 0x1e, false),
            key(true, 0x48, true),
            key(false, 0x48, true),
            Input::Unicode { down: true, codepoint: 0xe9 },
            Input::Unicode { down: false, codepoint: 0xe9 },
            Input::Locks { caps: true, num: true, scroll: false },
        ];
        within(session.input(events)).await.unwrap();
        assert_eq!(
            seen_input(&mut server, 7).await,
            [
                Seen::Key { code: 30, extended: false, down: true },
                Seen::Key { code: 30, extended: false, down: false },
                Seen::Key { code: 72, extended: true, down: true },
                Seen::Key { code: 72, extended: true, down: false },
                Seen::Unicode { unit: 233, down: true },
                Seen::Unicode { unit: 233, down: false },
                Seen::Locks(0x06),
            ]
        );
    }

    #[tokio::test]
    async fn each_lock_alone_reaches_the_real_server() {
        let (mut server, session, _ops) = rig(SIZE).await;
        let locks = |caps, num, scroll| Input::Locks { caps, num, scroll };
        let events = vec![
            locks(true, false, false),
            locks(false, true, false),
            locks(false, false, true),
            locks(false, false, false),
        ];
        within(session.input(events)).await.unwrap();
        let wanted = [Seen::Locks(0x04), Seen::Locks(0x02), Seen::Locks(0x01), Seen::Locks(0)];
        assert_eq!(seen_input(&mut server, 4).await, wanted);
    }

    #[tokio::test]
    async fn pointer_events_reach_the_real_server() {
        let (mut server, session, _ops) = rig(SIZE).await;
        let events = vec![
            mouse_at(0, 100, 50),
            mouse_at(0x01, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x04, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x08, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x10, 100, 50),
        ];
        within(session.input(events)).await.unwrap();
        assert_eq!(
            seen_input(&mut server, 7).await,
            [
                Seen::Mouse(Mouse::Move { x: 100, y: 50 }),
                Seen::Mouse(Mouse::Left(true)),
                Seen::Mouse(Mouse::Left(false)),
                Seen::Mouse(Mouse::Right(true)),
                Seen::Mouse(Mouse::Right(false)),
                Seen::Mouse(Mouse::Wheel(120)),
                Seen::Mouse(Mouse::Wheel(-120)),
            ]
        );
    }

    #[tokio::test]
    async fn buttons_the_real_server_misreads_are_right_on_the_wire() {
        let (server, session, _ops) =
            scripted(SIZE, |framed, result| collecting(framed, result, 9)).await;
        let events = vec![
            mouse_at(0, 100, 50),
            mouse_at(0x02, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x80, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x100, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x20, 100, 50),
            mouse_at(0, 100, 50),
            mouse_at(0x40, 100, 50),
        ];
        within(session.input(events)).await.unwrap();
        let wire: Vec<_> = within(server).await.unwrap().into_iter().flatten().collect();
        let extra = |flags| {
            FastPathInputEvent::MouseEventEx(MouseXPdu {
                flags,
                x_position: 100,
                y_position: 50,
            })
        };
        let mouse = |flags, units| {
            FastPathInputEvent::MouseEvent(MousePdu {
                flags,
                number_of_wheel_rotation_units: units,
                x_position: 100,
                y_position: 50,
            })
        };
        let middle = PointerFlags::MIDDLE_BUTTON_OR_WHEEL;
        let horizontal = PointerFlags::HORIZONTAL_WHEEL;
        let left = horizontal | PointerFlags::WHEEL_NEGATIVE | PointerFlags::from_bits_retain(0x88);
        let right = horizontal | PointerFlags::from_bits_retain(0x78);
        assert_eq!(
            wire,
            [
                mouse(PointerFlags::MOVE, 0),
                mouse(middle | PointerFlags::DOWN, 0),
                mouse(middle, 0),
                extra(PointerXFlags::BUTTON1 | PointerXFlags::DOWN),
                extra(PointerXFlags::BUTTON1),
                extra(PointerXFlags::BUTTON2 | PointerXFlags::DOWN),
                extra(PointerXFlags::BUTTON2),
                mouse(left, -120),
                mouse(right, 120),
            ]
        );
    }

    #[tokio::test]
    async fn six_hundred_events_arrive_complete_in_order_and_in_chunks() {
        let (server, session, _ops) =
            scripted(SIZE, |framed, result| collecting(framed, result, 600)).await;
        let keys: Vec<_> =
            (0..600).map(|n| key(n % 2 == 0, (n / 2 % 100 + 1) as u8, n / 2 % 3 == 0)).collect();
        within(session.input(keys)).await.unwrap();
        let frames = within(server).await.unwrap();
        assert!(frames.iter().all(|frame| frame.len() <= 255));
        let wire: Vec<_> = frames.into_iter().flatten().collect();
        assert_eq!(wire.len(), 600);
        for (n, event) in wire.iter().enumerate() {
            let mut flags =
                if n / 2 % 3 == 0 { KeyboardFlags::EXTENDED } else { KeyboardFlags::empty() };
            if n % 2 == 1 {
                flags |= KeyboardFlags::RELEASE;
            }
            let code = (n / 2 % 100 + 1) as u8;
            assert_eq!(*event, FastPathInputEvent::KeyboardEvent(flags, code), "{n}");
        }
    }

    #[tokio::test]
    async fn input_after_the_server_ended_the_session_is_not_found() {
        let (server, session, mut ops) = scripted(SIZE, |mut framed, _| async move {
            write(&mut framed, mcs_disconnect(DisconnectReason::ProviderInitiated)).await;
        })
        .await;
        within(server).await.unwrap();
        assert_eq!(rest(&mut ops).await.len(), 1);
        for events in [vec![key(true, 0x1e, false)], vec![mouse_at(0, 1, 1)]] {
            let sent = timeout(Duration::from_secs(1), session.input(events))
                .await
                .expect("input blocked");
            assert!(matches!(sent, Err(AppError::NotFound(_))), "{sent:?}");
        }
    }

    #[tokio::test]
    async fn input_after_close_is_not_found() {
        let (_server, session, _ops) = rig(SIZE).await;
        within(session.close()).await;
        let sent = timeout(Duration::from_secs(1), session.input(vec![key(true, 0x1e, false)]))
            .await
            .expect("input blocked");
        assert!(matches!(sent, Err(AppError::NotFound(_))), "{sent:?}");
    }

    #[tokio::test]
    async fn moves_do_not_wait_behind_a_writer_that_is_stuck() {
        let (_server, session, _ops) = scripted(SIZE, |framed, _result| async move {
            std::future::pending::<()>().await;
            drop(framed);
        })
        .await;
        let mut blocked = false;
        for n in 0..50_000u32 {
            let batch = (0..255).map(|k| key(k % 2 == 0, 1 + (n % 100) as u8, false)).collect();
            if timeout(Duration::from_millis(50), session.input(batch)).await.is_err() {
                blocked = true;
                break;
            }
        }
        assert!(blocked, "the queue never filled");
        for n in 0..1000 {
            within(session.input(vec![mouse_at(0, n, n)])).await.unwrap();
        }
    }

    // The queue is full and the writer is stuck until `release`; then the server returns every
    // input event up to the first button press.
    async fn full_queue() -> (oneshot::Sender<()>, JoinHandle<Vec<FastPathInputEvent>>, Session) {
        let (release, released) = oneshot::channel::<()>();
        let (server, session, _ops) = scripted(SIZE, move |mut framed, result| async move {
            released.await.ok();
            let mut events = Vec::new();
            while let Ok(pdu) = read_client(&mut framed, &result).await {
                if let ClientPdu::Input(batch) = pdu {
                    events.extend(batch);
                }
                let down = |event: &FastPathInputEvent| {
                    matches!(event, FastPathInputEvent::MouseEvent(mouse)
                        if mouse.flags.contains(PointerFlags::DOWN))
                };
                if events.iter().any(down) {
                    break;
                }
            }
            events
        })
        .await;
        for n in 0..50_000u32 {
            let batch = (0..255).map(|k| key(k % 2 == 0, 1 + (n % 100) as u8, false)).collect();
            if timeout(Duration::from_millis(50), session.input(batch)).await.is_err() {
                return (release, server, session);
            }
        }
        panic!("the queue never filled");
    }

    fn pressed_at_7_9() -> [FastPathInputEvent; 2] {
        let mouse = |flags| {
            FastPathInputEvent::MouseEvent(MousePdu {
                flags,
                number_of_wheel_rotation_units: 0,
                x_position: 7,
                y_position: 9,
            })
        };
        [mouse(PointerFlags::MOVE), mouse(PointerFlags::DOWN | PointerFlags::LEFT_BUTTON)]
    }

    #[tokio::test]
    async fn a_button_change_waits_for_room_and_moves_do_not() {
        let (release, server, session) = full_queue().await;
        for n in 0..10 {
            let sent = timeout(Duration::from_millis(100), session.input(vec![mouse_at(0, n, n)]));
            sent.await.expect("a move waited").unwrap();
        }
        let mut press = Box::pin(session.input(vec![mouse_at(0x01, 7, 9)]));
        let waited = timeout(Duration::from_millis(100), &mut press).await;
        assert!(waited.is_err(), "a button change did not wait");
        release.send(()).unwrap();
        within(&mut press).await.unwrap();
        let events = within(server).await.unwrap();
        assert!(events.ends_with(&pressed_at_7_9()), "{:?}", &events[events.len() - 2..]);
    }

    #[tokio::test]
    async fn a_press_whose_first_attempt_was_cancelled_is_still_sent() {
        let (release, server, session) = full_queue().await;
        let first = timeout(Duration::from_millis(100), session.input(vec![mouse_at(0x01, 7, 9)]));
        assert!(first.await.is_err(), "the press did not wait");
        let mut again = Box::pin(session.input(vec![mouse_at(0x01, 7, 9)]));
        let waited = timeout(Duration::from_millis(100), &mut again).await;
        assert!(waited.is_err(), "the press was taken for a move");
        release.send(()).unwrap();
        within(&mut again).await.unwrap();
        let events = within(server).await.unwrap();
        assert!(events.ends_with(&pressed_at_7_9()), "{:?}", &events[events.len() - 2..]);
    }

    #[tokio::test]
    async fn a_move_outside_the_picture_is_held_at_its_edge() {
        let (mut server, session, _ops) = rig(SIZE).await;
        within(session.input(vec![mouse_at(0, 65535, 65535), mouse_at(0, 256, 0)])).await.unwrap();
        assert_eq!(
            seen_input(&mut server, 2).await,
            [
                Seen::Mouse(Mouse::Move { x: SIZE.0 - 1, y: SIZE.1 - 1 }),
                Seen::Mouse(Mouse::Move { x: SIZE.0 - 1, y: 0 }),
            ]
        );
    }

    async fn bounded<Fut: Future<Output = ()>>(work: impl FnOnce() -> Fut + Send + 'static) {
        off_thread(Duration::from_secs(30), work).await.expect("the client hung");
    }

    async fn clip_rig() -> (TestServer, Session, Ops) {
        clip_rig_within(CLIPBOARD_TICK, PASTE_LIMIT).await
    }

    async fn clip_rig_within(tick: Duration, limit: Duration) -> (TestServer, Session, Ops) {
        let (client, stream) = duplex(1 << 16);
        let options = Options { login: Login::Tls, size: SIZE, remotefx: false };
        let server = TestServer::on(options, stream).await;
        let target = target(HOST, PORT);
        let known = pinned(HOST, PORT);
        let sent = AtomicBool::default();
        let connected =
            within(login(client, local(), &target, &known, clipboard::attach, &sent)).await;
        let connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
        let (sink, ops) = mpsc::unbounded_channel();
        let session = Session::from_connected_within(
            connected,
            move |op| sink.send(op).unwrap(),
            RESIZE_LIMIT,
            None,
            tick,
            limit,
        );
        (server, session, ops)
    }

    async fn heard_where(server: &mut TestServer, wanted: impl Fn(&Seen) -> bool) -> Seen {
        loop {
            let seen = within(server.seen.recv()).await.expect("the server is gone");
            if wanted(&seen) {
                return seen;
            }
            assert!(!matches!(seen, Seen::Gone(_)), "the connection ended: {seen:?}");
        }
    }

    async fn heard(server: &mut TestServer, wanted: Seen) {
        heard_where(server, |seen| *seen == wanted).await;
    }

    async fn clip_ready() -> (TestServer, Session, Ops) {
        let (mut server, session, ops) = clip_rig().await;
        heard(&mut server, Seen::ClipReady).await;
        heard(&mut server, Seen::ClipFormats(Vec::new())).await;
        (server, session, ops)
    }

    // What the server hears about the clipboard during the next `millis`.
    async fn clip_events(server: &mut TestServer, millis: u64) -> Vec<Seen> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(millis);
        let mut heard = Vec::new();
        while let Ok(Some(seen)) = tokio::time::timeout_at(deadline, server.seen.recv()).await {
            if matches!(
                seen,
                Seen::ClipReady | Seen::ClipFormats(_) | Seen::ClipRequest(_) | Seen::ClipData(_)
            ) {
                heard.push(seen);
            }
        }
        heard
    }

    async fn remote_text(server: &mut TestServer, text: &str) {
        server.remote_copies(&[13]);
        heard(server, Seen::ClipRequest(13)).await;
        server.remote_sends(text);
    }

    async fn pasted(server: &mut TestServer) -> Seen {
        server.remote_pastes(13);
        heard_where(server, |seen| matches!(seen, Seen::ClipData(_))).await
    }

    #[tokio::test]
    async fn text_copied_on_the_remote_arrives_once() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            remote_text(&mut server, "from the server \u{1f980}").await;
            let text = "from the server \u{1f980}";
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard(text.into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            quiet(&mut ops).await;
        })
        .await;
    }

    #[tokio::test]
    async fn remote_line_ends_become_line_feeds_and_nuls_vanish() {
        bounded(|| async {
            let (mut server, session, mut ops) = clip_ready().await;
            remote_text(&mut server, "a\r\nb\0c\r\n").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("a\nbc\n".into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            session.ack();
            remote_text(&mut server, "x\ny").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("x\ny".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn remote_formats_without_text_are_not_requested() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            server.remote_copies(&[2, 8]);
            server.remote_copies(&[]);
            assert_eq!(clip_events(&mut server, 500).await, []);
            quiet(&mut ops).await;
            remote_text(&mut server, "text").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("text".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn remote_copies_in_a_row_keep_one_request_in_flight_and_the_last_text_wins() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            for _ in 0..3 {
                server.remote_copies(&[13]);
            }
            heard(&mut server, Seen::ClipRequest(13)).await;
            assert_eq!(clip_events(&mut server, 300).await, []);

            server.remote_sends("old");
            heard(&mut server, Seen::ClipRequest(13)).await;
            assert_eq!(clip_events(&mut server, 300).await, []);
            quiet(&mut ops).await;

            server.remote_sends("last");
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("last".into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            assert_eq!(clip_events(&mut server, 300).await, []);
            quiet(&mut ops).await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_copy_after_the_answer_is_requested_again() {
        bounded(|| async {
            let (mut server, session, mut ops) = clip_ready().await;
            remote_text(&mut server, "one").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("one".into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            session.ack();
            remote_text(&mut server, "two").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("two".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn a_local_text_reaches_the_remote_with_carriage_returns() {
        bounded(|| async {
            let (mut server, session, _ops) = clip_ready().await;
            session.clipboard("x\ny".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("x\r\ny".into())));

            session.clipboard("a\r\nb\n\nc".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("a\r\nb\r\n\r\nc".into())));
        })
        .await;
    }

    #[tokio::test]
    async fn a_second_local_text_replaces_the_first() {
        bounded(|| async {
            let (mut server, session, _ops) = clip_ready().await;
            session.clipboard("first".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            session.clipboard("second".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("second".into())));
        })
        .await;
    }

    async fn announced_once(mut server: TestServer) {
        let lists = clip_events(&mut server, 700).await;
        let with_text = lists.iter().filter(|seen| **seen == Seen::ClipFormats(vec![13]));
        assert_eq!(with_text.count(), 1, "{lists:?}");
        assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("early".into())));
    }

    #[tokio::test]
    async fn a_local_text_set_before_the_channel_is_ready_is_announced_once() {
        bounded(|| async {
            let (server, session, _ops) = clip_rig().await;
            session.clipboard("early".into()).await.unwrap();
            announced_once(server).await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_local_text_set_while_the_channel_starts_is_announced_once() {
        bounded(|| async {
            let (mut server, session, _ops) = clip_rig().await;
            heard_where(&mut server, |seen| matches!(seen, Seen::ClipFormats(_))).await;
            session.clipboard("early".into()).await.unwrap();
            announced_once(server).await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_request_for_a_format_that_is_not_text_gets_an_error_and_the_session_goes_on() {
        bounded(|| async {
            let (mut server, session, mut ops) = clip_ready().await;
            session.clipboard("held".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            server.remote_pastes(2);
            let answer = heard_where(&mut server, |seen| matches!(seen, Seen::ClipData(_))).await;
            assert_eq!(answer, Seen::ClipData(None));
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("held".into())));
            remote_text(&mut server, "still here").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("still here".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn a_request_for_text_when_none_is_held_gets_an_error_and_the_session_goes_on() {
        bounded(|| async {
            let (mut server, session, mut ops) = clip_ready().await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(None));
            session.clipboard("now".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("now".into())));
            remote_text(&mut server, "and back").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("and back".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn empty_and_oversized_remote_text_send_nothing() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            remote_text(&mut server, "").await;
            remote_text(&mut server, "\0\0").await;
            remote_text(&mut server, &"x".repeat(MAX_TEXT + 1)).await;
            quiet(&mut ops).await;
            remote_text(&mut server, "after").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("after".into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            quiet(&mut ops).await;
        })
        .await;
    }

    #[tokio::test]
    async fn remote_text_of_exactly_a_mebibyte_arrives() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            remote_text(&mut server, &"x".repeat(MAX_TEXT)).await;
            match next(&mut ops).await {
                FrameOp::Clipboard(text) => assert_eq!(text.len(), MAX_TEXT),
                other => panic!("expected the text, got {other:?}"),
            }
        })
        .await;
    }

    #[tokio::test]
    async fn local_text_over_a_mebibyte_is_not_announced_and_a_mebibyte_is() {
        bounded(|| async {
            let (mut server, session, _ops) = clip_ready().await;
            session.clipboard("x".repeat(MAX_TEXT + 1)).await.unwrap();
            assert_eq!(clip_events(&mut server, 500).await, []);

            session.clipboard("x".repeat(MAX_TEXT)).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            match pasted(&mut server).await {
                Seen::ClipData(Some(text)) => assert_eq!(text.len(), MAX_TEXT),
                other => panic!("expected the text, got {other:?}"),
            }
        })
        .await;
    }

    #[tokio::test]
    async fn local_text_over_a_mebibyte_withdraws_the_text_held_before() {
        bounded(|| async {
            let (mut server, session, _ops) = clip_ready().await;
            session.clipboard("secret".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("secret".into())));

            session.clipboard("x".repeat(MAX_TEXT + 1)).await.unwrap();
            heard(&mut server, Seen::ClipFormats(Vec::new())).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(None));
        })
        .await;
    }

    #[tokio::test]
    async fn text_nobody_asked_for_is_not_passed_on() {
        bounded(|| async {
            let (mut server, _session, mut ops) = clip_ready().await;
            server.remote_sends("unasked");
            quiet(&mut ops).await;
            remote_text(&mut server, "asked").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("asked".into()));
        })
        .await;
    }

    #[tokio::test]
    async fn remote_texts_pushed_without_acknowledgement_stop_after_two_and_the_last_wins() {
        bounded(|| async {
            let (mut server, session, mut ops) = clip_ready().await;
            for n in 0..2 {
                remote_text(&mut server, &n.to_string()).await;
                assert_eq!(next(&mut ops).await, FrameOp::Clipboard(n.to_string()));
                assert_eq!(next(&mut ops).await, FrameOp::Sync);
            }
            for n in 2..10 {
                remote_text(&mut server, &n.to_string()).await;
            }
            quiet(&mut ops).await;
            quiet(&mut ops).await;

            session.ack();
            session.ack();
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("9".into()));
            assert_eq!(next(&mut ops).await, FrameOp::Sync);
            quiet(&mut ops).await;
        })
        .await;
    }

    #[tokio::test]
    async fn the_clipboard_still_works_after_the_timeout_tick() {
        bounded(|| async {
            let tick = Duration::from_millis(100);
            let (mut server, session, mut ops) = ready_within(tick, PASTE_LIMIT).await;
            tokio::time::sleep(tick * 3).await;
            remote_text(&mut server, "late").await;
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("late".into()));
            session.clipboard("local".into()).await.unwrap();
            heard(&mut server, Seen::ClipFormats(vec![13])).await;
            assert_eq!(pasted(&mut server).await, Seen::ClipData(Some("local".into())));
        })
        .await;
    }

    async fn ready_within(tick: Duration, limit: Duration) -> (TestServer, Session, Ops) {
        let (mut server, session, ops) = clip_rig_within(tick, limit).await;
        heard(&mut server, Seen::ClipReady).await;
        heard(&mut server, Seen::ClipFormats(Vec::new())).await;
        (server, session, ops)
    }

    #[tokio::test]
    async fn an_unanswered_paste_request_is_given_up_and_the_next_copy_is_requested() {
        bounded(|| async {
            let limit = Duration::from_millis(600);
            let (mut server, _session, mut ops) =
                ready_within(Duration::from_millis(100), limit).await;
            let asked = Instant::now();
            server.remote_copies(&[13]);
            heard(&mut server, Seen::ClipRequest(13)).await;
            server.remote_copies(&[13]);
            assert_eq!(clip_events(&mut server, 300).await, []);
            heard(&mut server, Seen::ClipRequest(13)).await;
            assert!(asked.elapsed() >= limit, "{:?}", asked.elapsed());
            server.remote_sends("second");
            assert_eq!(next(&mut ops).await, FrameOp::Clipboard("second".into()));
        })
        .await;
    }

    fn clip_pdu(kind: u16, flags: u16, data: &[u8]) -> Vec<u8> {
        let mut pdu = [kind.to_le_bytes(), flags.to_le_bytes()].concat();
        pdu.extend((data.len() as u32).to_le_bytes());
        pdu.extend(data);
        pdu
    }

    // A short format list entry: the id and 32 bytes of name.
    fn format_entry(id: u32, ascii_name: &str) -> Vec<u8> {
        let mut entry = id.to_le_bytes().to_vec();
        entry.extend(ascii_name.as_bytes());
        entry.resize(36, 0);
        entry
    }

    const ASCII_NAMES: u16 = 4;
    const RESPONSE_OK: u16 = 1;

    fn clip_frame(channels: &Channels, pdu: Vec<u8>) -> Vec<u8> {
        let flags = ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST;
        channel_chunk(channels, clipboard_channel(), flags, pdu.len() as u32, &pdu)
    }

    // Capabilities (one general set, no flags), Monitor Ready and Format List Response: the
    // channel is ready once the client has read them.
    async fn clip_start(framed: &mut ScriptedFramed<DuplexStream>, channels: &Channels) {
        let general = [1, 0, 12, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        let capabilities = [&[1, 0, 0, 0][..], &general].concat();
        write(framed, clip_frame(channels, clip_pdu(7, 0, &capabilities))).await;
        write(framed, clip_frame(channels, clip_pdu(1, 0, &[]))).await;
        write(framed, clip_frame(channels, clip_pdu(3, RESPONSE_OK, &[]))).await;
    }

    #[tokio::test]
    async fn a_request_for_data_before_the_channel_is_ready_is_ignored() {
        let script = |mut framed, channels| async move {
            let request = clip_pdu(4, 0, &13u32.to_le_bytes());
            write(&mut framed, clip_frame(&channels, request)).await;
            clip_start(&mut framed, &channels).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        let mut expected = Screen::new(SIZE.0, SIZE.1);
        expected.paint(8, 8, 16, 16, shade(4));
        let mut screen = Screen::new(SIZE.0, SIZE.1);
        while screen.rgba != expected.rgba {
            flush(&mut ops, &mut screen).await;
        }
        quiet(&mut ops).await;
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn a_paste_request_answered_as_a_file_list_is_given_up_too() {
        let (tick, limit) = (Duration::from_millis(100), Duration::from_millis(600));
        let script = move |mut framed, channels| async move {
            let send = |pdu: Vec<u8>| clip_frame(&channels, pdu);
            clip_start(&mut framed, &channels).await;

            let files = format_entry(13, "FileGroupDescriptorW");
            write(&mut framed, send(clip_pdu(2, ASCII_NAMES, &files))).await;
            let no_files = 0u32.to_le_bytes();
            write(&mut framed, send(clip_pdu(5, RESPONSE_OK, &no_files))).await;

            let text = format_entry(13, "");
            write(&mut framed, send(clip_pdu(2, 0, &text))).await;
            tokio::time::sleep(limit + tick * 3).await;
            let late: Vec<u8> = "late\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
            write(&mut framed, send(clip_pdu(5, RESPONSE_OK, &late))).await;
            framed
        };
        let (server, _session, mut ops) = scripted_with(SIZE, attach, (tick, limit), script).await;
        assert_eq!(next(&mut ops).await, FrameOp::Clipboard("late".into()));
        drop(within(server).await.unwrap());
    }

    #[tokio::test]
    async fn the_clipboard_after_close_is_not_found() {
        bounded(|| async {
            let (_server, session, _ops) = clip_ready().await;
            within(session.close()).await;
            let result = within(session.clipboard("x".into())).await;
            assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
        })
        .await;
    }

    const SHORT_LIMIT: Duration = Duration::from_millis(800);

    type Layout = (u32, u32, Option<u32>);

    async fn display_rig(limit: Duration) -> (TestServer, Session, Ops) {
        display_rig_first(limit, None).await
    }

    async fn display_rig_first(
        limit: Duration,
        first: Option<Duration>,
    ) -> (TestServer, Session, Ops) {
        display_rig_at(SIZE, limit, first).await
    }

    async fn display_rig_at(
        size: (u16, u16),
        limit: Duration,
        first: Option<Duration>,
    ) -> (TestServer, Session, Ops) {
        let (client, stream) = duplex(1 << 16);
        let options = Options { login: Login::Tls, size, remotefx: false };
        let server = TestServer::on(options, stream).await;
        let (target, known) = (target(HOST, PORT), pinned(HOST, PORT));
        let sent = AtomicBool::default();
        let connected = within(login(client, local(), &target, &known, attach, &sent)).await;
        let connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
        let (sink, ops) = mpsc::unbounded_channel();
        let sink = move |op| sink.send(op).unwrap();
        let session = Session::from_connected_within(
            connected,
            sink,
            limit,
            first,
            CLIPBOARD_TICK,
            PASTE_LIMIT,
        );
        // A layout waits for the server's first picture.
        server.display.send(bitmap(0, 0, 8, 8, shade(1))).unwrap();
        let mut ops = ops;
        drawn(&session, &mut ops).await;
        (server, session, ops)
    }

    // Until the end of the flush that brings the view a rectangle; the flushes are acknowledged.
    async fn drawn(session: &Session, ops: &mut Ops) {
        let mut rect = false;
        loop {
            match next(ops).await {
                FrameOp::Rect { .. } => rect = true,
                FrameOp::Sync => {
                    session.ack();
                    if rect {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    // The layout requests the server hears during the next `millis`.
    async fn layouts(server: &mut TestServer, millis: u64) -> Vec<Layout> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(millis);
        let mut heard = Vec::new();
        while let Ok(Some(seen)) = tokio::time::timeout_at(deadline, server.seen.recv()).await {
            if let Seen::Layout { width, height, scale } = seen {
                heard.push((width, height, scale));
            }
        }
        heard
    }

    async fn layout(server: &mut TestServer) -> Layout {
        let seen = heard_where(server, |seen| matches!(seen, Seen::Layout { .. })).await;
        let Seen::Layout { width, height, scale } = seen else { unreachable!() };
        (width, height, scale)
    }

    // The next size the view is told; the flushes on the way are acknowledged.
    async fn resized(session: &Session, ops: &mut Ops) -> (u16, u16) {
        loop {
            match next(ops).await {
                FrameOp::Resize { w, h } => return (w, h),
                FrameOp::Sync => session.ack(),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn resize_asks_the_server_and_the_new_size_reaches_the_view() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert_eq!(resized(&session, &mut ops).await, (640, 480));
            drawn(&session, &mut ops).await;
            server.display.send(bitmap(600, 440, 40, 40, shade(90))).unwrap();
            loop {
                match next(&mut ops).await {
                    FrameOp::Rect { x, y, w, h, .. } => {
                        assert_eq!((x, y, w, h), (600, 440, 40, 40));
                        break;
                    }
                    FrameOp::Sync => session.ack(),
                    other => panic!("unexpected {other:?}"),
                }
            }
        })
        .await;
    }

    #[tokio::test]
    async fn a_burst_of_sizes_reaches_the_server_as_first_and_last() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            // Once this one is through the channel is ready, so the burst starts at the first call.
            session.resize(320, 240, 100).await.unwrap();
            assert_eq!(resized(&session, &mut ops).await, (320, 240));
            assert_eq!(layouts(&mut server, 0).await.len(), 1);

            let sizes: Vec<(u16, u16)> = (0..10).map(|i| (400 + 20 * i, 300 + 10 * i)).collect();
            for (width, height) in &sizes {
                session.resize(*width, *height, 100).await.unwrap();
            }
            let last = *sizes.last().unwrap();
            while resized(&session, &mut ops).await != last {}
            let asked = layouts(&mut server, 400).await;
            let end = (u32::from(last.0), u32::from(last.1), Some(100));
            assert!((2..=3).contains(&asked.len()), "{asked:?}");
            let ends = (asked[0], asked[asked.len() - 1]);
            assert_eq!(ends, ((400, 300, Some(100)), end), "{asked:?}");
            for (width, height, _) in &asked {
                assert!(sizes.contains(&(*width as u16, *height as u16)), "{asked:?}");
            }
        })
        .await;
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_request_leaves_input_working_and_a_new_size_goes_out_later()
    {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(SHORT_LIMIT).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));

            session.resize(800, 600, 100).await.unwrap();
            session.input(vec![key(true, 0x1e, false)]).await.unwrap();
            let seen = heard_where(&mut server, |seen| {
                matches!(seen, Seen::Key { .. } | Seen::Layout { .. })
            })
            .await;
            assert_eq!(seen, Seen::Key { code: 0x1e, extended: false, down: true });

            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert!(asked.elapsed() >= SHORT_LIMIT, "{:?}", asked.elapsed());
            while let Ok(op) = ops.try_recv() {
                assert!(!matches!(op, FrameOp::Resize { .. }), "the picture changed size");
            }
        })
        .await;
    }

    #[tokio::test]
    async fn sizes_are_adjusted_and_what_is_already_the_size_is_not_asked() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            session.resize(641, 150, 90).await.unwrap();
            assert_eq!(resized(&session, &mut ops).await, (640, 200));
            assert_eq!(layouts(&mut server, 0).await, [(640, 200, None)]);

            session.resize(640, 200, 100).await.unwrap();
            session.resize(641, 200, 100).await.unwrap();
            session.resize(641, 100, 100).await.unwrap();
            assert!(layouts(&mut server, 500).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn a_size_set_before_the_channel_is_ready_is_asked_once() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            session.resize(500, 300, 100).await.unwrap();
            assert_eq!(resized(&session, &mut ops).await, (500, 300));
            assert_eq!(layouts(&mut server, 500).await, [(500, 300, Some(100))]);
        })
        .await;
    }

    #[tokio::test]
    async fn sizes_and_scales_outside_the_ranges_are_brought_inside_before_they_are_sent() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            let cases = [
                ((1, 1, 99), (200, 200, None)),
                ((401, 301, 500), (400, 301, Some(500))),
                ((500, 400, 501), (500, 400, None)),
                ((300, 250, 0), (300, 250, None)),
                ((301, 201, u32::MAX), (300, 201, None)),
            ];
            for ((width, height, scale), expected) in cases {
                session.resize(width, height, scale).await.unwrap();
                assert_eq!(layout(&mut server).await, expected);
                resized(&session, &mut ops).await;
            }
        })
        .await;
    }

    #[tokio::test]
    async fn a_size_over_the_pixel_cap_is_not_sent_and_the_next_one_is() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            session.resize(8192, 8192, 100).await.unwrap();
            session.resize(u16::MAX, u16::MAX, 100).await.unwrap();
            assert!(layouts(&mut server, 500).await.is_empty());
            session.resize(700, 500, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (700, 500, Some(100)));
            assert_eq!(resized(&session, &mut ops).await, (700, 500));
        })
        .await;
    }

    #[tokio::test]
    async fn a_server_that_picks_another_size_is_not_asked_again() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(RESIZE_LIMIT).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            let chosen = ironrdp_server::DesktopSize { width: 600, height: 400 };
            server.display.send(DisplayUpdate::Resize(chosen)).unwrap();
            assert_eq!(resized(&session, &mut ops).await, (600, 400));
            assert!(layouts(&mut server, 600).await.is_empty());
        })
        .await;
    }

    const AFTER_LIMITS: u64 = 3 * SHORT_LIMIT.as_millis() as u64;
    const SHORT_FIRST: Duration = Duration::from_millis(200);

    #[tokio::test]
    async fn the_first_request_is_asked_again_soon_and_then_once_more_at_the_limit() {
        bounded(|| async {
            let (mut server, session, _ops) =
                display_rig_first(SHORT_LIMIT, Some(SHORT_FIRST)).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            let again = asked.elapsed();
            assert!(again >= SHORT_FIRST && again < SHORT_LIMIT, "{again:?}");
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert!(asked.elapsed() >= SHORT_FIRST + SHORT_LIMIT, "{:?}", asked.elapsed());
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    // What both test servers do while their login goes on.
    #[tokio::test]
    async fn a_first_request_refused_at_the_open_is_shown_soon() {
        bounded(|| async {
            let (mut server, session, mut ops) =
                display_rig_first(SHORT_LIMIT, Some(SHORT_FIRST)).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            server.answers_layouts.store(true, Ordering::SeqCst);
            assert_eq!(resized(&session, &mut ops).await, (640, 480));
            assert!(asked.elapsed() < SHORT_LIMIT, "{:?}", asked.elapsed());
        })
        .await;
    }

    #[tokio::test]
    async fn a_later_request_keeps_the_limit_and_the_one_repeat() {
        bounded(|| async {
            let (mut server, session, mut ops) =
                display_rig_first(SHORT_LIMIT, Some(SHORT_FIRST)).await;
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(resized(&session, &mut ops).await, (640, 480));
            assert_eq!(layouts(&mut server, 0).await.len(), 1);

            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(800, 600, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert!(asked.elapsed() >= SHORT_LIMIT, "{:?}", asked.elapsed());
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    // The view asks for its size right after the open, which is mostly the size the server gave.
    #[tokio::test]
    async fn a_first_request_for_the_size_shown_uses_up_the_early_ask_again() {
        bounded(|| async {
            let (mut server, session, _ops) =
                display_rig_at((640, 480), SHORT_LIMIT, Some(SHORT_FIRST)).await;
            session.resize(640, 480, 100).await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(layouts(&mut server, 0).await.is_empty());

            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(800, 600, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert!(asked.elapsed() >= SHORT_LIMIT, "{:?}", asked.elapsed());
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn a_new_size_asked_before_the_first_is_asked_again_takes_its_place() {
        bounded(|| async {
            let (mut server, session, _ops) =
                display_rig_first(SHORT_LIMIT, Some(SHORT_FIRST)).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            session.resize(800, 600, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert_eq!(layout(&mut server).await, (800, 600, Some(100)));
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn an_ignored_request_is_asked_once_more_and_then_given_up() {
        bounded(|| async {
            let (mut server, session, _ops) = display_rig(SHORT_LIMIT).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            let asked = Instant::now();
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert!(asked.elapsed() >= SHORT_LIMIT, "{:?}", asked.elapsed());
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn a_size_asked_again_after_the_server_chose_another_is_repeated_once() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(SHORT_LIMIT).await;
            server.answers_layouts.store(false, Ordering::SeqCst);
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            let chosen = ironrdp_server::DesktopSize { width: 600, height: 400 };
            server.display.send(DisplayUpdate::Resize(chosen)).unwrap();
            assert_eq!(resized(&session, &mut ops).await, (600, 400));

            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn the_size_already_shown_is_not_asked_back_after_the_server_changes_it() {
        bounded(|| async {
            let (mut server, session, mut ops) = display_rig(SHORT_LIMIT).await;
            session.resize(640, 480, 100).await.unwrap();
            assert_eq!(layout(&mut server).await, (640, 480, Some(100)));
            assert_eq!(resized(&session, &mut ops).await, (640, 480));

            session.resize(640, 480, 100).await.unwrap();
            let chosen = ironrdp_server::DesktopSize { width: 600, height: 400 };
            server.display.send(DisplayUpdate::Resize(chosen)).unwrap();
            assert_eq!(resized(&session, &mut ops).await, (600, 400));
            assert!(layouts(&mut server, AFTER_LIMITS).await.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn without_display_control_resize_is_accepted_and_nothing_is_written() {
        bounded(|| async {
            let (client, stream) = duplex(1 << 16);
            let (quiet_tx, quiet_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut framed, channels) = accept_scripted(stream, SIZE).await;
                let read = timeout(Duration::from_millis(500), read_client(&mut framed, &channels));
                let silent = read.await.is_err();
                quiet_tx.send(silent).unwrap();
                read_client(&mut framed, &channels).await
            });
            let (target, known) = (target(HOST, PORT), pinned(HOST, PORT));
            let sent = AtomicBool::default();
            let connected = within(login(client, local(), &target, &known, attach, &sent)).await;
            let connected = connected.unwrap_or_else(|e| panic!("login failed: {e:?}"));
            let (sink, _ops) = mpsc::unbounded_channel();
            let session = Session::from_connected(connected, move |op| sink.send(op).unwrap());
            session.resize(640, 480, 100).await.unwrap();
            session.resize(800, 600, 100).await.unwrap();
            assert!(within(quiet_rx).await.unwrap(), "the client wrote something");
            // The script does read what the client writes, so the silence above means something.
            session.input(vec![key(true, 0x1e, false)]).await.unwrap();
            let read = within(server).await.unwrap().unwrap();
            assert!(matches!(read, ClientPdu::Input(_)), "{read:?}");
        })
        .await;
    }

    #[tokio::test]
    async fn resize_after_close_is_not_found() {
        bounded(|| async {
            let (_server, session, _ops) = display_rig(RESIZE_LIMIT).await;
            within(session.close()).await;
            let result = within(session.resize(640, 480, 100)).await;
            assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
        })
        .await;
    }

    // The layouts and frame acknowledgements the client writes during the next `millis`.
    async fn layouts_and_acks(
        framed: &mut ScriptedFramed<DuplexStream>,
        channels: &Channels,
        millis: u64,
    ) -> Vec<ClientPdu> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(millis);
        let mut heard = Vec::new();
        loop {
            let read = tokio::time::timeout_at(deadline, read_client(framed, channels)).await;
            let Ok(Ok(pdu)) = read else { break };
            if matches!(pdu, ClientPdu::Layout | ClientPdu::FrameAcknowledge(_)) {
                heard.push(pdu);
            }
        }
        heard
    }

    async fn display_ready(framed: &mut ScriptedFramed<DuplexStream>, channels: &Channels) {
        open_display(framed, channels).await;
        dynamic_message(framed, channels, &display_caps(12)).await;
    }

    // xrdp 0.10.1 crashes on a frame acknowledgement that reaches it after a layout it has begun to
    // apply, which is what a layout sent during the first picture after the login gets.
    #[tokio::test]
    async fn a_layout_waits_for_the_first_picture_and_the_end_of_its_frame() {
        let (asked_tx, asked) = oneshot::channel();
        let (drawn_tx, drawn_rx) = oneshot::channel();
        let script = move |mut framed, channels| async move {
            display_ready(&mut framed, &channels).await;
            asked.await.unwrap();
            let before = layouts_and_acks(&mut framed, &channels, 300).await;
            write(&mut framed, frame_marker(FrameAction::Begin, 7)).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            drawn_rx.await.unwrap();
            let inside = layouts_and_acks(&mut framed, &channels, 300).await;
            write(&mut framed, frame_marker(FrameAction::End, 7)).await;
            let after = layouts_and_acks(&mut framed, &channels, 300).await;
            (before, inside, after)
        };
        let (server, session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        drawn(&session, &mut ops).await;
        drawn_tx.send(()).unwrap();
        let (before, inside, after) = within(server).await.unwrap();
        assert_eq!(before, []);
        assert_eq!(inside, []);
        assert_eq!(after, [ClientPdu::FrameAcknowledge(7), ClientPdu::Layout]);
    }

    // Windows marks no frames.
    #[tokio::test]
    async fn without_frame_markers_a_layout_waits_for_the_first_picture() {
        let (asked_tx, asked) = oneshot::channel();
        let script = move |mut framed, channels| async move {
            display_ready(&mut framed, &channels).await;
            asked.await.unwrap();
            let before = layouts_and_acks(&mut framed, &channels, 300).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            let after = layouts_and_acks(&mut framed, &channels, 300).await;
            (before, after)
        };
        let (server, session, _ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        let (before, after) = within(server).await.unwrap();
        assert_eq!(before, []);
        assert_eq!(after, [ClientPdu::Layout]);
    }

    #[tokio::test]
    async fn a_reactivation_closes_the_frame_the_server_left_open() {
        const BIGGER: (u16, u16) = (512, 256);
        let (asked_tx, asked) = oneshot::channel();
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (mut framed, channels, acceptor) =
                accept_scripted_keeping(stream, SIZE, u32::MAX).await;
            display_ready(&mut framed, &channels).await;
            write(&mut framed, frame_marker(FrameAction::Begin, 7)).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            let mut framed = reactivate_scripted(framed, &channels, acceptor, BIGGER, &[]).await;
            asked.await.unwrap();
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(5))).await;
            layouts_and_acks(&mut framed, &channels, 300).await
        });
        let (session, mut ops) = join_with(client, &pinned(HOST, PORT), attach, CLIPBOARD).await;
        assert_eq!(resized(&session, &mut ops).await, BIGGER);
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        assert_eq!(within(server).await.unwrap(), [ClientPdu::Layout]);
    }

    // The scripted servers below listen up to just before the limit, then just after it.
    const BEFORE_LIMIT: u64 = RESIZE_LIMIT.as_millis() as u64 - 100;

    #[tokio::test(start_paused = true)]
    async fn without_a_picture_a_layout_goes_out_at_the_limit() {
        let (asked_tx, asked) = oneshot::channel();
        let script = move |mut framed, channels| async move {
            display_ready(&mut framed, &channels).await;
            asked.await.unwrap();
            let before = layouts_and_acks(&mut framed, &channels, BEFORE_LIMIT).await;
            let after = layouts_and_acks(&mut framed, &channels, 200).await;
            (before, after)
        };
        let (server, session, _ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        let (before, after) = timeout(Duration::from_secs(60), server).await.unwrap().unwrap();
        assert_eq!(before, []);
        assert_eq!(after, [ClientPdu::Layout]);
    }

    // A frame end the client cannot see, in a fragmented update, or one that never comes.
    #[tokio::test(start_paused = true)]
    async fn inside_a_frame_that_does_not_end_a_layout_goes_out_at_the_limit() {
        let (asked_tx, asked) = oneshot::channel();
        let script = move |mut framed, channels| async move {
            display_ready(&mut framed, &channels).await;
            write(&mut framed, frame_marker(FrameAction::Begin, 7)).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            asked.await.unwrap();
            let before = layouts_and_acks(&mut framed, &channels, BEFORE_LIMIT).await;
            let after = layouts_and_acks(&mut framed, &channels, 200).await;
            (before, after)
        };
        let (server, session, mut ops) = scripted_with(SIZE, attach, CLIPBOARD, script).await;
        drawn(&session, &mut ops).await;
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        let (before, after) = timeout(Duration::from_secs(60), server).await.unwrap().unwrap();
        assert_eq!(before, []);
        assert_eq!(after, [ClientPdu::Layout]);
    }

    #[tokio::test]
    async fn after_a_reactivation_a_layout_waits_for_the_first_picture_at_the_new_size() {
        const BIGGER: (u16, u16) = (512, 256);
        let (asked_tx, asked) = oneshot::channel();
        let (client, stream) = duplex(1 << 16);
        let server = tokio::spawn(async move {
            let (mut framed, channels, acceptor) =
                accept_scripted_keeping(stream, SIZE, u32::MAX).await;
            display_ready(&mut framed, &channels).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(4))).await;
            let mut framed = reactivate_scripted(framed, &channels, acceptor, BIGGER, &[]).await;
            asked.await.unwrap();
            let before = layouts_and_acks(&mut framed, &channels, 300).await;
            write(&mut framed, surface_bits(8, 8, 16, 16, shade(5))).await;
            let after = layouts_and_acks(&mut framed, &channels, 300).await;
            (before, after)
        });
        let (session, mut ops) = join_with(client, &pinned(HOST, PORT), attach, CLIPBOARD).await;
        assert_eq!(resized(&session, &mut ops).await, BIGGER);
        session.resize(640, 480, 100).await.unwrap();
        asked_tx.send(()).unwrap();
        let (before, after) = within(server).await.unwrap();
        assert_eq!(before, []);
        assert_eq!(after, [ClientPdu::Layout]);
    }
}
