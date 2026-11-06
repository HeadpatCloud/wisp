use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};

// Counts the audio bytes it hands on, which is what the live tests can check.
#[derive(Debug)]
pub(crate) struct Player {
    inner: Box<dyn RdpsndClientHandler>,
    played: Arc<AtomicU64>,
}

impl Player {
    pub(crate) fn new(inner: impl RdpsndClientHandler + 'static, played: Arc<AtomicU64>) -> Self {
        Player { inner: Box::new(inner), played }
    }
}

impl RdpsndClientHandler for Player {
    fn get_flags(&self) -> AudioFormatFlags {
        self.inner.get_flags()
    }

    fn get_formats(&self) -> &[AudioFormat] {
        self.inner.get_formats()
    }

    fn wave(&mut self, format_no: usize, ts: u32, data: Cow<'_, [u8]>) {
        self.played.fetch_add(data.len() as u64, Ordering::Relaxed);
        self.inner.wave(format_no, ts, data);
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        self.inner.set_volume(volume);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        self.inner.set_pitch(pitch);
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use ironrdp_rdpsnd::client::Rdpsnd;
    use ironrdp_rdpsnd::pdu::{
        ClientAudioOutputPdu as Reply, QualityMode, ServerAudioOutputPdu, Version, WaveConfirmPdu,
        WaveFormat, WavePdu,
    };
    use ironrdp_rdpsnd_native::cpal::{DecodeStream, Progress, RdpsndBackend, RxBuffer};
    use ironrdp_rdpsnd_native::RdpsndNativeErrorKind;
    use ironrdp_svc::SvcProcessor as _;

    use super::*;
    use crate::rdp::testserver::{
        pcm, sound_formats, sound_training, wave, wave2, wave_info, windows_formats, xrdp_formats,
    };

    #[derive(Debug, PartialEq)]
    enum Heard {
        Wave(usize, Vec<u8>),
        Close,
    }

    #[derive(Debug)]
    struct Recorder {
        formats: Vec<AudioFormat>,
        heard: Arc<Mutex<Vec<Heard>>>,
    }

    impl RdpsndClientHandler for Recorder {
        fn get_formats(&self) -> &[AudioFormat] {
            &self.formats
        }

        fn wave(&mut self, format_no: usize, _ts: u32, data: Cow<'_, [u8]>) {
            self.heard.lock().unwrap().push(Heard::Wave(format_no, data.into_owned()));
        }

        fn set_volume(&mut self, _volume: VolumePdu) {}

        fn set_pitch(&mut self, _pitch: PitchPdu) {}

        fn close(&mut self) {
            self.heard.lock().unwrap().push(Heard::Close);
        }
    }

    struct Client {
        rdpsnd: Rdpsnd,
        heard: Arc<Mutex<Vec<Heard>>>,
    }

    impl Client {
        fn new(formats: Vec<AudioFormat>) -> Self {
            let heard = Arc::default();
            let recorder = Recorder { formats, heard: Arc::clone(&heard) };
            Client { rdpsnd: Rdpsnd::new(Box::new(recorder)), heard }
        }

        // Past the negotiation, with what the app's player offers.
        fn ready(version: Version, formats: Vec<AudioFormat>) -> Self {
            let mut client = Client::new(offered());
            client.answer(&sound_formats(version, formats));
            client.answer(&sound_training());
            client
        }

        // Nothing the server sends may fail the channel.
        fn answer(&mut self, pdu: &[u8]) -> Vec<Reply> {
            let replies = self.rdpsnd.process(pdu).expect("the channel failed");
            let decode = |reply: &ironrdp_svc::SvcMessage| {
                ironrdp_core::decode(&reply.encode_unframed_pdu().unwrap()).unwrap()
            };
            replies.iter().map(decode).collect()
        }

        fn heard(&self) -> Vec<Heard> {
            std::mem::take(&mut *self.heard.lock().unwrap())
        }
    }

    fn offered() -> Vec<AudioFormat> {
        RdpsndBackend::new().get_formats().to_vec()
    }

    fn block(len: usize) -> Vec<u8> {
        (0..len).map(|n| (n * 7 + 3) as u8).collect()
    }

    fn confirm(timestamp: u16, block_no: u8) -> Reply {
        Reply::WaveConfirm(WaveConfirmPdu { timestamp, block_no })
    }

    fn trained(replies: &[Reply]) -> bool {
        matches!(replies, [Reply::TrainingConfirm(_)])
    }

    #[test]
    fn v8_formats_get_one_pcm_format_and_quality_mode() {
        let windows = windows_formats();
        assert_eq!((windows.len(), &windows[8]), (30, &pcm(2, 44100)));
        let mut client = Client::new(offered());
        let replies = client.answer(&sound_formats(Version::V8, windows));
        let [Reply::AudioFormat(formats), Reply::QualityMode(quality)] = &replies[..] else {
            panic!("{replies:?}");
        };
        assert_eq!(formats.version, Version::V8);
        assert!(formats.flags.contains(AudioFormatFlags::ALIVE));
        assert_eq!(formats.formats, [pcm(2, 44100)]);
        assert_eq!(quality.quality_mode, QualityMode::High);
    }

    #[test]
    fn v5_formats_get_no_quality_mode() {
        let mut client = Client::new(offered());
        let replies = client.answer(&sound_formats(Version::V5, xrdp_formats()));
        let [Reply::AudioFormat(formats)] = &replies[..] else {
            panic!("{replies:?}");
        };
        assert_eq!(formats.version, Version::V5);
        assert!(formats.flags.contains(AudioFormatFlags::ALIVE));
        assert_eq!(formats.formats, [pcm(2, 44100)]);
    }

    #[test]
    fn the_client_lists_the_formats_in_the_handlers_order() {
        let ours = [48000, 44100, 32000, 22050, 16000, 11025, 8000].map(|rate| pcm(2, rate));
        let offered_too = |format: &&AudioFormat| format.n_samples_per_sec != 32000;
        let mut theirs: Vec<_> = ours.iter().rev().filter(offered_too).cloned().collect();
        theirs.insert(3, pcm(1, 44100));
        let mut client = Client::new(ours.to_vec());
        let replies = client.answer(&sound_formats(Version::V8, theirs));
        let [Reply::AudioFormat(formats), _] = &replies[..] else {
            panic!("{replies:?}");
        };
        let common: Vec<_> = ours.iter().filter(offered_too).cloned().collect();
        assert_eq!(formats.formats, common);
    }

    #[test]
    fn no_common_format_answers_an_empty_list() {
        let mut client = Client::new(offered());
        let mut others = windows_formats();
        others.retain(|format| format.format != WaveFormat::PCM);
        let replies = client.answer(&sound_formats(Version::V8, others));
        let [Reply::AudioFormat(formats), Reply::QualityMode(_)] = &replies[..] else {
            panic!("{replies:?}");
        };
        assert_eq!(formats.formats, []);
    }

    #[test]
    fn wave2_is_played_and_confirmed() {
        let mut client = Client::ready(Version::V8, windows_formats());
        let data = block(32768);
        assert_eq!(client.answer(&wave2(4660, 7, &data)), [confirm(4660, 7)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn wave_info_then_wave_is_played_whole_and_confirmed() {
        let mut client = Client::ready(Version::V5, xrdp_formats());
        let data = block(8192);
        let (info, wave) = (wave_info(20052, 1, &data), wave(&data));
        assert_eq!((info.len(), wave.len()), (16, 4 + 8188));
        assert_eq!(client.answer(&info), []);
        assert_eq!(client.heard(), []);
        assert_eq!(client.answer(&wave), [confirm(20052, 1)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn wave_info_and_wave_in_one_message_are_played() {
        let mut client = Client::ready(Version::V5, xrdp_formats());
        let data = block(8192);
        let joined = [wave_info(20094, 2, &data), wave(&data)].concat();
        let theirs = WavePdu { timestamp: 20094, format_no: 0, block_no: 2, data: data[..].into() };
        let theirs = ironrdp_core::encode_vec(&ServerAudioOutputPdu::Wave(theirs)).unwrap();
        assert_eq!(theirs, joined, "not what ironrdp-rdpsnd's own server sends");
        assert_eq!(client.answer(&joined), [confirm(20094, 2)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn a_wave_without_its_info_is_dropped() {
        let mut client = Client::ready(Version::V5, xrdp_formats());
        assert_eq!(client.answer(&wave(&block(8192))), []);
        assert_eq!(client.heard(), []);
        let data = block(8192);
        assert_eq!(client.answer(&wave_info(1, 3, &data)), []);
        assert_eq!(client.answer(&wave(&data)), [confirm(1, 3)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn garbage_on_the_channel_is_dropped_and_the_next_wave_plays() {
        let mut client = Client::ready(Version::V8, windows_formats());
        let random = [
            0x5a, 0xc3, 0x17, 0x9e, 0x42, 0x08, 0xf1, 0x6d, 0xb0, 0x33, 0x8c, 0x01, 0xe7, 0x7f,
            0x24, 0x99, 0x0d, 0xa6, 0x5b, 0x10,
        ];
        let truncated = wave2(1, 1, &block(64))[..10].to_vec();
        let unknown = [0x42, 0, 4, 0, 1, 2, 3, 4];
        const W_VERSION: usize = 21;
        let mut unknown_version = sound_formats(Version::V8, vec![pcm(2, 44100)]);
        unknown_version[W_VERSION] = 7;
        for garbage in [random.to_vec(), truncated, unknown.to_vec(), unknown_version] {
            assert_eq!(client.answer(&garbage), [], "{garbage:?}");
        }
        assert_eq!(client.answer(&wave_info(2, 2, &block(64))), []);
        assert_eq!(client.answer(&[0, 0]), [], "a Wave too short for its block");
        assert_eq!(client.heard(), []);
        let data = block(4096);
        assert_eq!(client.answer(&wave2(3, 3, &data)), [confirm(3, 3)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn pdus_out_of_turn_are_dropped_and_the_negotiation_goes_on() {
        let mut client = Client::new(offered());
        assert_eq!(client.answer(&wave2(1, 1, &block(64))), []);
        assert_eq!(client.answer(&sound_training()), []);
        assert_eq!(client.answer(&sound_formats(Version::V8, windows_formats())).len(), 2);
        assert_eq!(client.answer(&wave2(2, 2, &block(64))), []);
        assert_eq!(client.answer(&wave_info(3, 3, &block(64))), []);
        let again = client.answer(&sound_formats(Version::V5, xrdp_formats()));
        assert!(matches!(again[..], [Reply::AudioFormat(_)]), "{again:?}");
        assert!(trained(&client.answer(&sound_training())));
        assert_eq!(client.heard(), []);
        let data = block(64);
        assert_eq!(client.answer(&wave2(4, 4, &data)), [confirm(4, 4)]);
        assert_eq!(client.heard(), [Heard::Wave(0, data)]);
    }

    #[test]
    fn new_formats_mid_stream_close_and_negotiate_again() {
        let mut client = Client::ready(Version::V8, windows_formats());
        let first = block(4096);
        assert_eq!(client.answer(&wave2(1, 1, &first)), [confirm(1, 1)]);
        let replies = client.answer(&sound_formats(Version::V8, windows_formats()));
        let negotiated = matches!(replies[..], [Reply::AudioFormat(_), Reply::QualityMode(_)]);
        assert!(negotiated, "{replies:?}");
        assert_eq!(client.heard(), [Heard::Wave(0, first), Heard::Close]);
        assert_eq!(client.answer(&wave2(2, 2, &block(64))), []);
        assert!(trained(&client.answer(&sound_training())));
        let second = block(4096);
        assert_eq!(client.answer(&wave2(3, 3, &second)), [confirm(3, 3)]);
        assert_eq!(client.heard(), [Heard::Wave(0, second)]);
    }

    #[test]
    fn the_player_offers_only_pcm_44100_2_16() {
        let player = Player::new(RdpsndBackend::new(), Arc::default());
        assert_eq!(player.get_formats(), [pcm(2, 44100)]);
    }

    // RdpsndBackend plays through this stream. A WASAPI stream stays silent until it is started,
    // and its data callback, which takes the blocks off the channel, never runs.
    #[test]
    fn the_output_stream_runs_once_it_is_made() {
        let (blocks, rx) = std::sync::mpsc::sync_channel(0);
        let stream = match DecodeStream::new(&pcm(2, 44100), rx, Arc::new(Progress::new())) {
            Ok(stream) => stream,
            Err(e) if matches!(e.kind(), RdpsndNativeErrorKind::AudioDevice) => {
                println!("no sound device ({e}), test skipped");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        let sent = std::thread::spawn(move || blocks.send(vec![0; 4096]).is_ok());
        let started = Instant::now();
        while !sent.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(5), "the stream never took a block");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(sent.join().unwrap());
        drop(stream);
    }

    // Whether a stream can be built here; the tests that need one are skipped without.
    fn has_device() -> bool {
        let (_blocks, rx) = std::sync::mpsc::channel();
        match DecodeStream::new(&pcm(2, 44100), rx, Arc::new(Progress::new())) {
            Ok(_) => true,
            Err(e) if matches!(e.kind(), RdpsndNativeErrorKind::AudioDevice) => {
                println!("no sound device ({e}), test skipped");
                false
            }
            Err(e) => panic!("{e}"),
        }
    }

    fn until(backend: &RdpsndBackend, playing: bool, within: Duration) -> Instant {
        let started = Instant::now();
        while backend.playing() != playing {
            assert!(started.elapsed() < within, "still playing: {}", !playing);
            std::thread::sleep(Duration::from_millis(10));
        }
        Instant::now()
    }

    // Windows keeps the sound open until the session ends, a running stream keeps the computer
    // from sleeping, and a stream stays on the device it was built on.
    #[test]
    fn the_output_stream_ends_a_second_after_its_last_block_and_the_next_block_gets_a_new_one() {
        if !has_device() {
            return;
        }
        let mut backend = RdpsndBackend::new();
        backend.wave(0, 0, vec![0; 4096].into());
        until(&backend, true, Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(700));
        backend.wave(0, 1, vec![0; 4096].into());
        let last = Instant::now();
        std::thread::sleep(Duration::from_millis(600));
        assert!(backend.playing(), "stopped less than a second after a block");
        let stopped = until(&backend, false, Duration::from_secs(2)) - last;
        let (second, late) = (Duration::from_secs(1), Duration::from_millis(1500));
        assert!(stopped >= second && stopped < late, "stopped after {stopped:?}");
        assert_eq!(backend.builds(), 1);
        backend.wave(0, 2, vec![0; 4096].into());
        until(&backend, true, Duration::from_secs(1));
        assert_eq!(backend.builds(), 2, "the block after the stop has no stream of its own");
        backend.close();
        assert!(!backend.playing());
    }

    // Taking the device away is a change to the host, so a host with a device runs none of this.
    #[test]
    fn a_sound_without_a_device_is_tried_again_at_the_next_sound_and_not_before() {
        if has_device() {
            println!("a sound device is here, test skipped");
            return;
        }
        let mut backend = RdpsndBackend::new();
        backend.wave(0, 0, vec![0; 4096].into());
        assert_eq!(backend.builds(), 1);
        std::thread::sleep(Duration::from_millis(300));
        backend.wave(0, 1, vec![0; 4096].into());
        assert_eq!(backend.builds(), 1, "tried again within the sound");
        std::thread::sleep(Duration::from_millis(1100));
        backend.wave(0, 2, vec![0; 4096].into());
        assert_eq!(backend.builds(), 2, "not tried again at the next sound");
        assert!(!backend.playing());
        backend.close();
    }

    // A server can send more than a second ahead; the second counts from the last block played.
    #[test]
    fn a_backlog_longer_than_a_second_plays_to_its_end_before_the_stream_stops() {
        if !has_device() {
            return;
        }
        let mut backend = RdpsndBackend::new();
        // 1.5 s at 176,400 bytes a second, sent at once.
        backend.wave(0, 0, vec![0; 264_600].into());
        let sent = Instant::now();
        until(&backend, true, Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(1300));
        assert!(backend.playing(), "stopped {:?} after the backlog came", sent.elapsed());
        let stopped = until(&backend, false, Duration::from_secs(3)) - sent;
        // The callback copies the last bytes a device buffer ahead of hearing them.
        let played_and_idle = Duration::from_millis(2400);
        assert!(stopped >= played_and_idle, "stopped after {stopped:?}");
        assert!(stopped < played_and_idle + Duration::from_millis(600), "after {stopped:?}");
        backend.close();
    }

    // Windows keeps the stream open after a sound, and the device buffer still holds what it
    // played last. Waiting for the next block would hold back the end of the sound until then.
    #[test]
    fn what_no_block_covers_is_silence_at_once() {
        for silence in [0, 0x80] {
            let (blocks, rx) = std::sync::mpsc::channel();
            let mut buffer = RxBuffer::new(rx, silence, Arc::new(Progress::new()));
            blocks.send(vec![1, 2, 3]).unwrap();
            let mut out = [0xaa; 8];
            let started = Instant::now();
            buffer.fill(&mut out);
            assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
            assert_eq!(out, [1, 2, 3, silence, silence, silence, silence, silence]);
            blocks.send(vec![4; 8]).unwrap();
            buffer.fill(&mut out);
            assert_eq!(out, [4; 8]);
        }
    }

    // The second a stream is kept counts from the last block bytes the callback copied.
    #[test]
    fn the_callback_marks_when_it_copies_block_bytes_and_not_silence() {
        let (blocks, rx) = std::sync::mpsc::channel();
        let progress = Arc::new(Progress::new());
        let mut buffer = RxBuffer::new(rx, 0, Arc::clone(&progress));
        std::thread::sleep(Duration::from_millis(100));
        assert!(progress.idle() >= Duration::from_millis(100), "{:?}", progress.idle());
        blocks.send(vec![1; 12]).unwrap();
        buffer.fill(&mut [0; 8]);
        assert!(progress.idle() < Duration::from_millis(50), "{:?}", progress.idle());
        assert_eq!(progress.taken(), 1);
        std::thread::sleep(Duration::from_millis(100));
        buffer.fill(&mut [0; 8]);
        assert!(progress.idle() < Duration::from_millis(50), "the rest of the block");
        std::thread::sleep(Duration::from_millis(100));
        buffer.fill(&mut [0; 8]);
        assert!(progress.idle() >= Duration::from_millis(100), "silence counted as played");
        assert_eq!(progress.taken(), 1);
    }

    #[test]
    fn the_player_counts_the_bytes_it_hands_on() {
        let played = Arc::new(AtomicU64::new(5));
        let heard = Arc::default();
        let recorder = Recorder { formats: vec![pcm(2, 44100)], heard: Arc::clone(&heard) };
        let mut player = Player::new(recorder, Arc::clone(&played));
        assert_eq!(player.get_formats(), [pcm(2, 44100)]);
        player.wave(0, 1, block(100).into());
        player.wave(0, 2, block(28).into());
        player.close();
        assert_eq!(played.load(Ordering::Relaxed), 5 + 128);
        let heard = std::mem::take(&mut *heard.lock().unwrap());
        assert_eq!(heard, [Heard::Wave(0, block(100)), Heard::Wave(0, block(28)), Heard::Close]);
    }
}
