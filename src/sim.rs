//! Fixed-tick ship simulation. Units: positions in tiles, velocity in tiles/second,
//! orientation in turns (0..1, 0 = facing up). One tick is 10 ms, matching the protocol.
//!
//! Movement and wall-bounce rules follow the behaviour documented by the open-source
//! nullspace client (MIT, plushmonkey).

use crate::lvl::Map;
use crate::settings::{ArenaSettings, ShipSettings};
use crate::weapons::{WeaponData, WeaponKind};

pub const TICK_SECONDS: f32 = 0.01;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
    pub fn scale(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k)
    }
    pub fn rotate(self, rads: f32) -> Self {
        let (s, c) = rads.sin_cos();
        Self::new(c * self.x - s * self.y, s * self.x + c * self.y)
    }
    fn axis(self, a: usize) -> f32 {
        if a == 0 { self.x } else { self.y }
    }
    fn axis_mut(&mut self, a: usize) -> &mut f32 {
        if a == 0 { &mut self.x } else { &mut self.y }
    }
    /// Snaps to the 1/16-tile (one pixel) grid, truncating toward zero.
    pub fn pixel_rounded(self) -> Self {
        Self::new((self.x * 16.0) as i32 as f32 / 16.0, (self.y * 16.0) as i32 as f32 / 16.0)
    }
}

/// Unit heading for one of the 40 discrete ship rotations (0 = up, increasing clockwise).
pub fn heading(direction: u8) -> Vec2 {
    // Integer remainder truncates toward zero, as in the reference implementation.
    let step = (40 - (direction as i32 + 30)) % 40;
    let rads = (step as f32 * 9.0).to_radians();
    Vec2::new(rads.cos(), -rads.sin())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub forward: bool,
    pub backward: bool,
    pub afterburner: bool,
    pub gun: bool,
    pub bomb: bool,
    pub mine: bool,
}

#[derive(Clone, Debug)]
pub struct Ship {
    pub ship: u8,
    pub position: Vec2,
    pub velocity: Vec2,
    pub orientation: f32,
    pub energy: f32,
    pub thrusting: bool,
    /// Gun and bomb levels, 1-based (0 = none).
    pub guns: u32,
    pub bombs: u32,
    pub multifire: bool,
    pub shrapnel: u32,
    pub bounty: u32,
    /// Current (prize-upgradable) stats, in the same units as the settings.
    pub rotation: u32,
    pub thrust: u32,
    pub speed: u32,
    pub recharge: u32,
    pub max_energy: u32,
    pub caps: Capabilities,
    /// Item counts in prize order: repel, burst, decoy, thor, brick, rocket, portal.
    pub items: [u32; 7],
    /// Ticks remaining on timed effects.
    pub super_ticks: u32,
    pub shield_ticks: u32,
    pub shutdown_ticks: u32,
    tick: u32,
    last_bounce_tick: Option<u32>,
    next_bullet_tick: u32,
    next_bomb_tick: u32,
}

/// Abilities granted by prizes (or by the ship's settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub bouncing_bullets: bool,
    pub multifire: bool,
    pub proximity: bool,
    pub stealth: bool,
    pub cloak: bool,
    pub xradar: bool,
    pub antiwarp: bool,
}

/// A weapon the ship decided to fire this tick, and its energy cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fire {
    pub data: WeaponData,
    pub cost: f32,
}

impl Ship {
    pub fn new(ship: u8, position: Vec2, settings: &ArenaSettings) -> Self {
        let s = &settings.ships[ship as usize];
        Self {
            ship,
            position,
            velocity: Vec2::default(),
            orientation: 0.0,
            energy: s.initial_energy as f32,
            thrusting: false,
            guns: s.initial_guns,
            bombs: s.initial_bombs,
            multifire: false,
            shrapnel: 0,
            bounty: s.initial_bounty,
            rotation: s.initial_rotation,
            thrust: s.initial_thrust,
            speed: s.initial_speed,
            recharge: s.initial_recharge,
            max_energy: s.initial_energy,
            caps: Capabilities {
                stealth: s.stealth_status == 2,
                cloak: s.cloak_status == 2,
                xradar: s.xradar_status == 2,
                antiwarp: s.antiwarp_status == 2,
                ..Default::default()
            },
            items: s.item_initial,
            super_ticks: 0,
            shield_ticks: 0,
            shutdown_ticks: 0,
            tick: 0,
            last_bounce_tick: None,
            next_bullet_tick: 0,
            next_bomb_tick: 0,
        }
    }

