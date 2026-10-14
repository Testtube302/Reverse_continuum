//! VIE ("SubSpace 1.35") packet encryption: an XOR keystream generated from the session key
//! with a Park-Miller style RNG, chained with the previous ciphertext word.
//!
//! Arithmetic follows the nullspace client (MIT, plushmonkey) exactly, including its unsigned
//! treatment of the seed, since that implementation interoperates with ASSS / Subspace Server .NET.

pub const KEYSTREAM_LEN: usize = 520;

/// The encryption keystream RNG step.
fn next_encrypt(seed: &mut u32) -> u16 {
    let old = *seed;
    let prod = (old as i64).wrapping_mul(0x834E_0B5F_i64);
    let mut t = (prod >> 48) as i32 as u32;
    t = t.wrapping_add(t >> 31);
    let mut new = (old % 0x1F31D).wrapping_mul(0x41A7).wrapping_sub(t.wrapping_mul(0xB14)).wrapping_add(0x7B);
    if (new as i32) < 1 {
        new = new.wrapping_add(0x7FFF_FFFF);
    }
    *seed = new;
    new as u16
}

/// The signed RNG used for door and prize seeds.
#[derive(Clone, Copy, Debug, Default)]
pub struct VieRng {
    pub seed: i32,
}

impl VieRng {
    pub fn new(seed: u32) -> Self {
        Self { seed: seed as i32 }
    }

    pub fn next(&mut self) -> u32 {
        let s = self.seed;
        let mut n = (s % 0x1F31D).wrapping_mul(0x41A7).wrapping_sub((s / 0x1F31D).wrapping_mul(0xB14)).wrapping_add(0x7B);
        if n < 1 {
            n = n.wrapping_add(0x7FFF_FFFF);
        }
        self.seed = n;
        n as u32
    }
}

pub struct Vie {
    pub client_key: u32,
    session_key: u32,
    keystream: [u8; KEYSTREAM_LEN],
}

impl Vie {
    /// Creates a cipher with a fresh client key (always has the high bit set).
    pub fn new(entropy: u32) -> Self {
        let mut key = entropy.wrapping_mul(0x9E37_79B9) ^ 0x5bd1_e995;
        if key <= 0x7FFF_FFFF {
            key = (!key).wrapping_add(1);
        }
        if key <= 0x7FFF_FFFF {
            key |= 0x8000_0000;
        }
        Self { client_key: key, session_key: 0, keystream: [0; KEYSTREAM_LEN] }
    }

    pub fn is_active(&self) -> bool {
        self.session_key != 0
    }

    /// Accepts the server's key from the 0x00 0x02 response. Returns false if it is not valid
    /// for our client key. A server echoing our own key means "no encryption".
    pub fn initialize(&mut self, server_key: u32) -> bool {
        let valid = server_key == self.client_key || server_key == (!self.client_key).wrapping_add(1);
        if !valid {
            return false;
        }
        if server_key == self.client_key {
            self.session_key = 0;
            self.keystream = [0; KEYSTREAM_LEN];
            return true;
        }
        self.session_key = server_key;
        let mut seed = server_key;
        for i in 0..KEYSTREAM_LEN / 2 {
            let v = next_encrypt(&mut seed);
            self.keystream[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        true
    }

    fn ks(&self, at: usize) -> u32 {
        let b = &self.keystream[at..at + 4];
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    fn header_len(pkt: &[u8]) -> usize {
        if pkt[0] == 0 { 2 } else { 1 }
    }

    pub fn encrypt(&self, pkt: &mut [u8]) {
        if !self.is_active() || pkt.is_empty() || (pkt[0] == 0 && pkt.len() <= 2) {
            return;
        }
        let mut i = Self::header_len(pkt);
        let mut ksi = 0;
        let mut iv = self.session_key;
        while i + 4 <= pkt.len() {
            let w = u32::from_le_bytes([pkt[i], pkt[i + 1], pkt[i + 2], pkt[i + 3]]) ^ self.ks(ksi) ^ iv;
            pkt[i..i + 4].copy_from_slice(&w.to_le_bytes());
            iv = w;
            i += 4;
            ksi += 4;
        }
        self.xor_tail(pkt, i, ksi, iv);
    }

    pub fn decrypt(&self, pkt: &mut [u8]) {
        if !self.is_active() || pkt.is_empty() || (pkt[0] == 0 && pkt.len() <= 2) {
            return;
        }
        let mut i = Self::header_len(pkt);
        let mut ksi = 0;
        let mut iv = self.session_key;
        while i + 4 <= pkt.len() {
            let c = u32::from_le_bytes([pkt[i], pkt[i + 1], pkt[i + 2], pkt[i + 3]]);
            pkt[i..i + 4].copy_from_slice(&(c ^ self.ks(ksi) ^ iv).to_le_bytes());
            iv = c;
            i += 4;
            ksi += 4;
        }
        self.xor_tail(pkt, i, ksi, iv);
    }

    fn xor_tail(&self, pkt: &mut [u8], i: usize, ksi: usize, iv: u32) {
        let n = pkt.len() - i;
        if n == 0 {
            return;
        }
        let mut word = [0u8; 4];
        word[..n].copy_from_slice(&pkt[i..]);
        let w = u32::from_le_bytes(word) ^ self.ks(ksi) ^ iv;
        pkt[i..].copy_from_slice(&w.to_le_bytes()[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_key_has_high_bit() {
        for e in [0, 1, 12345, u32::MAX, 0x7FFF_FFFF] {
            assert!(Vie::new(e).client_key > 0x7FFF_FFFF);
        }
    }

    #[test]
    fn rejects_unrelated_server_key() {
        let mut v = Vie::new(7);
        assert!(!v.initialize(0x1234_5678));
        assert!(!v.is_active());
    }

    #[test]
    fn echoed_key_disables_encryption() {
        let mut v = Vie::new(7);
        let key = v.client_key;
        assert!(v.initialize(key));
        let mut p = [0x03, 1, 2, 3, 4, 5];
        v.encrypt(&mut p);
        assert_eq!(p, [0x03, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn roundtrip_all_lengths_and_headers() {
        let mut v = Vie::new(99);
        let key = (!v.client_key).wrapping_add(1);
        assert!(v.initialize(key));
        for len in 1..=520 {
            for first in [0u8, 0x03] {
                let original: Vec<u8> =
                    (0..len).map(|i| if i == 0 { first } else { (i * 7 + 3) as u8 }).collect();
                let mut p = original.clone();
                v.encrypt(&mut p);
                if len > 3 || (first != 0 && len > 1) {
                    assert_ne!(p, original, "len {len} first {first} unchanged");
                }
                assert_eq!(p[0], original[0]);
                v.decrypt(&mut p);
                assert_eq!(p, original, "len {len} first {first}");
            }
        }
    }

    #[test]
    fn keystream_matches_minimal_standard_for_small_seeds() {
        // For seeds < 2^31 the magic-multiply division equals plain integer division.
        let mut a = 1u32;
        let mut b = VieRng::new(1);
        for _ in 0..1000 {
            assert_eq!(next_encrypt(&mut a) as u32, b.next() & 0xFFFF);
        }
    }
}
