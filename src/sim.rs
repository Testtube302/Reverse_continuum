//! Fixed-tick ship simulation. Units: positions in tiles, velocity in tiles/second,
//! orientation in turns (0..1, 0 = facing up). One tick is 10 ms, matching the protocol.
//!
//! Movement and wall-bounce rules follow the behaviour documented by the open-source
//! nullspace client (MIT, plushmonkey).

use crate::lvl::Map;
use crate::settings::{ArenaSettings, ShipSettings};

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
    fn axis(self, a: usize) -> f32 {
        if a == 0 { self.x } else { self.y }
    }
    fn axis_mut(&mut self, a: usize) -> &mut f32 {
        if a == 0 { &mut self.x } else { &mut self.y }
    }
    /// Snaps to the 1/16-tile (one pixel) grid, truncating toward zero.
    fn pixel_rounded(self) -> Self {
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
}

#[derive(Clone, Debug)]
pub struct Ship {
    pub ship: u8,
    pub position: Vec2,
    pub velocity: Vec2,
    pub orientation: f32,
    pub energy: f32,
    pub thrusting: bool,
    tick: u32,
    last_bounce_tick: Option<u32>,
}

impl Ship {
    pub fn new(ship: u8, position: Vec2, settings: &ArenaSettings) -> Self {
        Self {
            ship,
            position,
            velocity: Vec2::default(),
            orientation: 0.0,
            energy: settings.ships[ship as usize].initial_energy as f32,
            thrusting: false,
            tick: 0,
            last_bounce_tick: None,
        }
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

        let rotation = s.initial_rotation as f32 / 400.0;
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
        let thrust = if afterburner { s.maximum_thrust } else { s.initial_thrust } as f32 * (10.0 / 16.0) * dt;
        let h = heading(self.direction());
        if input.forward {
            self.velocity.x += h.x * thrust;
            self.velocity.y += h.y * thrust;
        } else if input.backward {
            self.velocity.x -= h.x * thrust;
            self.velocity.y -= h.y * thrust;
        }
        self.thrusting = thrusting;

        let speed = if afterburner { s.maximum_speed } else { s.initial_speed };
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
        self.energy = (self.energy + s.initial_recharge as f32 / 10.0 * dt).min(s.initial_energy as f32);

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
