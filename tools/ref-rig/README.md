# Reference rig: original Continuum vs AIContinuum, pixel for pixel

Two throwaway containers on any x86_64 Linux Docker host (used: misc-vm). No network exposure:
`aic-ref` runs with `--network none`; `aic-wine5` shares its loopback.

- `Dockerfile.zone` → image `aic-ref`: Ubuntu 24.04, Subspace Server .NET (zone on 127.0.0.1:5000),
  Wine 9 + Xvfb + ImageMagick. Runs **AIContinuum.exe** (`--fullscreen` gives an exact 1024×768).
- `Dockerfile.wine5` → image `aic-wine5`: Ubuntu 18.04 + **wine-staging 5.5** (newer Wine hangs the
  original client right after "Play"), display :98 at 16 bpp. Runs the **original Continuum.exe**.

Both need `cont-app.tgz` (the `app/` folder from the official installer, unpacked with
`innoextract`) next to the Dockerfiles. It is not in this repo.

```sh
sudo docker build -f Dockerfile.zone  -t aic-ref .
sudo docker build -f Dockerfile.wine5 -t aic-wine5 .
sudo docker run -d --name aic-ref --network none --init --shm-size=512m aic-ref
sudo docker exec aic-ref bash -c 'setsid Xvfb :99 -screen 0 1024x768x16 -nolisten tcp & cd /opt/zone && setsid dotnet bin/SubspaceServer.dll >/tmp/zone.log 2>&1 &'
sudo docker run -d --name aic-wine5 --network container:aic-ref --init --shm-size=512m aic-wine5
sudo docker cp join5.sh aic-wine5:/usr/local/bin/ && sudo docker exec aic-wine5 chmod +x /usr/local/bin/join5.sh
sudo docker exec -d aic-wine5 /usr/local/bin/join5.sh      # original joins "AIC Local"
sudo docker exec aic-wine5 import -window root /work/ref.png
```

Then (in `aic-wine5`) `xdotool key Escape` / `xdotool key 1` etc. to change the scene, and in
`aic-ref`: `wine /opt/AIContinuum.exe --fullscreen --assets Z:/opt/cont --connect 127.0.0.1:5000
--screenshot Z:/work/ours.png --frames 200 [--menu] [--ship N]`.

Compare a region (both quantised to the 16-bit display):
`compare -metric AE -fuzz 4% <(convert ref.png -crop R -depth 5 png:-) ...` — see NOTES.md.

Matched so far at 1024×768 (0 differing pixels): Esc menu, radar frame + sector grid, chat line,
spectator X-radar icon; status panel / icon columns / energy bar positions verified.

Animation timing: `ring.sh` (copy into a container, run with `setsid`, `touch /work/stop` to end)
keeps the last ~8 s of 400×400 centre captures named by nanosecond timestamp (~14 ms apart for the
original). Convert to PNG, fetch, then `match_anim.py SHEET FW FH DIR [cx cy search]` prints the
best-matching sprite frame per capture, from which per-frame durations come out by regression.
`fetch-cap.sh CONTAINER NAME` converts a ring capture to PNG and unpacks it under
`~/aicontinuum-ref/NAME/cp`. Don't `pkill -f AIContinuum.exe` inside an `rx.sh` command: it
matches the shell running that command.
