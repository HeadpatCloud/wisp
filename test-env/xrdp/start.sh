#!/bin/sh
set -eu

# /tmp survives a container restart, and with the X lock of the session that was running the
# next session would get display :11 instead of :10
rm -f /tmp/.X1*-lock /tmp/.X11-unix/X1*

xrdp-sesman --nodaemon &
sesman=$!

sleep 1
kill -0 "$sesman"

exec xrdp --nodaemon
