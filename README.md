# AIContinuum

A clean-room SubSpace client written in Rust, aiming to look and play like Continuum 0.40
and to join zones running ASSS / Subspace Server .NET.

The core client uses public protocol documentation, open-source servers, and the
MIT-licensed [nullspace](https://github.com/plushmonkey/nullspace) client
(© plushmonkey), credited where used. This branch also contains an experimental
Continuum encryption adapter developed through owner-authorized binary analysis;
it requires a separately supplied local reference image. No original executable,
art, sound, or server key cache is distributed.

**Continuum encryption review:** start with the
[Claude/reviewer handoff](docs/research/continuum/REVIEW_HANDOFF.md).
Working local login is verified, but this branch is not yet a self-contained
production implementation of key expansion.

## Run

```sh
cargo run --release -- --assets /path/to/Continuum      # or set AICONTINUUM_ASSETS
cargo run --release -- --assets DIR --map some.lvl
```

Online (VIE protocol):

```sh
cargo run --release -- --assets DIR --connect 127.0.0.1:5000 --name Pilot [--arena NAME] [--ship 0-7]
cargo run --release --example bot -- 127.0.0.1:5000 BotName 30 0   # headless test pilot
```

Controls: arrows fly (or move the camera while spectating), Shift = afterburner, Ctrl = guns,
Tab = bomb, Shift+Tab = mine, Del = multifire, 1–8 change ship,
S = spectate, Esc quits.

Screenshot mode (for automated checks): `--screenshot out.png --frames 5 [--thrust-ticks N]`.

## Build AIContinuum.exe (from macOS)

```sh
rustup target add x86_64-pc-windows-gnu && brew install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
# -> target/x86_64-pc-windows-gnu/release/AIContinuum.exe
```

## Test

```sh
cargo test                                      # unit tests
AICONTINUUM_ASSETS=/path/to/Continuum cargo test # + decoding real assets
AICONTINUUM_SERVER=127.0.0.1:5000 cargo test -- --test-threads=1  # + live zone tests (~1 min)
```
