# VNC test servers

Seven VNC servers in Docker for the live VNC tests. Every port is published on `127.0.0.1`
only. The passwords are fixed and written down here, so none of this may ever be reachable
from a network.

## Start and stop

```bash
docker compose -f test-env/docker-compose.yml up -d --build
docker compose -f test-env/docker-compose.yml down
docker compose -f test-env/docker-compose.yml restart x11vnc
```

The containers are named `wisp-test-<service>`. Nothing is mounted from the host; read things
back with `docker exec` or `docker cp`.

After a start or restart five servers send their banner at once. `x11vnc` needs about 1.5 s,
and until then Docker accepts the TCP connection and closes it without a banner. Wait for the
banner, not for the port.

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

The versions are whatever `debian:stable-slim` installs when the images are built.

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
  Control down, the next client's `a` and `B` arrived as `\x01\x02`. A restart clears it.

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
