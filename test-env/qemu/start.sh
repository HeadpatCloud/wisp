#!/bin/sh
set -eu

rm -f /run/qmp.sock

qemu-system-x86_64 -name wisp-test-qemu -display none -vnc 0.0.0.0:0,password=on \
    -m 64 -nic none -qmp unix:/run/qmp.sock,server=on,wait=off &
qemu=$!

until [ -S /run/qmp.sock ]; do
    kill -0 "$qemu"
    sleep 0.2
done

# shut-none: QEMU drops commands still queued when the QMP client half-closes
reply=$(printf '%s\n' \
    '{"execute":"qmp_capabilities"}' \
    '{"execute":"set_password","arguments":{"protocol":"vnc","password":"wisptest"}}' \
    | socat -t 2 - UNIX-CONNECT:/run/qmp.sock,shut-none)
echo "$reply"
[ "$(echo "$reply" | grep -c '"return"')" -eq 2 ]

wait "$qemu"
