//! S2C 0x2F: NUL-terminated arena names followed by signed LE player counts.
//! A negative count marks the current arena (SS.NET PlayerCommand.Command_arena).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArenaInfo {
    pub name: String,
    pub players: u16,
    pub current: bool,
}

pub fn parse(packet: &[u8]) -> Option<Vec<ArenaInfo>> {
    if packet.first() != Some(&0x2f) { return None; }
    let mut rest = &packet[1..];
    let mut arenas = Vec::new();
    while !rest.is_empty() {
        let end = rest.iter().position(|&b| b == 0)?;
        let count = rest.get(end + 1..end + 3)?;
        let count = i16::from_le_bytes([count[0], count[1]]);
        arenas.push(ArenaInfo {
            name: rest[..end].iter().map(|&b| b as char).collect(),
            players: count.unsigned_abs(),
            current: count < 0,
        });
        rest = &rest[end + 3..];
    }
    Some(arenas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_current_empty_and_populated_arenas() {
        let packet = b"\x2f0\0\xfd\xffduel\0\x0c\0empty\0\0\0";
        assert_eq!(parse(packet), Some(vec![
            ArenaInfo { name: "0".into(), players: 3, current: true },
            ArenaInfo { name: "duel".into(), players: 12, current: false },
            ArenaInfo { name: "empty".into(), players: 0, current: false },
        ]));
        assert_eq!(parse(b"\x2fa\0\0\x80").unwrap()[0].players, 32768);
        assert_eq!(parse(&[0x2f]), Some(Vec::new()));
    }

    #[test]
    fn rejects_truncated_entries_without_partial_results() {
        for packet in [&b""[..], b"\x07a\0\x01\0", b"\x2fabc", b"\x2fa\0", b"\x2fa\0\x01", b"\x2fa\0\x01\0b"] {
            assert!(parse(packet).is_none());
        }
    }
}
