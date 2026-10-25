# Vendored ironrdp-async 0.10.0

Source: crates.io `ironrdp-async` 0.10.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-async` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`. The vendored
`ironrdp-connector` and `ironrdp-acceptor` and the published `ironrdp-tokio`
all use this one copy.

## Why

`Framed::read_by_hint` hangs a task when the peer sends a frame that announces
a length of zero (a fast-path header `00 00`, or any run of zero bytes).
`read_exact(0)` returns at once without taking anything out of the buffer, the
hint gives the same answer again, and the loop never waits for the stream, so
it never yields: the thread runs at 100 % CPU and a `tokio::time::timeout`
around it cannot fire. Only `read_by_hint` loops, which is the login and the
reactivation after a Deactivate All. `Framed::read_pdu`, which the established
session reads with, returns an empty frame for the same input and the session
rejects it, so it never spun.

## Changes from 0.10.0

`src/framed.rs`
- `Framed::read_exact` returns an `io::Error` carrying an `ironrdp_core::DecodeError`
  ("frame length is zero") when asked for zero bytes. Both `read_pdu` and
  `read_by_hint` go through it.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once a published
`ironrdp-async` rejects frames of length zero.
