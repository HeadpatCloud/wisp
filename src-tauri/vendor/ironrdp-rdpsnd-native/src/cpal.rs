use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, Stream, StreamConfig};
use ironrdp_error::bail;
use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use tracing::{debug, error, trace, warn};

use crate::error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};

// Windows sends no Close after a sound, so a stream left running would keep the audio session
// active for the rest of the session, and cpal keeps the device a stream was built on. A stream
// with nothing to play for this long ends, and the next block gets a new one on the device that
// is the default then. Its blocks come every 186 ms while it plays.
const IDLE: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct RdpsndBackend {
    // Unfortunately, Stream is not `Send`, so we move it to a separate thread. It returns whether
    // it built the stream.
    stream_handle: Option<JoinHandle<bool>>,
    stream_ended: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    // What the stream's callback has done with the blocks, and how many were sent to it.
    progress: Arc<Progress>,
    sent: Arc<AtomicU64>,
    // When the last block came, and whether the last stream could not be built.
    arrived: Option<Instant>,
    unbuilt: bool,
    builds: u64,
    tx: Option<Sender<Vec<u8>>>,
    format_no: Option<usize>,
}

impl Default for RdpsndBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpsndBackend {
    pub fn new() -> Self {
        Self {
            tx: None,
            format_no: None,
            stream_handle: None,
            stream_ended: Arc::new(AtomicBool::new(false)),
            playing: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Progress::new()),
            sent: Arc::new(AtomicU64::new(0)),
            arrived: None,
            unbuilt: false,
            builds: 0,
        }
    }

    #[doc(hidden)]
    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// How many streams were begun, built or not.
    #[doc(hidden)]
    pub fn builds(&self) -> u64 {
        self.builds
    }
}

impl Drop for RdpsndBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl RdpsndClientHandler for RdpsndBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &[
            #[cfg(feature = "opus")]
            AudioFormat {
                format: WaveFormat::OPUS,
                n_channels: 2,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 192000,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 44100,
                n_avg_bytes_per_sec: 176400,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
        ]
    }

    fn wave(&mut self, format_no: usize, _ts: u32, data: Cow<'_, [u8]>) {
        if Some(format_no) != self.format_no {
            debug!("New audio format");
            self.close();
        }

        let quiet = self.arrived.is_none_or(|arrived| arrived.elapsed() >= IDLE);
        self.arrived = Some(Instant::now());
        // Counted before the stream is looked at, so that its thread never finds every block taken
        // while this one is on its way.
        self.sent.fetch_add(1, Ordering::Relaxed);
        // A stream that has had nothing to play for IDLE ends here too, so that this block does
        // not go to it as its thread ends it.
        let ended = self.stream_handle.as_ref().is_some_and(JoinHandle::is_finished);
        if ended || (self.playing() && self.progress.idle() >= IDLE) {
            self.close();
        }

        if self.stream_handle.is_none() {
            // Without a device at this sound, the next sound tries again.
            if self.unbuilt && !quiet {
                return;
            }
            let (tx, rx) = mpsc::channel();
            self.tx = Some(tx);

            self.format_no = Some(format_no);
            let Some(format) = self.get_formats().get(format_no) else {
                warn!(?format_no, "Invalid format_no");
                return;
            };
            let format = format.clone();
            self.stream_ended.store(false, Ordering::Relaxed);
            self.progress = Arc::new(Progress::new());
            self.sent.store(1, Ordering::Relaxed);
            self.builds += 1;
            let stream_ended = Arc::clone(&self.stream_ended);
            let playing = Arc::clone(&self.playing);
            let progress = Arc::clone(&self.progress);
            let sent = Arc::clone(&self.sent);
            self.stream_handle = Some(thread::spawn(move || {
                let stream = match DecodeStream::new(&format, rx, Arc::clone(&progress)) {
                    Ok(stream) => stream,
                    Err(e) => {
                        error!(error = %e.report());
                        return false;
                    }
                };
                playing.store(true, Ordering::Relaxed);
                debug!("Stream thread parking loop");
                while !stream_ended.load(Ordering::Relaxed) {
                    let idle = progress.idle();
                    if idle >= IDLE && progress.taken() == sent.load(Ordering::Relaxed) {
                        break;
                    }
                    // A block not taken yet is taken at the callback's next run, unless the
                    // stream is dead; then the next block ends it.
                    thread::park_timeout(IDLE.checked_sub(idle).unwrap_or(IDLE));
                }
                debug!("Stream thread ends");
                drop(stream);
                playing.store(false, Ordering::Relaxed);
                true
            }));
        }

        if let Some(ref tx) = self.tx {
            if let Err(error) = tx.send(data.to_vec()) {
                error!(%error);
            }
        };
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        debug!(?volume);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        debug!(?pitch);
    }

    fn close(&mut self) {
        self.tx = None;
        if let Some(stream) = self.stream_handle.take() {
            self.stream_ended.store(true, Ordering::Relaxed);
            stream.thread().unpark();
            match stream.join() {
                Ok(built) => self.unbuilt = !built,
                Err(err) => error!(?err, "Failed to join a stream thread"),
            }
        }
    }
}

