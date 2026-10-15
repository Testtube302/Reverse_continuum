#!/bin/bash
# Restart the local zone (kills the running server by PID, never by pattern).
for p in $(pgrep -x dotnet); do kill "$p"; done
for i in $(seq 20); do pgrep -x dotnet >/dev/null || break; sleep 0.5; done
cd /opt/zone && setsid dotnet bin/SubspaceServer.dll > /tmp/zone.log 2>&1 < /dev/null &
for i in $(seq 40); do grep -q "Created permanent arena" /tmp/zone.log 2>/dev/null && break; sleep 1; done
grep -c "Created permanent arena" /tmp/zone.log
