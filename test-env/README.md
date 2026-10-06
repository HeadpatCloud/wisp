# Test servers

Seven VNC servers and one OpenSSH server in Docker for the live tests. Every port is
published on `127.0.0.1` only. The passwords are fixed and written down here, so none of this
may ever be reachable from a network.

## Start and stop

```bash
docker compose -f test-env/docker-compose.yml up -d --build
docker compose -f test-env/docker-compose.yml down
docker compose -f test-env/docker-compose.yml restart x11vnc
```

The containers are named `wisp-test-<service>`. Nothing is mounted from the host; read things
back with `docker exec` or `docker cp`.

After a start or restart six servers send their banner at once. `x11vnc` needs about 1.5 s,
and until then Docker accepts the TCP connection and closes it without a banner. Wait for the
banner, not for the port.

## Live tests

`src-tauri/tests/live_vnc.rs` drives the app's VNC session against these servers. A plain
`cargo test` skips them. With the seven containers up, from the repository root:

```bash
docker compose -f test-env/docker-compose.yml up -d
cargo test --manifest-path src-tauri/Cargo.toml --test live_vnc -- --ignored --test-threads=1
```

`--test-threads=1` is required: the tests share the servers, and they resize a screen, stop
containers and move the one pointer each server has. A run takes about a minute, and some
20 s more when `x11vnc` has just been started (its clipboard, see below).

## Servers

Password `wisptest` everywhere. `tigervnc-plain` wants user `wisp` and password `wisptest`.

| service | address | server | security types offered | screen | desktop name |
|---|---|---|---|---|---|
| `tigervnc-x509` | `127.0.0.1:5901` | TigerVNC 1.15 | 19 VeNCrypt 0.2, subtype 261 X509Vnc | 1280x800 | `wisp-test-tigervnc-x509` |
| `tigervnc-vncauth` | `127.0.0.1:5902` | TigerVNC 1.15 | 2 VncAuth | 1280x800 | `wisp-test-tigervnc-vncauth` |
| `tigervnc-plain` | `127.0.0.1:5903` | TigerVNC 1.15 | 19 VeNCrypt 0.2, subtype 262 X509Plain | 1280x800 | `wisp-test-tigervnc-plain` |
| `tigervnc-default` | `127.0.0.1:5907` | TigerVNC 1.15, default `SecurityTypes` | 19 VeNCrypt 0.2 (subtypes 258 TLSVnc, 2 VncAuth) and 2 VncAuth | 1280x800 | `wisp-test-tigervnc-default` |
| `x11vnc` | `127.0.0.1:5904` | x11vnc 0.9.17 on Xvfb | 2 VncAuth | 1280x800 | `wisp-test-x11vnc` |
| `tightvnc` | `127.0.0.1:5905` | TightVNC 1.3.10 | 2 VncAuth, 16 Tight | 1280x800 | `root's wisp-test-tightvnc desktop (wisp-test-tightvnc:1)` |
| `qemu` | `127.0.0.1:5906` | QEMU 10.0 | 2 VncAuth | 720x400 | `QEMU (wisp-test-qemu)` |

The Dockerfiles pin Debian 13 (`debian:trixie-slim`). The versions above and every measured
fact in this file belong to that release, as it was on 2026-10-01 (Debian 13.7).

All seven send `RFB 003.008\n` and a pixel format of 32 bpp, depth 24, little endian, true
colour, max 255/255/255, shifts 16/8/0.

Reply to a wrong password (SecurityResult 1 plus reason):

| server | reason |
|---|---|
| TigerVNC (all four) | `Authentication failed` |
| x11vnc | `password check failed!` |
| TightVNC | `Authentication failed` |
| QEMU | `Authentication failed` followed by a NUL byte inside the reason string |

Encoding used for the first rectangle of a full update:

| client offers | TigerVNC, x11vnc, QEMU | TightVNC |
|---|---|---|
| ZRLE, Hextile, Raw | ZRLE | Hextile |
| Hextile, Raw | Hextile | Hextile |
| Raw | Raw | Raw |

## X509 certificate

`tigervnc-x509` and `tigervnc-plain` present the same self-signed certificate (RSA 2048,
`CN=wisp-test`). It is generated when the image is built, so it stays the same across
container restarts and recreation, and changes when that image layer is built again (no build
cache, or another machine). SHA-256 of the DER form:

```bash
docker exec wisp-test-tigervnc-x509 openssl x509 -in /etc/wisp/cert.pem -noout -fingerprint -sha256
```

A Python/OpenSSL client negotiated TLS 1.3 with `TLS_AES_256_GCM_SHA384`.

## What is on screen

