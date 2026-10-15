# Task: team, private and channel chat

## Why

The client sends every message as public chat (`Session::send_public_chat`, `src/net/session.rs`).
Continuum players address messages with prefixes, and macros (`macro.dat`, sent by
`MacroN` key definitions in `src/main.rs`) use the same prefixes. Today a team or private
macro goes to everyone.

## What to build

1. One way to send a typed line: `Session::send_chat(&mut self, text: &str)`. It picks the chat
   type from the prefix and strips the prefix. Continuum's conventions (**verify each one** against
   nullspace's chat code and SS.NET's `Chat` module before relying on it):
   - `//text` or `'text` — team (your frequency)
   - `"text` — the selected player's frequency (needs a target pid; take it as an argument)
   - `/text` — private to the selected player (target pid)
   - `:name:text` — private to a named player, in this zone or elsewhere on the network
   - `;text` or `;N;text` — chat channel
   - `?command` and anything else — public. The zone interprets `?` commands itself.
2. Use it for both typed messages (the Enter handler in `src/main.rs`) and macros.
3. Received chat already arrives as `Chat { kind, pid, text }` (S2C 0x07). Check the HUD shows the
   types in the original's colours and formats (`hud.rs`). Record what you find in NOTES.md.

The packet is C2S `0x06`: `type u8, sound u8, target pid u16, text NUL-terminated` (see
`send_public_chat`). Get the type numbers from the sources above; don't guess them.

## Done when

- Unit tests cover prefix parsing: each prefix, an empty message, a lone `:`, `:name:` with no
  text, and a long line truncated like today's 240 bytes.
- `cargo test` and `cargo clippy --all-targets` are clean.
- Live check against `47.207.117.75:5000` (or a local Subspace Server .NET), using two clients.
  `examples/bot.rs` can be the second one. A team message reaches only teammates, a `:name:`
  message reaches only that player, and the zone's chat log shows the right types.
  `--press Macro1@300`, plus a `macro.dat` holding `1 = //hello team`, exercises the macro path.
- NOTES.md gains a short "Chat types" entry with sources.

## Out of scope

Don't change how the chat area is drawn, beyond colours that are clearly wrong. Pixel-exact
chat rendering is reference-rig work.
