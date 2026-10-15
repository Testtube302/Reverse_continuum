# AGENTS.md — working on AIContinuum

AIContinuum is a clean-room Rust client for SubSpace / Continuum (an abandoned 1990s
space shooter; the community still runs zones). It joins zones over the SubSpace UDP
protocol, plays the game, and shows a startup screen that copies the original
Continuum 0.40 launcher pixel for pixel.

**Read `NOTES.md` first.** It records every measured fact, file format and protocol
detail, round by round. Add your new facts there, in the same terse style.

## Ground rules (not negotiable)

- **Clean room.** Never disassemble, decompile, unpack or patch `Continuum.exe`, its DLLs'
  code, or any `enc_cont` / encryption binary. Reading *data* from the install at run
  time (bitmaps, dialog/menu templates, `.dat` text files) is fine.
- **No anti-cheat or checksum spoofing.** Don't impersonate Continuum or SubSpace 1.34 to
  get into zones that would otherwise refuse us.
- **Sources you may use:** public protocol docs; MIT/GPL/LGPL open-source code such as
  [nullspace](https://github.com/plushmonkey/nullspace) (MIT),
  [Subspace Server .NET](https://github.com/gigamon-dev/SubspaceServer) and
  [sharvil/subspace](https://github.com/sharvil/subspace). Cite what you used in NOTES.md.
- **Never commit Continuum's own files** (`graphics/`, `sound/`, `*.dll`, `*.dat`, maps).
  They are loaded at run time from the player's install. `third_party/wine-fonts` holds
  converted Wine fonts (LGPL, see its README); that's the only third-party data allowed.

## Build and test

```sh
cargo build --release                                  # Mac/Linux binary: target/release/AIContinuum
cargo build --release --target x86_64-pc-windows-gnu   # Windows .exe (needs mingw-w64)
cargo test                                             # unit tests; must pass before you commit
cargo clippy --all-targets                             # don't add warnings (some older ones exist)
```

Tests that need more than the repo skip themselves unless configured:

- `AICONTINUUM_ASSETS=/path/to/Continuum` — a Continuum 0.40 install folder (one that contains
  `graphics/`). It enables `tests/real_assets.rs` and is needed to run the client at all.
  Get the installer from getcontinuum.com (`Continuum040Setup.zip`, unpack with `innoextract`).
- `AICONTINUUM_SERVER=127.0.0.1:5000` — a zone for `tests/live_server.rs`. Run Subspace Server
  .NET locally, or use the public test zone at `47.207.117.75:5000` ("Testtube Test Zone").
- `cargo test --lib -- --ignored live_directory` talks to the live directory servers.

## Running it

```sh
AICONTINUUM_ASSETS=~/Continuum target/release/AIContinuum            # startup screen
target/release/AIContinuum --assets ~/Continuum --offline            # fly alone
target/release/AIContinuum --assets ~/Continuum --connect 47.207.117.75:5000 --name "Some Name" --password pw
```

Headless test aids (use them to check your work; see `parse_args` in `src/main.rs`):
`--screenshot out.png --frames N` saves exact pixels after N frames; on the startup screen
`--launcher`, `--open <dialog|menu:N>`, `--mouse x,y`, `--clicks "x,y;x,y"`,
`--type 'text\n'`; in the game `--press Action@frame` (any keyboard.dat action, e.g. `Macro1@300`).

## Code map

- `src/net/` — `transport.rs` (core protocol, reliable delivery, VIE encryption in `vie.rs`),
  `session.rs` (game packets, players, chat, login).
- `src/game.rs`, `src/sim.rs`, `src/weapons.rs`, `src/soccer.rs` — simulation.
- `src/main.rs` (frontend/game loop, macroquad), `src/hud.rs` — drawing and input.
- Startup screen: `src/launcher.rs` (screen logic), `src/dialog.rs` (Win32 dialog templates drawn
  like Wine), `src/menubar.rs`, `src/skin.rs`, `src/keys.rs` (keyboard.dat / macro.dat),
  `src/menu.rs` (frontend glue).
- `tools/ref-rig/` — scripts for the reference rig (original Continuum under Wine in Docker),
  used for pixel comparisons. It runs on a private host; you won't have it. **Don't take on
  pixel-matching work without it** — say so instead of guessing.

## Style

- Match the surrounding code: small functions, comments only where something isn't obvious,
  and measured constants get a one-line note saying where they came from.
- Keep tests next to the code (`#[cfg(test)] mod tests`). Add one for every new parser or rule.
- Work on a branch; don't rewrite `src/launcher.rs`, `src/dialog.rs` or `src/main.rs` wholesale.
  Other work happens in those files at the same time.
- Commit messages: a short imperative subject, a blank line, then what changed and why.
  Don't commit build output or anything from a Continuum install.

## Task briefs

Scoped tasks live in `docs/tasks/`. Each one says what's done, what's wanted, and how to verify.
