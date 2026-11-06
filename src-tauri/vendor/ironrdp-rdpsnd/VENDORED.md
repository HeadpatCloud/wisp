# Vendored ironrdp-rdpsnd 0.9.0

Source: crates.io `ironrdp-rdpsnd` 0.9.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-rdpsnd` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`. The app, the
published `ironrdp-rdpsnd-native` and the test server's `ironrdp-server` all
use this one copy.

## Why

xrdp (rdpsnd version 5) sends every audio block as a WaveInfo PDU and a Wave
PDU in two channel messages. The 0.9.0 client decodes the two only as one
message, so the WaveInfo PDU fails to decode and `process` returns the error,
which ends the session at the first sound. Received as one message, they
decode but are not played: any PDU the client does not expect in its state
stops the channel, which then answers nothing, new formats included. The
formats the client sends are collected through a `HashSet`, so their order is
random.

0.10.0 plays WaveInfo and Wave and ignores malformed PDUs, but it needs
`ironrdp-pdu` 0.10, `ironrdp-svc` 0.9, `ironrdp-core` 0.3 and `ironrdp-dvc`
0.9, which means moving the whole IronRDP set.

## Changes from 0.9.0

`src/client.rs`
- A WaveInfo PDU on its own (16 bytes, msgType 0x02) in the Ready state is
  kept, and the next channel message is taken as its Wave PDU: its first 4
  bytes (padding) are replaced by the WaveInfo's 4 data bytes, the block is
  passed to `RdpsndClientHandler::wave` (`ts` is `wTimeStamp`) and answered
  with a Wave Confirm carrying the WaveInfo's `wTimeStamp` and `cBlockNo`. A
  Wave PDU shorter than 4 bytes is dropped. WaveInfo and Wave in one message
  (what this crate's own server sends) are played and confirmed the same way.
- `client_formats`: the formats sent are the handler's formats that the server
  also offers, in the handler's order (a filtered `Vec` instead of a `HashSet`
  intersection).
- `process` drops a PDU that does not decode or does not fit the state with a
  `warn!` and stays in its state; the `Stop` state is gone. A Formats PDU
  starts the negotiation again in every state, closing the handler first when
  it was Ready, as before. A Wave PDU without a WaveInfo PDU before it does not
  decode and is dropped. `process` returns an error only when a reply of its
  own fails to encode.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`. `Cargo.toml` is the published
(normalized) manifest, unchanged.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once the app moves to
`ironrdp-rdpsnd` 0.10 or later together with the rest of the IronRDP set.
`src/rdp/sound.rs` then has to follow the new `wave()` signature, which takes
the negotiated `AudioFormat` instead of an index.