    /// Decides what (if anything) fires this tick, after `tick()` has moved the ship.
    /// The caller spawns the weapon and then calls [`Ship::commit_fire`].
    /// In a safe zone, trying to fire stops the ship instead.
    pub fn try_fire(&mut self, input: Input, settings: &ArenaSettings, in_safe: bool) -> Option<Fire> {
        let s = *self.stats(settings);
        let now = self.tick;
        let can_fast_shoot = !input.afterburner || !s.disable_fast_shooting;
        let ready = |next: u32| now.wrapping_sub(next) as i32 >= 0;
        let shrapnel = |d: WeaponData, guns: u32| {
            if guns > 0 { d.with_shrapnel(self.shrapnel as u16, (guns - 1) as u16, false) } else { d }
        };

        let bomb_kind = if self.caps.proximity { WeaponKind::ProximityBomb } else { WeaponKind::Bomb };
        let mut fire = None;
        if input.gun && ready(self.next_bullet_tick) && self.guns > 0 && can_fast_shoot {
            let level = (self.guns - 1) as u16;
            let kind = if self.caps.bouncing_bullets { WeaponKind::BouncingBullet } else { WeaponKind::Bullet };
            let multifire = self.multifire && self.caps.multifire;
            let data = WeaponData::new(kind, level).with_alternate(multifire);
            let (delay, per_level) =
                if multifire { (s.multi_fire_delay, s.multi_fire_energy) } else { (s.bullet_fire_delay, s.bullet_fire_energy) };
            let cost = (per_level * (level as u32 + 1)) as f32;
            if cost < self.energy {
                self.next_bullet_tick = now + delay;
                self.next_bomb_tick = self.next_bomb_tick.max(self.next_bullet_tick);
                fire = Some(Fire { data, cost });
            }
        }
        if input.mine && ready(self.next_bomb_tick) && self.bombs > 0 {
            let level = (self.bombs - 1) as u16;
            let data = shrapnel(WeaponData::new(bomb_kind, level).with_alternate(true), self.guns);
            let cost = (s.landmine_fire_energy + s.landmine_fire_energy_upgrade * level as u32) as f32;
            if cost < self.energy {
                self.next_bomb_tick = now + s.bomb_fire_delay;
                if !s.emp_bomb {
                    self.next_bullet_tick = self.next_bomb_tick;
                }
                fire = Some(Fire { data, cost });
            }
        } else if input.bomb && ready(self.next_bomb_tick) && self.bombs > 0 && can_fast_shoot {
            let level = (self.bombs - 1) as u16;
            let data = shrapnel(WeaponData::new(bomb_kind, level), self.guns);
            let cost = (s.bomb_fire_energy + s.bomb_fire_energy_upgrade * level as u32) as f32;
            if cost < self.energy {
                // Recoil.
                let thrust = s.bomb_thrust as f32 / 100.0 * 10.0 / 16.0;
                let h = heading(self.direction());
                self.velocity = self.velocity.sub(h.scale(thrust));
                self.next_bomb_tick = now + s.bomb_fire_delay;
                if !s.emp_bomb {
                    self.next_bullet_tick = self.next_bomb_tick;
                }
                fire = Some(Fire { data, cost });
            }
        }

        let fire = fire?;
        if in_safe {
            self.velocity = Vec2::default();
            return None;
        }
        Some(fire)
    }

    /// Pays for a weapon that was actually spawned.
    pub fn commit_fire(&mut self, fire: Fire) {
        self.energy -= fire.cost;
    }

    /// Sprite frame / heading index, 0..40.
    pub fn direction(&self) -> u8 {
        (self.orientation * 40.0) as u8
    }

