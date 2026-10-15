//! Arena/ship settings. Online these arrive from the zone server in the 1428-byte 0x0F packet;
//! offline we use the standard SVS values (from Subspace Server .NET's `conf/svs`).
//! Times are in 1/100 s ticks, speeds in pixels per 10 s, unless noted.

pub const SHIP_NAMES: [&str; 8] =
    ["Warbird", "Javelin", "Spider", "Leviathan", "Terrier", "Weasel", "Lancaster", "Shark"];

#[derive(Clone, Copy, Debug, Default)]
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

    pub bullet_fire_energy: u32,
    pub multi_fire_energy: u32,
    pub bomb_fire_energy: u32,
    pub bomb_fire_energy_upgrade: u32,
    pub landmine_fire_energy: u32,
    pub landmine_fire_energy_upgrade: u32,
    pub bullet_speed: u32,
    pub bomb_speed: u32,
    pub disable_fast_shooting: bool,
    /// Multifire spread; 111 units per degree.
    pub multi_fire_angle: u32,
    /// Recoil applied when firing a bomb.
    pub bomb_thrust: u32,
    pub bullet_fire_delay: u32,
    pub multi_fire_delay: u32,
    pub bomb_fire_delay: u32,
    pub landmine_fire_delay: u32,
    pub initial_bounty: u32,
    pub damage_factor: u32,
    pub max_mines: u32,
    pub bomb_bounce_count: u32,
    pub shrapnel_max: u32,
    pub shrapnel_rate: u32,
    /// Gun/bomb levels are 1-based here (0 = none), max 3.
    pub initial_guns: u32,
    pub max_guns: u32,
    pub initial_bombs: u32,
    pub max_bombs: u32,
    pub double_barrel: bool,
    pub emp_bomb: bool,

    pub upgrade_rotation: u32,
    pub upgrade_thrust: u32,
    pub upgrade_speed: u32,
    pub upgrade_recharge: u32,
    pub upgrade_energy: u32,
    pub maximum_recharge: u32,
    /// Ticks.
    pub super_time: u32,
    pub shields_time: u32,
    pub prize_share_limit: u32,
    /// 0 = never, 1 = can be prized, 2 = starts with it.
    pub stealth_status: u32,
    pub cloak_status: u32,
    pub xradar_status: u32,
    pub antiwarp_status: u32,
    /// Item limits and starting counts, in prize order: repel, burst, decoy, thor, brick, rocket, portal.
    pub item_max: [u32; 7],
    pub item_initial: [u32; 7],
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
    /// Damage values are in 1/1000 energy units, as sent by the server.
    pub bullet_damage_level: i32,
    pub bullet_damage_upgrade: i32,
    pub bomb_damage_level: i32,
    pub inactive_shrap_damage: i32,
    pub bullet_alive_time: u32,
    pub bomb_alive_time: u32,
    /// Ticks a ship may stay in a safe zone (shown as "Safety Time Left"; 0 = no limit).
    pub safety_limit: u32,
    pub mine_alive_time: u32,
    pub shrapnel_speed: u32,
    pub bomb_explode_delay: u32,
    pub bomb_explode_pixels: u32,
    pub enter_delay: u32,
    pub proximity_distance: u32,
    pub bounty_increase_for_kill: u32,
    /// Percentages are in tenths of a percent (1000 = 100%).
    pub bbomb_damage_percent: u32,
    pub shrapnel_damage_percent: u32,
    pub exact_damage: bool,
    pub shrapnel_random: bool,
    pub team_max_mines: u32,
    pub bomb_safety: bool,
    pub engine_shutdown_time: u32,
    /// Radar scale: the whole map is drawn `screen width * 8 / map_zoom` pixels across.
    pub map_zoom_factor: u32,
    /// 0 = flags can't be carried; 1 = unlimited; n = up to n-1.
    pub carry_flags: u32,
    /// Ticks between door changes.
    pub door_delay: u32,
    /// -2 = random per step, -1 = weighted random, >= 0 = fixed door pattern.
    pub door_mode: i32,
    pub death_prize_time: u32,
    pub prize_factor: u32,
    pub prize_delay: u32,
    pub minimum_virtual: u32,
    pub upgrade_virtual: u32,
    pub prize_max_exist: u32,
    pub prize_min_exist: u32,
    pub prize_negative_factor: u32,
    pub multi_prize_count: u32,
    pub prize_hide_count: u32,
    pub take_prize_reliable: bool,
    /// Relative odds of prizes 1..=28.
    pub prize_weights: [u8; 28],
    /// The raw 0x0F settings packet when received from a server (needed for its checksum).
    pub raw: Vec<u8>,
}

/// Size of the S2C 0x0F arena settings packet.
pub const SETTINGS_PACKET_LEN: usize = 1428;
const SHIP_BLOCK_START: usize = 4;
const SHIP_BLOCK_LEN: usize = 144;

