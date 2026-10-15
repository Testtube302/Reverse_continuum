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

/// Ticks a ship explosion is shown (explode1 animation, 0.8 s).
pub const SHIP_EXPLOSION_TICKS: u32 = 80;
pub const BULLET_EXPLOSION_TICKS: u32 = 40;
pub const BOMB_EXPLOSION_TICKS: u32 = 125;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    BulletHit,
    Bomb,
    EmpBomb,
    ShipExplosion,
}

impl EffectKind {
    pub fn duration(self) -> u32 {
        match self {
            Self::BulletHit => BULLET_EXPLOSION_TICKS,
            Self::Bomb | Self::EmpBomb => BOMB_EXPLOSION_TICKS,
            Self::ShipExplosion => SHIP_EXPLOSION_TICKS,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Effect {
    pub kind: EffectKind,
    pub position: Vec2,
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
    pub weapons: Weapons,
    pub greens: Greens,
    flag_ticks: u32,
    flag_requests: std::collections::HashMap<u16, u32>,
    pub effects: Vec<Effect>,
    pub sounds: Vec<SoundCue>,
    pub notices: Vec<String>,
    /// Spectator camera when not in a ship.
    pub camera: Vec2,
    pub kills: u32,
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
            weapons: Weapons::default(),
            greens: Greens::default(),
            flag_ticks: 0,
            flag_requests: std::collections::HashMap::new(),
            effects: Vec::new(),
            sounds: Vec::new(),
            notices: Vec::new(),
            // Spectators start at the map's top-left corner, as in the original.
            camera: Vec2::new(0.0, 0.0),
            kills: 0,
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

        // Our ship: respawn countdown, or movement and firing.
        if self.dead_ticks > 0 {
            self.dead_ticks -= 1;
            if self.dead_ticks == 0 {
                let seed = self.random(u32::MAX);
                if let Some(kind) = self.ship.as_ref().map(|s| s.ship) {
                    self.ship = Some(fresh_ship(&map, &settings, kind, seed, seed.rotate_left(13) ^ 0xA5A5_5A5A));
                }
            }
        } else if let Some(ship) = self.ship.as_mut() {
            if ship.tick(&map, &settings, input) {
                self.sounds.push(SoundCue { name: "bounce".into(), position: ship.position });
            }
            let in_safe = map.tile(ship.position.x as i32, ship.position.y as i32) == TILE_SAFE;
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
            if d.killed == local {
                continue; // We already showed our own explosion.
            }
            if let Some(p) = self.session.as_ref().and_then(|s| s.players.get(&d.killed)) {
                self.effects.push(Effect { kind: EffectKind::ShipExplosion, position: p.position, age: 0 });
                self.sounds.push(SoundCue { name: "explode1".into(), position: p.position });
                // Only moving victims drop their prize.
                if d.green > 0 && (p.velocity.x != 0.0 || p.velocity.y != 0.0) {
                    self.greens.add_death_green(p.position, d.green as i16, &settings);
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
        fired
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
            let (pos, bounty) = (ship.position, ship.bounty);
            ship.energy = 0.0;
            ship.velocity = Vec2::default();
            self.deaths += 1;
            self.dead_ticks = settings.enter_delay + SHIP_EXPLOSION_TICKS;
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
