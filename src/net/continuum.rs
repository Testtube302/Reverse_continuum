//! Continuum packet cipher, separate from negotiation and key expansion.
//!
//! Ported from nullspace (MIT, Copyright (c) 2021 plushmonkey).
//! See docs/research/continuum/LICENSE-nullspace and README.md for sources and limits.
//! Callers must supply an expanded or finalized session table; the server's two
//! handshake words are not this table. Transport supports an external key lookup;
//! the game's default session still uses VIE.

pub const MAX_PLAINTEXT: usize = 520;
pub const MAX_CIPHERTEXT: usize = MAX_PLAINTEXT + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherError {
    InvalidLength,
    InvalidEscape,
    ChecksumMismatch,
}

pub struct ContinuumCipher {
    key: [u32; 20],
}

impl ContinuumCipher {
    /// Finalizes an already-expanded key2 table with this handshake's key1.
    /// This does not derive the expanded table from key2.
    pub fn from_expanded_table(table: [u32; 20], key1: u32) -> Self {
        Self::from_final_session_table(super::continuum_key::finalize(table, key1))
    }

    pub fn from_final_session_table(key: [u32; 20]) -> Self {
        Self { key }
    }

    /// Encrypts a nonempty game/core packet after negotiation. Handshake packets
    /// must be routed separately; this method always applies packet encryption.
    pub fn encrypt(&self, packet: &[u8]) -> Result<Vec<u8>, CipherError> {
        if packet.is_empty() || packet.len() > MAX_PLAINTEXT {
            return Err(CipherError::InvalidLength);
        }
        let mut data = Vec::with_capacity(packet.len() + 2);
        data.push(crc8(packet));
        data.extend_from_slice(packet);
        self.transform(&mut data, false);
        if needs_escape(&data) {
            data.insert(0, 0xff);
        }
        Ok(data)
    }

    /// Checks framing and the packet CRC before returning plaintext.
    pub fn decrypt(&self, packet: &[u8]) -> Result<Vec<u8>, CipherError> {
        if packet.len() < 2 || packet.len() > MAX_CIPHERTEXT {
            return Err(CipherError::InvalidLength);
        }
        let encrypted = if packet[0] == 0xff {
            let rest = &packet[1..];
            if rest.len() < 2 || !needs_escape(rest) {
                return Err(CipherError::InvalidEscape);
            }
            rest
        } else {
            if needs_escape(packet) {
                return Err(CipherError::InvalidEscape);
            }
            packet
        };
        if encrypted.len() > MAX_PLAINTEXT + 1 {
            return Err(CipherError::InvalidLength);
        }
        let mut data = encrypted.to_vec();
        self.transform(&mut data, true);
        if crc8(&data[1..]) != data[0] {
            return Err(CipherError::ChecksumMismatch);
        }
        Ok(data[1..].to_vec())
    }