Every X server shows one `xterm` titled `pattern` at +40+40 (80x24 cells, font `10x20`, white
background): the line `WISP VNC TEST PATTERN`, the digits, the upper-case and lower-case
alphabet, and a band of eight colour bars. Its text cursor is hidden. The whole screen has 9
distinct colours. On TigerVNC `twm` frames the window; on TightVNC the root window is a
black and white stipple, elsewhere it is black.

QEMU shows the SeaBIOS text screen ending in `No bootable device.` (grey on black, 2 colours).

## Looking from inside

`DISPLAY=:1` is set in the X containers and everything runs as root, so `docker exec` works
without extra flags. QEMU has no X server and nothing to look at from inside. In Git Bash
set `MSYS_NO_PATHCONV=1` first, otherwise paths like `/tmp/typed` are rewritten to Windows
paths before Docker sees them.

Pointer position:

```bash
docker exec wisp-test-x11vnc xdotool getmouselocation   # x:1000 y:700 screen:0 window:543
docker exec wisp-test-tightvnc pointer                  # x:1000 y:700
```

Typed text. Keys go to the window under the pointer, so move the pointer into the xterm
first (300,660 is inside this one). `cat` gets a line when Return is pressed. Ctrl+D ends it.

```bash
docker exec -d wisp-test-x11vnc xterm -geometry 80x4+40+600 -fn 10x20 -e sh -c 'cat > /tmp/typed'
docker exec wisp-test-x11vnc cat /tmp/typed
```

Typing `a`, Shift+`B`, `1`, Shift+`!`, eacute (keysym 0xe9), Return gives `aB1!\xe9\n`. Start
the xterm with `docker exec -d -e LANG=C.UTF-8 ...` to get UTF-8 instead (`aB1!\xc3\xa9\n`).

Wheel and buttons. Move the pointer into the window (1100,200 here). Wheel up is button 4,
wheel down is button 5.

```bash
docker exec -d wisp-test-x11vnc sh -c 'xev -geometry 200x200+1000+100 > /tmp/xev.log 2>&1'
docker exec wisp-test-x11vnc grep -A3 '^Button' /tmp/xev.log
```

Clipboard (not on TightVNC, see below). `xclip` stays in the background to own the
selection, so its output has to be redirected or `docker exec` does not return.

```bash
docker exec wisp-test-x11vnc xclip -o -selection clipboard
printf 'text' | docker exec -i wisp-test-x11vnc sh -c 'xclip -selection clipboard >/dev/null 2>&1'
```

Screenshot. The first form is raw RGB, 3 bytes per pixel, row by row.

```bash
docker exec wisp-test-x11vnc sh -c 'xwd -root -silent | convert xwd:- rgb:-' > shot.rgb
docker exec wisp-test-x11vnc sh -c 'xwd -root -silent | convert xwd:- png:-' > shot.png
```

Screen size: first line of `docker exec wisp-test-x11vnc xrandr`, or
`xdpyinfo | grep dimensions` (TightVNC has no RandR).

Server log. `docker logs` also has the lines of earlier starts of the same container; `--since`
with the start time leaves only the current one. When a client leaves TigerVNC logs
`Closing <address>: Clean disconnection`, x11vnc and TightVNC log `Client <address> gone`, and
QEMU logs nothing.

```bash
docker logs --since "$(docker inspect -f '{{.State.StartedAt}}' wisp-test-x11vnc)" wisp-test-x11vnc
```

Keys held down. Pointer events carry the modifier state, so move the pointer while `xev`
listens on the root window: `state 0x0` is none, `0x1` Shift, `0x4` Control.

```bash
docker exec wisp-test-x11vnc sh -c 'xev -root -event mouse > /tmp/state & sleep 1
  xdotool mousemove 700 500 mousemove 640 400; sleep 1; kill $!
  grep -o "state 0x[0-9a-f]*" /tmp/state; rm /tmp/state'
```

Processes (the images have no `ps`):

```bash
docker exec wisp-test-x11vnc sh -c 'cat /proc/[0-9]*/comm'
```

## Resizing (TigerVNC only)

```bash
docker exec wisp-test-tigervnc-vncauth xrandr -s 1024x768
docker exec wisp-test-tigervnc-vncauth xrandr -s 1280x800
```

`-s` takes the sizes `xrandr` lists (640x480 up to 1920x1200) and exits 0.

`xrandr --fb 1024x768` also shrinks the screen and the client is told about it, but xrandr
prints an error and exits 1, and the output `VNC-0` is left disconnected. After that `-s`
fails with `Size ... not found in available modes` until the container is restarted.
`xrandr --fb` to a larger size (for example 1400x900) works and exits 0.

