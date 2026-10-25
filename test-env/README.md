# Test servers

Seven VNC servers, one OpenSSH server and one xrdp server in Docker for the live tests. Every
port is published on `127.0.0.1` only. The passwords are fixed and written down here, so none
of this may ever be reachable from a network. A Windows Server VM in Hyper-V for the Windows
RDP tests is described in [windows/README.md](windows/README.md).

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
  client has gone). One test starts a third `socat` on `127.0.0.1:7002` that sends zeros
  without end, and ends it. `socat`, `sha256sum`, `dd` and `head` are there for checks from inside.
- `docker exec` runs as root; add `-u wisp` to act as the user.

### Live tests

`src-tauri/tests/live_ssh.rs` drives the app's own SSH, SFTP and tunnel functions against this
server. A plain `cargo test` skips them.

```bash
docker compose -f test-env/docker-compose.yml up -d openssh
cargo test --manifest-path src-tauri/Cargo.toml --test live_ssh -- --ignored --test-threads=1 --nocapture
```

A run takes about four minutes, 90 s of it in `idle_session_survives`. It writes up to 1.3 GB
into the system's temporary folder and 2.5 GB into `/home/wisp/live` in the container, and
removes both. Two things stay and grow with every run until the container is recreated: `/home/wisp/.bash_history` (about 2 KB a run) and the container's log (about
0.25 MB a run). The tests count key exchanges in that log, from the moment a transfer starts
(`docker logs --since`).

Every test that measures prints one line `MEASURE <test> <name>=<value> ...`.

The same with an optimised build (the first build takes about two and a half minutes and
2.2 GB under `src-tauri/target/release`):

```bash
cargo test --release --manifest-path src-tauri/Cargo.toml --test live_ssh -- --ignored --test-threads=1 --nocapture
```

### Measured with russh 0.61.2 and 0.62.4

2026-10-06, server and client on the same machine, nothing else running. russh and the
server agreed on `mlkem768x25519-sha256`, host key `ssh-ed25519` and
`chacha20-poly1305@openssh.com`.

The 0.61.2 debug column is the range over 12 runs of the unoptimised `test` profile on that day,
widened where 0.61.2 itself measured outside it when it was run again on 2026-10-14 (3 suite
runs, 6 more of `sftp_past_one_gibibyte`): the machine was slower that day, the quiet shell echo
0.7 to 0.8 ms instead of 0.5 to 0.6. The 0.62.4 debug column is 6 suite runs: the first 3 ran
back to back, the other 3 each right after a run of 0.61.2. Both release columns are 2 runs
each, taking turns. One debug run of 0.62.4 (the second) took 241 s against 184 to 202 s for the
others, and in it every login and transfer row was two to four times slower (jump host 905 ms,
RSA login 125 ms against 37 ms, 1200 MiB download 22 s); its cause is not known. It alone is
the low end of 0.62.4 in `sftp_round_trip` (download 31 against 51 MiB/s, 39 % below the
0.61.2 minimum; upload 93 against 106, 12 % below) and of the forced rekeys upload. Without it, 0.62.4's
upload rates are at most 11 % below the lowest 0.61.2 value of that day (1200 MiB: 152
against 170 MiB/s; the 6 paired runs of that test alone: 164 to 198 against 164 to 181).
A few echo maxima are above the 0.61.2 range in runs that were not the slow one (blocked
tunnel 2.2 and 2.4 ms against 1.8, `shell_echo_and_resize` 1.4 ms against 1.0). Compare library
versions on the rows marked yes; the others are too noisy, or are not the client's doing.

