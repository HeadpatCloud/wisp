#!/bin/sh

# text cursor hidden: its shape follows keyboard focus and would change the picture
printf '\033[?25l'
printf 'WISP VNC TEST PATTERN\n\n'
printf '0123456789\n'
printf 'ABCDEFGHIJKLMNOPQRSTUVWXYZ\n'
printf 'abcdefghijklmnopqrstuvwxyz\n\n'
for row in 1 2 3; do
    for colour in 0 1 2 3 4 5 6 7; do
        printf '\033[4%sm        ' "$colour"
    done
    printf '\033[0m\n'
done
exec sleep infinity
