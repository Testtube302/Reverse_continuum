//! Prizes ("greens"): deterministic spawning shared by every client through the server's prize
//! seed, pickup, and the effect of each prize on a ship. Rules follow the behaviour documented
//! by the nullspace client (MIT, plushmonkey).

use crate::lvl::Map;
use crate::net::vie::VieRng;
use crate::settings::ArenaSettings;
use crate::sim::{Ship, Vec2};

pub const RECHARGE: i16 = 1;
pub const ENERGY: i16 = 2;
pub const ROTATION: i16 = 3;
pub const STEALTH: i16 = 4;
pub const CLOAK: i16 = 5;
pub const XRADAR: i16 = 6;
pub const WARP: i16 = 7;
pub const GUNS: i16 = 8;
pub const BOMBS: i16 = 9;
pub const BOUNCING_BULLETS: i16 = 10;
pub const THRUSTER: i16 = 11;
pub const TOP_SPEED: i16 = 12;
pub const FULL_CHARGE: i16 = 13;
pub const ENGINE_SHUTDOWN: i16 = 14;
pub const MULTIFIRE: i16 = 15;
pub const PROXIMITY: i16 = 16;
pub const SUPER: i16 = 17;
pub const SHIELDS: i16 = 18;
pub const SHRAPNEL: i16 = 19;
pub const ANTIWARP: i16 = 20;
pub const REPEL: i16 = 21;
pub const BURST: i16 = 22;
pub const DECOY: i16 = 23;
pub const THOR: i16 = 24;
pub const MULTIPRIZE: i16 = 25;
pub const BRICK: i16 = 26;
pub const ROCKET: i16 = 27;
pub const PORTAL: i16 = 28;

const POSITIVE: [&str; 29] = [
    "", "Charge rate increased.", "Maximum energy level increased.", "Rotation speed increased.",
    "Stealth available.", "Cloak available.", "X-Radar available.", "Warp!", "Guns upgraded.",
    "Bombs upgraded.", "Bouncing bullets.", "Thrusters upgraded.", "Top speed increased.", "Full charge.",
    "Engines shut-down.", "MultiFire bullets.", "Proximity bombs.", "Temporary SuperPower!",
    "Temporary Shields.", "Shrapnel increased.", "AntiWarp available.", "Repeller increased.",
    "Burst increased.", "Decoy increased.", "Thor's hammer increased.", "MultiPrize!", "Brick increased.",
    "Rocket Increased.", "Portal increased.",
];
const NEGATIVE: [&str; 29] = [
    "", "Charge rate decreased.", "Maximum energy level decreased.", "Rotation speed decreased.",
    "Stealth lost.", "Cloak lost.", "X-Radar lost.", "Warp!", "Guns downgraded.", "Bombs downgraded.",
    "Bouncing bullets lost.", "Thrusters downgraded.", "Top speed reduced.", "Energy depleted.",
    "Engines shut-down (severe).", "MultiFire lost.", "Proximity bombs lost.", "", "", "Shrapnel reduced.",
    "AntiWarp lost.", "Repeller lost.", "Burst lost.", "Decoy lost.", "Thor's hammer lost.", "",
    "Brick lost.", "Rocket lost.", "Portal lost.",
];

/// Picks a prize by weight using the shared seed. Negative ids are bad prizes.
/// Returns 0 when the arena has no prize weights.
pub fn generate(seed: &mut u32, settings: &ArenaSettings, negative_allowed: bool) -> i16 {
    let total: u32 = settings.prize_weights.iter().map(|&w| w as u32).sum();
    if total == 0 {
        return 0;
    }
    let mut rng = VieRng::new(*seed);
    let mut random = rng.next();
    let mut result = 0;
    let mut weight = 0u32;
    for (i, &w) in settings.prize_weights.iter().enumerate() {
        weight += w as u32;
        if random % total < weight {
            random = rng.next();
            let negative_factor = settings.prize_negative_factor.max(1);
            result = if !negative_allowed || random % negative_factor != 0 { i as i16 + 1 } else { -(i as i16 + 1) };
            break;
        }
    }
    *seed = random;
    result
}

/// What applying a prize did, beyond changing the ship.
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub message: Option<String>,
    /// The ship must be moved to a fresh spawn point.
    pub warp: bool,
}

