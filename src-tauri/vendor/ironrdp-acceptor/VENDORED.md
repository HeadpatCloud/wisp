# Vendored ironrdp-acceptor 0.10.0

Source: crates.io `ironrdp-acceptor` 0.10.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-acceptor` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`. Only the test
server (`ironrdp-server` and the scripted server in `src/rdp/testserver.rs`)
depends on it; it is a dev-dependency.

## Why

The published crate does not compile against sspi 0.23, which the vendored
`ironrdp-connector` uses (see `../ironrdp-connector/VENDORED.md`).

## Changes from 0.10.0

`src/credssp.rs`
- `buffer_len()` returns a `Result` in sspi 0.23, so the error is mapped with
  `custom_err!("TsRequest", e)`.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once a published
`ironrdp-acceptor` builds against the sspi version of the connector.