The server announces the new size only inside a framebuffer update, so the client has to ask
for one. That update carries just the DesktopSize or ExtendedDesktopSize rectangle (the
latter up to four times in one update); the pixels come with the next request. A client that
offered neither pseudo-encoding is disconnected when the screen is resized.

## Quirks

TigerVNC
- Until the client sends a pointer event the server draws the pointer into the picture and
  sends a 0x0 cursor. That changes 28 pixels around 640,400 compared to a screenshot from
  inside. After a pointer event at the pointer's position the picture matches and a real
  cursor shape arrives: 9x16 with hotspot 4,8 and 86 opaque pixels over the xterm, 16x16 with
  hotspot 7,7 and 176 opaque pixels over the root window. Moving the pointer from inside with
  `xdotool mousemove` switches it back.
- `twm` runs in all four containers. It adds a title bar and border (the pattern xterm's
  contents start at 42,61 instead of 40,40) and highlights the frame of the window under the
  pointer.
- `-BlacklistThreshold 1000000` is set in `tigervnc/start.sh`. With the default, five failed
  or abandoned handshakes in a row get every later connection refused with
  `Too many security failures` for 10 s, then 20 s, and all test clients come from the same
  Docker address. A successful login resets the count.
- Clipboard text is Latin-1 on the wire and UTF-8 on the X side: ClientCutText `\xe9` reads
  back as `\xc3\xa9` with `xclip -o`, and `\xc3\xa9` put in with `xclip` arrives as `\xe9`.
  Both CLIPBOARD and PRIMARY are set and both are sent.

x11vnc
- The true-colour flag in ServerInit is 255, not 1.
- The clipboard does nothing in either direction until x11vnc has created its selection
  window, which it logs as `created selwin`. That happened 9 to 19 s into the first
  connection after a start, and never while no client was connected. Text sent from inside
  only started to arrive 15 to 18 s into that first connection. Later connections to the same
  process worked within 2 s. Once, the client's own text came back as a ServerCutText. On a
  first connection an empty ServerCutText came before the first text.
- Clipboard bytes pass through unchanged in both directions (no Latin-1/UTF-8 conversion).
- No window manager. `xrandr` prints the size but Xvfb cannot be resized.
- With the cursor pseudo-encoding offered, an 18x18 cursor (an arrow, hotspot 0,0) arrives in
  the first update and the picture matches a screenshot from inside exactly. Once the client
  moves the pointer the cursor is the one TigerVNC sends for the same place.
- No lockout seen after 8 wrong passwords in a row.
- A key a client leaves pressed stays pressed when that client has gone. After one left
  Control down, the next client's `a` and `B` arrived as `\x01\x02`. `xdotool keyup Control_L`
  from inside lets it go (measured with Shift: state `0x1` before, `0x0` after), and so does
  a restart.
- A pointer event for the place the last client left the pointer at is ignored, also when
  the pointer has been moved from inside since: after a client's 500,500 and
  `xdotool mousemove 640 400`, the next client's 500,500 left it at 640,400, and 501,500
  moved it. TigerVNC and TightVNC moved it both times.

TightVNC
- The X server has no XKEYBOARD, RandR or XFIXES. `xdotool` segfaults on every command, which
  is why the image has `pointer`. `xrandr` prints `RandR extension missing`.
- The clipboard is the cut buffer, not the selections, so `xclip` sees nothing:

  ```bash
  docker exec wisp-test-tightvnc xprop -root CUT_BUFFER0
  docker exec wisp-test-tightvnc xprop -root -f CUT_BUFFER0 8s -set CUT_BUFFER0 'text'
  ```
- The fifth wrong password in a row is answered with
  `Authentication failed, too many tries`. After that every connection is refused before
  authentication with `Too many authentication failures`: still refused 5 s later, accepted
  again 16 s later. A successful login resets the count; abandoned handshakes do not count.
  Xtightvnc has no option to switch this off.
- No window manager.

QEMU
- No guest, so nothing reacts to keys or the pointer, and there is no clipboard.
- The text cursor blinks: while a client is connected the 18 pixels at 0..8,173..174 change
  about every 0.4 s, so two pictures are only the same every other time.
- The VNC password is set over QMP right after QEMU starts; `docker logs wisp-test-qemu`
  shows the two `{"return": {}}` replies.
- No lockout seen after 8 wrong passwords in a row.

All servers
- A client that does not offer the cursor pseudo-encoding gets the pointer drawn into the
  picture by all three X servers: 28 pixels for the xterm's I-beam at 640,400. A freshly
  started x11vnc drew an arrow instead (54 pixels) and the I-beam only later on; moving the
  pointer did not make it switch.
