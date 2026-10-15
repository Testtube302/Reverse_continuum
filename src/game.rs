//! The game loop independent of any window: ships, weapons, damage, death and respawn,
//! optionally connected to a zone through a [`Session`]. Frontends call [`Game::frame`] once
//! per frame and [`Game::tick`] once per 10 ms, then draw the state and play `sounds`.

use std::sync::Arc;

use crate::lvl::{Map, TILE_SAFE};
use crate::net::session::{PrizeEvent, SPECTATOR, Session};
use crate::prizes::{self, Greens};
use crate::settings::ArenaSettings;
use crate::sim::{self, Input, Ship, Vec2};
use crate::weapons::{ExplosionKind, Shooter, Target, WeaponData, WeaponEvent, WeaponKind, Weapons, damage_to_local};

/// Ticks a ship explosion is shown: explode1's 34 frames at ~28 ms each, measured from the
/// original (visible 957-985 ms).
pub const SHIP_EXPLOSION_TICKS: u32 = 96;
/// Ticks from a death to the respawn: EnterDelay plus this (measured 3.03 s with EnterDelay=200).
pub const RESPAWN_EXTRA_TICKS: u32 = 100;
/// The warp-in flash over a respawning ship: warp.png's 16 frames, measured ~410-425 ms.
pub const WARP_TICKS: u32 = 42;
/// Ship debris: one piece per row of the ship's junk sheet, flying outward and tumbling.
pub const DEBRIS_PIECES: u8 = 9;
pub const DEBRIS_TICKS: u32 = 300;
/// explode0's 7 frames at ~70 ms (measured from single shots: visible ~470-490 ms).
pub const BULLET_EXPLOSION_TICKS: u32 = 49;
/// explode2's 44 frames at ~30 ms (measured 29.9 ms/frame).
pub const BOMB_EXPLOSION_TICKS: u32 = 132;
/// Bullets and shrapnel drop a trail dot every 2 ticks; each dot fades through
/// [`TRAIL_COLORS`], holding each shade for [`TRAIL_SHADE_TICKS`] (from the share of each
/// shade among ~1000 trail dots in the original: 5.0 dots per frame).
pub const TRAIL_INTERVAL_TICKS: u32 = 2;
pub const TRAIL_SHADE_TICKS: [u32; 6] = [5, 1, 4, 1, 1, 1];
/// Trail fades per bullet colour (red, yellow, blue, purple) and for burst pieces, newest first.
/// Red, blue and burst were measured from the original's 16-bit screen; yellow and purple are not.
pub const TRAIL_COLORS: [[[u8; 3]; 6]; 5] = [
    [[206, 8, 8], [165, 40, 0], [115, 24, 0], [99, 32, 0], [74, 44, 0], [57, 44, 8]],
    [[231, 199, 41], [206, 150, 8], [165, 105, 8], [115, 81, 0], [82, 56, 0], [57, 44, 8]],
    [[123, 121, 198], [99, 97, 165], [90, 89, 148], [66, 65, 90], [49, 56, 82], [33, 24, 49]],
    [[173, 105, 222], [140, 81, 181], [115, 65, 148], [82, 48, 107], [57, 32, 74], [41, 24, 49]],
    [[123, 121, 198], [115, 113, 181], [99, 97, 165], [74, 73, 107], [66, 65, 90], [33, 24, 49]],
];
/// Row of [`TRAIL_COLORS`] for burst pieces.
const BURST_TRAIL: u8 = 4;
/// Engine exhaust while thrusting: a pair of puffs every 8 ticks, 18 px behind the ship's
/// centre and 9 px apart, blown backward at ~135 px/s and slowing by 2.5% a tick, each playing
/// exhaust.png's 19 frames at ~28 ms (measured from the original's warbird; other ships use the
/// same engine layout for now).
pub const EXHAUST_INTERVAL_TICKS: u32 = 8;
pub const EXHAUST_TICKS: u32 = 53;
const EXHAUST_SPEED_PX: f32 = 135.0;
const EXHAUST_DRAG: f32 = 0.975;

/// Bombs leave a puff from trail.png every 5 ticks; each plays its row's 10 frames at ~28 ms
/// (measured from the original). Row = bomb level (red, yellow, blue, purple), grey for thors.
pub const BOMB_TRAIL_INTERVAL_TICKS: u32 = 5;
pub const BOMB_TRAIL_TICKS: u32 = 28;

/// Ticks a trail dot lasts.
pub const TRAIL_TICKS: u32 = 13;
/// How long centre-screen messages stay up (measured from the original: ~4 s).
pub const CENTER_MESSAGE_TICKS: u32 = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    BulletHit,
    Bomb,
    EmpBomb,
    ShipExplosion,
    Warp,
    /// repel.png: 10 frames over ~0.59 s.
    Repel,
}