| test | value | 0.61.2 debug | 0.62.4 debug | 0.61.2 release | 0.62.4 release | compare |
|---|---|---|---|---|---|---|
| `sftp_round_trip` (64 MiB) | upload MiB/s | 106 to 157 | 93 to 118 | 86.5 to 172.5 | 151 to 172 | yes |
| | download MiB/s | 51 to 70 | 31 to 76 | 128.2 to 136.3 | 127 to 134 | yes |
| `sftp_through_forced_rekeys` (64 MiB, port 2202) | upload MiB/s | 58 to 82 | 53 to 61 | 129.8 to 179.1 | 134 to 166 | yes |
| | download MiB/s | 23.9 to 28.8 | 21.6 to 25.9 | 50.5 to 50.9 | 49.4 to 50.2 | yes |
| | key exchanges during the upload | 25 to 40 | 33 to 39 | 36 to 37 | 37 | no, depends on the rate |
| | key exchanges during the download | 85 | 85 | 85 | 85 | yes |
| `sftp_past_one_gibibyte` (1200 MiB) | upload MiB/s | 164 to 203 | 132 to 198 | 263.5 to 296.2 | 270 to 273 | yes |
| | download MiB/s | 57.6 to 69 | 54.6 to 71 | 125.2 to 133.6 | 126 to 131 | yes |
| | key exchanges during the download / upload | 1 / 1 | 1 / 1 | 1 / 1 | 1 / 1 | yes |
| `local_forward_carries_bulk_both_ways` (8 MiB) | MiB/s | 28.0 to 52.4 | 30.3 to 36.6 | 21.2 to 50.8 | 45.9 to 46.8 | no, pass or fail only |
| `shell_stays_responsive_while_a_tunnel_is_blocked` | echo median ms | 0.5 to 0.8 | 0.6 to 0.9 | 0.5 to 0.6 | 0.5 to 0.7 | yes |
| | echo maximum ms | 1.2 to 1.8 | 0.7 to 2.4 | 1.1 to 1.2 | 0.6 to 24.9 (2 of 32 runs over 3 ms) | yes |
| | the same before the tunnel, median / maximum ms | 0.5 to 0.8 / 0.6 to 1.4 | 0.6 to 0.8 / 0.8 to 1.2 | 0.5 / 0.6 to 0.7 | 0.5 / 0.6 | yes |
| | KiB written before the write blocked | 4224 to 5440 | 4480 to 5440 | 4480 to 5568 | 5312 | no |
| `shell_while_a_tunnel_download_is_blocked` | echo median ms | 0.7 to 0.9 | 0.7 to 1.0 | 0.5 to 0.6 | 0.6 to 0.7 | yes |
| | echo maximum ms (the one that meets the freeze) | 2985 to 3015, the grace time (it runs from the last write) | 3003 to 3012 | 3000 to 3010 | 3013 to 3015 | yes |
| | echoes slower than 100 ms, of 20 | 1 | 1 | 1 | 1 | yes |
| | KiB delivered before the forward ended | 272 to 411 | 272 to 411 | 272 to 411 | 411 | no |
| `shell_stays_responsive_during_a_transfer` (256 MiB) | echo median ms | 7.7 to 10.0 | 7.7 to 11.8 | 3.3 to 4.6 | 3.5 to 4.4 | yes |
| | echo maximum ms | 10.2 to 24.7 | 12.3 to 30.8 | 26.8 to 28.5 | 6.6 to 29.3 | no, one late echo decides it |
| | the same before the upload, median / maximum ms | 0.5 to 0.7 / 0.6 to 1.0 | 0.6 to 0.7 / 0.8 to 1.0 | 0.5 / 0.6 to 0.7 | 0.5 / 0.6 to 0.7 | yes |
| | upload MiB/s | 120 to 173 | 116 to 165 | 226 to 282 | 245 | yes |
| `shell_echo_and_resize` | echo ms | 0.5 to 1.0 | 0.8 to 1.4 | 0.5 to 0.6 | 0.6 | yes |
| `idle_session_survives` | echo after 90 s, ms | 1.1 to 2.0 | 1.6 to 2.3 | 1.3 to 1.6 | 1.4 to 1.7 | yes |
| `password_login` | connect ms | 21.9 to 31.2 | 26.3 to 33.0 | 14.7 to 15.5 | 16.9 to 17.5 | yes |
| | wrong password refused after ms | 2143, 2737 or 4285 | 2737 | 2737 | 1369 or 2737 | no, the server's delay |
| | login ms | 19.1 to 46.7 | 24.3 to 47.3 | 24.3 to 24.4 | 24.3 to 45.7 | no |
| `key_login` | ed25519 ms | 16.5 to 23.8 | 16.5 to 23.5 | 14.8 to 20.6 | 15.2 to 15.8 | yes |
| | RSA ms | 29.8 to 42.6 | 32.1 to 125.1 | 16.5 to 21.6 | 20.0 to 22.1 | no |
| | ed25519 with passphrase ms | 339 to 477 | 343 to 769 | 137 to 139 | 143 | no, mostly unlocking the key |
| `jump_host` | channel, connect and login of the second hop, ms | 233 to 298 | 272 to 905 | 267 to 275 | 256 to 271 | yes |
| | echo ms | 0.7 to 1.2 | 0.9 to 1.6 | 0.7 to 0.8 | 0.6 to 0.8 | yes |
| `dynamic_forward` | SOCKS5 greeting to reply, ms | 1.3 to 4.4 | 1.7 to 6.5 | 1.2 to 1.8 | 1.4 to 1.6 | no |
| `remote_forward` | exchange ms | 179 to 213 | 202 to 293 | 201 to 204 | 203 to 223 | no, mostly `docker exec` |

