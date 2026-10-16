# Continuum transport investigation — 2026-10-15

## Status

This branch contains the packet cipher, table finalization, and an opt-in transport
API: `Transport::connect_continuum(server, expanded_key_lookup)`. The lookup receives
the handshake's key2 and must return its already-expanded 20-word table promptly.
The transport finalizes that table with key1, sends the plaintext acknowledgement,
waits for the encrypted server acknowledgement, then transports encrypted packets.
Returning None fails explicitly; no dummy table or VIE fallback is substituted.

**Continuum encrypted login now works in the controlled test using a local reference
adapter.** `Session::connect_continuum` sends the 165-byte login and the CLI accepts
`--continuum-expander /path/to/helper`. The adapter expands arbitrary key2 values
locally by emulating a pinned original-code image. It is **not a standalone Rust
rewrite** of expansion, and the original-code image is not bundled or committed.
Default connections still use VIE. A tested [billing identity codec](IDENTITY.md)
is available, but native identity collection and launcher integration remain
unimplemented. Executable integrity responses are also unimplemented;
compatibility with all public zones is not established.

The public nullspace cipher was ported under MIT, with its license included here.
REA/Ghidra observations were used to corroborate the transport structure; no
proprietary binary or decompiled source is committed.

## Reference identity and tools

- Official archive: https://www.getcontinuum.com/downloads/continuum/Continuum040Setup.zip
- Archive SHA-256: `d36fe8ab648633f2e131d0c618946353b7d533f9b384ae492fb7378dd58e1f46`
- Extracted Continuum.exe SHA-256: `b059db20b82ddbd41fc47b2122a31a1f1b06c88e81c20649c5dc6f9f2501c16c`
- The executable in the existing `aic-wine5` reference container has exactly that hash.
- REA CLI 6.5.0, Node 24.22.0, Ghidra 12.1.4, Homebrew OpenJDK 21.0.10.
- Ghidra's native Apple Silicon decompiler was built from its shipped source with
  `make -j4 ghidra_opt ARCH_TYPE='-arch arm64' OSDIR=mac_arm_64 ADDITIONAL_FLAGS='-mmacosx-version-min=11.0'`.
- REA is used through its CLI; no global agent registration was added.
- Local tool paths: `/private/tmp/aic-rea-runtime/node_modules/rea-agents/scripts/rea.mjs`,
  `/private/tmp/aic-rea-runtime/node_modules/node/bin/node`,
  `/private/tmp/aic-ghidra/ghidra_12.1.4_PUBLIC`. These are temporary, not installed dependencies.

## Observed handshake

`handshake-observation.json` records UDP payloads from the official client on the
isolated reference rig's loopback interface. Server port: 5000. Both encryption
modules were enabled, with `EncryptionContNative` before `EncryptionVIE`.

| Direction | Payload | Observation |
| --- | --- | --- |
| Client → server | `00 01 bc 2f d9 9a 11 00` | Eight-byte Continuum negotiation request |
| Server → client | `00 10 60 c0 6a 8e b1 0c cd 83 01 00` | Twelve-byte Continuum response |
| Client → server | `00 11 60 c0 6a 8e 01 00` | Echoes the response's first 32-bit word |

The server log subsequently recorded login, arena entry, and a security response
from the **official** client. This does not establish equivalent behavior by our client.
The capture filter recorded only core types 01, 02, 10, 11, 12, and 13; it did not
record the later encrypted packets. This is not a complete connection transcript.

A separate VIE probe to the same isolated server, using the same initial 32-bit word:

| Direction | Payload |
| --- | --- |
| Client → server | `00 01 bc 2f d9 9a 01 00` |
| Server → client | `00 02 44 d0 26 65` |

The VIE reply contains the negation of the initial word. The Continuum exchange is
structurally different. Never select a cipher solely from the executable's name:
SS.NET's VIE module can also accept the Continuum request type as a fallback.

## REA findings