    fn transform(&self, data: &mut [u8], decrypt: bool) {
        let (mut lower, mut upper) = (0u32, 0u32);
        let mut offset = 0;
        while offset + 8 <= data.len() {
            let a = word(data, offset);
            let b = word(data, offset + 4);
            let (mut x, mut y) = if decrypt {
                (a, b)
            } else {
                (a ^ lower, b ^ upper)
            };
            if decrypt {
                for keys in self.key.chunks_exact(2).rev() {
                    y = y.rotate_right(x & 31).wrapping_sub(keys[1]);
                    x = x.rotate_right(y & 31).wrapping_sub(keys[0]);
                }
                x ^= lower;
                y ^= upper;
                (lower, upper) = (a, b);
            } else {
                for keys in self.key.chunks_exact(2) {
                    x = x.wrapping_add(keys[0]).rotate_left(y & 31);
                    y = y.wrapping_add(keys[1]).rotate_left(x & 31);
                }
                (lower, upper) = (x, y);
            }
            data[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
            data[offset + 4..offset + 8].copy_from_slice(&y.to_le_bytes());
            offset += 8;
        }
        if offset + 4 <= data.len() {
            let original = word(data, offset);
            let mut x = original;
            if decrypt {
                for keys in self.key.chunks_exact(2) {
                    upper = upper.wrapping_add(keys[1]);
                }
                let mut rotation = upper;
                for keys in self.key.chunks_exact(2).rev() {
                    rotation = rotation.wrapping_sub(keys[1]);
                    x = x.rotate_right(rotation & 31).wrapping_sub(keys[0]);
                }
                x ^= lower;
                lower = original;
            } else {
                x ^= lower;
                for keys in self.key.chunks_exact(2) {
                    x = x.wrapping_add(keys[0]).rotate_left(upper & 31);
                    upper = upper.wrapping_add(keys[1]);
                }
                lower = x;
            }
            data[offset..offset + 4].copy_from_slice(&x.to_le_bytes());
            offset += 4;
        }
        for byte in &mut data[offset..] {
            let original = *byte;
            let mut x = original;
            if decrypt {
                for keys in self.key.chunks_exact(2) {
                    upper = upper.wrapping_add(keys[1]);
                }
                let mut accum = upper;
                for keys in self.key.chunks_exact(2).rev() {
                    accum = accum.wrapping_sub(keys[1]);
                    x = x.wrapping_sub(keys[0] as u8).wrapping_sub(accum as u8);
                }
                x ^= lower as u8;
                lower = original as u32;
            } else {
                x ^= lower as u8;
                for keys in self.key.chunks_exact(2) {
                    x = x.wrapping_add(keys[0] as u8).wrapping_add(upper as u8);
                    upper = upper.wrapping_add(keys[1]);
                }
                lower = x as u32;
            }
            *byte = x;
        }
    }
}

fn needs_escape(data: &[u8]) -> bool {
    data[0] == 0xff || (data[0] == 0 && matches!(data.get(1), Some(1 | 0x10 | 0x11)))
}

fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

// Reflected CRC-8/MAXIM polynomial, matching nullspace's lookup table.
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x8c } else { 0 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn matches_cpp_reference_vectors() {
        for line in include_str!("../../docs/research/continuum/cipher-vectors.txt").lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            let key_bytes = unhex(fields[0]);
            let key = std::array::from_fn(|i| word(&key_bytes, i * 4));
            let cipher = ContinuumCipher::from_final_session_table(key);
            let plaintext = unhex(fields[1]);
            let ciphertext = unhex(fields[2]);
            assert_eq!(cipher.encrypt(&plaintext).unwrap(), ciphertext);
            assert_eq!(cipher.decrypt(&ciphertext).unwrap(), plaintext);
        }
    }

    #[test]
    fn all_lengths_roundtrip() {
        let cipher = ContinuumCipher::from_final_session_table(std::array::from_fn(|i| {
            (i as u32).wrapping_mul(0x9e3779b9)
        }));
        for len in 1..=MAX_PLAINTEXT {
            let packet: Vec<_> = (0..len).map(|i| i as u8).collect();
            assert_eq!(
                cipher.decrypt(&cipher.encrypt(&packet).unwrap()).unwrap(),
                packet
            );
        }
    }

    #[test]
    fn rejects_invalid_framing_and_crc() {
        let cipher = ContinuumCipher::from_final_session_table([0; 20]);
        for packet in [
            vec![],
            vec![0],
            vec![0; MAX_CIPHERTEXT + 1],
            vec![0; MAX_PLAINTEXT + 2],
        ] {
            assert_eq!(cipher.decrypt(&packet), Err(CipherError::InvalidLength));
        }
        for packet in [vec![0xff, 0], vec![0xff, 1, 2], vec![0, 0x10]] {
            assert_eq!(cipher.decrypt(&packet), Err(CipherError::InvalidEscape));
        }
        assert_eq!(cipher.encrypt(&[]), Err(CipherError::InvalidLength));
        assert_eq!(
            cipher.encrypt(&vec![0; MAX_PLAINTEXT + 1]),
            Err(CipherError::InvalidLength)
        );
        let mut encrypted = cipher.encrypt(&[3, 4, 5, 6]).unwrap();
        *encrypted.last_mut().unwrap() ^= 1;
        assert_eq!(
            cipher.decrypt(&encrypted),
            Err(CipherError::ChecksumMismatch)
        );
    }
}
