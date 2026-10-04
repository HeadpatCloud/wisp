#!/bin/sh
set -eu

# /tmp survives a container restart and Xtightvnc refuses to start on a stale lock
rm -f /tmp/.X1-lock /tmp/.X11-unix/X1

Xtightvnc :1 -geometry 1280x800 -depth 24 -rfbport 5900 -rfbwait 120000 \
    -rfbauth /etc/wisp/passwd -desktop wisp-test-tightvnc \
    -fp /usr/share/fonts/X11/misc/ -co /etc/X11/rgb &
xvnc=$!

until xdpyinfo >/dev/null 2>&1; do
    kill -0 "$xvnc"
    sleep 0.2
done

xterm -geometry 80x24+40+40 -fn 10x20 -T pattern -e pattern &
xterm=$!

sleep 1
kill -0 "$xterm"

wait "$xvnc"