/// What a stream's callback has done with the blocks: when it last copied block bytes, and how
/// many it has taken.
#[doc(hidden)]
#[derive(Debug)]
pub struct Progress {
    since: Instant,
    /// Milliseconds after `since`.
    copied: AtomicU64,
    taken: AtomicU64,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self {
            since: Instant::now(),
            copied: AtomicU64::new(0),
            taken: AtomicU64::new(0),
        }
    }

    fn copy(&self) {
        let now = u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.copied.store(now, Ordering::Relaxed);
    }

    /// How long ago the callback last copied block bytes, or this was made if it never did.
    pub fn idle(&self) -> Duration {
        let copied = Duration::from_millis(self.copied.load(Ordering::Relaxed));
        self.since.elapsed().saturating_sub(copied)
    }

    pub fn taken(&self) -> u64 {
        self.taken.load(Ordering::Relaxed)
    }
}

#[doc(hidden)]
pub struct DecodeStream {
    _dec_thread: Option<JoinHandle<()>>,
    stream: Stream,
}

impl DecodeStream {
    // The receiver and the decoder thread are reassigned only with Opus.
    #[cfg_attr(not(feature = "opus"), allow(unused_mut))]
    pub fn new(
        rx_format: &AudioFormat,
        mut rx: Receiver<Vec<u8>>,
        progress: Arc<Progress>,
    ) -> RdpsndNativeResult<Self> {
        let mut dec_thread = None;
        match rx_format.format {
            #[cfg(feature = "opus")]
            WaveFormat::OPUS => {
                let chan = match rx_format.n_channels {
                    1 => opus2::Channels::Mono,
                    2 => opus2::Channels::Stereo,
                    _ => bail!(
                        "unsupported channel count for Opus",
                        RdpsndNativeErrorKind::UnsupportedFormat,
                    ),
                };
                let (dec_tx, dec_rx) = mpsc::channel();
                let mut dec = opus2::Decoder::new(rx_format.n_samples_per_sec, chan).map_err(|e| {
                    RdpsndNativeError::new("creating Opus decoder", RdpsndNativeErrorKind::OpusInit).with_source(e)
                })?;
                dec_thread = Some(thread::spawn(move || {
                    while let Ok(pkt) = rx.recv() {
                        let nb_samples = match dec.get_nb_samples(&pkt) {
                            Ok(nb_samples) => nb_samples,
                            Err(error) => {
                                error!(?error, "Failed to get the number of samples of an Opus packet");
                                continue;
                            }
                        };

                        #[expect(
                            clippy::as_conversions,
                            reason = "opus::Channels has no conversions to usize implemented"
                        )]
                        let mut pcm_i16 = vec![0i16; nb_samples * chan as usize];
                        if let Err(error) = dec.decode(&pkt, &mut pcm_i16, false) {
                            error!(?error, "Failed to decode an Opus packet");
                            continue;
                        }
                        // Vec<u8> is what the channel carries downstream. Reinterpreting
                        // Vec<i16> -> Vec<u8> via cast_slice is safe (smaller alignment).
                        // Allocating as Vec<i16> in the first place avoids the alignment
                        // hazard of `bytemuck::cast_slice_mut::<u8, i16>` panicking when
                        // the allocator hands back a u8 buffer that is not 2-byte aligned
                        // (which manifested as a hard crash in #1202 under the burst of
                        // malformed Opus packets generated by a server reboot).
                        let pcm = bytemuck::cast_slice(&pcm_i16).to_vec();

                        if dec_tx.send(pcm).is_err() {
                            error!("Failed to send the decoded Opus packet over the channel");
                            // If send has failed, it means that the receiver has been dropped.
                            // There is no point in continuing the loop in this case.
                            break;
                        }
                    }
                }));
                rx = dec_rx;
            }
            WaveFormat::PCM => {}
            _ => bail!(
                "matching server-requested wave format",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ),
        }

        let sample_format = match rx_format.bits_per_sample {
            8 => SampleFormat::U8,
            16 => SampleFormat::I16,
            _ => bail!(
                "only PCM 8/16 bit formats supported",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ),
        };

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| RdpsndNativeError::new("no default output device", RdpsndNativeErrorKind::AudioDevice))?;
        let _supported_configs_range = device.supported_output_configs().map_err(|e| {
            RdpsndNativeError::new("no supported output configs", RdpsndNativeErrorKind::AudioDevice).with_source(e)
        })?;
        let default_config = device.default_output_config().map_err(|e| {
            RdpsndNativeError::new("default output config", RdpsndNativeErrorKind::AudioDevice).with_source(e)
        })?;
        debug!(?default_config);

        let silence = if sample_format == SampleFormat::U8 { 0x80 } else { 0 };
        let mut rx = RxBuffer::new(rx, silence, progress);
        let config = StreamConfig {
            channels: rx_format.n_channels,
            sample_rate: rx_format.n_samples_per_sec,
            buffer_size: cpal::BufferSize::Default,
        };
        debug!(?config);

        let stream = device
            .build_output_stream_raw(
                &config,
                sample_format,
                move |data, _info: &cpal::OutputCallbackInfo| {
                    let data = data.bytes_mut();
                    rx.fill(data)
                },
                |error| error!(%error),
                None,
            )
            .map_err(|e| {
                RdpsndNativeError::new("building cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
            })?;
        // A WASAPI stream stays stopped until play().
        stream.play().map_err(|e| {
            RdpsndNativeError::new("starting cpal output stream", RdpsndNativeErrorKind::StreamBuild).with_source(e)
        })?;

        Ok(Self {
            _dec_thread: dec_thread,
            stream,
        })
    }

    pub fn stream(&self) -> &Stream {
        &self.stream
    }
}

