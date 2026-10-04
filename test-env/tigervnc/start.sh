#!/bin/sh
set -eu

# /tmp survives a container restart and Xtigervnc refuses to start on a stale lock
rm -f /tmp/.X1-lock /tmp/.X11-unix/X1

# BlacklistThreshold: every test client arrives from the same Docker gateway address, and
# the default locks that address out after 5 failed or abandoned handshakes
Xtigervnc :1 -geometry 1280x800 -depth 24 -rfbport 5900 \
    -PasswordFile /etc/wisp/passwd \
    -X509Cert /etc/wisp/cert.pem -X509Key /etc/wisp/key.pem \
    -PAMService tigervnc -PlainUsers wisp \
    -BlacklistThreshold 1000000 \
    "$@" &
xvnc=$!

until xdpyinfo >/dev/null 2>&1; do
    kill -0 "$xvnc"
    sleep 0.2
done

twm &
twm=$!
xterm -geometry 80x24+40+40 -fn 10x20 -T pattern -e pattern &
xterm=$!

sleep 1
kill -0 "$twm" "$xterm"

wait "$xvnc"
