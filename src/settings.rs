//! Arena/ship settings. Online these arrive from the zone server; offline we use the
//! standard SVS values (from Subspace Server .NET's `conf/svs`).

pub const SHIP_NAMES: [&str; 8] =
    ["Warbird", "Javelin", "Spider", "Leviathan", "Terrier", "Weasel", "Lancaster", "Shark"];

#[derive(Clone, Copy, Debug)]
pub struct ShipSettings {
    /// 400 = one full rotation per second.
    pub initial_rotation: u32,
    pub maximum_rotation: u32,
    /// Acceleration; velocity (tiles/s) gains `thrust * 10 / 16` per second.
    pub initial_thrust: u32,
    pub maximum_thrust: u32,
    /// Top speed in pixels per 10 seconds.
    pub initial_speed: u32,
    pub maximum_speed: u32,
    pub initial_energy: u32,
    pub maximum_energy: u32,
    /// Energy regained per 10 seconds.
    pub initial_recharge: u32,
    /// Energy spent per 10 seconds of afterburner.
    pub afterburner_energy: u32,
    /// Collision radius in pixels; 0 means the default of 14.
    pub radius: u32,
}

impl ShipSettings {
    pub fn radius_tiles(&self) -> f32 {
        let r = if self.radius == 0 { 14 } else { self.radius };
        r as f32 / 16.0
    }
}

#[derive(Clone, Debug)]
pub struct ArenaSettings {
    pub ships: [ShipSettings; 8],
    /// Walls scale velocity by `16 / bounce_factor` on impact.
    pub bounce_factor: u32,
    /// The raw 0x0F settings packet when received from a server (needed for its checksum).
    pub raw: Vec<u8>,
}

/// Size of the S2C 0x0F arena settings packet.
pub const SETTINGS_PACKET_LEN: usize = 1428;
const SHIP_BLOCK_START: usize = 4;
const SHIP_BLOCK_LEN: usize = 144;
const BOUNCE_FACTOR_OFFSET: usize = 1272;

fn u16_at(p: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([p[o], p[o + 1]]) as u32
}

impl ShipSettings {
    /// Decodes one 144-byte ship block of the settings packet.
    fn from_block(b: &[u8]) -> Self {
        Self {
            radius: (u16_at(b, 28) >> 3) & 0xFF,
            maximum_rotation: u16_at(b, 40),
            maximum_thrust: u16_at(b, 42),
            maximum_speed: u16_at(b, 44),
            maximum_energy: u16_at(b, 48),
            initial_rotation: u16_at(b, 50),
            initial_thrust: u16_at(b, 52),
            initial_speed: u16_at(b, 54),
            initial_recharge: u16_at(b, 56),
            initial_energy: u16_at(b, 58),
            afterburner_energy: u16_at(b, 70),
        }
    }
}

impl ArenaSettings {
    pub fn from_packet(p: &[u8]) -> Option<Self> {
        if p.len() < SETTINGS_PACKET_LEN || p[0] != 0x0F {
            return None;
        }
        let ships = std::array::from_fn(|i| {
            let o = SHIP_BLOCK_START + i * SHIP_BLOCK_LEN;
            ShipSettings::from_block(&p[o..o + SHIP_BLOCK_LEN])
        });
        Some(Self {
            ships,
            bounce_factor: u16_at(p, BOUNCE_FACTOR_OFFSET),
            raw: p[..SETTINGS_PACKET_LEN].to_vec(),
        })
    }

    /// Security checksum over the raw settings packet: sum of each 32-bit word XOR key.
    pub fn checksum(&self, key: u32) -> u32 {
        self.raw
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) ^ key)
            .fold(0u32, u32::wrapping_add)
    }
}

const fn ship(rot: (u32, u32), thrust: (u32, u32), speed: (u32, u32), max_energy: u32, recharge: u32) -> ShipSettings {
    ShipSettings {
        initial_rotation: rot.0,
        maximum_rotation: rot.1,
        initial_thrust: thrust.0,
        maximum_thrust: thrust.1,
        initial_speed: speed.0,
        maximum_speed: speed.1,
        initial_energy: 1000,
        maximum_energy: max_energy,
        initial_recharge: recharge,
        afterburner_energy: 1200,
        radius: 0,
    }
}

impl Default for ArenaSettings {
    fn default() -> Self {
        Self {
            ships: [
                ship((210, 300), (16, 19), (2010, 3250), 1700, 400),
                ship((200, 230), (15, 17), (2200, 3750), 1700, 400),
                ship((200, 230), (15, 17), (2010, 3250), 1700, 500),
                ship((200, 230), (15, 17), (2010, 3250), 1700, 400),
                ship((200, 230), (15, 17), (2010, 3250), 1700, 400),
                ship((200, 230), (15, 17), (2010, 3250), 1700, 400),
                ship((200, 230), (15, 17), (2010, 3250), 1700, 400),
                ship((200, 230), (15, 17), (2010, 3250), 1750, 400),
            ],
            bounce_factor: 22,
            raw: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(p: &mut [u8], o: usize, v: u16) {
        p[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parses_ship_blocks_and_bounce_factor() {
        let mut p = vec![0u8; SETTINGS_PACKET_LEN];
        p[0] = 0x0F;
        let javelin = SHIP_BLOCK_START + SHIP_BLOCK_LEN;
        put16(&mut p, javelin + 50, 200);
        put16(&mut p, javelin + 52, 15);
        put16(&mut p, javelin + 54, 2200);
        put16(&mut p, javelin + 28, 15 << 3 | 0b101);
        put16(&mut p, BOUNCE_FACTOR_OFFSET, 22);
        let s = ArenaSettings::from_packet(&p).unwrap();
        let j = s.ships[1];
        assert_eq!((j.initial_rotation, j.initial_thrust, j.initial_speed, j.radius), (200, 15, 2200, 15));
        assert_eq!(s.bounce_factor, 22);
        assert!(ArenaSettings::from_packet(&p[..100]).is_none());
    }

    #[test]
    fn checksum_sums_words_xor_key() {
        let mut s = ArenaSettings::default();
        s.raw = vec![1, 0, 0, 0, 2, 0, 0, 0];
        assert_eq!(s.checksum(0), 3);
        assert_eq!(s.checksum(0xFFFF_FFFF), (!1u32).wrapping_add(!2u32));
    }
}
