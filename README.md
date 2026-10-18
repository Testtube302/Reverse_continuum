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

One installation can use Continuum, VIE, or unencrypted transport. On macOS, put the
existing private helper at `.continuum/expand` beside `AIContinuum`; the executable
finds it automatically (the `--continuum-expander PATH` override still works).
The server explicitly chooses its transport during negotiation. Once Continuum
keys are exchanged, the client will not downgrade to plaintext. Without the
helper, startup uses the existing VIE/plaintext path. The private runtime image
and helper dependencies are not included in this repository; this is not a
standalone key-expansion implementation or a portable distribution installer.

Chat: `'message` or `//message` sends to your team; `/message` sends privately to
the selected player; `:name:message` addresses a player by name; `"message` sends
to the selected player's team. `;message` and `;N;message` use chat channels when
supported/configured by the server. Typed messages and macros use the same routing.
Malformed private input produces a local notice rather than a public message.

Arena switching: Esc, then A opens the arena list. Up/Down selects an arena;
Enter joins it as spectator. You can also type `?go NAME`, `?go 0`, or `?go` for
the server's default public arena. These preserve the zone connection.

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