fn u8_at(p: &[u8], o: usize) -> u32 {
    p[o] as u32
}
fn u16_at(p: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([p[o], p[o + 1]]) as u32
}
fn u32_at(p: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

impl ShipSettings {
    /// Decodes one 144-byte ship block of the settings packet.
    fn from_block(b: &[u8]) -> Self {
        let bits16 = u16_at(b, 28);
        let bits32 = u32_at(b, 124);
        let field = |shift: u32, width: u32| (bits32 >> shift) & ((1 << width) - 1);
        Self {
            bullet_fire_energy: u16_at(b, 12),
            multi_fire_energy: u16_at(b, 14),
            bomb_fire_energy: u16_at(b, 16),
            bomb_fire_energy_upgrade: u16_at(b, 18),
            landmine_fire_energy: u16_at(b, 20),
            landmine_fire_energy_upgrade: u16_at(b, 22),
            bullet_speed: u16_at(b, 24),
            bomb_speed: u16_at(b, 26),
            disable_fast_shooting: bits16 & 0b100 != 0,
            radius: (bits16 >> 3) & 0xFF,
            multi_fire_angle: u16_at(b, 30),
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
            bomb_thrust: u16_at(b, 72),
            bullet_fire_delay: u16_at(b, 80),
            multi_fire_delay: u16_at(b, 82),
            bomb_fire_delay: u16_at(b, 84),
            landmine_fire_delay: u16_at(b, 86),
            initial_bounty: u16_at(b, 90),
            damage_factor: u16_at(b, 92),
            max_mines: u8_at(b, 108),
            bomb_bounce_count: u8_at(b, 123),
            shrapnel_max: field(0, 5),
            shrapnel_rate: field(5, 5),
            initial_guns: field(18, 2),
            max_guns: field(20, 2),
            initial_bombs: field(22, 2),
            max_bombs: field(24, 2),
            double_barrel: field(26, 1) != 0,
            emp_bomb: field(27, 1) != 0,
            super_time: u32_at(b, 0),
            shields_time: u32_at(b, 4),
            maximum_recharge: u16_at(b, 46),
            upgrade_rotation: u16_at(b, 60),
            upgrade_thrust: u16_at(b, 62),
            upgrade_speed: u16_at(b, 64),
            upgrade_recharge: u16_at(b, 66),
            upgrade_energy: u16_at(b, 68),
            prize_share_limit: u16_at(b, 94),
            cloak_status: field(10, 2),
            stealth_status: field(12, 2),
            xradar_status: field(14, 2),
            antiwarp_status: field(16, 2),
            // Byte layout: RepelMax 109, BurstMax, DecoyMax, ThorMax, BrickMax, RocketMax, PortalMax,
            // then InitialRepel 116, InitialBurst, InitialBrick, InitialRocket, InitialThor, InitialDecoy, InitialPortal.
            item_max: [b[109], b[110], b[111], b[112], b[113], b[114], b[115]].map(u32::from),
            item_initial: [b[116], b[117], b[121], b[120], b[118], b[119], b[122]].map(u32::from),
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
        let i32_at = |o: usize| u32_at(p, o) as i32;
        Some(Self {
            ships,
            exact_damage: u32_at(p, 0) & (1 << 8) != 0,
            bullet_damage_level: i32_at(1156),
            bomb_damage_level: i32_at(1160),
            bullet_alive_time: u32_at(p, 1164),
            bomb_alive_time: u32_at(p, 1168),
            safety_limit: u32_at(p, 1176),
            mine_alive_time: u32_at(p, 1192),
            bullet_damage_upgrade: i32_at(1200),
            inactive_shrap_damage: i32_at(1220),
            shrapnel_speed: u32_at(p, 1232),
            bomb_explode_delay: u16_at(p, 1254),
            bomb_explode_pixels: u16_at(p, 1258),
            enter_delay: u16_at(p, 1264),
            proximity_distance: u16_at(p, 1268),
            bounty_increase_for_kill: u16_at(p, 1270),
            bounce_factor: u16_at(p, 1272),
            bbomb_damage_percent: u16_at(p, 1332),
            shrapnel_damage_percent: u16_at(p, 1334),
            shrapnel_random: p[1368] != 0,
            team_max_mines: u8_at(p, 1375),
            bomb_safety: p[1377] != 0,
            death_prize_time: u16_at(p, 1260),
            engine_shutdown_time: u16_at(p, 1266),
            map_zoom_factor: u16_at(p, 1274),
            carry_flags: u8_at(p, 1384),
            door_delay: u16_at(p, 1306),
            door_mode: u16_at(p, 1310) as u16 as i16 as i32,
            prize_factor: u16_at(p, 1292),
            prize_delay: u16_at(p, 1294),
            minimum_virtual: u16_at(p, 1296),
            upgrade_virtual: u16_at(p, 1298),
            prize_max_exist: u16_at(p, 1300),
            prize_min_exist: u16_at(p, 1302),
            prize_negative_factor: u16_at(p, 1304),
            multi_prize_count: u16_at(p, 1316),
            take_prize_reliable: p[1379] != 0,
            prize_hide_count: u8_at(p, 1381),
            prize_weights: p[1400..1428].try_into().unwrap(),
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
        bullet_fire_energy: 20,
        multi_fire_energy: 20,
        bomb_fire_energy: 300,
        bomb_fire_energy_upgrade: 0,
        landmine_fire_energy: 300,
        landmine_fire_energy_upgrade: 0,
        bullet_speed: 2000,
        bomb_speed: 2000,
        disable_fast_shooting: false,
        multi_fire_angle: 500,
        bomb_thrust: 0,
        bullet_fire_delay: 25,
        multi_fire_delay: 25,
        bomb_fire_delay: 150,
        landmine_fire_delay: 150,
        initial_bounty: 0,
        damage_factor: 0,
        max_mines: 5,
        bomb_bounce_count: 0,
        shrapnel_max: 8,
        shrapnel_rate: 2,
        initial_guns: 1,
        max_guns: 3,
        initial_bombs: 1,
        max_bombs: 3,
        double_barrel: false,
        emp_bomb: false,
        upgrade_rotation: 20,
        upgrade_thrust: 1,
        upgrade_speed: 150,
        upgrade_recharge: 166,
        upgrade_energy: 150,
        maximum_recharge: 1150,
        super_time: 600,
        shields_time: 2000,
        prize_share_limit: 0,
        stealth_status: 1,
        cloak_status: 1,
        xradar_status: 1,
        antiwarp_status: 1,
        item_max: [3, 2, 2, 1, 2, 2, 2],
        item_initial: [0; 7],
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
            bullet_damage_level: 100_000,
            bullet_damage_upgrade: 50_000,
            bomb_damage_level: 750_000,
            inactive_shrap_damage: 3_000,
            bullet_alive_time: 550,
            bomb_alive_time: 6000,
            safety_limit: 90000,
            mine_alive_time: 12000,
            shrapnel_speed: 3000,
            bomb_explode_delay: 150,
            bomb_explode_pixels: 80,
            enter_delay: 200,
            proximity_distance: 3,
            bounty_increase_for_kill: 0,
            bbomb_damage_percent: 1000,
            shrapnel_damage_percent: 1000,
            exact_damage: true,
            shrapnel_random: false,
            team_max_mines: 12,
            bomb_safety: true,
            death_prize_time: 1000,
            engine_shutdown_time: 300,
            map_zoom_factor: 8,
            carry_flags: 1,
            door_delay: 1000,
            door_mode: -1,
            prize_factor: 1500,
            prize_delay: 1000,
            minimum_virtual: 300,
            upgrade_virtual: 15,
            prize_max_exist: 18000,
            prize_min_exist: 1500,
            prize_negative_factor: 30,
            multi_prize_count: 4,
            prize_hide_count: 30,
            take_prize_reliable: true,
            prize_weights: [
                10, 10, 10, 3, 3, 3, 5, 15, 15, 8, 10, 10, 15, 3, 10, 5, 1, 1, 10, 2, 4, 2, 2, 1, 1, 2, 2, 2,
            ],
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
    fn parses_ship_blocks_and_arena_fields() {
        let mut p = vec![0u8; SETTINGS_PACKET_LEN];
        p[0] = 0x0F;
        p[1] = 1; // ExactDamage is bit 8 of the header word
        let javelin = SHIP_BLOCK_START + SHIP_BLOCK_LEN;
        put16(&mut p, javelin + 50, 200);
        put16(&mut p, javelin + 52, 15);
        put16(&mut p, javelin + 54, 2200);
        put16(&mut p, javelin + 28, 15 << 3 | 0b011);
        put16(&mut p, javelin + 24, 2000);
        put16(&mut p, javelin + 80, 25);
        // ShrapnelMax=8, ShrapnelRate=2, InitialGuns=1, MaxGuns=3, DoubleBarrel.
        let bits: u32 = 8 | 2 << 5 | 1 << 18 | 3 << 20 | 1 << 26;
        p[javelin + 124..javelin + 128].copy_from_slice(&bits.to_le_bytes());
        put16(&mut p, 1272, 22);
        p[1156..1160].copy_from_slice(&100_000i32.to_le_bytes());
        put16(&mut p, 1258, 80);
        let s = ArenaSettings::from_packet(&p).unwrap();
        let j = s.ships[1];
        assert_eq!((j.initial_rotation, j.initial_thrust, j.initial_speed, j.radius), (200, 15, 2200, 15));
        assert!(!j.disable_fast_shooting);
        assert_eq!((j.bullet_speed, j.bullet_fire_delay), (2000, 25));
        assert_eq!((j.shrapnel_max, j.shrapnel_rate, j.initial_guns, j.max_guns), (8, 2, 1, 3));
        assert!(j.double_barrel && !j.emp_bomb);
        assert_eq!((s.bounce_factor, s.bullet_damage_level, s.bomb_explode_pixels), (22, 100_000, 80));
        assert!(s.exact_damage);
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
