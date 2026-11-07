# Vendored ironrdp-rdpsnd-native 0.7.0

Source: crates.io `ironrdp-rdpsnd-native` 0.7.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-rdpsnd-native` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`.

## Why

`RdpsndBackend` plays each block through the cpal stream that
`DecodeStream::new` builds, and nothing ever calls `play()` on it. cpal 0.17
starts a WASAPI stream only on `play()`, so on Windows the stream stays
stopped, its data callback never runs and nothing is heard: the app's audio
session shows up when the first block arrives and stays inactive. The crate's
own example calls `play()` itself after `DecodeStream::new`.

Once the stream runs, its data callback waits up to 4 s for the next block and
then hands the device buffer back with only the part the blocks covered
written. Windows keeps the stream open after a sound, and the buffer still
holds what was played before: after a 0.4 s tone the app's audio session
showed the tone's level again every 4 s. With the rest filled with silence,
the level still came back once, 4 s after the tone: the end of the sound,
held in the buffer while the callback waited.

Windows sends no Close after a sound, so the started stream ran for the rest of
the session: after a 0.4 s tone the app's audio session stayed active for the
131 s it was watched, with nothing playing. xrdp closes the stream about 0.2 s
after a sound.

cpal builds a stream on the device that is the default at that moment and
keeps it: a stream kept for the session, running or paused, never moves to
headphones plugged in later, and once its device is gone it stays silent.

## Changes from 0.7.0

`src/cpal.rs`
- `DecodeStream::new` calls `play()` on the stream it built and returns an
  error with `RdpsndNativeErrorKind::StreamBuild` when that fails, which
  `RdpsndBackend` logs like a failed build.
- `RxBuffer::fill` takes only the blocks already received (`try_recv`
  instead of `recv_timeout` of 4 s) and writes silence (0 for 16-bit samples,
  0x80 for 8-bit, set by `DecodeStream::new`) into the part of the buffer no
  block covers. An empty queue is no longer logged as a warning.
  `RxBuffer` is public but hidden from the docs, like `DecodeStream`, so that
  the app's tests can check it (`what_no_block_covers_is_silence_at_once`).
- A stream ends when it has had nothing to play for 1 s (`IDLE`): its thread
  drops the stream once the callback has copied no block bytes for that long
  and has taken every block sent to it. `RxBuffer::fill` keeps that count and
  writes the time of its last copy into a `Progress` (atomics, no waiting in
  the callback), which `DecodeStream::new` and `RxBuffer::new` take; the time
  counts from the copy, so a backlog longer than a second plays to its end.
  `wave` builds a new stream, on the device that is the default then, when
  the thread has ended or the stream is past `IDLE` (so that no block goes to
  a stream its thread is ending). The thread returns whether it built the
  stream; after a build that failed, such as without a device, the blocks of
  the same sound (each less than 1 s after the one before) are dropped and the
  next sound tries again. In 0.7.0 the one stream lived for the session.
  `RdpsndBackend::playing` and `RdpsndBackend::builds` and `Progress` are
  public but hidden from the docs, for the app's tests in `src/rdp/sound.rs`.
- On Windows, the first `DecodeStream::new` starts a thread that makes cpal's
  device enumerator and stays parked for the rest of the process. cpal 0.17
  keeps the first enumerator for the process, in the COM apartment of the
  thread that made it (cpal issue #1302). That was a stream's thread: when it
  ended as the last thread using COM, as on a host without a sound device,
  `MMDevApi.dll` was unloaded and the next stream crashed the process with an
  access violation. The app's window keeps COM going on its main thread; the
  test binary has no window. `a_stream_after_the_thread_of_the_one_before_has_ended_is_built_or_refused`
  in `src/rdp/sound.rs` crashed on Windows Server 2025 without a sound device.
- `DecodeStream::new` allows `unused_mut` without the `opus` feature: its
  receiver and decoder thread change only with Opus, and as a path dependency
  the crate's warnings are no longer capped, so the app's build printed them.

`src/error.rs`
- The `StreamBuild` doc names `cpal::PlayStreamError` too, and its `Display`
  reads "output audio stream build or start".

`examples/cpal.rs`
- Passes a `Progress` to `DecodeStream::new`.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`. `Cargo.toml` is the published
(normalized) manifest, unchanged.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once a release whose
`RdpsndBackend` starts its stream, fills underruns with silence at once, ends
the stream a second after the last sound played and builds the next one on the
device that is the default then, is used, together with the rest of the
IronRDP set, and with a cpal that keeps its enumerator alive on its own. `the_output_stream_runs_once_it_is_made` in `src/rdp/sound.rs`
then checks the first; the others need a listen or the audio session's state
and peak meter, as `RxBuffer`, `Progress`, `playing` and `builds` do not exist
upstream.
