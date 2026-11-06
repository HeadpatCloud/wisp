#!/bin/sh

export XDG_RUNTIME_DIR=/tmp/xdg-$(id -u)
mkdir -p -m 700 "$XDG_RUNTIME_DIR"
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
pipewire > /tmp/pipewire.log 2>&1 &
sleep 0.5
wireplumber > /tmp/wireplumber.log 2>&1 &
pipewire-pulse > /tmp/pipewire-pulse.log 2>&1 &
sleep 1
/usr/libexec/pipewire-module-xrdp/load_pw_modules.sh > /tmp/load_pw_modules.log 2>&1 &
xterm -geometry 80x24+40+40 -fn 10x20 -T pattern -e pattern &
exec twm
