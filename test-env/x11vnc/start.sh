#!/bin/sh
set -eu

# /tmp survives a container restart and Xvfb refuses to start on a stale lock
rm -f /tmp/.X1-lock /tmp/.X11-unix/X1

Xvfb :1 -screen 0 1280x800x24 &
xvfb=$!

until xdpyinfo >/dev/null 2>&1; do
    kill -0 "$xvfb"
    sleep 0.2
done

xterm -geometry 80x24+40+40 -fn 10x20 -T pattern -e pattern &
xterm=$!

sleep 1
kill -0 "$xterm"

exec x11vnc -display :1 -rfbport 5900 -rfbauth /etc/wisp/passwd \
    -desktop wisp-test-x11vnc -forever -shared
