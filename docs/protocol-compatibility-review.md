# Protocol compatibility review — 2026-10-15

Reviewed client commit `aeda9ce1f49bddc37e74ae38ef435f717f01a6e6` and
SubspaceServer commit `750f16298253d51e7ce4b2b55078acc8a3122ff9`.
This branch fixes a receive-path crash; it does **not** implement Continuum encryption
or establish compatibility with Continuum-only zones.

## Findings

1. `src/net/transport.rs::send_encryption_request` requests protocol 1 (VIE).
   `src/net/vie.rs` implements only the VIE cipher. Changing the protocol field alone
   cannot implement the other cipher or its handshake.
2. `src/net/session.rs::send_login` emits the 101-byte VIE login (0x09).
   Continuum support also requires its login format, not just packet encryption.
3. `try_answer_security` emits a VIE security response with executable checksum zero.
   Server-side integrity validation is independent of transport encryption. A
   successful handshake therefore does not prove that a client can remain connected.
4. The server has separate `AllowVIE` / `AllowContinuum` listener policies. Its security
   module compares executable, settings, and map checksums; Continuum additionally
   supplies a map CRC and timer drift. Whether a mismatch causes disconnection depends
   on server configuration and capabilities. These are findings about the inspected
   SS.NET revision, not verified claims about every community zone.
5. SS.NET's public tree does not contain the Continuum cipher implementation. Its
   documentation identifies `EncryptionContNative` as closed source, supplied as
   platform-specific binaries in releases. No compatible Rust implementation or
   independently verified Continuum test vectors were established in this review.

## Crash fixed

The UDP receive buffer accepts 2048 bytes, but VIE has a fixed 520-byte keystream.
Previously, a connected peer sending a 2048-byte encrypted datagram caused an
out-of-bounds panic in `Vie::ks` (range end 524, length 520). Reject datagrams larger
than the transport's 520-byte limit before decryption, accounting, or timeout refresh.
Keep the receive buffer larger than that limit so truncation cannot disguise an
oversized packet as an acceptable one.

The regression covers 521-, 2048-, and 4096-byte datagrams, then verifies that an
encrypted 520-byte packet still arrives and the connection stays open. The 4096-byte
case also exercises receive-buffer truncation on this platform.

## Verification and remaining work

- Reproduced the bounds panic before the fix using a loopback UDP test.
- `cargo test --offline`: 75 unit tests passed, one ignored. The eight integration
  test functions returned successfully but their external-asset/live-server work was
  skipped because the required environment variables were not set.
- `cargo clippy --offline --all-targets`: completed with existing warnings.
- No public zone connection or original-client binary analysis was performed.
- A future transport implementation needs independently sourced handshake and
  cipher vectors, encrypted loopback interoperability, login-format coverage, and
  sustained testing against a controlled zone through multiple security-request cycles.
  An actual rejection trace or server log is needed to establish which gate affects
  a particular zone; a timeout alone does not identify encryption as the cause.

## Sources

- [SS.NET listener policies](https://github.com/gigamon-dev/SubspaceServer/blob/750f16298253d51e7ce4b2b55078acc8a3122ff9/src/Core/Modules/Network.cs)
- [SS.NET security validation and kick policy](https://github.com/gigamon-dev/SubspaceServer/blob/750f16298253d51e7ce4b2b55078acc8a3122ff9/src/Core/Modules/Security.cs)
- [SS.NET login processing](https://github.com/gigamon-dev/SubspaceServer/blob/750f16298253d51e7ce4b2b55078acc8a3122ff9/src/Core/Modules/Core.cs)
- [Closed-source encryption module description](https://github.com/gigamon-dev/SubspaceServer/blob/750f16298253d51e7ce4b2b55078acc8a3122ff9/doc/asss-equivalents.md)
- [Native module packaging](https://github.com/gigamon-dev/SubspaceServer/blob/750f16298253d51e7ce4b2b55078acc8a3122ff9/scripts/build/prebuilt/README.md)
- [REA](https://github.com/morluto/rea): the linked tool provides analysis integrations;
  its overview does not supply a Continuum protocol implementation. Not installed.