    fn stats<'a>(&self, settings: &'a ArenaSettings) -> &'a ShipSettings {
        &settings.ships[self.ship as usize]
    }

    /// Advances one 10 ms tick. Returns true if the ship hit a wall.
    pub fn tick(&mut self, map: &Map, settings: &ArenaSettings, input: Input) -> bool {
        let dt = TICK_SECONDS;
        let s = *self.stats(settings);

        // An engine shutdown leaves only a slow turn and no thrust.
        let shutdown = self.shutdown_ticks > 0;
        self.shutdown_ticks = self.shutdown_ticks.saturating_sub(1);
        self.super_ticks = self.super_ticks.saturating_sub(1);
        self.shield_ticks = self.shield_ticks.saturating_sub(1);
        let rotation = if shutdown { 40 } else { self.rotation } as f32 / 400.0;
        if input.left {
            self.orientation -= rotation * dt;
            if self.orientation < 0.0 {
                self.orientation += 1.0;
            }
        }
        if input.right {
            self.orientation += rotation * dt;
            if self.orientation >= 1.0 {
                self.orientation -= 1.0;
            }
        }

        let thrusting = input.forward || input.backward;
        // Energy rates are per 10 seconds.
        let ab_cost = s.afterburner_energy as f32 / 10.0 * dt;
        let afterburner = input.afterburner && thrusting && self.energy > ab_cost;
        let thrust = if shutdown { 0 } else if afterburner { s.maximum_thrust } else { self.thrust } as f32 * (10.0 / 16.0) * dt;
        let h = heading(self.direction());
        if input.forward {
            self.velocity.x += h.x * thrust;
            self.velocity.y += h.y * thrust;
        } else if input.backward {
            self.velocity.x -= h.x * thrust;
            self.velocity.y -= h.y * thrust;
        }
        self.thrusting = thrusting;

        let speed = if afterburner { s.maximum_speed } else { self.speed };
        let max = speed as f32 / 10.0 / 16.0;
        let len = self.velocity.length();
        if len > max {
            self.velocity.x = self.velocity.x / len * max;
            self.velocity.y = self.velocity.y / len * max;
        }

        // Afterburner cost first, then recharge: a ship whose recharge exceeds the
        // afterburner cost stays at full energy.
        if afterburner {
            self.energy -= ab_cost;
        }
        self.energy = (self.energy + self.recharge as f32 / 10.0 * dt).min(self.max_energy as f32);

        let radius = s.radius_tiles();
        let bounce = settings.bounce_factor.max(1);
        let bx = self.simulate_axis(map, 0, radius, bounce, dt);
        let by = self.simulate_axis(map, 1, radius, bounce, dt);
        if bx || by {
            self.last_bounce_tick = Some(self.tick);
        }
        self.tick = self.tick.wrapping_add(1);
        bx || by
    }

    fn simulate_axis(&mut self, map: &Map, axis: usize, radius: f32, bounce: u32, dt: f32) -> bool {
        let flip = 1 - axis;
        let previous = self.position.axis(axis);
        let delta = self.velocity.axis(axis) * dt;
        *self.position.axis_mut(axis) += delta;

        let pos = self.position.axis(axis);
        let check = if delta < 0.0 { (pos - radius).floor() as i32 } else { (pos + radius) as i32 };
        let start = (self.position.axis(flip) - radius - 1.0) as i32;
        let end = (self.position.axis(flip) + radius + 1.0) as i32;

        let rounded = self.position.pixel_rounded();
        let (min, max) = (Vec2::new(rounded.x - radius, rounded.y - radius), Vec2::new(rounded.x + radius, rounded.y + radius));

        let mut collided = !(0..=1023).contains(&check);
        let mut other = start;
        while !collided && other < end {
            let (tx, ty) = if axis == 0 { (check, other) } else { (other, check) };
            if map.is_solid(tx, ty) {
                let (tx, ty) = (tx as f32, ty as f32);
                if max.x >= tx && min.x < tx + 1.0 && max.y >= ty && min.y < ty + 1.0 {
                    collided = true;
                }
            }
            other += 1;
        }

        if collided {
            // Consecutive-tick bounces don't lose speed, so ships don't grind to a halt on walls.
            let factor = if self.last_bounce_tick.is_some_and(|t| self.tick.wrapping_sub(t) < 1) {
                1.0
            } else {
                16.0 / bounce as f32
            };
            *self.position.axis_mut(axis) = previous;
            *self.velocity.axis_mut(axis) *= -factor;
            *self.velocity.axis_mut(flip) *= factor;
        }
        collided
    }
}

