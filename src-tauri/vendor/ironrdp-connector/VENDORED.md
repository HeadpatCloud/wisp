# Vendored ironrdp-connector 0.10.0

Source: crates.io `ironrdp-connector` 0.10.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-connector` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`.

## Why

0.10.0 depends on `sspi ^0.21` and `picky =7.0.0-rc.25`. Both pin RustCrypto
pre-releases (`curve25519-dalek =5.0.0-rc.1`, `ed25519-dalek =3.0.0-rc.1`,
`p256/p384/p521 =0.14.0-rc.14`, ...) that cannot be in one lock file with
`russh`, which needs the released versions. `sspi` 0.23 uses the released
versions, but `^0.21` does not allow it.

## Changes from 0.10.0

`Cargo.toml`
- `sspi`: `version = "0.21", features = ["scard"]` ->
  `version = "0.23", default-features = false`.
- Removed the `picky`, `picky-asn1-der` and `picky-asn1-x509` dependencies.
  They were only used by the smart card code below.

`src/credssp.rs`
- `CredsspSequence::init`: the `Credentials::SmartCard` arm returns
  `general_err!("smart card logon is not supported")` instead of building an
  `sspi::SmartCardIdentity`. Without the `scard` feature sspi has no smart
  card support, and dropping it removes `winscard`, `cryptoki`, a C build of
  zlib (`libz-sys`) and the unmaintained `atomic-polyfill`.
- Removed `extract_user_name` and `extract_user_principal_name` and the
  `picky`, `picky_asn1_x509` and `sspi::Secret` imports, which only that arm
  used.
- `write_credssp_request`: `ts_request.buffer_len()` returns
  a `Result` in sspi 0.23, so the error is mapped with
  `custom_err!("TsRequest", e)`.

`src/connection_activation.rs`, `src/connection.rs`
- `ConnectionActivationSequence::max_request_size()` and
  `ConnectionResult::max_request_size`: the MultifragmentUpdate maximum
  request size sent in the Client Confirm Active PDU. The 8 MB sent when the
  server sends none is `DEFAULT_MAX_REQUEST_SIZE`. wisp holds the server to
  this size, because `ironrdp-session` reassembles fast-path fragments
  without a limit.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`. `Cargo.toml` is the published
(normalized) manifest with the two edits above.

## Constraint on the lock file

`picky-krb` has to stay at 0.12.4, which is pinned in `src-tauri/Cargo.toml`.
0.12.5 adds an enum variant that sspi 0.23 matches exhaustively, so sspi does
not compile against it.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once a published
`ironrdp-connector` depends on an sspi version that uses the released
RustCrypto crates. The session then needs the advertised maximum request
size some other way.