- Download rates include writing the file here, through eight handles at once; upload rates
  include reading it.
- `russh` is held at `=0.62.4`. From 0.62.5 on the session takes no outgoing data from any
  channel while one of them waits for window, so a tunnel whose far end reads nothing can hold
  the shell back. `shell_stays_responsive_while_a_tunnel_is_blocked` alone failed with a
  terminal silent for over 20 s in 5 of 30 runs on 0.63.3 and 7 of 30 on 0.64.0 (debug), 3 and
  2 of 30 in release; on 0.62.4 it passed 30 of 30 in debug and in release, on 0.61.2 30 of 30
  in debug.
  The 0.62.0 to 0.62.5 releases also hold a self-deadlock: a handler that awaits
  `reply.accept()` inside the session loop can wait for a slot in a queue only that loop
  empties (fixed in 0.62.6). The app's handler therefore accepts in a task of its own.
- Echo through a remote forward, in-process server, debug, 140 rounds each of 0.61.2 and 0.62.4
  with the shipped relay: no frozen session on either; forwards ended by the grace 8 and 13
  (Fisher's exact test, two-sided, p = 0.36: noise). The relay before the grace froze the
  session in 17 and 28 rounds (p = 0.10: suggestive at most).
- A wrong password is refused by `sshd` after 2.14 s or after 4.28 s, with one
  `Failed password` line in the log either way.
- The echo times of the blocked tunnel are those of the quiet connection: on this version a
  channel that waits for window does not hold the others back. An upload does slow the
  shell, from 0.5 ms to 8 or 9.
- The other direction used to freeze the whole connection for as long as the program stayed
  silent (20 of 20 echoes unanswered after 5 s each, 3 runs of 3, until the program closed
  its connection). `tunnel::relay` now keeps what the service sends in a backlog of 2 MiB per
  forward and holds the service back when it is full, as before. A forward whose backlog is
  full and whose program has taken nothing for `GRACE` (3 s) is closed. What that means:
  - A program that reads at about 100 KiB/s or faster loses nothing. Measured through a
    forward to a service that sends without end, a client reading at full speed, 100, 50, 20,
    5, 1 MiB/s and 100 KiB/s (256 MiB, 70 MiB at 5, 14 MiB at 1 MiB/s, 1.4 MiB at 100 KiB/s;
    five connections each): at the shipped 3 s all 35 completed. With 500 ms and 1 s the
    100 KiB/s client was cut in 5 of 5 and with 2 s in 1 of 5; the faster rows completed at
    500 ms already. The 100 KiB/s row also completed 10 of 10 at 3 s and at 4 s. The kernel
    lets the relay write again in steps of about 256 KiB once the program reads, so the longest
    silence seen was 2.57 s at 100 KiB/s, 0.26 s at 1 MiB/s and 0.065 s at 5 MiB/s. A program
    that reads slower than about 100 KiB/s with a full backlog will be cut. The step size and
    these times were measured on Windows loopback only; the cut-off rate may differ on
    another system.
  - While the backlog is full the whole SSH connection waits for that forward, the shell and
    the other forwards with it. For a program that takes nothing this lasts `GRACE` at most
    (it used to be without end): the test sees one echo of 20 take that long and the rest
    under 100 ms. A program that trickles keeps the forward alive and the wait goes on: with a
    client reading 100 KiB/s and a full backlog, shell echoes took 14.4 to 23.0 s (median
    20.3 s, 8 echoes) and the connection was not ended; the relay before this change gave
    11.9 to 20.4 s (median 19.0 s). A shell reply queues behind the data already on its way
    to the slow forward, so it waits as long as the program needs to take that.
  - A program that stops reading altogether while the service keeps sending loses its
    connection once about 2 MiB are waiting and `GRACE` has passed. The shell and the other
    forwards go on.
  - A program that writes back what it reads, with a loop that blocks in its own write, can
    stop for good: it waits to write, the relay's pump waits for the connection, the connection
    waits for the full backlog. Nothing moves then and nothing will; only ending that one
    forward helps, and it is ended after `GRACE`. The committed relay froze the whole connection
    in that case, for ever. In an echo of 8 MiB through a remote forward (in-process server,
    unoptimised build) the old relay froze the connection in 4 to 10 of 60 rounds and the new
    one ended the forward in 2 to 6 of 60 (counters of both directions and of the program
    unchanged over the whole grace in all 14 ends checked).
  - A backlog deeper than about 100 KiB costs the unoptimised build speed in that echo. Debug
    medians over 60 interleaved rounds: the relay before this change 79.5 to 97.2 MiB/s over
    all runs, the new one 61.3 to 75.8, about 15 to 30 % lower; at the shipped 3 s, 64.0 against
    79.5 in the same run (old relay: 2 rounds frozen for good, new relay: 1 forward ended,
    counters unchanged over the whole grace). With the hold-back at 64 KiB the new code
    measured 87.7 MiB/s, at 256 KiB 64.0, at 2 MiB 64.5 to 71.6. The optimised build shows no
    difference beyond the spread (old 178.9, 2 MiB 170.8 MiB/s).
  `shell_while_a_tunnel_download_is_blocked` asserts that every echo is answered within
  `GRACE` plus 1 s, that at most 3 of 20 take more than 100 ms, that the blocked client's
  connection is closed by the app and that a new connection through the forward works.

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

## xrdp server

`xrdp` (container `wisp-test-xrdp`, `127.0.0.1:3391`) is xrdp 0.10.1 with xorgxrdp 0.10.2 on
the same Debian 13.7. `start.sh` runs `xrdp-sesman` and `xrdp` in the foreground. User `wisp`,
password `wisptest`. Every fact below was measured on 2026-10-21 with the app's own client
(IronRDP), a hand-made X.224 request and Python's `ssl`.

```bash
docker compose -f test-env/docker-compose.yml up -d --build xrdp
cargo test --manifest-path src-tauri/Cargo.toml --test live_rdp -- --ignored --test-threads=1
```

A run of `src-tauri/tests/live_rdp.rs` takes about 20 s. It stops and starts the container once.

### Security

`security_layer=negotiate`, so xrdp offers TLS and the old RDP security layer, and no NLA
(CredSSP):

| client requests | xrdp selects |
|---|---|
| RDP security only | RDP security |
| TLS | TLS |
| NLA (HYBRID) only, or HYBRID_EX only | RDP security, not a refusal |
| TLS, HYBRID and HYBRID_EX (what the app sends) | TLS |

The log says so for every connection:
`Security protocol: configured [SSL|RDP], requested [SSL|HYBRID|HYBRID_EX|RDP], selected [SSL]`.
The client gets TLS 1.3 with `TLS_AES_256_GCM_SHA384`. Without NLA the password goes to xrdp
inside the TLS connection and xrdp checks it after the connection is up (see Wrong password).

The certificate is made when the image is built (RSA 2048, `CN=wisp-test`), so it stays the
same until that layer is built again. What a client is shown:

```bash
docker exec wisp-test-xrdp openssl x509 -in /etc/xrdp/cert.pem -noout -fingerprint -sha256
```

### The session

- The size is the one the client asks for: 1280x800 gives 1280x800. The X server is
  xorgxrdp's at depth 24 (32 bits per pixel in its framebuffer). xrdp sends the picture as
  RemoteFX, which the client offers (`xrdp_caps_process_codecs: RemoteFX`). The colour depth
  the RDP layer settled on is not in xrdp's log, and with RemoteFX it does not show.
- The session gets display `:10` and runs as `wisp`. To look into it from outside:

  ```bash
  docker exec -u wisp -e DISPLAY=:10 wisp-test-xrdp xdotool getmouselocation
  docker exec -u wisp -e DISPLAY=:10 wisp-test-xrdp sh -c 'xwd -root -silent | convert xwd:- rgb:-' > shot.rgb
  ```

  The same commands as for the VNC servers work there (`xrandr`, `xev`, `xclip`, `xwininfo`).
- `startwm.sh` starts `twm` and the pattern `xterm` the VNC servers show, at +40+40. `twm`
  frames it (the window with its frame is 804x503 at 40,40). The root window is black.
- The session outlives the connection. The next login of `wisp` gets the same session back
  (`++ reconnected session` in `/var/log/xrdp-sesman.log`) at the size that client asks for,
  so a test that leaves the session resized is undone by the next login. A stop of the
  container ends the session; the first login after a start makes a new one.
- The picture: the whole screen arrives about 0.5 s after the login. It has all 9 colours of
  the screenshot from inside, but RemoteFX is lossy: 58991 pixels differ from the screenshot,
  by at most 25 in a channel, all of them within 8 px of the pattern window's frame
  (32..852 x 32..551), where the text and the colour bars have edges. The black around it is
  exact. The test allows a difference of up to 32 inside that box and none outside it, and does
  not pin the count. The pointer is not in the picture; xrdp sends it as a cursor shape.
- Keys: scancodes for `a`, Shift+`b`, `1`, Shift+`1` and Return type `aB1!\n` into an xterm
  running `cat` (keyboard layout 0x409, `km-00000409.ini`). A Unicode key event is typed only
  when the layout has a key for the character: `z` (0x7A) and `A` arrive as typed, `é` (0xE9)
  and `€` (0x20AC) are dropped without a trace in the log.
- Pointer: positions arrive as sent. `xev` sees left as button 1, right as 3, wheel up and
  down as 4 and 5, wheel left and right as 6 and 7, and the two extra buttons as 8 and 9.
- Clipboard: works both ways at once, no warm-up as with x11vnc. Text copied with `xclip`
  inside reached the client, and the client's text was there for `xclip -o`, both 0.19 to
  0.24 s after the `docker exec` that copied or the call that sent it, which is about what
  one `docker exec` takes. `é` passes as UTF-8 both ways.
- Display Control: offered. The client's request for 1024x768 resizes the X screen (`xrandr`
  says `current 1024 x 768`) in about 10 ms on xrdp's side (`dynamic_monitor_process_queue:
  Clearing completed resize`), and the client has the whole picture at the new size 0.19 to
  0.28 s after asking. xrdp then deactivates and reactivates the connection, and its Deactivate
  All PDU is the share control header alone (6 bytes, without the share id), which IronRDP
  cannot decode. The app treats it as a Deactivate All.
- No PDU the app counts as unhandled (`Session::skipped`) arrived in any test.

### Wrong password

There is no NLA, so a wrong password does not fail the login. The client is connected
(about 70 ms) and sees xrdp's own window, the rest of the screen is never drawn: a 400x400
`Connection Log` at 10,10 (colours `e1e1e1`, `0078d7`, `ffffff`, `b4b4b4`, `000000`) whose last
line, about 1.5 s after the login, is `User does not exist, or could not be authenticated`, with
an `OK` button. The log has the same line. The connection stays open; nothing ended it within
68 s. Clicking `OK` was not tried.

### How a connection ends

- `close()` takes about 2 ms. The client asks for a shutdown, xrdp answers by ending the
  connection and logs, at its default level `INFO`:

  ```
  [ERROR] xrdp_sec_recv: xrdp_mcs_recv failed
  [ERROR] xrdp_rdp_recv: xrdp_sec_recv failed
  [ERROR] libxrdp_process_data: xrdp_rdp_recv failed
  [ERROR] xrdp_process_data_in: xrdp_process_loop failed
  [ERROR] SSL_shutdown: Failure in SSL library (protocol error?)
  [ERROR] SSL: error:0A000126:SSL routines::unexpected eof while reading
  ```

  With `LogLevel=DEBUG` the first line before these is `Received disconnection request`. A
  connection that is cut instead starts with `SSL_read: Failure in SSL library`.
- `docker stop` ends a connected session as a lost connection about 0.3 s after the stop
  began; `docker stop` itself takes about 0.27 s. `docker start` brings xrdp back: it answers
  a connection request 1.2 s after the start began, as `start.sh` gives `xrdp-sesman` a second
  first.
- The log is `/var/log/xrdp.log` (and `/var/log/xrdp-sesman.log`), not `docker logs`; console
  logging did not reach `docker logs`. The files stay across restarts of the container, so the
  tests read only the lines after a count they take first.

## Windows Server VM

`src-tauri/tests/live_rdp.rs` also drives the session against Windows Server 2025 in the Hyper-V
VM `wisp-test-rdp`, which `windows/New-TestVm.ps1` creates and `windows/Remove-TestVm.ps1`
removes. [windows/README.md](windows/README.md) lists what the scripts do, the probe that runs
in the VM's session and what was measured there. With the VM running, the same command runs
those tests too; without it they print why and pass.
