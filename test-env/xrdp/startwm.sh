#!/bin/sh

xterm -geometry 80x24+40+40 -fn 10x20 -T pattern -e pattern &
exec twm
