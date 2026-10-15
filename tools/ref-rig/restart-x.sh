#!/bin/bash
# Restart the virtual display at the given depth (default 16) and stop any Wine programs.
DEPTH=${1:-16}
wineserver -k 2>/dev/null; sleep 1
pkill -9 -x Xvfb; sleep 1
setsid Xvfb :99 -screen 0 1024x768x$DEPTH -nolisten tcp >/tmp/xvfb.log 2>&1 < /dev/null &
for i in $(seq 20); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.5; done
xdpyinfo | grep -E "depth of root"
