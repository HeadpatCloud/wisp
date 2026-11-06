# Vendored ironrdp-mstsgu 0.0.1

Source: crates.io `ironrdp-mstsgu` 0.0.1 (upstream commit
`11a0810cfbbabd8b8023875a05e3041216d4b01b`, `crates/ironrdp-mstsgu` in
https://github.com/Devolutions/IronRDP). Licence: MIT OR Apache-2.0. The
published crate carries no licence files; `LICENSE-MIT` and `LICENSE-APACHE`
are the IronRDP repository's at the same commit, as published with
`ironrdp-rdpdr` 0.7.0.

Used as a path dependency in `src-tauri/Cargo.toml`.

## Why

- 0.0.1 opens its own TCP connection and TLS through `ironrdp_tls::upgrade`,
  which accepts any certificate, and then sends the gateway login as
  `Authorization: Basic`. wisp checks the gateway's certificate before the
  login goes out, so it has to hand the client a stream it verified.
- `poll_write` passes the whole buffer it is given to the worker as one data
  packet, and the worker encodes that into an 8192-byte buffer. A write of
  more than 8182 bytes fails to encode and ends the tunnel; the first copy of
  a file from a shared folder did that against Windows Server 2025.
- The target port is fixed to 3389.
- Server data reaches `assert!`s, unchecked subtractions and an out-of-bounds
  read, all panics.
- A 401, any other HTTP status, and a refused tunnel, tunnel authorization or
  channel are all the same `Connect` error, so a wrong password, a gateway
  that wants NTLM and a refused target look alike.

0.0.2 and 0.1.0 add certificate validation and a port, but keep the asserts
and the 8 KB write limit, and need `ironrdp-core` 0.3, `ironrdp-tls` 0.3 and
`sspi` with `network_client` (aws-lc-rs), which means moving the whole
IronRDP set.

## Changes from 0.0.1

`Cargo.toml`
- Removed the `ironrdp-tls` dependency and the `native-tls` and `rustls`
  features, which only chose its TLS backend.

`src/lib.rs`
- `GwClient::connect` takes the stream to the gateway, TLS done, as its first
  argument (generic over `AsyncRead + AsyncWrite + Unpin + Send + 'static`)
  and returns the client alone. The TCP connect, `local_addr` and
  `ironrdp_tls::upgrade` (lines 119-134) are gone; the caller knows its local
  address. `gw_endpoint` is no longer dialed and is sent as the Host header
  as it is, instead of the part before the first `:` (which broke IPv6
  addresses). `GwConn` and `connect_ws` are generic over the stream for the
  same reason.
- `GwConnectTarget::server_port`: the port in the channel request, which was
  `port: 3389`.
- `poll_write` takes at most `MAX_WRITE` bytes (8192 less the 8-byte packet
  header and the 2-byte length) and returns that count, instead of
  `Bytes::from(buf.to_vec())` and `Ok(buf.len())`. The worker's buffer is
  `[0u8; PACKET_BUFFER]` instead of `[0u8; 8192]`, so the two stay in step.
- The worker task runs one loop per direction, `read_packets` and
  `write_packets` joined with `tokio::try_join!`, instead of one `select!`
  over the keepalive, the WebSocket and `out_rx`. That loop awaited
  `in_tx.send` while the 4-slot channel to the reader was full and drained
  nothing from `out_rx` meanwhile. With writes cut to one packet each
  (above), a write of more than four packets made while the reader is not
  reading then waited for good. Both channels keep their 4 slots, and
  `read_packets` reads nothing from the WebSocket while it waits for room, so
  what the reader has not taken stays bounded.
- `GwErrorKind` gains `Unauthorized { basic }`, `HttpStatus(u16)`,
  `TunnelRefused(u32)`, `TunnelAuthRefused(u32)` and `ChannelRefused(u32)`,
  each in `Display`.
  - A 401 to the upgrade is `Unauthorized`, with `basic` true when any
    `WWW-Authenticate` value offers Basic (`offers_basic`: challenges split
    at commas outside quotes, the scheme compared without case). Any other
    status than 101 is `HttpStatus`. Both were
    `Error::new("WS Upgrade", GwErrorKind::Connect)`.
  - A tunnel response, tunnel authorization response or channel response
    with a non-zero code keeps the code in its variant; each was
    `GwErrorKind::Connect`.
- Panics on server data become errors:
  - worker, `assert!(cur.len() >= header_length - hdr.size())`: a length
    under the header size is `Decode`, a packet shorter than its length is
    `PacketEof`;
  - `read_packet`, `header_length - hdr.size()`: a length under the header
    size is `Decode`;
  - `tunnel` and `channel`, `assert!(cur.eof())`: bytes left over are
    `Decode`;
  - `channel`, `assert!(hdr.ty == PktTy::ChannelResp)`: another packet type
    is `Decode`;
  - `GwClient::work` is an `Option`, emptied once the worker's end was
    reported (`poll_work`). Polling its `JoinHandle` again, as the next read
    or write after a lost tunnel did, panicked with "JoinHandle polled after
    completion".
- `poll_read` hands out what the worker has queued before it reports the
  worker's end, instead of checking `poll_work` first, which threw away the
  last packets the gateway sent before it closed: a logoff, a takeover or an
  admin disconnect through a gateway read as a lost connection, and the end
  of a sign-in as an `Other` error carrying rustls' text. The worker's end is
  reported only once its channel is empty, and a read takes every queued
  packet that fits. A WebSocket that closes or ends (`None`, a Close frame,
  `ConnectionClosed`, `AlreadyClosed`, `ResetWithoutClosingHandshake`, an
  `UnexpectedEof` underneath) is `GwErrorKind::Closed`, which reads as
  `io::ErrorKind::UnexpectedEof`; the kind of the first error is kept for the
  reads and writes after it.
- While connecting, a WebSocket that closes or ends (`None`, a Close frame,
  the cases above, or a reset or aborted connection) when sending or reading
  a packet is `Closed` too, except before the handshake answer, where it stays
  `Connect` (nothing has shown a gateway yet). After the channel request it is
  the new `GwErrorKind::ClosedAtChannel`. Windows Server 2025's gateway resets
  the connection right after it refuses a channel (RAP), and on some runs the
  reset came before the answer was read, which read as "not a gateway".
- After the gateway's close (`Closed`), writes and flushes are taken and
  dropped until a read has met the end, as a socket takes writes after the
  peer's FIN; then they fail with `UnexpectedEof`. A session answering a PDU
  between the close and reading the server's last PDUs (a frame
  acknowledgement before a logoff) ended as a lost connection. Whether those
  PDUs were read cannot be told from the channel: TLS above reads ahead. Any
  other end of the worker fails writes at once, as before.
