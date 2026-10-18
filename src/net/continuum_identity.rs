//! Codec for the 64-byte Continuum login billing field.
//!
//! This only encodes caller-supplied information. It does not collect machine
//! identifiers. The native macOS collector and launcher integration are separate
//! in native_identity.rs.
//! See docs/research/continuum/IDENTITY.md for measured field sources and limits.
use super::continuum::ContinuumCipher;
use crate::lvl::crc32;

/// Prepared login metadata. Deliberately does not implement Debug: these values
/// identify the local installation and must not enter connection logs.
#[derive(Clone, Copy)]
pub struct LoginIdentity {
    pub machine_id: u32,
    /// Minutes west of UTC, including the current daylight-saving adjustment.
    pub timezone_bias: i16,
    pub billing: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    InvalidLength,
    ChecksumMismatch,
}

/// Encode fourteen information words in their measured wire order.
/// `identity_table` must come from the identity-specific expansion of `nonce`;
/// neither a session key2 expansion nor a finalized session table is suitable.
/// The checksum is a format check, not cryptographic authentication.
pub fn encode(nonce: u32, fields: &[u32; 14], identity_table: [u32; 20]) -> [u8; 64] {
    let mut wire = [0u8; 64];
    wire[..4].copy_from_slice(&nonce.to_le_bytes());
    for (chunk, field) in wire[8..].chunks_exact_mut(4).zip(fields) {
        chunk.copy_from_slice(&field.to_le_bytes());
    }
    let checksum = !crc32(&wire) ^ nonce;
    wire[4..8].copy_from_slice(&checksum.to_le_bytes());
    // This field uses the raw transform, without packet CRC8 or escape bytes.
    ContinuumCipher::from_final_session_table(identity_table).transform(&mut wire[4..], false);
    wire
}

/// Decode and validate a billing field with the table expanded from its nonce.
/// Do not log returned machine information or use another machine's captured data.
pub fn decode(wire: &[u8], identity_table: [u32; 20]) -> Result<[u32; 14], IdentityError> {
    let mut plain: [u8; 64] = wire.try_into().map_err(|_| IdentityError::InvalidLength)?;
    ContinuumCipher::from_final_session_table(identity_table).transform(&mut plain[4..], true);
    let nonce = u32::from_le_bytes(plain[..4].try_into().unwrap());
    let checksum = u32::from_le_bytes(plain[4..8].try_into().unwrap());
    plain[4..8].fill(0);
    if checksum != (!crc32(&plain) ^ nonce) {
        return Err(IdentityError::ChecksumMismatch);
    }
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(plain[8 + i * 4..12 + i * 4].try_into().unwrap())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn matches_original_routines_on_synthetic_inputs() {
        let vectors = include_str!("../../docs/research/continuum/identity-vectors.txt");
        let mut count = 0;
        for line in vectors
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let parts: Vec<_> = line.split_whitespace().collect();
            let nonce = u32::from_str_radix(parts[0], 16).unwrap();
            let key = unhex(parts[1]);
            let fields = unhex(parts[2]);
            let expected = unhex(parts[3]);
            let key: [u32; 20] = std::array::from_fn(|i| {
                u32::from_le_bytes(key[i * 4..i * 4 + 4].try_into().unwrap())
            });
            let fields: [u32; 14] = std::array::from_fn(|i| {
                u32::from_le_bytes(fields[i * 4..i * 4 + 4].try_into().unwrap())
            });
            assert_eq!(encode(nonce, &fields, key).as_slice(), expected);
            assert_eq!(decode(&expected, key), Ok(fields));
            // Cover the clear nonce, encrypted checksum, all blocks and trailing word.
            for i in 0..64 {
                let mut damaged = expected.clone();
                damaged[i] ^= 1;
                assert_eq!(decode(&damaged, key), Err(IdentityError::ChecksumMismatch));
            }
            count += 1;
        }
        assert_eq!(count, 24);
    }

    #[test]
    fn rejects_wrong_length_and_wrong_table() {
        for size in [0, 4, 60, 63, 65, 128] {
            assert_eq!(
                decode(&vec![0; size], [0; 20]),
                Err(IdentityError::InvalidLength)
            );
        }
        let wire = encode(123, &[42; 14], [7; 20]);
        assert_eq!(decode(&wire, [8; 20]), Err(IdentityError::ChecksumMismatch));
    }
}
