# Continuum encryption review handoff

## Checkout and scope

Repository: https://github.com/Testtube302/Reverse_continuum
Current compatibility branch: `codex/continuum-ack-compatibility`
Original implementation commit: `3ed810e7f0c5324059f15e199ddb049f5db9e95a`.
Later commits may improve this handoff without changing that implementation.

```sh
git fetch origin
git switch --track origin/codex/continuum-ack-compatibility
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

> Review `codex/continuum-ack-compatibility` in Testtube302/Reverse_continuum.
> Read docs/research/continuum/REVIEW_HANDOFF.md, the investigation README and NOTES.md.
> Inspect the implementation, identify production blockers, and make justified fixes
> on a separate review branch. Verify tests and supported builds. Treat the local
> reference adapter as an experimental dependency, not completed standalone Rust
> encryption. Preserve existing user changes, do not upload original-code images,
> and keep network verification local. Report what is ready to merge and what remains.

### Connection trace for the unresolved public-zone disconnect

Set `AICONTINUUM_TRACE=1` to write `[NetTrace +Nms]` metadata to stderr.
The desktop Play launcher enables this and appends stderr to
`connection-diagnostics.log`. Capture covers acknowledgement IDs, nested packet
kinds, send results, cipher rejections and map-transfer progress, without raw
payloads, credentials or key material. It stops at 8,192 events per connection
and prints a limit marker; nested metadata is limited to eight levels.
The older session diagnostics also retain recent server arena/warning/error text.

Owner-run Trench Wars attempts still accept login and begin map transfer before
an unexplained core disconnect. The indexed map request and padded expansion
challenge corrections did not resolve the observed attempt. Do not infer a
confirmed cause from passing synthetic tests. Use the next owner-run trace to
identify the actual divergence before changing protocol behavior again.

### New-user questionnaire (2026-10-16)

On `codex/continuum-ack-compatibility`, login diagnostics now include the
questionnaire flag at offset 19 of S2C 0x0a. Successful login plus a nonzero flag,
or login result 0x10, produces an explicit questionnaire-required error before
arena entry. Complete the form in the original client with the **same account and
server**, then retry. AIContinuum has no form submission UI yet and never sends
fabricated demographics. Regressions cover eight response variants and preserve
the error if subsequent arena packets arrive in the same update.

The owner successfully connected on Windows and submitted the form on a different
account. This is not proof that registration explains the AIContinuum account's
disconnect: old traces omitted the flag. The next owner-run attempt can distinguish
this condition. Sources: public nullspace `Connection.cpp` PasswordResponse and
SS.NET `S2C_LoginResponse.cs` / `IAuth.cs`, with revisions recorded in NOTES.md.

### Arena-entry layout correction (2026-10-16)

The owner's 11:16:20 trace explicitly says questionnaire **not requested**, yet
disconnects at +733 ms after successful login during map transfer. Questionnaire
handling is not the blocker in that attempt.

New local captures of original 0.40 confirm its 27-byte arena-entry packet, while
AIContinuum sent the 26-byte VIE form. Continuum now appends byte 26 with optional
graphics disabled; VIE stays unchanged. Tests reproduce the previous mismatch,
verify both layouts, and pass local encrypted login/map entry. The reference
captures also verified byte-for-byte encryption agreement for outgoing original
client packets. Captures remain local, outside Git. Public-server success is still
unverified; do not represent this packet correction as a confirmed Trench Wars fix.

### Successful original-client comparison and remaining disconnect (2026-10-16)

Original 0.40 on Wine 5.5 successfully entered Trench Wars using the currently
selected saved account over direct UDP from the reference host. The previous
UDP-over-SSH comparison added a transport variable and failed during download.
All 2,313 captured encrypted datagrams decoded; every outgoing original packet
re-encrypted byte-for-byte. Captures and credentials remain outside Git.

Two explicitly approved AIContinuum public tests still failed after login during
map download: the 27-byte arena correction alone, then a scratch build with
immediate individual ACKs. The ACK experiment is not included. Neither trace
received an expansion challenge or requested the questionnaire. Public tests are
bounded by individual approvals; both approvals were consumed.

Further confirmed layout corrections in this branch:

- Core 0x13 includes a zero u16 version before the seed and 20 words (88 bytes).
  Verified all reference response words against the expansion helper; this fixes
  a separate bug, not the immediate disconnect seen without a challenge.
- Login offset 89 reports the actual destination IPv4 in little-endian numeric
  order. The previous zero can be classified as SOCKS5 by SS.NET; Trench Wars'
  treatment of the field is unknown. The bound port, zero redirect login ID and
  reserved word retain their protocol offsets. No client identity is borrowed.

The cipher comparison establishes agreement for captured packets, not complete
server acceptance. Login sequencing/timing, host/network differences and billing
identity remain possible factors. Do not label the build production-ready or the
Trench Wars issue resolved without a successful AIContinuum arena-entry test.

### Login-address result and 24-bit map support (2026-10-16)

An additional explicitly approved public test of b80e9f4 downloaded the complete
Trench Wars map instead of receiving the former immediate server disconnect.
The next failure was local: `bmp: unsupported bit depth 24`. The client itself
then disconnected. That approval is consumed; arena readiness remains unverified.

The map loader now accepts 24-bit BI_RGB embedded tilesets without quantization,
while preserving indexed/RLE8 support. The actual downloaded map parses offline
and all 194,560 RGBA bytes match independent Pillow decoding. Regression fixtures
are synthetic; the downloaded map remains outside Git. Full local suite: 89 unit
and 14 synthetic protocol tests pass, one ignored; lint/release builds succeed.
The desktop build includes these changes with a rollback backup. The next
explicitly authorized public test must verify map readiness and connection hold.

### Latest result: arena entry succeeds, then SSC reports a restriction

The approved f4edf72 test entered the arena at +354 ms with map/settings loaded,
then received an SSC network-ban notice and disconnected at +15.549 seconds.
The server reports BanId #1 with a 2014 expiry. That inconsistent date does not
prove a current user-account ban; the matching rule (account, route, client
identity or another server check) is unknown. No expansion/security challenge
preceded this disconnect. The 90-second hold did not complete.

The UI now retains this server notice instead of only showing a generic close.
The synthetic regression excludes player chat from this diagnostic. Current
suite: 89 unit +15 synthetic UDP tests pass, one ignored; clippy/release pass.
Repo and desktop include all measured fixes and rollback remains available.

Encrypted login, full map download and arena readiness are now demonstrated on
Trench Wars. Sustained acceptance remains blocked by its explicit restriction.
The reported BanId/matching rule is unknown. Operator feedback could explain it,
but local protocol investigation can continue without that feedback. Do not work around it with borrowed
identities, fabricated integrity values or alternate network routes. No further
public-test approval remains outstanding.

### Follow-up: missing client identification remains a compatibility hypothesis

The owner reports the same account works in original Continuum on the same
internet connection as the Mac. This makes a simple account/public-IP ban less
likely; it does not prove which client field triggers the rejection.

Local-only comparison of the existing reference captures found:

- Original reference traffic continues through 83.462 seconds, with no extra
  key-expansion challenge or executable-security response in that interval.
- Its login contains a populated 64-byte billing field; AIContinuum supplies zeros.
- Two local reference sessions retain the same account and four-byte machine field,
  but differ in all 64 billing-field bytes. The local/public reference comparison
  also retains the same machine field with a different account and billing bytes.
  Only equality/counts were printed; identity values and credentials stay private.
- AIContinuum currently derives its machine field from the account name, which
  does not model the reference's account-independent machine field.

SS.NET BillingUdp forwards the machine field and opaque 64-byte field to billing;
its source does not supply their generation algorithm. The public TCP billing
specification also distinguishes macid and contid:
https://wiki.minegoboom.com/index.php/TCP_Billing_Protocol
This is evidence about protocol roles, not evidence of SSC's matching rule.

Next bounded work: trace construction of the billing field in the isolated
reference client against the local server, establish its inputs and session
variation, and validate a legitimate implementation locally. Do not infer that
any nonzero bytes suffice, replay captured identities, or claim a proven root
cause from the missing field alone. Missing transport responses are a weaker
current hypothesis because no additional challenge precedes the rejection.