fn bump(value: &mut u32, step: u32, min: u32, max: u32, negative: bool) {
    *value = if negative { value.saturating_sub(step).max(min) } else { (*value + step).min(max) };
}

fn bump_item(value: &mut u32, max: u32, negative: bool) {
    *value = if negative { value.saturating_sub(1) } else { (*value + 1).min(max) };
}

/// Applies prize `id` (negative = bad) to `ship`. `seed` is the shared prize seed, consumed by
/// multiprizes and restored afterwards so all clients stay in sync. `random` supplies local
/// randomness (super duration).
pub fn apply(
    ship: &mut Ship,
    id: i16,
    settings: &ArenaSettings,
    seed: &mut u32,
    random: &mut dyn FnMut(u32) -> u32,
) -> Applied {
    let negative = id < 0;
    let mut prize = id.abs();
    if !negative {
        ship.bounty += 1;
    }
    let s = settings.ships[ship.ship.min(7) as usize];
    let mut out = Applied::default();
    let mut show = true;
    // Capability prizes the ship can never have turn into a (possibly negative) full charge.
    let capability = |has: &mut bool, status: u32, prize: &mut i16| {
        if negative {
            if !*has && status == 0 {
                *prize = FULL_CHARGE;
            }
            *has = false;
        } else if status > 0 {
            *has = true;
        } else {
            *prize = FULL_CHARGE;
        }
    };
    match prize {
        RECHARGE => bump(&mut ship.recharge, s.upgrade_recharge, s.initial_recharge, s.maximum_recharge, negative),
        ENERGY => bump(&mut ship.max_energy, s.upgrade_energy, s.initial_energy, s.maximum_energy, negative),
        ROTATION => bump(&mut ship.rotation, s.upgrade_rotation, s.initial_rotation, s.maximum_rotation, negative),
        THRUSTER => bump(&mut ship.thrust, s.upgrade_thrust, s.initial_thrust, s.maximum_thrust, negative),
        TOP_SPEED => bump(&mut ship.speed, s.upgrade_speed, s.initial_speed, s.maximum_speed, negative),
        GUNS => bump(&mut ship.guns, 1, s.initial_guns, s.max_guns, negative),
        BOMBS => bump(&mut ship.bombs, 1, s.initial_bombs, s.max_bombs, negative),
        STEALTH => capability(&mut ship.caps.stealth, s.stealth_status, &mut prize),
        CLOAK => capability(&mut ship.caps.cloak, s.cloak_status, &mut prize),
        XRADAR => capability(&mut ship.caps.xradar, s.xradar_status, &mut prize),
        ANTIWARP => capability(&mut ship.caps.antiwarp, s.antiwarp_status, &mut prize),
        BOUNCING_BULLETS => ship.caps.bouncing_bullets = !negative,
        MULTIFIRE => ship.caps.multifire = !negative,
        PROXIMITY => ship.caps.proximity = !negative,
        WARP => {
            if negative {
                prize = FULL_CHARGE;
            } else {
                out.warp = true;
            }
        }
        ENGINE_SHUTDOWN => ship.shutdown_ticks = settings.engine_shutdown_time * if negative { 3 } else { 1 },
        SUPER => {
            ship.super_ticks = ship.super_ticks.max(random(s.super_time.max(1)));
        }
        SHIELDS => ship.shield_ticks = s.shields_time,
        SHRAPNEL => {
            ship.shrapnel = if negative {
                ship.shrapnel.saturating_sub(s.shrapnel_rate)
            } else {
                (ship.shrapnel + s.shrapnel_rate).min(s.shrapnel_max)
            }
        }
        REPEL..=THOR => {
            let i = (prize - REPEL) as usize;
            bump_item(&mut ship.items[i], s.item_max[i], negative);
        }
        BRICK | ROCKET | PORTAL => {
            let i = (prize - BRICK) as usize + 4;
            bump_item(&mut ship.items[i], s.item_max[i], negative);
        }
        MULTIPRIZE if !negative => {
            let pristine = *seed;
            let (mut given, mut attempts) = (0, 0);
            while given < settings.multi_prize_count && attempts < 9999 {
                attempts += 1;
                let p = generate(seed, settings, false);
                if matches!(p, 0 | ENGINE_SHUTDOWN | SHIELDS | SUPER | MULTIPRIZE | WARP | BRICK) {
                    continue;
                }
                let bounty = ship.bounty;
                let _ = apply(ship, p, settings, seed, random);
                ship.bounty = bounty;
                given += 1;
            }
            *seed = pristine;
        }
        _ => show = false,
    }
    if prize == FULL_CHARGE {
        ship.energy = if negative { 1.0 } else { ship.max_energy as f32 };
    }
    if show && (1..=28).contains(&prize) {
        let text = if negative { NEGATIVE[prize as usize] } else { POSITIVE[prize as usize] };
        if !text.is_empty() {
            out.message = Some(text.to_string());
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Green {
    pub x: u16,
    pub y: u16,
    pub prize: i16,
    /// Ticks left before it disappears.
    pub ticks_left: u32,
}

/// The prizes lying on the map, spawned in lockstep with every other client.
#[derive(Default)]
pub struct Greens {
    pub list: Vec<Green>,
    /// The shared prize seed (from the server's security packet). 0 = not synced yet.
    pub seed: u32,
    ticks_since_spawn: u32,
}

/// Most greens that can exist at once.
pub const MAX_GREENS: usize = 256;

impl Greens {
    /// Resynchronises with the server: new seed, restart the spawn timer.
    pub fn sync(&mut self, seed: u32) {
        self.seed = seed;
        self.ticks_since_spawn = 0;
    }

    /// Advances one tick; `player_count` is everyone in the arena.
    pub fn tick(&mut self, map: &Map, settings: &ArenaSettings, player_count: usize) {
        for g in &mut self.list {
            g.ticks_left = g.ticks_left.saturating_sub(1);
        }
        self.list.retain(|g| g.ticks_left > 0);
        if self.seed == 0 {
            return;
        }
        self.ticks_since_spawn += 1;
        if self.ticks_since_spawn < settings.prize_delay {
            return;
        }
        self.ticks_since_spawn = 0;
        let max = ((settings.prize_factor as usize * player_count) / 1000).min(MAX_GREENS);
        let extent = (settings.minimum_virtual + settings.upgrade_virtual * player_count as u32).clamp(3, 1024);
        for _ in 0..settings.prize_hide_count {
            let mut rng = VieRng::new(self.seed);
            let x = (rng.next() % (extent - 2)) + 1 + (1024 - extent) / 2;
            let y = (rng.next() % (extent - 2)) + 1 + (1024 - extent) / 2;
            self.seed = rng.seed as u32;
            let prize = generate(&mut self.seed, settings, true);
            let mut rng = VieRng::new(self.seed);
            let duration_roll = rng.next();
            self.seed = rng.seed as u32;
            if prize != 0 && self.list.len() < max && map.tile(x as i32, y as i32) == 0 {
                let spread = settings.prize_max_exist.saturating_sub(settings.prize_min_exist);
                let duration = duration_roll % (spread + 1) + settings.prize_min_exist;
                self.list.push(Green { x: x as u16, y: y as u16, prize, ticks_left: duration });
            }
        }
    }

    /// Removes and returns a green touched by a ship of `radius` tiles at `position`.
    pub fn collect(&mut self, position: Vec2, radius: f32) -> Option<Green> {
        let i = self.list.iter().position(|g| {
            let (gx, gy) = (g.x as f32, g.y as f32);
            position.x + radius >= gx && position.x - radius < gx + 1.0 && position.y + radius >= gy && position.y - radius < gy + 1.0
        })?;
        Some(self.list.swap_remove(i))
    }

    /// Removes the green on a tile (someone else picked it up).
    pub fn remove_at(&mut self, x: u16, y: u16) {
        if let Some(i) = self.list.iter().position(|g| g.x == x && g.y == y) {
            self.list.swap_remove(i);
        }
    }

    pub fn add_death_green(&mut self, position: Vec2, prize: i16, settings: &ArenaSettings) {
        if self.list.len() < MAX_GREENS {
            let (x, y) = (position.x as u16, position.y as u16);
            self.list.push(Green { x, y, prize, ticks_left: settings.death_prize_time });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship() -> (Ship, ArenaSettings) {
        let s = ArenaSettings::default();
        (Ship::new(0, Vec2::new(512.0, 512.0), &s), s)
    }

    fn no_random(_: u32) -> u32 {
        0
    }

    #[test]
    fn generation_is_deterministic_and_weighted() {
        let s = ArenaSettings::default();
        let (mut a, mut b) = (12345u32, 12345u32);
        let seq_a: Vec<i16> = (0..50).map(|_| generate(&mut a, &s, true)).collect();
        let seq_b: Vec<i16> = (0..50).map(|_| generate(&mut b, &s, true)).collect();
        assert_eq!(seq_a, seq_b, "same seed, same prizes on every client");
        assert!(seq_a.iter().all(|&p| p != 0 && p.abs() <= 28));
        let mut only_guns = s.clone();
        only_guns.prize_weights = [0; 28];
        only_guns.prize_weights[(GUNS - 1) as usize] = 1;
        let mut seed = 99;
        assert!((0..20).all(|_| generate(&mut seed, &only_guns, false) == GUNS));
        only_guns.prize_weights = [0; 28];
        assert_eq!(generate(&mut seed, &only_guns, true), 0);
    }

    #[test]
    fn upgrades_clamp_at_limits() {
        let (mut sh, s) = ship();
        let mut seed = 1;
        for _ in 0..10 {
            apply(&mut sh, GUNS, &s, &mut seed, &mut no_random);
        }
        assert_eq!(sh.guns, s.ships[0].max_guns);
        apply(&mut sh, -GUNS, &s, &mut seed, &mut no_random);
        assert_eq!(sh.guns, s.ships[0].max_guns - 1);
        let r = apply(&mut sh, ROTATION, &s, &mut seed, &mut no_random);
        assert_eq!(sh.rotation, s.ships[0].initial_rotation + s.ships[0].upgrade_rotation);
        assert_eq!(r.message.as_deref(), Some("Rotation speed increased."));
        for _ in 0..20 {
            apply(&mut sh, SHRAPNEL, &s, &mut seed, &mut no_random);
        }
        assert_eq!(sh.shrapnel, s.ships[0].shrapnel_max);
        assert_eq!(sh.bounty, 10 + 1 + 20, "good prizes raise bounty");
    }

    #[test]
    fn capabilities_and_full_charge() {
        let (mut sh, mut s) = ship();
        let mut seed = 1;
        apply(&mut sh, MULTIFIRE, &s, &mut seed, &mut no_random);
        assert!(sh.caps.multifire);
        s.ships[0].cloak_status = 0;
        sh.energy = 10.0;
        apply(&mut sh, CLOAK, &s, &mut seed, &mut no_random);
        assert!(!sh.caps.cloak);
        assert_eq!(sh.energy, sh.max_energy as f32, "unavailable cloak becomes a full charge");
        apply(&mut sh, -FULL_CHARGE, &s, &mut seed, &mut no_random);
        assert_eq!(sh.energy, 1.0);
    }

    #[test]
    fn multiprize_restores_shared_seed() {
        let (mut sh, s) = ship();
        let mut seed = 777;
        let before = sh.clone();
        apply(&mut sh, MULTIPRIZE, &s, &mut seed, &mut no_random);
        assert_eq!(seed, 777);
        assert_eq!(sh.bounty, before.bounty + 1, "only the multiprize itself counts for bounty");
    }

    #[test]
    fn greens_spawn_on_schedule_and_identically() {
        let mut s = ArenaSettings::default();
        s.prize_delay = 10;
        let map = Map::empty();
        let (mut a, mut b) = (Greens::default(), Greens::default());
        a.sync(4242);
        b.sync(4242);
        for _ in 0..9 {
            a.tick(&map, &s, 3);
        }
        assert!(a.list.is_empty());
        a.tick(&map, &s, 3);
        for _ in 0..10 {
            b.tick(&map, &s, 3);
        }
        assert!(!a.list.is_empty());
        assert_eq!(a.list, b.list, "clients with the same seed agree");
        let extent = (s.minimum_virtual + s.upgrade_virtual * 3).clamp(3, 1024);
        let lo = (1024 - extent) / 2;
        assert!(a.list.iter().all(|g| (g.x as u32) > lo && (g.x as u32) < lo + extent));
    }

    #[test]
    fn collect_by_touch() {
        let mut g = Greens::default();
        g.list.push(Green { x: 100, y: 100, prize: GUNS, ticks_left: 50 });
        assert!(g.collect(Vec2::new(98.0, 100.5), 0.875).is_none());
        assert_eq!(g.collect(Vec2::new(99.5, 100.5), 0.875).map(|g| g.prize), Some(GUNS));
        assert!(g.list.is_empty());
    }
}