impl EffectKind {
    pub fn duration(self) -> u32 {
        match self {
            Self::BulletHit => BULLET_EXPLOSION_TICKS,
            Self::Bomb | Self::EmpBomb => BOMB_EXPLOSION_TICKS,
            Self::ShipExplosion => SHIP_EXPLOSION_TICKS,
            Self::Warp => WARP_TICKS,
            Self::Repel => 59, // 10 frames at ~59 ms, measured
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Effect {
    pub kind: EffectKind,
    pub position: Vec2,
    pub age: u32,
}

/// A tumbling piece of a destroyed ship (`junk<ship>.png` row `piece`).
#[derive(Clone, Copy, Debug)]
pub struct Debris {
    pub ship: u8,
    pub piece: u8,
    pub position: Vec2,
    /// Tiles per tick.
    pub velocity: Vec2,
    pub age: u32,
    /// Rotation frame (0..10) at age 0, and ticks per frame (negative = turning the other way).
    pub frame0: u8,
    pub spin: i32,
}

impl Debris {
    pub fn frame(&self) -> u32 {
        let steps = self.age as i32 / self.spin.abs().max(1) * self.spin.signum();
        (self.frame0 as i32 + steps).rem_euclid(10) as u32
    }
}

/// A fading dot left behind a bullet or piece of shrapnel.
#[derive(Clone, Copy, Debug)]
pub struct TrailDot {
    pub position: Vec2,
    /// Row of [`TRAIL_COLORS`].
    pub palette: u8,
    pub age: u32,
}

impl TrailDot {
    pub fn color(&self) -> [u8; 3] {
        let shades = &TRAIL_COLORS[self.palette as usize % TRAIL_COLORS.len()];
        let mut age = self.age;
        for (shade, ticks) in shades.iter().zip(TRAIL_SHADE_TICKS) {
            if age < ticks {
                return *shade;
            }
            age -= ticks;
        }
        shades[shades.len() - 1]
    }
}

/// One puff of engine exhaust (row 0 of exhaust.png — also with the afterburner on, verified
/// against the original while its energy drained; what row 1 is for is unknown).
#[derive(Clone, Copy, Debug)]
pub struct Exhaust {
    pub position: Vec2,
    /// Tiles per tick.
    pub velocity: Vec2,
    pub age: u32,
    /// Rocket exhaust (rocket.png) rather than engine exhaust.
    pub rocket: bool,
}

impl Exhaust {
    /// Frame within a row of exhaust.png (19 frames at ~28 ms).
    pub fn frame(&self) -> u32 {
        (self.age * 10 / 28).min(18)
    }
}

/// One puff of a bomb's trail.
#[derive(Clone, Copy, Debug)]
pub struct BombPuff {
    pub position: Vec2,
    /// Row of trail.png.
    pub row: u8,
    pub age: u32,
}

impl BombPuff {
    pub fn frame(&self) -> u32 {
        (self.age * 10 / 28).min(9)
    }
}

/// A centre-screen message: arena enter/leave lines and kill messages.
#[derive(Clone, Debug, PartialEq)]
pub struct CenterMessage {
    pub text: String,
    /// Yellow (kills, refusals) rather than blue (arena enter/leave).
    pub yellow: bool,
    pub age: u32,
}

/// A sound to play, named after its `sound/<name>.wa2` file, at a map position.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundCue {
    pub name: String,
    pub position: Vec2,
}

pub struct Game {
    pub session: Option<Session>,
    offline_world: Option<(Arc<Map>, Arc<ArenaSettings>)>,
    pub ship: Option<Ship>,
    /// Ticks left before respawning after death.
    pub dead_ticks: u32,
    /// Ticks our ship has spent in a safe zone since it last entered one (0 = outside).
    pub safe_ticks: u32,
    /// Ticks since our last flag pickup while we carry flags, and the count then.
    flag_carry: (u32, u16),
    pub weapons: Weapons,
    pub greens: Greens,
    flag_ticks: u32,
    flag_requests: std::collections::HashMap<u16, u32>,
    pub effects: Vec<Effect>,
    pub debris: Vec<Debris>,
    pub trails: Vec<TrailDot>,
    pub exhaust: Vec<Exhaust>,
    pub bomb_puffs: Vec<BombPuff>,
    thrust_ticks: u32,
    pub sounds: Vec<SoundCue>,
    pub notices: Vec<String>,
    /// Spectator camera when not in a ship.
    pub camera: Vec2,
    pub kills: u32,
    /// While spectating, keep the camera on this player.
    pub follow: Option<String>,
    seen_door_generation: u32,
    followed_pid: Option<u16>,
    follow_released: bool,
    pub center_messages: Vec<CenterMessage>,
    pub deaths: u32,
    rng: u32,
}

const OFFLINE_PID: u16 = 0;

impl Game {
    pub fn offline(map: Map, settings: ArenaSettings, ship: u8) -> Self {
        let mut g = Self::new(None, Some((Arc::new(map), Arc::new(settings))));
        // Offline there's no server seed; any fixed seed gives a playable prize field.
        g.greens.sync(0x5EED_1234);
        let (map, settings) = g.world().unwrap();
        g.ship = Some(fresh_ship(map, settings, ship, 0, 0x5EED_1234));
        g.camera = g.ship.as_ref().unwrap().position;
        g
    }

    pub fn online(session: Session) -> Self {
        Self::new(Some(session), None)
    }

    fn new(session: Option<Session>, offline_world: Option<(Arc<Map>, Arc<ArenaSettings>)>) -> Self {
        Self {
            session,
            offline_world,
            ship: None,
            dead_ticks: 0,
            safe_ticks: 0,
            flag_carry: (0, 0),
            weapons: Weapons::default(),
            greens: Greens::default(),
            flag_ticks: 0,
            flag_requests: std::collections::HashMap::new(),
            effects: Vec::new(),
            debris: Vec::new(),
            trails: Vec::new(),
            exhaust: Vec::new(),
            bomb_puffs: Vec::new(),
            thrust_ticks: 0,
            sounds: Vec::new(),
            notices: Vec::new(),
            // Spectators start at the map's top-left corner, as in the original.
            camera: Vec2::new(0.0, 0.0),
            kills: 0,
            follow: None,
            seen_door_generation: 0,
            followed_pid: None,
            follow_released: false,
            center_messages: Vec::new(),
            deaths: 0,
            rng: 0x1234_5678,
        }
    }

    /// The map and settings in play, once known.
    pub fn world(&self) -> Option<(&Map, &ArenaSettings)> {
        match &self.session {
            Some(s) if s.ready() => Some((&**s.map.as_ref()?, &**s.settings.as_ref()?)),
            Some(_) => None,
            None => self.offline_world.as_ref().map(|(m, s)| (&**m, &**s)),
        }
    }

    fn world_handles(&self) -> Option<(Arc<Map>, Arc<ArenaSettings>)> {
        match &self.session {
            Some(s) if s.ready() => Some((s.map.clone()?, s.settings.clone()?)),
            Some(_) => None,
            None => self.offline_world.clone(),
        }
    }

    fn random(&mut self, n: u32) -> u32 {
        // xorshift32; gameplay randomness only.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if n == 0 { 0 } else { self.rng % n }
    }

    pub fn local_pid(&self) -> u16 {
        self.session.as_ref().and_then(|s| s.pid).unwrap_or(OFFLINE_PID)
    }

    fn local_freq(&self) -> u16 {
        self.session.as_ref().and_then(|s| s.me()).map_or(0, |m| m.freq)
    }

    /// Seconds left before our flags must be dropped, while we carry any (FlagDropDelay).
    pub fn flag_drop_seconds(&self) -> Option<f32> {
        let delay = self.world()?.1.flag_drop_delay;
        (delay > 0 && self.flag_carry.1 > 0).then(|| delay.saturating_sub(self.flag_carry.0) as f32 / 100.0)
    }

    /// "Safety Time Left" while our ship sits in a safe zone: (hours, minutes, seconds).
    pub fn safety_time_left(&self) -> Option<(u32, u32, u32)> {
        let limit = self.world()?.1.safety_limit;
        if limit == 0 || self.safe_ticks == 0 || !self.alive() {
            return None;
        }
        let secs = limit.saturating_sub(self.safe_ticks) / 100;
        Some((secs / 3600, secs / 60 % 60, secs % 60))
    }

    /// F7: attach to a teammate as a turret. Turrets aren't implemented yet; with too little
    /// bounty the original refuses with this yellow centre message (measured).
    pub fn request_attach(&mut self) {
        let Some((_, settings)) = self.world() else { return };
        let Some(ship) = self.ship.as_ref().filter(|_| self.alive()) else { return };
        if ship.bounty < settings.ships[ship.ship as usize].attach_bounty {
            let text = "Bounty not high enough to attach.".to_string();
            self.center_messages.push(CenterMessage { text, yellow: true, age: 0 });
        }
    }

    pub fn alive(&self) -> bool {
        self.ship.is_some() && self.dead_ticks == 0
    }

    /// Requests a ship (0-7) or spectator mode (8). Offline the change is immediate.
    pub fn request_ship(&mut self, ship: u8) {
        if let Some(s) = self.session.as_mut() {
            s.request_ship(ship);
            return;
        }
        let local = self.random(u32::MAX);
        let Some((map, settings)) = self.world() else { return };
        let new = (ship < 8).then(|| fresh_ship(map, settings, ship, 0, local));
        self.ship = new;
        self.dead_ticks = 0;
    }

    /// Per-frame work: network traffic and following the server's ship assignments.
    pub fn frame(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        s.update();
        // Apply new prize seeds immediately: a ship spawned below draws its starting prizes from it.
        let mut i = 0;
        while i < s.prize_events.len() {
            if let PrizeEvent::Seed(seed) = s.prize_events[i] {
                self.greens.sync(seed);
                s.prize_events.remove(i);
            } else {
                i += 1;
            }
        }
        // A door closing on our ship sends it back to a spawn point, as in the original.
        if s.door_generation != self.seen_door_generation {
            self.seen_door_generation = s.door_generation;
            let blocked = match (&self.ship, s.map.as_ref(), s.settings.as_ref()) {
                (Some(sh), Some(map), Some(set)) if self.dead_ticks == 0 => {
                    !map.can_fit(sh.position.x, sh.position.y, set.ships[sh.ship as usize].radius_tiles())
                }
                _ => false,
            };
            if blocked {
                let seed = self.random(u32::MAX);
                let s = self.session.as_ref().unwrap();
                let (map, settings) = (s.map.clone().unwrap(), s.settings.clone().unwrap());
                if let Some(sh) = self.ship.as_mut() {
                    let fresh = spawn_ship(&map, &settings, sh.ship, seed);
                    sh.position = fresh.position;
                    sh.velocity = Vec2::default();
                }
            }
        }
        let Some(s) = self.session.as_mut() else { return };
        for at in s.warp_flashes.drain(..) {
            self.effects.push(Effect { kind: EffectKind::Warp, position: at, age: 0 });
        }
        for m in s.arena_messages.drain(..) {
            self.center_messages.push(CenterMessage { text: m, yellow: false, age: 0 });
        }
        if self.ship.is_none() {
            if let Some(name) = &self.follow {
                if let Some(p) = s.players.values().find(|p| &p.name == name) {
                    let (pid, seen) = (p.pid, p.position_updates > 0);
                    if seen {
                        self.camera = p.position;
                    }
                    // A spectate request gets us the target's position wherever it is, but this
                    // server then relays only that player. Once we know where it is, cancel it
                    // and rely on proximity: our reported position is the camera, on the target.
                    if self.followed_pid != Some(pid) && s.ready() && !seen {
                        self.followed_pid = Some(pid);
                        s.send_spectate_request(pid);
                    } else if self.followed_pid == Some(pid) && seen && !self.follow_released {
                        self.follow_released = true;
                        s.send_spectate_request(0xFFFF);
                    }
                }
            }
        }
        let Some(me) = s.me().map(|m| m.ship) else { return };
        if !s.ready() {
            return;
        }
        match (&self.ship, me) {
            (_, SPECTATOR) => self.ship = None,
            (Some(cur), n) if cur.ship == n => {}
            (_, n) if n < 8 => {
                let seed = self.random(u32::MAX);
                let (map, settings) = self.world().unwrap();
                self.ship = Some(fresh_ship(map, settings, n, seed, seed.rotate_left(13) ^ 0xA5A5_5A5A));
                self.dead_ticks = 0;
            }
            _ => {}
        }
    }

    fn targets(&self) -> Vec<Target> {
        let mut t = Vec::new();
        if let (Some(ship), true) = (&self.ship, self.alive()) {
            t.push(Target { pid: self.local_pid(), freq: self.local_freq(), ship: ship.ship, position: ship.position });
        }
        if let Some(s) = &self.session {
            for p in s.players.values() {
                if Some(p.pid) != s.pid && p.ship < 8 && !p.dead && p.position_updates > 0 {
                    t.push(Target { pid: p.pid, freq: p.freq, ship: p.ship, position: p.position });
                }
            }
        }
        t
    }

    fn owners_in_safe(&self, map: &Map) -> Vec<u16> {
        let in_safe = |p: Vec2| map.tile(p.x as i32, p.y as i32) == TILE_SAFE;
        let mut v = Vec::new();
        if let Some(ship) = &self.ship {
            if in_safe(ship.position) {
                v.push(self.local_pid());
            }
        }
        if let Some(s) = &self.session {
            v.extend(s.players.values().filter(|p| Some(p.pid) != s.pid && in_safe(p.position)).map(|p| p.pid));
        }
        v
    }

    /// Advances the game by one 10 ms tick. Returns the weapon we fired, if any.
    pub fn tick(&mut self, input: Input) -> WeaponData {
        // Shared handles keep the borrow checker out of the way of the &mut self calls below.
        let Some((map, settings)) = self.world_handles() else { return WeaponData::default() };
        let mut fired = WeaponData::default();
        let (local_pid, local_freq) = (self.local_pid(), self.local_freq());

        // Flag-drop clock: restarts whenever we pick up another flag.
        let carried = self.session.as_ref().and_then(|s| s.me()).map_or(0, |m| m.flags_carried);
        self.flag_carry = match (carried, self.flag_carry) {
            (0, _) => (0, 0),
            (n, (_, before)) if n > before => (0, n),
            (n, (t, _)) => (t + 1, n),
        };

        // Our ship: respawn countdown, or movement and firing.
        if self.dead_ticks > 0 {
            self.dead_ticks -= 1;
            self.safe_ticks = 0;
            if self.dead_ticks == 0 {
                let seed = self.random(u32::MAX);
                if let Some(kind) = self.ship.as_ref().map(|s| s.ship) {
                    let ship = fresh_ship(&map, &settings, kind, seed, seed.rotate_left(13) ^ 0xA5A5_5A5A);
                    self.effects.push(Effect { kind: EffectKind::Warp, position: ship.position, age: 0 });
                    self.ship = Some(ship);
                }
            }
        } else if let Some(ship) = self.ship.as_mut() {
            if ship.tick(&map, &settings, input) {
                self.sounds.push(SoundCue { name: "bounce".into(), position: ship.position });
            }
            let in_safe = map.tile(ship.position.x as i32, ship.position.y as i32) == TILE_SAFE;
            if ship.rocket_ticks > 0 {
                // Rocket exhaust: three puffs from rocket.png behind the ship (reference client).
                if self.thrust_ticks % EXHAUST_INTERVAL_TICKS == 0 {
                    let a = ship.orientation * std::f32::consts::TAU;
                    let (fx, fy) = (a.sin(), -a.cos());
                    for side in [-0.3f32, 0.0, 0.3] {
                        let back = if side == 0.0 { 0.3 } else { 0.2 } * 0.6;
                        let position = Vec2::new(
                            ship.position.x - fx * back - fy * side,
                            ship.position.y - fy * back + fx * side,
                        );
                        let (vx, vy) = (-fx - fy * side, -fy + fx * side);
                        let len = (vx * vx + vy * vy).sqrt().max(0.001);
                        let velocity = Vec2::new(vx / len * 0.6 / 100.0, vy / len * 0.6 / 100.0);
                        self.exhaust.push(Exhaust { position, velocity, age: 0, rocket: true });
                    }
                }
                self.thrust_ticks += 1;
            } else if input.forward || input.backward {
                if self.thrust_ticks % EXHAUST_INTERVAL_TICKS == 0 {
                    let a = ship.orientation * std::f32::consts::TAU;
                    let (fx, fy) = (a.sin(), -a.cos());
                    for side in [-4.5f32, 4.5] {
                        // 18 px behind the centre, 4.5 px to either side, in tiles.
                        let (x, y) = (-fx * 18.0 - fy * side, -fy * 18.0 + fx * side);
                        let position = Vec2::new(ship.position.x + x / 16.0, ship.position.y + y / 16.0);
                        let speed = EXHAUST_SPEED_PX / 16.0 / 100.0;
                        let velocity = Vec2::new(-fx * speed, -fy * speed);
                        self.exhaust.push(Exhaust { position, velocity, age: 0, rocket: false });
                    }
                }
                self.thrust_ticks += 1;
            } else {
                self.thrust_ticks = 0;
            }
            // The safety clock restarts from SafetyLimit on every entry (measured).
            self.safe_ticks = if in_safe { self.safe_ticks + 1 } else { 0 };
            let old_pos = ship.position;
            let used = ship.use_utilities(input, &settings, in_safe);
            let (pos, kind) = (ship.position, ship.ship);
            self.apply_utilities(used, old_pos, pos, kind, &map, &settings);
            let ship = self.ship.as_mut().unwrap();
            if let Some(f) = ship.try_fire(input, &settings, in_safe) {
                let shooter = Shooter {
                    pid: local_pid,
                    freq: local_freq,
                    ship: ship.ship,
                    direction: ship.direction(),
                    x: (ship.position.x * 16.0) as u32,
                    y: (ship.position.y * 16.0) as u32,
                    vel_x: (ship.velocity.x * 160.0) as i32,
                    vel_y: (ship.velocity.y * 160.0) as i32,
                };
                let ship_pos = ship.position;
                if self.mine_allowed(f.data, ship_pos, &settings) {
                    let ship = self.ship.as_mut().unwrap();
                    if ship.super_ticks == 0 {
                        ship.commit_fire(f);
                    }
                    self.sounds.push(SoundCue { name: fire_sound(f.data), position: ship_pos });
                    let targets = self.targets();
                    let events = self.weapons.fire(shooter, f.data, 0, &map, &settings, &targets, Some(shooter.pid));
                    self.handle_events(events, &map, &settings);
                    fired = f.data;
                }
            }
        } else {
            let d = 40.0 * sim::TICK_SECONDS;
            self.camera.x = (self.camera.x + (input.right as i32 - input.left as i32) as f32 * d).clamp(0.0, 1024.0);
            self.camera.y = (self.camera.y + (input.backward as i32 - input.forward as i32) as f32 * d).clamp(0.0, 1024.0);
        }
        if let (Some(ship), true) = (&self.ship, self.dead_ticks == 0) {
            self.camera = ship.position;
        }

        // Other players: extrapolate, spawn their shots, show their deaths.
        let (fires, deaths) = match self.session.as_mut() {
            Some(s) => {
                s.tick_players();
                (std::mem::take(&mut s.remote_fires), std::mem::take(&mut s.deaths))
            }
            None => (Vec::new(), Vec::new()),
        };
        let local = self.local_pid();
        for rf in fires {
            let targets = self.targets();
            let pos = Vec2::new(rf.shooter.x as f32 / 16.0, rf.shooter.y as f32 / 16.0);
            self.sounds.push(SoundCue { name: fire_sound(rf.data), position: pos });
            let events = self.weapons.fire(rf.shooter, rf.data, rf.catch_up, &map, &settings, &targets, Some(local));
            self.handle_events(events, &map, &settings);
        }
        for d in deaths {
            let text = format!("{}({}) killed by: {}", d.victim_name, d.bounty, d.killer_name);
            self.center_messages.push(CenterMessage { text, yellow: true, age: 0 });
            if d.killed == local {
                continue; // We already showed our own explosion.
            }
            if d.victim_ship < 8 {
                let (pos, vel) = (d.victim_position, d.victim_velocity);
                self.spawn_debris(d.victim_ship, pos, vel);
                self.effects.push(Effect { kind: EffectKind::ShipExplosion, position: pos, age: 0 });
                self.sounds.push(SoundCue { name: "explode1".into(), position: pos });
                // Only moving victims drop their prize.
                if d.green > 0 && (vel.x != 0.0 || vel.y != 0.0) {
                    self.greens.add_death_green(pos, d.green as i16, &settings);
                }
            }
            self.weapons.clear_owner(d.killed);
            if d.killer == local {
                self.kills += 1;
                if let Some(ship) = self.ship.as_mut() {
                    ship.bounty += settings.bounty_increase_for_kill;
                }
            }
        }

        self.tick_prizes(&map, &settings);
        self.tick_flags(&settings);

        // Weapons.
        let targets = self.targets();
        let safe = self.owners_in_safe(&map);
        let events = self.weapons.tick(&map, &settings, &targets, Some(local), &safe);
        self.handle_events(events, &map, &settings);

        for e in &mut self.effects {
            e.age += 1;
        }
        self.effects.retain(|e| e.age < e.kind.duration());
        for t in &mut self.trails {
            t.age += 1;
        }
        for e in &mut self.exhaust {
            e.age += 1;
            e.position.x += e.velocity.x;
            e.position.y += e.velocity.y;
            e.velocity.x *= EXHAUST_DRAG;
            e.velocity.y *= EXHAUST_DRAG;
        }
        self.exhaust.retain(|e| e.age < EXHAUST_TICKS);
        self.trails.retain(|t| t.age < TRAIL_TICKS);
        for p in &mut self.bomb_puffs {
            p.age += 1;
        }
        self.bomb_puffs.retain(|p| p.age < BOMB_TRAIL_TICKS);
        for w in &self.weapons.list {
            let row = match w.data.kind() {
                WeaponKind::Bomb | WeaponKind::ProximityBomb if !w.data.alternate() => Some(w.data.level().min(3) as u8),
                WeaponKind::Thor => Some(4),
                _ => None,
            };
            if let (Some(row), 0) = (row, w.age % BOMB_TRAIL_INTERVAL_TICKS) {
                self.bomb_puffs.push(BombPuff { position: w.position, row, age: 0 });
            }
        }
        for w in &self.weapons.list {
            let kind = w.data.kind();
            if !matches!(kind, WeaponKind::Bullet | WeaponKind::BouncingBullet | WeaponKind::Burst)
                || w.age % TRAIL_INTERVAL_TICKS != 0
            {
                continue;
            }
            // Same colour as the sprite: bullet level, or shrapnel level (red, yellow, blue).
            let palette = if kind == WeaponKind::Burst {
                BURST_TRAIL
            } else if w.data.shrap() > 0 {
                w.data.shrap_level() as u8
            } else {
                w.data.level() as u8
            };
            self.trails.push(TrailDot { position: w.position, palette, age: 0 });
        }
        for d in &mut self.debris {
            d.position.x += d.velocity.x;
            d.position.y += d.velocity.y;
            d.age += 1;
        }
        self.debris.retain(|d| d.age < DEBRIS_TICKS);
        for m in &mut self.center_messages {
            m.age += 1;
        }
        self.center_messages.retain(|m| m.age < CENTER_MESSAGE_TICKS);
        fired
    }

    /// Breaks a ship into tumbling pieces flying outward at 6-14 tiles/s (measured from the
    /// original), carried along with the ship's own velocity.
    fn spawn_debris(&mut self, ship: u8, position: Vec2, velocity: Vec2) {
        for piece in 0..DEBRIS_PIECES {
            let angle = self.random(3600) as f32 / 3600.0 * std::f32::consts::TAU;
            let speed = (6.0 + self.random(800) as f32 / 100.0) / 100.0;
            let spin = 3 + self.random(6) as i32;
            let frame0 = self.random(10) as u8;
            let spin = if self.random(2) == 0 { spin } else { -spin };
            self.debris.push(Debris {
                ship,
                piece,
                position,
                velocity: Vec2::new(velocity.x / 100.0 + angle.cos() * speed, velocity.y / 100.0 + angle.sin() * speed),
                age: 0,
                frame0,
                spin,
            });
        }
    }

    /// Effects of rocket, brick, portal and warp use: sounds, the brick request, warps.
    fn apply_utilities(&mut self, used: sim::Utility, old: Vec2, now: Vec2, ship: u8, map: &Map, settings: &ArenaSettings) {
        if used.brick {
            if let Some(s) = self.session.as_mut() {
                s.send_drop_brick(now.x as u16, now.y as u16);
            }
        }
        if used.rocket {
            self.sounds.push(SoundCue { name: "rocket1".into(), position: now });
        }
        if used.portal {
            self.sounds.push(SoundCue { name: "warppnt".into(), position: now });
        }
        let warped = match used.warp {
            Some(sim::WarpKind::NotEnoughEnergy) => {
                let text = "Not enough energy to warp.".to_string();
                self.center_messages.push(CenterMessage { text, yellow: true, age: 0 });
                false
            }
            Some(sim::WarpKind::ToPortal) => true,
            Some(sim::WarpKind::Respawn) => {
                // A warp lands on a fresh spawn point; the old spot gets the warp animation too.
                let seed = self.random(u32::MAX);
                let to = spawn_ship(map, settings, ship, seed).position;
                if let Some(s) = self.ship.as_mut() {
                    s.position = to;
                }
                self.effects.push(Effect { kind: EffectKind::Warp, position: old, age: 0 });
                true
            }
            None => false,
        };
        if warped {
            let at = self.ship.as_ref().map_or(now, |s| s.position);
            self.effects.push(Effect { kind: EffectKind::Warp, position: at, age: 0 });
            self.sounds.push(SoundCue { name: "warp".into(), position: at });
            if let Some(s) = self.session.as_mut() {
                s.pending_status |= 0x10;
            }
        }
    }

    fn tick_prizes(&mut self, map: &Map, settings: &ArenaSettings) {
        let local = self.local_pid();
        let events = self.session.as_mut().map(|s| std::mem::take(&mut s.prize_events)).unwrap_or_default();
        for e in events {
            match e {
                PrizeEvent::Seed(seed) => self.greens.sync(seed),
                PrizeEvent::Taken { x, y, prize, pid } => {
                    self.greens.remove_at(x, y);
                    // Teammates with a low bounty share prizes.
                    let team = self.session.as_ref().and_then(|s| s.players.get(&pid)).map(|p| p.freq);
                    if pid != local && prize != prizes::WARP && team == Some(self.local_freq()) {
                        if let Some(ship) = self.ship.as_ref().filter(|_| self.alive()) {
                            if ship.bounty < settings.ships[ship.ship as usize].prize_share_limit {
                                self.give_prize(prize, 1, map, settings, false);
                            }
                        }
                    }
                }
                PrizeEvent::Granted { prize, count } => {
                    self.sounds.push(SoundCue { name: "prize".into(), position: self.camera });
                    self.give_prize(prize, count, map, settings, true);
                }
            }
        }

        let players = self.session.as_ref().map_or(1, |s| s.players.len());
        self.greens.tick(map, settings, players);

        // Ships sweep up greens they touch; only ours applies the prize.
        if let (Some(ship), true) = (&self.ship, self.alive()) {
            let r = settings.ships[ship.ship as usize].radius_tiles();
            if let Some(g) = self.greens.collect(ship.position, r) {
                self.sounds.push(SoundCue { name: "prize".into(), position: ship.position });
                self.give_prize(g.prize, 1, map, settings, true);
                if let Some(s) = self.session.as_mut() {
                    s.send_take_green(g.x, g.y, g.prize);
                }
            }
        }
        for t in self.targets().into_iter().filter(|t| t.pid != local) {
            let _ = self.greens.collect(t.position, settings.ships[t.ship as usize].radius_tiles());
        }
    }

    /// Applies a prize to our ship without disturbing the shared green seed.
    fn give_prize(&mut self, prize: i16, count: u16, map: &Map, settings: &ArenaSettings, notify: bool) {
        let Some(mut ship) = self.ship.take() else { return };
        let mut seed = self.greens.seed;
        let warp_seed = self.random(u32::MAX);
        let mut local_seed = self.random(u32::MAX);
        let mut random = |n: u32| {
            local_seed = local_seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if n == 0 { 0 } else { local_seed % n }
        };
        for _ in 0..count {
            let applied = prizes::apply(&mut ship, prize, settings, &mut seed, &mut random);
            if applied.warp {
                let fresh = spawn_ship(map, settings, ship.ship, warp_seed);
                ship.position = fresh.position;
                ship.velocity = Vec2::default();
            }
            if let (true, Some(m)) = (notify, applied.message) {
                self.notices.push(m);
            }
        }
        self.ship = Some(ship);
    }

    /// Touching a dropped flag we don't own asks the server for it (at most every 3 s per flag).
    fn tick_flags(&mut self, settings: &ArenaSettings) {
        const REQUEST_DELAY: u32 = 300;
        self.flag_ticks = self.flag_ticks.wrapping_add(1);
        let (Some(ship), true) = (&self.ship, self.alive()) else { return };
        let freq = self.local_freq();
        let r = settings.ships[ship.ship as usize].radius_tiles() + 1.0 / 16.0;
        let pos = ship.position;
        let carried = self.session.as_ref().and_then(|s| s.me()).map_or(0, |m| m.flags_carried as u32);
        let carry = settings.carry_flags;
        if carry == 0 || (carry > 1 && carried >= carry - 1) {
            return;
        }
        let Some(s) = self.session.as_mut() else { return };
        let touching: Vec<u16> = s
            .flags
            .values()
            .filter(|f| f.dropped && f.owner != freq)
            .filter(|f| {
                let (fx, fy) = (f.x as f32, f.y as f32);
                pos.x + r >= fx && pos.x - r < fx + 1.0 && pos.y + r >= fy && pos.y - r < fy + 1.0
            })
            .map(|f| f.id)
            .collect();
        for id in touching {
            let last = self.flag_requests.get(&id).copied();
            if last.is_none_or(|t| self.flag_ticks.wrapping_sub(t) >= REQUEST_DELAY) {
                s.send_flag_request(id);
                self.flag_requests.insert(id, self.flag_ticks);
            }
        }
    }

    fn mine_allowed(&mut self, data: WeaponData, at: Vec2, settings: &ArenaSettings) -> bool {
        if !data.is_mine() {
            return true;
        }
        let local = self.local_pid();
        let mine_tile = |p: Vec2| (p.x as i32, p.y as i32);
        let mines: Vec<_> = self.weapons.list.iter().filter(|w| w.data.is_mine()).collect();
        if mines.iter().any(|w| mine_tile(w.position) == mine_tile(at)) {
            self.notices.push("You cannot lay mines on top of each other.".into());
            return false;
        }
        let ship = self.ship.as_ref().map_or(0, |s| s.ship as usize);
        if mines.iter().filter(|w| w.pid == local).count() >= settings.ships[ship].max_mines as usize {
            self.notices.push("You have too many mines.".into());
            return false;
        }
        true
    }

    fn handle_events(&mut self, events: Vec<WeaponEvent>, map: &Map, settings: &ArenaSettings) {
        for e in events {
            match e {
                WeaponEvent::Explosion { kind, position } => {
                    let (effect, sound) = match kind {
                        ExplosionKind::Bullet => (EffectKind::BulletHit, "explode0"),
                        ExplosionKind::Bomb => (EffectKind::Bomb, "explode2"),
                        ExplosionKind::EmpBomb => (EffectKind::EmpBomb, "ebombex"),
                    };
                    self.effects.push(Effect { kind: effect, position, age: 0 });
                    self.sounds.push(SoundCue { name: sound.into(), position });
                }
                WeaponEvent::HitLocal(w) => self.take_hit(&w, map, settings),
                WeaponEvent::Repel { position, pushes_local } => {
                    self.effects.push(Effect { kind: EffectKind::Repel, position, age: 0 });
                    if let (Some(v), Some(ship)) = (pushes_local, self.ship.as_mut()) {
                        ship.velocity = v;
                        ship.repel_ticks = settings.repel_time;
                    }
                }
            }
        }
    }

    fn take_hit(&mut self, w: &crate::weapons::Weapon, map: &Map, settings: &ArenaSettings) {
        if !self.alive() {
            return;
        }
        let local = self.local_pid();
        let ship = self.ship.as_ref().unwrap();
        let me = Target { pid: local, freq: self.local_freq(), ship: ship.ship, position: ship.position };
        let (shooter_pos, shooter_ship) = if w.pid == local {
            (Some(ship.position), ship.ship)
        } else {
            match self.session.as_ref().and_then(|s| s.players.get(&w.pid)) {
                Some(p) => (Some(p.position), p.ship),
                None => return, // Shooter left; nobody to credit.
            }
        };
        let mut seed = self.random(u32::MAX);
        let damage = damage_to_local(w, &me, shooter_pos, shooter_ship, settings, map, |n| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if n == 0 { 0 } else { seed % n }
        });
        let ship = self.ship.as_mut().unwrap();
        let shields_time = settings.ships[ship.ship as usize].shields_time;
        let damage = if ship.shield_ticks > 0 && shields_time > 0 {
            damage - (damage as f32 * ship.shield_ticks as f32 / shields_time as f32) as i32
        } else {
            damage
        };
        if damage <= 0 {
            return;
        }
        if ship.energy < damage as f32 {
            if w.data.kind().is_bomb() && w.pid == local {
                ship.energy = 1.0; // Your own bomb never kills you.
                return;
            }
            let (pos, vel, kind, bounty) = (ship.position, ship.velocity, ship.ship, ship.bounty);
            ship.energy = 0.0;
            ship.velocity = Vec2::default();
            self.deaths += 1;
            self.dead_ticks = settings.enter_delay + RESPAWN_EXTRA_TICKS;
            self.spawn_debris(kind, pos, vel);
            self.effects.push(Effect { kind: EffectKind::ShipExplosion, position: pos, age: 0 });
            self.sounds.push(SoundCue { name: "explode1".into(), position: pos });
            self.weapons.clear_owner(local);
            if let Some(s) = self.session.as_mut() {
                s.send_death(w.pid, bounty as u16);
            }
            let killer = self.session.as_ref().and_then(|s| s.players.get(&w.pid)).map(|p| p.name.clone());
            self.notices.push(format!("You were killed by {}", killer.unwrap_or_else(|| "yourself".into())));
        } else {
            ship.energy -= damage as f32;
        }
    }

    /// Sends our position (and any weapon just fired) to the zone.
    pub fn send_updates(&mut self, fired: WeaponData) {
        let dead = self.dead_ticks > 0;
        let camera = self.camera;
        if let Some(s) = self.session.as_mut() {
            s.spectate_at = camera;
            s.send_position(self.ship.as_ref(), fired, dead, false);
        }
    }
}

fn fire_sound(data: WeaponData) -> String {
    let level = data.level() + 1;
    match data.kind() {
        WeaponKind::Bullet | WeaponKind::BouncingBullet => format!("gun{level}"),
        _ if data.is_mine() => format!("mine{level}"),
        WeaponKind::Bomb | WeaponKind::ProximityBomb => format!("bomb{level}"),
        WeaponKind::Thor => "thor".into(),
        WeaponKind::Burst => "burst".into(),
        WeaponKind::Decoy => "decoy".into(),
        WeaponKind::Repel => "repel".into(),
        _ => "gun1".into(),
    }
}

/// A newly (re)spawned ship: placed by [`spawn_ship`], then given InitialBounty random prizes.
/// The original client draws these from local randomness (verified: three respawns within one
/// prize-seed window gave three different loadouts), so `prize_seed` here is a local random value,
/// not the shared green seed. Bounty ends at InitialBounty.
pub fn fresh_ship(map: &Map, settings: &ArenaSettings, ship: u8, spawn_seed: u32, prize_seed: u32) -> Ship {
    let mut s = spawn_ship(map, settings, ship, spawn_seed);
    let initial_bounty = settings.ships[ship as usize].initial_bounty;
    let mut seed = prize_seed;
    let mut no_random = |_: u32| 0;
    let (mut given, mut attempts) = (0, 0);
    while given < initial_bounty && attempts < 9999 {
        attempts += 1;
        let p = prizes::generate(&mut seed, settings, false);
        if p == 0 {
            break; // No prize weights in this arena.
        }
        if matches!(p, prizes::FULL_CHARGE | prizes::ENGINE_SHUTDOWN | prizes::SHIELDS | prizes::SUPER | prizes::WARP | prizes::BRICK) {
            continue;
        }
        let _ = prizes::apply(&mut s, p, settings, &mut seed, &mut no_random);
        given += 1;
    }
    s.energy = s.max_energy as f32;
    s.bounty = initial_bounty;
    if std::env::var_os("AIC_DEBUG_SPAWN").is_some() {
        eprintln!(
            "spawn ship={ship} seed={prize_seed:08X} bounty={initial_bounty} energy={} guns={} bombs={} items={:?} caps={:?} shrap={}",
            s.max_energy, s.guns, s.bombs, s.items, s.caps, s.shrapnel
        );
    }
    s
}

/// Places a fresh ship near the map centre, offset pseudo-randomly by `seed`.
pub fn spawn_ship(map: &Map, settings: &ArenaSettings, ship: u8, seed: u32) -> Ship {
    let r = settings.ships[ship as usize].radius_tiles();
    let off = |s: u32| (s % 41) as f32 - 20.0;
    let near = Vec2::new(512.0 + off(seed), 512.0 + off(seed >> 8));
    Ship::new(ship, sim::find_spawn(map, near, r), settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_gun_fires_and_bullet_explodes_on_wall() {
        let mut d = Vec::new();
        for x in 480..560u32 {
            d.extend_from_slice(&(x | (490 << 12) | (1 << 24)).to_le_bytes());
        }
        let map = Map::parse(&d).unwrap();
        let mut g = Game::offline(map, ArenaSettings::default(), 0);
        g.ship.as_mut().unwrap().position = Vec2::new(512.5, 512.5);
        let fired = g.tick(Input { gun: true, ..Default::default() });
        assert_eq!(fired.kind(), WeaponKind::Bullet);
        assert_eq!(g.weapons.list.len(), 1);
        assert!(g.sounds.iter().any(|s| s.name == "gun1"));
        for _ in 0..300 {
            g.tick(Input::default());
        }
        assert!(g.weapons.list.is_empty());
        assert!(g.sounds.iter().any(|s| s.name == "explode0"));
    }

    #[test]
    fn safety_clock_runs_in_safe_zone_and_restarts_on_entry() {
        let d: Vec<u8> = (511..514u32).flat_map(|x| (x | (512 << 12) | (171 << 24)).to_le_bytes()).collect();
        let mut g = Game::offline(Map::parse(&d).unwrap(), ArenaSettings::default(), 0);
        g.ship.as_mut().unwrap().position = Vec2::new(512.5, 512.5);
        assert_eq!(g.safety_time_left(), None);
        for _ in 0..301 {
            g.tick(Input::default());
        }
        assert_eq!(g.safety_time_left(), Some((0, 14, 56)));
        g.ship.as_mut().unwrap().position = Vec2::new(520.5, 520.5);
        g.tick(Input::default());
        assert_eq!(g.safety_time_left(), None);
        g.ship.as_mut().unwrap().position = Vec2::new(512.5, 512.5);
        g.tick(Input::default());
        assert_eq!(g.safety_time_left(), Some((0, 14, 59)));
    }

    #[test]
    fn own_bomb_never_kills() {
        let mut d = Vec::new();
        for x in 500..525u32 {
            d.extend_from_slice(&(x | (508 << 12) | (1 << 24)).to_le_bytes());
        }
        let map = Map::parse(&d).unwrap();
        let mut g = Game::offline(map, ArenaSettings::default(), 0);
        let ship = g.ship.as_mut().unwrap();
        ship.position = Vec2::new(512.5, 510.5);
        ship.energy = 400.0;
        g.tick(Input { bomb: true, ..Default::default() });
        for _ in 0..50 {
            g.tick(Input::default());
        }
        assert!(g.alive());
        assert!(g.ship.as_ref().unwrap().energy < 400.0, "splash damage applied");
    }
}

#[cfg(test)]
mod prize_tests {
    use super::*;

    #[test]
    fn offline_greens_spawn_and_are_collected() {
        let mut settings = ArenaSettings::default();
        settings.prize_delay = 5;
        settings.prize_weights = [0; 28];
        settings.prize_weights[(prizes::GUNS - 1) as usize] = 1;
        settings.prize_negative_factor = u32::MAX;
        let mut g = Game::offline(Map::empty(), settings, 0);
        for _ in 0..5 {
            g.tick(Input::default());
        }
        assert!(!g.greens.list.is_empty(), "greens spawned");
        let green = g.greens.list[0];
        let guns = g.ship.as_ref().unwrap().guns;
        g.ship.as_mut().unwrap().position = Vec2::new(green.x as f32 + 0.5, green.y as f32 + 0.5);
        g.tick(Input::default());
        assert_eq!(g.ship.as_ref().unwrap().guns, guns + 1);
        assert!(g.notices.iter().any(|n| n == "Guns upgraded."));
        assert!(g.sounds.iter().any(|s| s.name == "prize"));
    }
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    #[test]
    fn initial_bounty_prizes_are_deterministic_and_restore_bounty() {
        let mut settings = ArenaSettings::default();
        settings.ships[0].initial_bounty = 40;
        let map = Map::empty();
        let a = fresh_ship(&map, &settings, 0, 1, 777);
        let b = fresh_ship(&map, &settings, 0, 2, 777);
        assert_eq!((a.guns, a.bombs, a.items, a.max_energy), (b.guns, b.bombs, b.items, b.max_energy));
        assert_eq!(a.bounty, 40);
        assert_eq!(a.energy, a.max_energy as f32);
        let plain = Ship::new(0, Vec2::new(0.0, 0.0), &settings);
        assert!(a.guns > plain.guns || a.max_energy > plain.max_energy || a.items != plain.items, "upgraded");
    }
}
