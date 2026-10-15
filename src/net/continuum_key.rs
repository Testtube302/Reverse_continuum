//! Continuum session-table finalization, derived from the RSA Data Security,
//! Inc. MD5 Message Digest Algorithm as modified by nullspace.
//! See docs/research/continuum/LICENSE-md5 and LICENSE-nullspace.
//!
//! This is NOT key2 expansion or a standard MD5 digest. The reference updates
//! 80 bytes but does not finalize: only the first 64 bytes are compressed.

const CONSTANTS: [u32; 64] = [
    0xd76a5478, 0xe867b756, 0x242073db, 0xc18d3eee, 0xf57308af, 0x4787c62a, 0xa8384613, 0xfd469501,
    0x688098d8, 0x8b44f7af, 0xff8f5bb1, 0x898cd7be, 0x6b901822, 0xfd987193, 0xa679438e, 0x49740821,
    0xf6112562, 0xc540b340, 0x265e4a51, 0xe9b4c7aa, 0xd627105d, 0x2541453, 0xd8a15681, 0xe7d3fbc8,
    0x21e4cde6, 0xc33707d6, 0xf4d50387, 0x455a14ed, 0xa933e905, 0xfcefa3f8, 0x676f01d9, 0x8d294c8a,
    0xf1fa3942, 0x8771f681, 0x6d3d6122, 0xfde5380c, 0xa4be6a44, 0x4bdec3a9, 0xf6b84b60, 0xbebfbc70,
    0x28937ec6, 0xeaa127fa, 0xd4ef8085, 0x4881d05, 0xd9d4d339, 0xe66b99e5, 0x1fa27cf8, 0xc4ac3665,
    0xf4342244, 0x432a6f97, 0xab9423a7, 0xfc95a039, 0x655559c3, 0x8f0ccc92, 0xf5eff47d, 0x85845dd1,
    0x6fa57e4f, 0xf52ce6e0, 0xa3014514, 0x4e0851a1, 0xf7557e82, 0x453af235, 0x2ad782bb, 0xeb56d391,
];

pub(super) fn finalize(mut table: [u32; 20], key1: u32) -> [u32; 20] {
    table[0] ^= key1;
    let initial = [0x67352301u32, 0xefcdab79, 0x98baccfe, 0x11325476].map(|v| v ^ 432);
    let [mut a, mut b, mut c, mut d] = initial;
    for (i, constant) in CONSTANTS.iter().enumerate() {
        let (f, word, shift) = match i {
            0..=15 => ((b & c) | (!b & d), i, [7, 12, 17, 22][i % 4]),
            16..=31 => ((b & d) | (c & !d), (5 * i + 1) % 16, [5, 9, 14, 20][i % 4]),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16, [4, 11, 16, 23][i % 4]),
            _ => (c ^ (b | !d), (7 * i) % 16, [6, 10, 15, 21][i % 4]),
        };
        let next = a
            .wrapping_add(f)
            .wrapping_add(table[word])
            .wrapping_add(constant ^ 432)
            .rotate_left(shift)
            .wrapping_add(b);
        (a, b, c, d) = (d, next, b, c);
    }
    let state = [a, b, c, d];
    let state: [u32; 4] = std::array::from_fn(|i| state[i].wrapping_add(initial[i]));
    let mut last = key1;
    for (i, word) in table.iter_mut().enumerate() {
        *word ^= state[i % 4].wrapping_add(last);
        last = *word;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(hex: &str) -> [u32; 20] {
        assert_eq!(hex.len(), 160);
        std::array::from_fn(|i| {
            let bytes: [u8; 4] = std::array::from_fn(|j| {
                let start = i * 8 + j * 2;
                u8::from_str_radix(&hex[start..start + 2], 16).unwrap()
            });
            u32::from_le_bytes(bytes)
        })
    }

    #[test]
    fn matches_public_cpp_finalization() {
        let mut count = 0;
        for line in include_str!("../../docs/research/continuum/finalization-vectors.txt").lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            let key1 = u32::from_str_radix(fields[0], 16).unwrap();
            assert_eq!(finalize(table(fields[1]), key1), table(fields[2]));
            count += 1;
        }
        assert_eq!(count, 32);
    }
}
