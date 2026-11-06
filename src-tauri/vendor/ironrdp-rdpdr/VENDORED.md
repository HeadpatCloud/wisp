# Vendored ironrdp-rdpdr 0.7.0

Source: crates.io `ironrdp-rdpdr` 0.7.0 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-rdpdr` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0, see
`LICENSE-MIT` and `LICENSE-APACHE`.

Used through `[patch.crates-io]` in `src-tauri/Cargo.toml`.

## Why

xrdp names a shared drive by its PreferredDosName, which 0.7.0 always sends
as `ignored`, so the folder shows up as `~/thinclient_drives/ignored`.
Windows names it by the DeviceData and is not affected.

0.7.0 also returns an error for every drive request it cannot decode: a
major function `MajorFunction` has no variant for (FLUSH_BUFFERS,
QUERY_SECURITY, SET_SECURITY), SET_VOLUME_INFORMATION, or an information
class it has no type for. The error ends the session. Neither Windows nor
xrdp sent such a request in the steps measured so far, but an application
that flushes, reads security descriptors or locks on the share would.

0.8.0 fixes both (#2174, and `PreferredDosName::for_drive` in 0.9.0), but it
needs `ironrdp-pdu` 0.10, `ironrdp-svc` 0.9 and `ironrdp-core` 0.3, which
means moving the whole IronRDP set.

## Changes from 0.7.0

`src/pdu/efs.rs`
- `DeviceAnnounceHeader::new_drive`: the PreferredDosName is the drive's
  name instead of `"ignored"`. `PreferredDosName::format` still cuts it to
  7 bytes; wisp only announces names of 1 to 7 ASCII letters, digits, `-`
  and `_`.

`src/lib.rs`
- `SvcProcessor::process`: a Device I/O Request for a filesystem device
  whose major function `MajorFunction` has no variant for is completed with
  a Device I/O Completion before the PDU is decoded: STATUS_SUCCESS for
  IRP_MJ_FLUSH_BUFFERS (0x09), STATUS_NOT_SUPPORTED for any other
  (`complete_unknown_drive_major`). Requests for other device types are
  handled as before.
- `handle_device_io_request`: a filesystem request that
  `ServerDriveIoRequest::decode` refuses (SET_VOLUME_INFORMATION, an unknown
  information class, a cut-short body) is completed with
  STATUS_NOT_SUPPORTED instead of returning the decode error.
- Both completions are laid out like a Device Close Response: the header,
  DeviceId, CompletionId, IoStatus and 4 bytes of padding, as the crate's
  own unsupported printer requests are.

Not copied from the published crate: `CHANGELOG.md`, `Cargo.lock`,
`Cargo.toml.orig`, `.cargo_vcs_info.json`. `Cargo.toml` is the published
(normalized) manifest, unchanged.

## Removing this

Drop the directory and the `[patch.crates-io]` entry once the app moves to
`ironrdp-rdpdr` 0.9 or later together with the rest of the IronRDP set.
`src/rdp/drive.rs` then answers LOCK_CONTROL and the new request types
through the new `ServerDriveIoRequest` variants.