- `docker stop` takes about 0.3 s and a connected client sees the connection close within
  0.1 s. `docker start` brings the server back as after a restart.
- If `up` builds the images and then fails to create some containers because their image is
  gone, the Docker host is deleting unused images. Docker Desktop with Kubernetes enabled
  does that while its disk is nearly full. Run the same command again.

## OpenSSH server

`openssh` (container `wisp-test-openssh`) is OpenSSH 10.0p2 on the same Debian 13.7, with two
`sshd` on the same host keys:

| address | sshd | for |
|---|---|---|
| `127.0.0.1:2201` | Debian's `sshd_config` plus `/etc/ssh/sshd_config.d/wisp.conf` | normal sessions |
| `127.0.0.1:2202` | the same, with `openssh/sshd_rekey.conf` in front: `RekeyLimit 1M 10s` | new keys after every mebibyte |

Both send `SSH-2.0-OpenSSH_10.0p2 Debian-7+deb13u4` and both log at `DEBUG1`, which has a
line for every key exchange. `docker logs wisp-test-openssh` has the log of both.

- User `wisp`, password `wisptest`, shell `bash`, home `/home/wisp`. Password and public key
  login are on, keyboard-interactive is off, TCP forwarding is on, `GatewayPorts` is off.
- Client keys in `/etc/wisp/keys/`, all three in `~wisp/.ssh/authorized_keys`: `id_ed25519`,
  `id_rsa` (3072 bit) and `id_ed25519_enc` (passphrase `wisptest`). They are readable by root
  only: `docker exec wisp-test-openssh cat /etc/wisp/keys/id_ed25519`.
- The host keys and the client keys are made when the image is built, so they stay the same
  until that image layer is built again. What a client is shown:

  ```bash
  docker exec wisp-test-openssh ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
  ```
- Inside, `socat` echoes on `127.0.0.1:7000`, and on `127.0.0.1:7001` it accepts connections
  and reads nothing from them (each is held for 10 minutes by a `sleep`, also after the
  client has gone). `socat`, `sha256sum`, `dd` and `head` are there for checks from inside.
- `docker exec` runs as root; add `-u wisp` to act as the user.

### Live tests

`src-tauri/tests/live_ssh.rs` drives the app's own SSH, SFTP and tunnel functions against this
server. A plain `cargo test` skips them.

```bash
docker compose -f test-env/docker-compose.yml up -d openssh
cargo test --manifest-path src-tauri/Cargo.toml --test live_ssh -- --ignored --test-threads=1 --nocapture
```

A run takes about three minutes, 90 s of it in `idle_session_survives`. It writes up to
1.3 GB into the system's temporary folder and 2.5 GB into `/home/wisp/live` in the
container, and removes both. Two things stay and grow with every run until the container is
recreated: `/home/wisp/.bash_history` (about 2 KB a run) and the container's log (about
0.25 MB a run). The tests count key exchanges in that log, from the moment a transfer starts
(`docker logs --since`).

Every test that measures prints one line `MEASURE <test> <name>=<value> ...`.

The same with an optimised build (the first build takes about two and a half minutes and
2.2 GB under `src-tauri/target/release`):

```bash
cargo test --release --manifest-path src-tauri/Cargo.toml --test live_ssh -- --ignored --test-threads=1 --nocapture
```

### Measured with russh 0.61.2

2026-10-06, server and client on the same machine, nothing else running. russh and the
server agreed on `mlkem768x25519-sha256`, host key `ssh-ed25519` and
`chacha20-poly1305@openssh.com`.

The range over 12 runs of the unoptimised `test` profile. Compare library versions on the
rows marked yes; the others are too noisy, or are not the client's doing.

