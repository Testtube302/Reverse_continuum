# Continuum billing identification — 2026-10-16

The 64-byte login field has a verified codec in
`src/net/continuum_identity.rs`. It is not yet enabled in the launcher: the native
machine-information collector and platform mapping remain incomplete. This is
not a demonstrated fix for the SSC rejection. No new public test was performed.

The owner authorized analysis of the original client for this compatibility work.
The findings below came from the pinned read-only image described in README.md,
and three existing original-client captures decoded in memory. No captured machine
identifiers, account information, proprietary code or decompilation are included.

## Format and evidence

At login offset 101, the field contains:

| Field-relative offset | Length | Meaning |
| --- | --- | --- |
| 0 | 4 | Session nonce, little-endian, not encrypted |
| 4 | 4 | Checksum, encrypted |
| 8 | 56 | Fourteen little-endian information words, encrypted |

Start with the nonce, a zero checksum word, and the fourteen words. Run reflected
CRC-32 with initial state `0xffffffff` and no final complement over all 64 bytes.
XOR that result with the nonce and store it in the checksum word. Equivalently,
using this repository's finalized CRC function: `!crc32(plain) ^ nonce`.

Expand the nonce with the original first-stage routine at `0x457d60` only.
Session key2 expansion also calls `0x458be0`; that second stage is **omitted** here.
Encrypt the last 60 bytes with the raw Continuum transform. Do not add a packet
CRC-8, packet escape byte, or session key1 finalization. The login construction
at `0x428910` corroborates this layout.

All three existing reference fields validate with this checksum after decryption.
Their fourteen plaintext words are identical despite their different encrypted
bytes, including a comparison with a different game account. That supports stable
machine inputs plus a changing nonce; it does not identify the server's policy.

The checksum detects format errors; it is not cryptographic authentication.

## Information sources and remaining uncertainty

The login constructor copies words from a metadata object in the order below.
Object offsets are internal reference offsets, not offsets into the wire field.

| Word index | Object offset | Recovered source or remaining question |
| --- | --- | --- |
| 0, 1 | `08`, `0c` | Legacy BIOS-region checksums; explicitly zero on the NT path (`0x42d030`) |
| 2 | `10` | Legacy device-information checksum (`0x412d60`); zero on failure; full input semantics unresolved |
| 3 | `14` | Registry value selected by lowercase-name CRC for `RegisteredOwner` |
| 4 | `04` | Registry value selected by lowercase-name CRC for `RegisteredOrganization` |
| 5 | `34` | Registry value selected by lowercase-name CRC for `ProductId` |
| 6 | `18` | XOR of two registry-derived results; selectors not yet resolved |
| 7 | `2c` | Registry value selected by lowercase-name CRC for `Identifier`; exact path not yet resolved |
| 8, 9 | `1c`, `28` | RAS connection/dial-parameter data (`0x412700`); exact selected string members require confirmation |
| 10 | `30` | Another registry-derived result; selector/path unresolved |
| 11, 12 | `20`, `38` | String CRCs from two OS calls (`0x4401e0`), consistent with user/computer-name APIs; imported API names still require direct verification |
| 13 | `24` | XOR of a selected Windows-directory entry's creation-time words (`0x4393e0`); filename selector unresolved |

The registry collector (`0x453e10`, traversal `0x412fa0`) selects entries by CRCs
of lowercased names. DWORD values are used directly; other selected values use
a running CRC over the returned bytes. Name selection is confirmed for the four
named entries above, but the ancestor-path filters must still be established.
Do not treat a matching value name anywhere in the registry as sufficient.

Static strings independently identify `RasEnumConnectionsA` and
`RasGetEntryDialParamsA`. In the three reference captures, words 0, 1, 2, 6, 7, 8,
9 and 10 are zero. The others are nonzero. No actual values are recorded here.

The separate login machine word (packet offset 66) comes from a volume-serial
lookup, with signed absolute-value handling and fallback 1 (`0x4459b0`). The
client's current account-name CRC is not equivalent. This is a separate gap
from the 64-byte field and remains unchanged by the codec work.

Native macOS does not provide these Windows registry paths or Windows directory
metadata. A collector needs an explicit, justified platform mapping and local
verification before launcher integration. Encrypting zeros, inventing Windows
registration data, or importing the reference VM's identity would not establish
that mapping. The server-side reason for rejecting the current client is unknown.

## Local verification and reproduction

`identity-vectors.txt` contains 24 synthetic fixtures covering eight nonces and
three field patterns. The generator invokes the original CRC and raw encrypt/
decrypt routines in bounded emulation, with synthetic memory and no Windows API
calls. It cross-checks the CRC against Python zlib. Rust tests match every output,
reject a flipped bit at each of the 64 byte positions, reject wrong lengths and
reject an incorrect table. These tests run without the original image.

```sh
cargo test --lib continuum_identity
# Requires the separately authorized pinned image and the reference Python venv:
python tools/protocol/generate_continuum_identity_vectors.py /path/to/runtime-code.bin
python tools/protocol/check_continuum_reference.py /path/to/runtime-code.bin
```

The reference helper accepts `<decimal-seed> --identity` for this table, while
its original single-argument session-expansion interface remains unchanged.
`CommandExpander::expand_identity` selects that mode explicitly; older helpers
must implement it before use. The image and its emulator remain a private local
dependency, not a standalone Rust implementation.

Next work: resolve the registry path filters and OS call names, confirm the
selected file/RAS inputs with synthetic OS responses, then define a truthful
collector for each supported platform. Verify encoding and collection locally
before changing the desktop's login behavior. Public acceptance still needs a
separately authorized test and cannot be inferred from codec correctness.