/// Finds the free spot nearest to `near` that fits a ship of `radius` tiles.
pub fn find_spawn(map: &Map, near: Vec2, radius: f32) -> Vec2 {
    for r in 0..512i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let p = Vec2::new(near.x.floor() + dx as f32 + 0.5, near.y.floor() + dy as f32 + 0.5);
                if map.can_fit(p.x, p.y, radius) {
                    return p;
                }
            }
        }
    }
    near
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lvl::Map;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn heading_cardinals() {
        let up = heading(0);
        assert!(approx(up.x, 0.0) && approx(up.y, -1.0));
        let right = heading(10);
        assert!(approx(right.x, 1.0) && approx(right.y, 0.0));
        let down = heading(20);
        assert!(approx(down.x, 0.0) && approx(down.y, 1.0));
        let left = heading(30);
        assert!(approx(left.x, -1.0) && approx(left.y, 0.0));
    }

    #[test]
    fn rotation_full_turn_takes_400_over_rate_seconds() {
        let settings = ArenaSettings::default();
        let map = Map::empty();
        let mut s = Ship::new(0, Vec2::new(512.5, 512.5), &settings);
        // Warbird InitialRotation 210: a quarter turn takes 400/210/4 s ≈ 47.6 ticks.
        let input = Input { right: true, ..Default::default() };
        for _ in 0..48 {
            s.tick(&map, &settings, input);
        }
        assert_eq!(s.direction(), 10);
    }

    #[test]
    fn thrust_accelerates_and_caps_at_speed() {
        let settings = ArenaSettings::default();
        let map = Map::empty();
        let mut s = Ship::new(0, Vec2::new(512.5, 512.5), &settings);
        let input = Input { forward: true, ..Default::default() };
        s.tick(&map, &settings, input);
        // Warbird thrust 16 => 10 tiles/s^2 => 0.1 tiles/s after one tick, straight up.
        assert!(approx(s.velocity.y, -0.1) && approx(s.velocity.x, 0.0));
        for _ in 0..2000 {
            s.tick(&map, &settings, input);
        }
        assert!(approx(s.velocity.length(), 2010.0 / 160.0));
        assert!(s.position.y < 400.0);
    }

    #[test]
    fn afterburner_raises_cap_and_drains_energy() {
        let settings = ArenaSettings::default();
        let map = Map::empty();
        let mut s = Ship::new(0, Vec2::new(512.5, 900.5), &settings);
        let input = Input { forward: true, afterburner: true, ..Default::default() };
        for _ in 0..100 {
            s.tick(&map, &settings, input);
        }
        assert!(s.energy < 1000.0);
        assert!(s.velocity.length() > 2010.0 / 160.0 * 0.5);
    }

    #[test]
    fn wall_bounce_reverses_and_damps_velocity() {
        let settings = ArenaSettings::default();
        let mut d = Vec::new();
        for y in 500..525u32 {
            d.extend_from_slice(&(520 | (y << 12) | (1 << 24)).to_le_bytes());
        }
        let map = Map::parse(&d).unwrap();
        let mut s = Ship::new(0, Vec2::new(515.5, 512.5), &settings);
        s.velocity = Vec2::new(10.0, 1.0);
        let mut bounced = false;
        for _ in 0..100 {
            if s.tick(&map, &settings, Input::default()) {
                bounced = true;
                break;
            }
        }
        assert!(bounced);
        let f = 16.0 / 22.0;
        assert!(approx(s.velocity.x, -10.0 * f) && approx(s.velocity.y, 1.0 * f));
        // The ship is restored to its pre-collision position, so it never overlaps the wall.
        assert!(s.position.x + 14.0 / 16.0 <= 520.0 + 1.0 / 16.0);
    }

    #[test]
    fn gun_respects_fire_delay_and_costs_energy() {
        let settings = ArenaSettings::default();
        let map = Map::empty();
        let mut s = Ship::new(0, Vec2::new(512.5, 512.5), &settings);
        let input = Input { gun: true, ..Default::default() };
        let mut shots = 0;
        for _ in 0..100 {
            s.tick(&map, &settings, Input::default());
            if let Some(f) = s.try_fire(input, &settings, false) {
                assert_eq!(f.data.kind(), WeaponKind::Bullet);
                assert_eq!(f.cost, 20.0);
                s.commit_fire(f);
                shots += 1;
            }
        }
        // BulletFireDelay 25 ticks over 1 second: shots at ticks 1, 26, 51, 76.
        assert_eq!(shots, 4);
        assert!(s.energy < 1000.0);
    }

    #[test]
    fn bomb_carries_shrapnel_and_needs_bomb_level() {
        let settings = ArenaSettings::default();
        let mut s = Ship::new(0, Vec2::new(512.5, 512.5), &settings);
        s.shrapnel = 6;
        s.guns = 2;
        let f = s.try_fire(Input { bomb: true, ..Default::default() }, &settings, false).unwrap();
        assert_eq!((f.data.kind(), f.data.shrap(), f.data.shrap_level(), f.data.alternate()), (WeaponKind::Bomb, 6, 1, false));
        s.bombs = 0;
        let mut s2 = s.clone();
        s2.tick = 1000;
        assert!(s2.try_fire(Input { bomb: true, ..Default::default() }, &settings, false).is_none());
    }

    #[test]
    fn firing_in_safe_zone_stops_ship_instead() {
        let settings = ArenaSettings::default();
        let mut s = Ship::new(0, Vec2::new(512.5, 512.5), &settings);
        s.velocity = Vec2::new(3.0, 0.0);
        assert!(s.try_fire(Input { gun: true, ..Default::default() }, &settings, true).is_none());
        assert_eq!(s.velocity, Vec2::default());
    }

    #[test]
    fn spawn_avoids_walls() {
        let mut d = Vec::new();
        for y in 508..517u32 {
            for x in 508..517u32 {
                d.extend_from_slice(&(x | (y << 12) | (1 << 24)).to_le_bytes());
            }
        }
        let map = Map::parse(&d).unwrap();
        let p = find_spawn(&map, Vec2::new(512.0, 512.0), 14.0 / 16.0);
        assert!(map.can_fit(p.x, p.y, 14.0 / 16.0));
    }
}
