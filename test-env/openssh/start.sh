#!/bin/sh
set -eu

socat TCP-LISTEN:7000,bind=127.0.0.1,reuseaddr,fork PIPE &
echo=$!
# -U: only from sleep to the socket, so what a client sends is never read
socat -U TCP-LISTEN:7001,bind=127.0.0.1,reuseaddr,fork EXEC:'sleep 600' &
sink=$!
/usr/sbin/sshd -D -e -f /etc/wisp/sshd_rekey.conf &
rekey=$!

sleep 1
kill -0 "$echo" "$sink" "$rekey"

exec /usr/sbin/sshd -D -e