The original executable's default Ghidra analysis recovered only eight procedures
and eleven defined strings. This limited inventory is consistent with protection
or packing; it is not a recovered implementation. See `client-analysis-summary.json`.

For the local SS.NET 4.0.0 macOS ARM64 native module:
SHA-256 `f85eeadfdac16c061f176903becbd89a3e7bebef1c277aa6679875d95ee5ff88`.
REA's managed metadata inspection of `SS.EncryptionCont.dll` established the adapter
layout and native delegation. Ghidra then identified these native-module offsets:

| Offset | Observed role |
| --- | --- |
| `0x74020` | Exported `_load`; registers callbacks |
| `0x74600` | Packet encryption wrapper: CRC prefix and escape handling |
| `0x74940` | Packet decryption wrapper: unescape and CRC validation |
| `0x759f0` | Connection initialization dispatch (`00 01` / `00 11`) |
| `0x76d80` | Block and tail encryption transform |
| `0x77000` | Inverse transform |

These offsets apply only to that exact module build. The observed wrapping-add,
rotation, block-chaining and tail structure agrees with the public implementation.
No direct execution of private internal functions was used to generate test vectors.

## Public implementation and remaining dependency

Reference revision: [nullspace 7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4](https://github.com/plushmonkey/nullspace/tree/7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4).

- [Crypt.cpp](https://github.com/plushmonkey/nullspace/blob/7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4/src/null/net/security/Crypt.cpp): packet transforms and finalization.
- [Connection.cpp](https://github.com/plushmonkey/nullspace/blob/7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4/src/null/net/Connection.cpp): treats the two response words as `key1` and `key2`, acknowledges `key1`, requests expansion of `key2`, then finalizes with `key1`.
- [README](https://github.com/plushmonkey/nullspace/blob/7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4/README.md): expansion and checksum solving depend on a private network service.
- [Checksum.cpp](https://github.com/plushmonkey/nullspace/blob/7ebdbc53a052fa62ff246b5f6a8b3347c395e5a4/src/null/net/security/Checksum.cpp): CRC-8 lookup table, reproduced by reflected polynomial `0x8c` here.

`ContinuumCipher::from_final_session_table` takes the **final** table, after both
expansion and finalization. `from_expanded_table` and the transport callback take
the pre-finalization table and finalize it with key1. Neither derives that table
from key2. Passing a handshake word or a dummy table cannot establish interoperability.

The finalization implementation is derived from the RSA Data Security, Inc. MD5
Message Digest Algorithm as modified by nullspace. Its copyright notice is in
`LICENSE-md5`. It uses modified initial/round constants and the intermediate state
after one 64-byte compression, not a standard MD5 digest: the reference updates
80 bytes but never calls MD5Final. The remaining 16 bytes remain buffered before
all 20 table words are mixed with the intermediate state and key1.

`decrypt` rejects bad CRCs and noncanonical escape prefixes. Transport now uses
`decrypt_compatible`, which additionally accepts the public nullspace receive
variant: CRC calculated over repetitions of the first plaintext byte. Other CRC
values are rejected. The inspected local native module uses the standard CRC;
the alternate behavior is tested against independently generated public C++ vectors.

## Validation

- 85 synthetic reference vectors: zero, all-ones, and mixed final tables; block/tail
  boundaries; maximum plaintext size; all four escape cases (`ff`, `00 01`, `00 10`, `00 11`).
- Vectors generated by compiling the pinned public C++ block transform and CRC,
  independently of the Rust port. The reference harness defines zero-bit rotation
  explicitly to avoid C++ shift-by-32 undefined behavior.
- 32 additional C++ reference vectors verify table finalization with boundary
  key1 values, zero/all-ones tables, and mixed synthetic tables.
- Six loopback tests cover negotiation, retransmission, duplicate/stale challenges,
  encrypted acknowledgement, reliable delivery, 522-byte wire packets, missing/wrong
  expansion, malformed responses, and invalid acknowledgements. The simulated server
  uses a precomputed C++ final table, independently of Rust finalization.
- The diagnostic probe against the existing local SS.NET server received a Continuum
  challenge and exited 2 with the explicit missing-expansion diagnostic. It sent no
  login packet or credentials. This verifies negotiation reachability, not real login.
- Every plaintext length 1–520 round-trips in Rust.
- Malformed lengths, escape prefixes, and a corrupted CRC are rejected.
- The 520-byte plaintext cap follows the existing client's packet budget. Ciphertext
  may require 522 bytes. The transport now uses the selected cipher's wire budget:
  520 bytes for VIE and 522 for Continuum, before any decryption.
- This is public-reference compatibility testing, **not** an official-client
  plaintext/ciphertext comparison using a real session table.

Regenerate vectors from a checkout at the pinned revision:

```sh
python3 tools/protocol/generate_continuum_vectors.py /path/to/nullspace > docs/research/continuum/cipher-vectors.txt
python3 tools/protocol/generate_continuum_finalization_vectors.py /path/to/nullspace > docs/research/continuum/finalization-vectors.txt
cargo test net::continuum
cargo test --test continuum_transport
```

Diagnostic probe (requires a controlled zone; no credentials or table supplied):

```sh
cargo run --example continuum_probe -- 127.0.0.1:5000
```

Exit 2 confirms that a supported Continuum challenge reached the key-expansion
lookup and no table was supplied. Other failures exit 1. This example does not
attempt game login.

## Next unresolved work

The encrypted handshake, Continuum login format, and a 90-second local session have
now been verified using the reference adapter described below. Remaining work is
production packaging or standalone key expansion, cross-platform validation, and
server-specific billing/integrity compatibility. See [the review handoff](REVIEW_HANDOFF.md)
for checkout instructions and concrete review priorities. Public-zone testing was
declined; no public-zone compatibility result is claimed.

## Controlled-server cache verification

`ExpansionCache` reads the public nullspace `scrty1` layout from revision
[5743bb7](https://github.com/plushmonkey/nullspace/blob/5743bb7/src/net/Crypt.cpp):
a four-byte version header (`28 00 01 00`), followed by two banks of 1,024
84-byte records. Only the second bank supplies expansions. Each record contains
key2 followed by twenty little-endian words. Malformed versions, lengths, and
conflicting duplicate entries are rejected. Unknown keys fail explicitly.

The optional second argument to `continuum_probe` is a server-owned cache path:

```sh
cargo run --example continuum_probe -- 127.0.0.1:5000 /path/to/server/scrty1
```

On 2026-10-15 this completed the encrypted handshake against the existing local
SS.NET 4.0.0 native Continuum module and verified its encrypted acknowledgement.
No login packet was sent. The cache is neither committed nor bundled; it only
works when the challenged key is present and does not solve arbitrary expansion.

With the user's explicit approval, the reference process's read-only executable
range `0x400000..0x4b8000` was copied locally for analysis (753,664 bytes, SHA-256
`551c950e264c752c5a0f49903cc9663d4f4c7b2b93a02ca7f34345bea77cad7b`). Writable
memory was excluded. REA/Ghidra raw x86 analysis with 552 prologue candidates
recovered 1,561 candidate procedures and 954 defined strings. These are analysis
observations, not verified function boundaries. The code copy and decompilation
remain outside the repository.

## Working local reference adapter and login

The key expansion path was identified at `0x457d60` followed by `0x458be0` in the
approved image. The first fills and transforms twenty words; the second applies
an additional transformation using obfuscated branches, floating-point operations,
and integer exceptions. The public key1 finalization remains the separate third
step in Rust. The local adapter interprets the original instructions with Unicorn
2.1.4, synthetic object/stack/CRT state, and a bounded C++ integer-catch adapter.
It has no host Windows API bridge or network service. Each expansion has instruction
and time limits; the Rust caller also limits helper execution to five seconds.

Validation on this Mac:

- All **1,024** expanded records in the independent local server cache match the
  adapter byte-for-byte. This comparison uses a pre-existing cache, not its own
  generated vectors. No server cache data is committed.
- `expansion-vectors.txt` contains 32 additional synthetic-key outputs, including
  zero, all-ones and high-bit boundaries. These are regression fixtures from the
  emulated reference, not independent mathematical proof for every 32-bit key.
- The Rust Session logged in through the native Continuum module, entered the
  arena, and downloaded map/settings using the adapter, without reading `scrty1`.
- A second connection remained active for **90 seconds**, sending one periodic
  security response. Server configuration and whitelist were not changed.
- Eight synthetic UDP integration tests cover the prior transport cases plus
  the 165-byte login layout and encrypted `00 12` / `00 13` expansion challenges.

The reference image must match the recorded SHA-256. A fresh runtime capture may
have different relocated imports; it is not accepted automatically. This adapter
is a reproducible development bridge for the approved image, not a distributable
replacement for independently implementing the expansion transformations.

The helper contract is one decimal key argument and exactly 80 binary stdout bytes.
Errors go to stderr and produce a failed connection; no VIE fallback is attempted.
The optional CLI mode sends an unspecified (all-zero) 64-byte billing identity.
Zone servers treat that field as opaque; billing systems may reject it. The existing
executable checksum remains zero. Servers enforcing the original executable's
integrity may reject the client even when encryption is correct.

To use a caller-provided compatible helper:

```sh
cargo run --example continuum_login -- 127.0.0.1:5000 /absolute/path/to/expander 90
cargo run --release -- --assets /path/to/Continuum --connect 127.0.0.1:5000 --continuum-expander /absolute/path/to/expander --name YourName --password YourPassword
```

For the included reference adapter, install `unicorn==2.1.4` into an isolated Python
environment, set `AICONTINUUM_RUNTIME_CODE` to the approved local image, and invoke
`tools/protocol/continuum_reference_expander.py` with that environment's interpreter.
Use an executable wrapper when passing it as `--continuum-expander`. On this Mac,
Unicorn's cache initialization needs execution outside Codex's restricted sandbox.
The emulator and original image are not included in the game executable.

Protocol sources: original 0.40 captures and TWCore document the 88-byte expansion
response (`00 13`, u16 version, seed, twenty words); public nullspace omitted the
version field. SS.NET `LoginPacket.cs` documents the 165-byte Continuum login and
opaque billing field. No executable-integrity spoofing
or borrowed billing identity is part of this implementation.

To reproduce the reference-adapter checks with its Python environment:

```sh
python tools/protocol/check_continuum_reference.py /path/to/approved-runtime-code.bin --server-cache /path/to/server/scrty1
```

Omit `--server-cache` for synthetic-vector checks only. The user elected to keep
all login verification local; no public-zone login was performed.

## Acknowledgement compatibility correction

After the owner reported an acknowledgement timeout on SSCU Trench Wars, comparison
with pinned nullspace `Connection.cpp` found that our handshake incorrectly required
a four-byte zero VIE-style key after decrypted `00 02`. Continuum has already
exchanged its keys; the reference client accepts that acknowledgement without
interpreting its optional body. The corrected transport accepts short `00 02` and
optional nonzero body bytes after successful decryption/CRC validation. VIE's key
validation and the rule against accepting a plaintext Continuum ack are preserved.

The transport also restores nullspace's repeated-first-byte CRC compatibility.
`legacy-crc-vectors.txt` is independently generated by the pinned public C++ cipher
and CRC functions, including a short acknowledgement used in a loopback regression.
Regenerate it with:

```sh
python3 tools/protocol/generate_continuum_vectors.py /path/to/nullspace --legacy-crc > docs/research/continuum/legacy-crc-vectors.txt
```

These are confirmed differences from the public reference, not a confirmed capture
of Trench Wars' response. No public-server connection was made during diagnosis.
Bounded stderr diagnostics report handshake packet lengths, decoded core type and
decode failure category, without packet contents, credentials or key tables.