#[doc(hidden)]
pub struct RxBuffer {
    receiver: Receiver<Vec<u8>>,
    last: Option<Vec<u8>>,
    idx: usize,
    silence: u8,
    progress: Arc<Progress>,
}

impl RxBuffer {
    pub fn new(receiver: Receiver<Vec<u8>>, silence: u8, progress: Arc<Progress>) -> Self {
        Self {
            receiver,
            last: None,
            idx: 0,
            silence,
            progress,
        }
    }

    pub fn fill(&mut self, data: &mut [u8]) {
        let mut filled = 0;

        while filled < data.len() {
            // Waiting here would hold back what the buffer already has, the end of a sound, until
            // the next sound.
            if self.last.is_none() {
                match self.receiver.try_recv() {
                    Ok(rx) => {
                        debug!(rx.len = rx.len());
                        self.progress.taken.fetch_add(1, Ordering::Relaxed);
                        self.last = Some(rx);
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(error) => {
                        warn!(%error);
                    }
                }
            }

            let Some(ref last) = self.last else {
                trace!("Playback rx underrun");
                // The device buffer is not cleared between callbacks: left as it is, it plays
                // old sound again.
                data[filled..].fill(self.silence);
                return;
            };
            if self.idx < last.len() {
                self.progress.copy();
            }

            #[expect(clippy::arithmetic_side_effects)]
            while self.idx < last.len() && filled < data.len() {
                data[filled] = last[self.idx];
                assert!(filled < usize::MAX);
                assert!(self.idx < usize::MAX);
                filled += 1;
                self.idx += 1;
            }

            // If all elements from last have been consumed, clear `self.last`
            if self.idx >= last.len() {
                self.last = None;
                self.idx = 0;
            }
        }
    }
}
