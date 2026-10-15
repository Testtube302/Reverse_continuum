//! Server-owned `scrty1` expansion cache, for controlled compatibility testing.
//! Layout: public nullspace Crypt.cpp at 5743bb7 (MIT); see research notes.
//! This is a lookup table, not the missing arbitrary-key expansion algorithm.

use std::collections::HashMap;
use std::io;

pub struct ExpansionCache(HashMap<u32, [u32; 20]>);

impl ExpansionCache {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        const RECORD: usize = 84;
        const COUNT: usize = 1024;
        if bytes.len() != 4 + 2 * COUNT * RECORD || bytes[..4] != [40, 0, 1, 0] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported scrty1 size or version",
            ));
        }
        let mut tables = HashMap::new();
        for record in bytes[4 + COUNT * RECORD..].chunks_exact(RECORD) {
            if record.iter().all(|&b| b == 0) {
                continue;
            }
            let key = u32::from_le_bytes(record[..4].try_into().unwrap());
            let table = std::array::from_fn(|i| {
                u32::from_le_bytes(record[4 + i * 4..8 + i * 4].try_into().unwrap())
            });
            if tables.insert(key, table).is_some_and(|old| old != table) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "conflicting scrty1 entries",
                ));
            }
        }
        Ok(Self(tables))
    }

    pub fn lookup(&self, key: u32) -> Option<[u32; 20]> {
        self.0.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 4 + 2048 * 84];
        bytes[..4].copy_from_slice(&[40, 0, 1, 0]);
        let offset = 4 + 1024 * 84;
        bytes[offset..offset + 4].copy_from_slice(&123u32.to_le_bytes());
        for i in 0..20 {
            bytes[offset + 4 + i * 4..offset + 8 + i * 4]
                .copy_from_slice(&(i as u32 + 1).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn reads_only_expansion_bank_and_requires_exact_key() {
        let mut bytes = fixture();
        bytes[4..8].copy_from_slice(&456u32.to_le_bytes());
        let cache = ExpansionCache::parse(&bytes).unwrap();
        assert_eq!(
            cache.lookup(123),
            Some(std::array::from_fn(|i| i as u32 + 1))
        );
        assert_eq!(cache.lookup(456), None);
        assert_eq!(cache.lookup(0), None);
    }

    #[test]
    fn rejects_truncation_wrong_version_and_conflicting_duplicates() {
        let mut bytes = fixture();
        assert!(ExpansionCache::parse(&bytes[..bytes.len() - 1]).is_err());
        bytes[0] = 39;
        assert!(ExpansionCache::parse(&bytes).is_err());
        bytes[0] = 40;
        let offset = 4 + 1024 * 84;
        bytes.copy_within(offset..offset + 84, offset + 84);
        assert!(ExpansionCache::parse(&bytes).is_ok());
        bytes[offset + 88] ^= 1;
        assert!(ExpansionCache::parse(&bytes).is_err());
    }
}
