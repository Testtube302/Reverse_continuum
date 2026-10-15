#!/bin/bash
# Wine 5.5 rig: start display, init prefix once, launch Continuum windowed, join "AIC Local".
if ! xdpyinfo >/dev/null 2>&1; then
  setsid Xvfb :98 -screen 0 1024x768x16 -nolisten tcp >/tmp/xvfb.log 2>&1 < /dev/null &
  for i in $(seq 20); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.5; done
fi
if [ ! -d /root/.wine ]; then
  WINEDLLOVERRIDES="mscoree,mshtml=" wineboot -i >/dev/null 2>&1
  (cd /opt/cont && wine regedit /S Continuum.reg)
fi
wineserver -k 2>/dev/null; sleep 1
cd /opt/cont
WINEDEBUG=${WINEDBG:--all} setsid wine explorer /desktop=cont,1024x768 Continuum.exe > /tmp/cont.log 2>&1 < /dev/null &
sleep 15
xdotool mousemove 668 481 click 1; sleep 1          # close tip of the day
xdotool mousemove 560 278 click 1; sleep 0.5         # select AIC Local
xdotool mousemove 590 629 click 1; sleep 2           # Play
xdotool mousemove 507 397 click 1; xdotool type --delay 40 refpass; sleep 0.5
xdotool key Return
sleep 4
xdotool mousemove 345 422 click 1; sleep 0.3          # "Don't ask about this zone again"
xdotool mousemove 557 424 click 1; sleep 3            # Zone warning: Yes
xdotool mousemove 547 423 click 1                     # Updated news: No
