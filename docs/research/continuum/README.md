# Continuum transport investigation — 2026-10-15

## Status

This branch contains the packet cipher, table finalization, and an opt-in transport
API: `Transport::connect_continuum(server, expanded_key_lookup)`. The lookup receives
the handshake's key2 and must return its already-expanded 20-word table promptly.
The transport finalizes that table with key1, sends the plaintext acknowledgement,
waits for the encrypted server acknowledgement, then transports encrypted packets.
Returning None fails explicitly; no dummy table or VIE fallback is substituted.

**This is not yet a working game-login mode.** Key2 expansion is still absent, and
the game session still uses VIE. No key-expansion service, executable-integrity
response, or client identity bypass is implemented.

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

`decrypt` rejects bad CRCs and noncanonical escape prefixes. Unlike nullspace's
fallback, it does not accept a CRC calculated over repetitions of the first byte;
that tolerance has not been established as necessary by our observations. The
native module's inspected wrapper uses a strict CRC check.

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

1. Establish an authorized, reproducible way to obtain the 20-word expanded table
   for the handshake's `key2`; this investigation has not recovered that operation.
2. Validate the complete encrypted handshake against the native server using a
   genuine expanded table. Synthetic loopback success is not proof of this.
3. Add the Continuum login format and test sustained operation on the controlled
   zone. Separately establish how that zone handles client-integrity checks.

The earlier protocol review should not be read as saying that no public Continuum
packet cipher exists. The public transform exists; usable session-key expansion
and complete client integration remain the missing pieces.