- WebSocket Ping and Pong frames are skipped (tungstenite answers a ping
  itself); they were decoded as MS-TSGU packets and ended the tunnel.
- `poll_flush` and `poll_shutdown` wait for the worker: the client hands it a
  flush or a close behind the data (`Out`), and the writer answers once
  everything before it has gone out; the close sends a WebSocket Close. Both
  returned at once before, so data still queued was lost when the stream was
  dropped right after. A write after `poll_shutdown` is `BrokenPipe`. A flush
  or close whose answer was dropped, or that went into the channel as the
  worker ended and so is never answered, waits for the worker's end and
  reports it as a write does.
- There is no `Drop` that aborts the worker. Once the client is gone, the
  writer sends what is queued and a WebSocket Close, and the reader drops
  what comes until the gateway ends the connection; `DRAIN_LIMIT` (2 s) after
  the client went the worker ends anyway. Before, a closed tab aborted the
  worker before the MCS Disconnect Provider Ultimatum went out.

`src/proto.rs`
- `HandshakeRespPkt::FIXED_PART_SIZE` counts 2 bytes for `extended_auth`
  instead of 1. `decode` reads a `u16` there, so a 9-byte response passed
  the size check and then read past its end.
- `ChannelPkt::size` counts the UTF-16 units of each resource name, which
  `encode` writes, instead of its UTF-8 bytes: for a server name outside
  ASCII the packet header gave a length larger than the packet.

Not copied from the published crate: `Cargo.lock`, `Cargo.toml.orig`,
`.cargo_vcs_info.json`. `Cargo.toml` is the published (normalized) manifest
with the edit above.

## Removing this

Drop the directory and depend on the published crate once a release takes a
verified stream or a certificate check that runs before the login, takes the
target port, cuts writes to what fits one packet, has no panics on server
data, runs each direction in its own loop, hands out everything it read
before it reports the end and reports a closed connection as `UnexpectedEof`,
takes writes after the gateway's close until a read meets the end,
sends what it was given and a WebSocket Close on a flush, a shutdown and a
drop, skips WebSocket pings, and fits the rest of the app's IronRDP set
without aws-lc-rs. `src/rdp/gateway.rs` then maps that release's errors to
the same messages; its tests for each of these run against the stand-in in
`src/rdp/gatewaytest.rs`.