| test | value | range | compare |
|---|---|---|---|
| `sftp_round_trip` (64 MiB) | upload MiB/s | 121 to 157 | yes |
| | download MiB/s | 51 to 70 | yes |
| `sftp_through_forced_rekeys` (64 MiB, port 2202) | upload MiB/s | 67 to 82 | yes |
| | download MiB/s | 25.8 to 28.8 | yes |
| | key exchanges during the upload | 25 to 32 | no, depends on the rate |
| | key exchanges during the download | 85 | yes |
| `sftp_past_one_gibibyte` (1200 MiB) | upload MiB/s | 183 to 203 | yes |
| | download MiB/s | 64 to 69 | yes |
| | key exchanges during the download / upload | 1 / 1 (one run) | yes |
| `local_forward_carries_bulk_both_ways` (8 MiB) | MiB/s | 28.0 to 52.4 | no, pass or fail only |
| `shell_stays_responsive_while_a_tunnel_is_blocked` | echo median ms | 0.5 to 0.6 | yes |
| | echo maximum ms | 1.2 to 1.5 | yes |
| | the same before the tunnel, median / maximum ms | 0.5 to 0.6 / 0.6 to 0.8 | yes |
| | KiB written before the write blocked | 4224 to 5440 | no |
| `shell_stays_responsive_during_a_transfer` (256 MiB) | echo median ms | 7.7 to 9.6 | yes |
| | echo maximum ms | 10.2 to 24.7 | no, one late echo decides it |
| | the same before the upload, median / maximum ms | 0.5 to 0.6 / 0.6 to 0.9 | yes |
| | upload MiB/s | 142 to 173 | yes |
| `shell_echo_and_resize` | echo ms | 0.5 to 0.7 | yes |
| `idle_session_survives` | echo after 90 s, ms | 1.1 to 1.6 | yes |
| `password_login` | connect ms | 21.9 to 25.8 | yes |
| | wrong password refused after ms | 2143 or 4285 | no, the server's delay |
| | login ms | 19.1 to 36.4 | no |
| `key_login` | ed25519 ms | 19.5 to 20.3 | yes |
| | RSA ms | 29.8 to 42.6 | no |
| | ed25519 with passphrase ms | 339 to 447 | no, mostly unlocking the key |
| `jump_host` | channel, connect and login of the second hop, ms | 233 to 255 | yes |
| | echo ms | 0.7 to 0.9 | yes |
| `dynamic_forward` | SOCKS5 greeting to reply, ms | 1.3 to 4.4 | no |
| `remote_forward` | exchange ms | 179 to 181 | no, mostly `docker exec` |

- Download rates include writing the file here, through eight handles at once; upload rates
  include reading it.
- A wrong password is refused by `sshd` after 2.14 s or after 4.28 s, with one
  `Failed password` line in the log either way.
- The echo times of the blocked tunnel are those of the quiet connection: on this version a
  channel that waits for window does not hold the others back. An upload does slow the
  shell, from 0.5 ms to 8 or 9.

One run of the optimised build (`--release`), for the order of magnitude:

```
MEASURE dynamic_forward open_ms=1.4
MEASURE idle_session_survives echo_ms=1.1
MEASURE jump_host login_ms=236.0 echo_ms=0.6
MEASURE key_login ed25519_ms=20.2 rsa_ms=19.9 ed25519_passphrase_ms=148.5
MEASURE local_forward_carries_bulk_both_ways mib_s=49.4
MEASURE password_login connect_ms=14.2 refuse_ms=4284.5 login_ms=36.0
MEASURE remote_forward exchange_ms=181.5
MEASURE sftp_past_one_gibibyte down_s=11.5 up_s=4.2 down_mib_s=104.7 up_mib_s=285.5 down_rekeys=1 up_rekeys=1
MEASURE sftp_round_trip up_mib_s=169.9 down_mib_s=111.7
MEASURE sftp_through_forced_rekeys up_mib_s=134.6 down_mib_s=52.4 up_rekeys=38 down_rekeys=85
MEASURE shell_echo_and_resize echo_ms=0.6
MEASURE shell_stays_responsive_during_a_transfer median_ms=4.3 max_ms=27.6 quiet_median_ms=0.5 quiet_max_ms=1.0 up_mib_s=191.1 echoes_from_mib=20 echoes_until_mib=53
MEASURE shell_stays_responsive_while_a_tunnel_is_blocked median_ms=0.5 max_ms=1.1 quiet_median_ms=0.5 quiet_max_ms=0.6 taken_kib=5248
```

### Quirks

- A download gets one key exchange per 789,196 bytes `sshd` sent on port 2202. During a
  64 MiB upload `sshd` had received 1.7 to 3.1 MB between two exchanges, so an upload sees
  about a third as many.
- On port 2201 `sshd` asks for new keys by itself after 134,217,728 cipher blocks of 8 bytes,
  1 GiB (`rekey out after 134217728 blocks` in its log). The 1200 MiB transfers cross that
  once each way.
- `sshd` logs a line with `rekeying out` for every exchange.
- `sshd` counts connections that end without a login against the client's address
  (`PerSourcePenalties`, logged as `srclimit_penalise`), and all clients come from the Docker
  gateway. A probe of the banner costs 1 s and a failed login 5 s; the address is refused
  once 15 s have added up. A run of the suite does not get there.
- `printf 'ping\n' | socat - TCP:127.0.0.1:7100` closes its sending side as soon as `ping`
  is out and then waits for the answer.
