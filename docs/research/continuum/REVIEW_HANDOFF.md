# Continuum encryption review handoff

## Checkout and scope

Repository: https://github.com/Testtube302/Reverse_continuum
Branch: `codex/continuum-handshake-investigation`
Implementation commit: `3ed810e7f0c5324059f15e199ddb049f5db9e95a`.
Later commits may improve this handoff without changing that implementation.

```sh
git fetch origin
git switch --track origin/codex/continuum-handshake-investigation
# If the local branch already exists, switch to it instead.
cargo test
cargo clippy --all-targets
cargo build --release
```

Review this branch before merging. The owner authorized binary investigation for
this task, superseding the earlier no-binary-analysis scope in AGENTS.md; this is
recorded in NOTES.md. Original binaries, memory images, server caches and raw
decompilation are not committed. Default connections still use VIE; Continuum
requires the explicit `--continuum-expander` option.

## What is available from Git alone

- Rust packet cipher, table finalization, negotiation, login integration and
  subsequent key-expansion challenge responses.
- Optional server-owned `scrty1` parser and local command-provider interface.
- Python reference adapter, pinned Unicorn dependency and validation script.
- Public-reference cipher/finalization vectors and synthetic expansion vectors.
- Eight synthetic UDP integration tests, examples, and investigation notes.

The checkout builds and runs the synthetic tests without the proprietary image.
Network sandbox restrictions must permit loopback UDP for transport tests.
The optional asset/live-server tests require their documented environment variables;
their default early returns are not live-test evidence. Clippy has existing warnings.

## What Git alone cannot reproduce

The reference adapter requires a separate 753,664-byte original runtime-code image:
SHA-256 `551c950e264c752c5a0f49903cc9663d4f4c7b2b93a02ca7f34345bea77cad7b`.
It is NOT reconstructed by the Python script and NOT downloadable from this repo.
A freshly captured image may differ due to runtime relocation and will be rejected.
Changing the expected hash alone does not establish compatibility.

On the owner's current Mac, the verified setup is preserved outside Git at:

- `/Users/ryancallicoat/Documents/New project/continuum-local-reference/runtime-code.bin`
- `/Users/ryancallicoat/Documents/New project/continuum-local-reference/expand`
- `/Users/ryancallicoat/Documents/New project/continuum-local-reference/venv`

A Claude session on that same Mac can use the executable wrapper directly. A cloud
or other-machine checkout does not have these files. No permission to upload this
image is implied by this handoff; the prior explicit approval covered local analysis.

Local test command, from this checkout on the owner's Mac:

```sh
cargo run --example continuum_login -- 127.0.0.1:5000 '/Users/ryancallicoat/Documents/New project/continuum-local-reference/expand' 90
```

This example uses a dedicated test account and an unspecified billing identity.
Only run it against an authorized local server. The owner chose to keep verification
local; do not treat this handoff as approval for public-zone testing.

## Evidence and limits

The adapter matched every one of 1,024 independent local server-cache records and
32 synthetic-key regression vectors. The Rust client logged in with Continuum
encryption, entered an arena, obtained map/settings and held a second session for
90 seconds with one security response. No server setting or whitelist was changed.
The release build, 82 active unit tests and eight synthetic UDP tests passed.

These results establish a working local compatibility bridge. They do not establish
standalone Rust expansion, Windows support for the helper, exhaustive coverage of
all seeds, or acceptance by every public zone. The 64-byte billing identity is zero
in the CLI path; the executable checksum remains zero. Strict servers may reject it.

## Production review priorities

1. Decide the supported expansion implementation: independently implement the
   recovered transformations, or design a reproducible user-supplied runtime-image
   setup. A dependency on one Mac's private snapshot is a release blocker.
2. Review `src/net/continuum_expander.rs`: expansion is synchronous on the poll
   path. Bound work under repeated challenges, avoid frame stalls, test timeout/
   crash/output failures, and guarantee cleanup. In particular, the five-second
   process wait does not itself bound a later stdout read if a descendant retains
   the pipe. Define supported executable invocation on Windows as well as Unix.
3. Review the reference adapter's floating-point and exception emulation assumptions
   across hosts and more seeds. Keep reference execution isolated; do not loosen
   the image identity check to make an unknown capture pass.
4. Review login identity, version/feature advertisement, retry behavior and server
   integrity policy. Do not substitute a borrowed client identity or a fabricated
   integrity result. Establish supported-server behavior explicitly.
5. Review transport framing, CRC, length limits, duplicate/reordered negotiation,
   challenge handling and failure paths; extend independent interoperability tests
   where coverage is missing. Keep VIE regression coverage.
6. Validate supported platform builds and fresh-checkout setup, then repeat local
   end-to-end tests without relying on a server cache. Record results and remaining
   limitations before deciding whether to merge the experimental mode.

## Suggested prompt for Claude

> Review `codex/continuum-handshake-investigation` in Testtube302/Reverse_continuum.
> Read docs/research/continuum/REVIEW_HANDOFF.md, the investigation README and NOTES.md.
> Inspect the implementation, identify production blockers, and make justified fixes
> on a separate review branch. Verify tests and supported builds. Treat the local
> reference adapter as an experimental dependency, not completed standalone Rust
> encryption. Preserve existing user changes, do not upload original-code images,
> and keep network verification local. Report what is ready to merge and what remains.
