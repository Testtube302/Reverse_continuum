//! Weapon simulation: bullets, bombs, mines, shrapnel; wall bounces, player hits, explosions,
//! and damage to the local ship. Ticks are 10 ms. Rules follow the behaviour documented by the
//! nullspace client (MIT, plushmonkey): every client simulates every weapon, but each client
//! only applies damage to its own ship.

use crate::lvl::{Map, TILE_SAFE, TILE_WORMHOLE};
use crate::net::vie::VieRng;
use crate::settings::ArenaSettings;
use crate::sim::{Vec2, heading};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponKind {
    None,
    Bullet,
    BouncingBullet,
    Bomb,
    ProximityBomb,
    Repel,
    Decoy,
    Burst,
    Thor,
}

impl WeaponKind {
    fn from_bits(v: u16) -> Self {
        match v {
            1 => Self::Bullet,
            2 => Self::BouncingBullet,
            3 => Self::Bomb,
            4 => Self::ProximityBomb,
            5 => Self::Repel,
            6 => Self::Decoy,
            7 => Self::Burst,
            8 => Self::Thor,
            _ => Self::None,
        }
    }
    pub fn is_bullet(self) -> bool {
        matches!(self, Self::Bullet | Self::BouncingBullet)
    }
    pub fn is_bomb(self) -> bool {
        matches!(self, Self::Bomb | Self::ProximityBomb | Self::Thor)
    }
}

/// The 16-bit weapon field of position packets:
/// type:5, level:2, shrap bouncing:1, shrap level:2, shrap count:5, alternate:1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponData(pub u16);

impl WeaponData {
    pub fn new(kind: WeaponKind, level: u16) -> Self {
        Self(kind as u16 & 0x1F | (level & 3) << 5)
    }
    pub fn kind(self) -> WeaponKind {
        WeaponKind::from_bits(self.0 & 0x1F)
    }
    pub fn level(self) -> u16 {
        self.0 >> 5 & 3
    }
    pub fn shrap_bouncing(self) -> bool {
        self.0 >> 7 & 1 != 0
    }
    pub fn shrap_level(self) -> u16 {
        self.0 >> 8 & 3
    }
    pub fn shrap(self) -> u16 {
        self.0 >> 10 & 0x1F
    }
    /// Multifire for guns, mine (instead of bomb) for bombs.
    pub fn alternate(self) -> bool {
        self.0 >> 15 != 0
    }
    pub fn with_kind(self, kind: WeaponKind) -> Self {
        Self(self.0 & !0x1F | kind as u16)
    }
    pub fn with_level(self, level: u16) -> Self {
        Self(self.0 & !(3 << 5) | (level & 3) << 5)
    }
    pub fn with_shrapnel(self, count: u16, level: u16, bouncing: bool) -> Self {
        Self(self.0 & !(0x1F << 10 | 3 << 8 | 1 << 7) | (count & 0x1F) << 10 | (level & 3) << 8 | (bouncing as u16) << 7)
    }
    pub fn with_alternate(self, on: bool) -> Self {
        Self(self.0 & 0x7FFF | (on as u16) << 15)
    }
    pub fn is_mine(self) -> bool {
        matches!(self.kind(), WeaponKind::Bomb | WeaponKind::ProximityBomb) && self.alternate()
    }
}

/// A ship that weapons can hit.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub pid: u16,
    pub freq: u16,
    pub ship: u8,
    pub position: Vec2,
}

/// Who fired, and from where (pixel position, velocity in pixels per 10 s).
#[derive(Clone, Copy, Debug)]
pub struct Shooter {
    pub pid: u16,
    pub freq: u16,
    pub ship: u8,
    pub direction: u8,
    pub x: u32,
    pub y: u32,
    pub vel_x: i32,
    pub vel_y: i32,
}

#[derive(Clone, Debug)]
pub struct Weapon {
    pub position: Vec2,
    pub velocity: Vec2,
    pub data: WeaponData,
    pub pid: u16,
    pub freq: u16,
    /// Ticks simulated so far, and total lifetime in ticks.
    pub age: u32,
    pub lifetime: u32,
    pub bounces_remaining: u32,
    pub emp: bool,
    /// Proximity bomb that has sensed a ship: (target pid, largest axis offset seen, ticks left).
    prox: Option<(u16, f32, u32)>,
    rng_seed: u32,
    link: Option<u32>,
    first_tick: bool,
    /// Burst pieces only hurt ships after their first wall bounce.
    pub burst_active: bool,
    /// A decoy shows the shooter's ship facing this direction (0..40).
    pub decoy_direction: u8,
    pub decoy_ship: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplosionKind {
    Bullet,
    Bomb,
    EmpBomb,
}

#[derive(Clone, Debug)]
pub enum WeaponEvent {
    Explosion { kind: ExplosionKind, position: Vec2 },
    /// The local ship may take damage from this weapon (direct hit, or a bomb exploding).
    HitLocal(Weapon),
    /// A repel went off: draw its animation; the game pushes our ship if `pushes_local`.
    Repel { position: Vec2, pushes_local: Option<Vec2> },
}

#[derive(PartialEq)]
enum Outcome {
    Continue,
    WallExplosion,
    PlayerExplosion,
    TimedOut,
}

#[derive(Default)]
pub struct Weapons {
    pub list: Vec<Weapon>,
    next_link: u32,
}

impl Weapons {
    pub fn clear_owner(&mut self, pid: u16) {
        self.list.retain(|w| w.pid != pid);
    }

    fn lifetime(settings: &ArenaSettings, data: WeaponData) -> u32 {
        match data.kind() {
            WeaponKind::Bullet | WeaponKind::BouncingBullet | WeaponKind::Burst => settings.bullet_alive_time,
            WeaponKind::Bomb | WeaponKind::ProximityBomb | WeaponKind::Thor => {
                if data.alternate() { settings.mine_alive_time } else { settings.bomb_alive_time }
            }
            WeaponKind::Repel => 1,
            WeaponKind::Decoy => settings.decoy_alive_time,
            _ => 0,
        }
    }

    /// Fires `data` from `shooter`, simulating `catch_up` ticks to account for latency.
    /// `local_pid` is our own player id (only our ship takes damage).
    pub fn fire(
        &mut self,
        shooter: Shooter,
        data: WeaponData,
        catch_up: u32,
        map: &Map,
        settings: &ArenaSettings,
        targets: &[Target],
        local_pid: Option<u16>,
    ) -> Vec<WeaponEvent> {
        let ship = &settings.ships[shooter.ship.min(7) as usize];
        let h = heading(shooter.direction);
        let mut events = Vec::new();
        let spawn = |w: &mut Self, x: u32, y: u32, dir: Vec2, link: Option<u32>, events: &mut Vec<WeaponEvent>| {
            w.generate(shooter, data, x, y, dir, link, catch_up, map, settings, targets, local_pid, events)
        };
        match data.kind() {
            WeaponKind::Bullet | WeaponKind::BouncingBullet => {
                let link = Some(self.next_link);
                self.next_link = self.next_link.wrapping_add(1);
                let mut hit = false;
                if ship.double_barrel {
                    let perp = Vec2::new(-h.y, h.x).scale(ship.radius_tiles() * 0.75);
                    let (ox, oy) = ((perp.x * 16.0) as i32, (perp.y * 16.0) as i32);
                    let (x, y) = (shooter.x as i32, shooter.y as i32);
                    hit |= spawn(self, (x - ox) as u32, (y - oy) as u32, h, link, &mut events);
                    hit |= spawn(self, (x + ox) as u32, (y + oy) as u32, h, link, &mut events);
                } else {
                    hit |= spawn(self, shooter.x, shooter.y, h, link, &mut events);
                }
                if data.alternate() {
                    let rads = (ship.multi_fire_angle as f32 / 111.0).to_radians();
                    hit |= spawn(self, shooter.x, shooter.y, h.rotate(rads), link, &mut events);
                    hit |= spawn(self, shooter.x, shooter.y, h.rotate(-rads), link, &mut events);
                }
                if hit {
                    self.remove_link(link, &mut events);
                }
            }
            WeaponKind::Bomb | WeaponKind::ProximityBomb | WeaponKind::Thor => {
                spawn(self, shooter.x, shooter.y, h, None, &mut events);
            }
            WeaponKind::Burst => {
                // BurstShrapnel pieces evenly around the ship (the orientation in 1/1000 degrees).
                let count = ship.burst_shrapnel.max(1) as i32;
                for i in 0..count {
                    let deg = ((i * 40000) / count * 9) as f32 / 1000.0;
                    let dir = Vec2::new(deg.to_radians().sin(), -deg.to_radians().cos());
                    spawn(self, shooter.x, shooter.y, dir, None, &mut events);
                }
            }
            WeaponKind::Decoy => {
                spawn(self, shooter.x, shooter.y, h, None, &mut events);
            }
            WeaponKind::Repel => {
                self.repel(shooter, map, settings, targets, local_pid, &mut events);
            }
            _ => {}
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(
        &mut self,
        shooter: Shooter,
        data: WeaponData,
        x: u32,
        y: u32,
        dir: Vec2,
        link: Option<u32>,
        catch_up: u32,
        map: &Map,
        settings: &ArenaSettings,
        targets: &[Target],
        local_pid: Option<u16>,
        events: &mut Vec<WeaponEvent>,
    ) -> bool {
        let ship = &settings.ships[shooter.ship.min(7) as usize];
        let (speed, bounces) = match data.kind() {
            WeaponKind::Bullet | WeaponKind::BouncingBullet => (ship.bullet_speed, 0),
            WeaponKind::Bomb | WeaponKind::ProximityBomb | WeaponKind::Thor if !data.alternate() => {
                (ship.bomb_speed, ship.bomb_bounce_count)
            }
            WeaponKind::Burst => (ship.burst_speed, 0),
            _ => (0, 0),
        };
        let shooter_vel = Vec2::new(shooter.vel_x as f32 / 160.0, shooter.vel_y as f32 / 160.0);
        let velocity = match data.kind() {
            _ if data.is_mine() => Vec2::default(),
            // Burst pieces leave from rest; decoys keep the shooter's velocity.
            WeaponKind::Burst => dir.scale(speed as f32 / 160.0),
            WeaponKind::Decoy => shooter_vel,
            _ => shooter_vel.add(dir.scale(speed as f32 / 160.0)),
        };
        let mut w = Weapon {
            position: Vec2::new(x as f32 / 16.0, y as f32 / 16.0),
            velocity,
            data,
            pid: shooter.pid,
            freq: shooter.freq,
            age: 0,
            lifetime: Self::lifetime(settings, data),
            bounces_remaining: bounces,
            emp: ship.emp_bomb && data.kind().is_bomb(),
            prox: None,
            rng_seed: 0,
            link,
            first_tick: true,
            burst_active: false,
            decoy_direction: shooter.direction,
            decoy_ship: shooter.ship,
        };
        for _ in 0..catch_up {
            let outcome = simulate(&mut w, map, settings, targets, local_pid, events);
            if outcome != Outcome::Continue {
                if outcome != Outcome::TimedOut {
                    self.explode(&w, map, settings, events);
                }
                return outcome == Outcome::PlayerExplosion;
            }
        }
        let vx = (w.velocity.x * 160.0) as i32 as u32;
        let vy = (w.velocity.y * 160.0) as i32 as u32;
        w.rng_seed = (data.shrap() as u32 + data.level() as u32)
            .wrapping_add(x.wrapping_mul(1000))
            .wrapping_add(y.wrapping_mul(1000))
            .wrapping_add(vx)
            .wrapping_add(vy)
            .wrapping_add(shooter.freq as u32);
        self.list.push(w);
        false
    }

    /// A repel acts once, where it's fired: enemy weapons inside the square of half-size
    /// RepelDistance fly straight away from it at RepelSpeed (mines turn into bombs), and so does
    /// our ship if it's an enemy outside a safe zone.
    fn repel(
        &mut self,
        shooter: Shooter,
        map: &Map,
        settings: &ArenaSettings,
        targets: &[Target],
        local_pid: Option<u16>,
        events: &mut Vec<WeaponEvent>,
    ) {
        let at = Vec2::new(shooter.x as f32 / 16.0, shooter.y as f32 / 16.0);
        let half = settings.repel_distance as f32 / 16.0;
        let speed = settings.repel_speed as f32 / 160.0;
        let inside = |p: Vec2| (p.x - at.x).abs() <= half && (p.y - at.y).abs() <= half;
        let away = |p: Vec2| {
            let d = p.sub(at);
            let len = d.length();
            if len > 0.0 { d.scale(speed / len) } else { Vec2::new(0.0, -speed) }
        };
        for w in &mut self.list {
            if w.freq == shooter.freq || w.data.kind() == WeaponKind::Repel || !inside(w.position) {
                continue;
            }
            w.velocity = away(w.position);
            if w.data.is_mine() {
                w.data = w.data.with_alternate(false);
            }
            w.age = 0;
            w.lifetime = Self::lifetime(settings, w.data);
        }
        let pushes_local = targets
            .iter()
            .find(|t| Some(t.pid) == local_pid && t.freq != shooter.freq && inside(t.position))
            .filter(|t| map.tile(t.position.x as i32, t.position.y as i32) != TILE_SAFE)
            .map(|t| away(t.position));
        events.push(WeaponEvent::Repel { position: at, pushes_local });
    }

    /// Removes the rest of a multifire/double-barrel volley after one of its bullets hit a ship.
    fn remove_link(&mut self, link: Option<u32>, events: &mut Vec<WeaponEvent>) {
        let Some(link) = link else { return };
        self.list.retain(|w| {
            if w.link == Some(link) {
                events.push(WeaponEvent::Explosion { kind: ExplosionKind::Bullet, position: w.position });
                false
            } else {
                true
            }
        });
    }

    /// Advances every weapon one tick. `owners_in_safe` lists players whose weapons vanish
    /// because they're sitting in a safe zone.
    pub fn tick(
        &mut self,
        map: &Map,
        settings: &ArenaSettings,
        targets: &[Target],
        local_pid: Option<u16>,
        owners_in_safe: &[u16],
    ) -> Vec<WeaponEvent> {
        let mut events = Vec::new();
        let mut hit_links = Vec::new();
        // Shrapnel created this tick starts moving next tick.
        let mut spawned = Vec::new();
        let mut i = 0;
        while i < self.list.len() {
            if owners_in_safe.contains(&self.list[i].pid) {
                self.list.swap_remove(i);
                continue;
            }
            let outcome = simulate(&mut self.list[i], map, settings, targets, local_pid, &mut events);
            match outcome {
                Outcome::Continue => i += 1,
                Outcome::TimedOut => {
                    self.list.swap_remove(i);
                }
                Outcome::WallExplosion | Outcome::PlayerExplosion => {
                    let w = self.list.swap_remove(i);
                    if outcome == Outcome::PlayerExplosion {
                        if let Some(l) = w.link {
                            hit_links.push(l);
                        }
                    }
                    explode(&w, map, settings, &mut events, &mut spawned);
                }
            }
        }
        for l in hit_links {
            self.remove_link(Some(l), &mut events);
        }
        self.list.extend(spawned);
        events
    }

    fn explode(&mut self, w: &Weapon, map: &Map, settings: &ArenaSettings, events: &mut Vec<WeaponEvent>) {
        let mut spawned = Vec::new();
        explode(w, map, settings, events, &mut spawned);
        self.list.extend(spawned);
    }
}

/// Emits the explosion for `w` and, for bombs, its shrapnel into `spawned`.
fn explode(w: &Weapon, map: &Map, settings: &ArenaSettings, events: &mut Vec<WeaponEvent>, spawned: &mut Vec<Weapon>) {
    if w.data.kind().is_bullet() {
        events.push(WeaponEvent::Explosion { kind: ExplosionKind::Bullet, position: w.position });
        return;
    }
    if !w.data.kind().is_bomb() {
        return;
    }
    let kind = if w.emp { ExplosionKind::EmpBomb } else { ExplosionKind::Bomb };
    events.push(WeaponEvent::Explosion { kind, position: w.position });

    let count = w.data.shrap() as u32;
    let mut rng = VieRng::new(w.rng_seed);
    let shrap_kind = if w.data.shrap_bouncing() { WeaponKind::BouncingBullet } else { WeaponKind::Bullet };
    for i in 0..count {
        let orientation = if settings.shrapnel_random {
            (rng.next() % 40000) as i32 * 9
        } else {
            (i as i32 * 40000) / count as i32 * 9
        };
        let a = (orientation as f32 / 1000.0).to_radians();
        let dir = Vec2::new(a.sin(), -a.cos());
        if map.is_solid(w.position.x as i32, w.position.y as i32) {
            continue;
        }
        spawned.push(Weapon {
            position: w.position,
            velocity: dir.scale(settings.shrapnel_speed as f32 / 160.0),
            data: w.data.with_kind(shrap_kind).with_level(w.data.shrap_level()),
            pid: w.pid,
            freq: w.freq,
            age: 0,
            lifetime: settings.bullet_alive_time,
            bounces_remaining: 0,
            emp: false,
            prox: None,
            rng_seed: 0,
            link: None,
            first_tick: false,
            burst_active: false,
            decoy_direction: 0,
            decoy_ship: 0,
        });
    }
}

fn overlap(a_min: Vec2, a_max: Vec2, b_min: Vec2, b_max: Vec2) -> bool {
    a_min.x <= b_max.x && a_max.x >= b_min.x && a_min.y <= b_max.y && a_max.y >= b_min.y
}

fn simulate(
    w: &mut Weapon,
    map: &Map,
    settings: &ArenaSettings,
    targets: &[Target],
    local_pid: Option<u16>,
    events: &mut Vec<WeaponEvent>,
) -> Outcome {
    if w.age >= w.lifetime {
        return Outcome::TimedOut;
    }
    w.age += 1;
    let first_tick = std::mem::take(&mut w.first_tick);
    let kind = w.data.kind();
    let previous = w.position;

    // Movement with wall bounces, one axis at a time.
    let mut collided = false;
    for axis in 0..2 {
        let before = w.position;
        if axis == 0 {
            w.position.x += w.velocity.x * 0.01;
        } else {
            w.position.y += w.velocity.y * 0.01;
        }
        if kind != WeaponKind::Thor && map.is_solid(w.position.x.floor() as i32, w.position.y.floor() as i32) {
            w.position = before;
            if axis == 0 {
                w.velocity.x = -w.velocity.x;
            } else {
                w.velocity.y = -w.velocity.y;
            }
            collided = true;
        }
    }
    if collided {
        // Shrapnel that hits a wall within its first 1/4 second just disappears.
        if kind.is_bullet() && w.data.shrap() > 0 {
            let remaining = w.lifetime as i32 - w.age as i32;
            let duration = settings.bullet_alive_time as i32 - remaining;
            if remaining < 0 || duration <= 25 {
                return Outcome::TimedOut;
            }
        }
        if kind == WeaponKind::Burst {
            w.burst_active = true;
        }
        if matches!(kind, WeaponKind::Bullet | WeaponKind::Bomb | WeaponKind::ProximityBomb) {
            if w.bounces_remaining == 0 {
                if kind != WeaponKind::Bullet {
                    events.push(WeaponEvent::HitLocal(w.clone()));
                }
                return Outcome::WallExplosion;
            }
            w.bounces_remaining -= 1;
        }
    }
    if map.tile(w.position.x as i32, w.position.y as i32) == TILE_WORMHOLE {
        return Outcome::TimedOut;
    }
    if kind == WeaponKind::Decoy || (kind == WeaponKind::Burst && !w.burst_active) {
        return Outcome::Continue;
    }

    let is_bomb = kind.is_bomb();
    let is_prox = matches!(kind, WeaponKind::ProximityBomb | WeaponKind::Thor);

    // A proximity bomb that has sensed a ship explodes once the ship moves away or the delay ends.
    if let Some((pid, highest, ticks_left)) = w.prox {
        let Some(t) = targets.iter().find(|t| t.pid == pid) else { return Outcome::PlayerExplosion };
        let d = (w.position.x - t.position.x).abs().max((w.position.y - t.position.y).abs());
        if d > highest || ticks_left == 0 {
            events.push(WeaponEvent::HitLocal(w.clone()));
            w.position = previous;
            return Outcome::PlayerExplosion;
        }
        w.prox = Some((pid, d, ticks_left - 1));
        return Outcome::Continue;
    }

    let mut outcome = Outcome::Continue;
    for t in targets {
        if t.ship >= 8 || t.freq == w.freq {
            continue;
        }
        let r = settings.ships[t.ship as usize].radius_tiles();
        let pos = t.position.pixel_rounded();
        let mut weapon_radius = 18.0f32;
        if is_prox {
            let mut prox = (settings.proximity_distance + w.data.level() as u32) as f32;
            if kind == WeaponKind::Thor {
                prox += 3.0;
            }
            weapon_radius = prox * 18.0;
        }
        weapon_radius = (weapon_radius - 14.0) / 16.0;
        let wp = w.position.pixel_rounded();
        let ship_min = Vec2::new(pos.x - r, pos.y - r);
        let ship_max = Vec2::new(pos.x + r, pos.y + r);
        let wr = Vec2::new(weapon_radius, weapon_radius);
        if !overlap(ship_min, ship_max, wp.sub(wr), wp.add(wr)) {
            continue;
        }
        if is_prox {
            let d = (w.position.x - t.position.x).abs().max((w.position.y - t.position.y).abs());
            // Spawned on top of an enemy: explode immediately.
            let delay = if first_tick { 0 } else { settings.bomb_explode_delay };
            w.prox = Some((t.pid, d, delay));
            let core = Vec2::new(4.0 / 16.0, 4.0 / 16.0);
            if !overlap(ship_min, ship_max, wp.sub(core), wp.add(core)) {
                continue;
            }
        }
        if is_bomb || Some(t.pid) == local_pid {
            events.push(WeaponEvent::HitLocal(w.clone()));
        }
        if matches!(kind, WeaponKind::Bomb | WeaponKind::ProximityBomb) {
            w.position = previous;
        }
        // A weapon explodes once; a bomb's splash damage is then judged by distance alone.
        outcome = Outcome::PlayerExplosion;
        break;
    }
    outcome
}

/// Damage the local ship takes from `w`. `random` returns a value in 0..n.
pub fn damage_to_local(
    w: &Weapon,
    me: &Target,
    shooter_position: Option<Vec2>,
    shooter_ship: u8,
    settings: &ArenaSettings,
    map: &Map,
    mut random: impl FnMut(u32) -> u32,
) -> i32 {
    let kind = w.data.kind();
    let mut damage = match kind {
        WeaponKind::Bullet | WeaponKind::BouncingBullet => {
            let base = settings.bullet_damage_level / 1000
                + (settings.bullet_damage_upgrade / 1000) * w.data.level() as i32;
            if w.data.shrap() > 0 {
                let remaining = w.lifetime as i32 - w.age as i32;
                let duration = settings.bullet_alive_time as i32 - remaining;
                if duration <= 25 {
                    (settings.inactive_shrap_damage / 1000) * w.data.level() as i32
                } else {
                    (base as f32 * settings.shrapnel_damage_percent as f32 / 1000.0) as i32
                }
            } else {
                base
            }
        }
        WeaponKind::Burst => settings.burst_damage_level / 1000,
        WeaponKind::Bomb | WeaponKind::ProximityBomb | WeaponKind::Thor => {
            let mut bomb = settings.bomb_damage_level;
            let mut level = w.data.level() as i32;
            if kind == WeaponKind::Thor {
                bomb += bomb * level * level;
                level += 3;
            }
            let mut bomb = bomb / 1000;
            if settings.ships[shooter_ship.min(7) as usize].bomb_bounce_count > 0 {
                bomb = (bomb as f32 * settings.bbomb_damage_percent as f32 / 1000.0) as i32;
            }
            let explode = (settings.bomb_explode_pixels as i32 * (1 + level)) as f32;
            let d = w.position.sub(me.position).scale(16.0).length();
            if d >= explode {
                return 0;
            }
            let mut dmg = ((explode - d) * (bomb as f32 / explode)) as i32;
            // Bombs that also catch the shooter are weaker against others.
            if w.pid != me.pid {
                if let Some(sp) = shooter_position {
                    let sd = w.position.sub(sp).scale(16.0).length();
                    if sd < explode {
                        dmg -= (((bomb as f32 / explode) * (explode - sd)) / 2.0) as i32;
                        dmg = dmg.max(0);
                    }
                }
            }
            dmg
        }
        _ => 0,
    };
    if map.tile(me.position.x as i32, me.position.y as i32) == TILE_SAFE {
        return 0;
    }
    if !settings.exact_damage && kind.is_bullet() && damage > 0 {
        let r = random((damage * damage + 1) as u32) * 1000 % (damage * damage + 1) as u32;
        damage = (r as f32).sqrt() as i32;
    }
    damage.max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lvl::Map;

    fn wall_map(x: u32) -> Map {
        let mut d = Vec::new();
        for y in 480..560u32 {
            d.extend_from_slice(&(x | (y << 12) | (1 << 24)).to_le_bytes());
        }
        Map::parse(&d).unwrap()
    }

    fn shooter(dir: u8) -> Shooter {
        Shooter { pid: 1, freq: 0, ship: 0, direction: dir, x: 512 * 16, y: 512 * 16, vel_x: 0, vel_y: 0 }
    }

    #[test]
    fn weapon_data_bitfields_roundtrip() {
        let d = WeaponData::new(WeaponKind::Bomb, 2).with_shrapnel(8, 1, true).with_alternate(true);
        assert_eq!(d.kind(), WeaponKind::Bomb);
        assert_eq!((d.level(), d.shrap(), d.shrap_level(), d.shrap_bouncing(), d.alternate()), (2, 8, 1, true, true));
        assert!(d.is_mine());
        assert_eq!(WeaponData(0x0021).kind(), WeaponKind::Bullet);
        assert_eq!(WeaponData(0x0021).level(), 1);
    }

    #[test]
    fn bullet_flies_at_bullet_speed_and_times_out() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[], None);
        assert_eq!(w.list.len(), 1);
        for _ in 0..100 {
            w.tick(&map, &s, &[], None, &[]);
        }
        // 2000 px/10s = 12.5 tiles/s, 1 second to the right.
        assert!((w.list[0].position.x - 524.5).abs() < 0.01);
        for _ in 0..s.bullet_alive_time {
            w.tick(&map, &s, &[], None, &[]);
        }
        assert!(w.list.is_empty());
    }

    #[test]
    fn multifire_spawns_three_and_double_barrel_two() {
        let mut s = ArenaSettings::default();
        let map = Map::empty();
        let mut w = Weapons::default();
        w.fire(shooter(0), WeaponData::new(WeaponKind::Bullet, 0).with_alternate(true), 0, &map, &s, &[], None);
        assert_eq!(w.list.len(), 3);
        // Side bullets fan out symmetrically at MultiFireAngle/111 degrees.
        let xs: Vec<f32> = w.list.iter().map(|b| b.velocity.x).collect();
        assert!((xs[1] + xs[2]).abs() < 1e-4 && xs[1].abs() > 0.5);
        s.ships[0].double_barrel = true;
        let mut w = Weapons::default();
        w.fire(shooter(0), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[], None);
        assert_eq!(w.list.len(), 2);
        assert!(w.list[0].position.x != w.list[1].position.x);
    }

    #[test]
    fn bullet_explodes_on_wall_and_bouncing_bullet_bounces() {
        let s = ArenaSettings::default();
        let map = wall_map(520);
        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[], None);
        let mut exploded = false;
        for _ in 0..200 {
            let ev = w.tick(&map, &s, &[], None, &[]);
            exploded |= ev.iter().any(|e| matches!(e, WeaponEvent::Explosion { kind: ExplosionKind::Bullet, .. }));
        }
        assert!(exploded && w.list.is_empty());

        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::BouncingBullet, 0), 0, &map, &s, &[], None);
        for _ in 0..80 {
            w.tick(&map, &s, &[], None, &[]);
        }
        assert_eq!(w.list.len(), 1);
        assert!(w.list[0].velocity.x < 0.0, "bounced back off the wall");
    }

    #[test]
    fn bullet_hits_enemy_but_not_teammate() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let enemy = Target { pid: 2, freq: 1, ship: 0, position: Vec2::new(516.0, 512.5) };
        let mate = Target { pid: 3, freq: 0, ship: 0, position: Vec2::new(516.0, 512.5) };

        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[mate], Some(3));
        for _ in 0..60 {
            assert!(w.tick(&map, &s, &[mate], Some(3), &[]).is_empty());
        }

        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[enemy], Some(2));
        let mut hit = false;
        for _ in 0..60 {
            hit |= w.tick(&map, &s, &[enemy], Some(2), &[]).iter().any(|e| matches!(e, WeaponEvent::HitLocal(_)));
        }
        assert!(hit && w.list.is_empty());
    }

    #[test]
    fn burst_fans_out_and_only_hurts_after_a_bounce() {
        let s = ArenaSettings::default();
        let enemy = Target { pid: 2, freq: 1, ship: 0, position: Vec2::new(514.5, 512.5) };
        let mut w = Weapons::default();
        w.fire(shooter(0), WeaponData::new(WeaponKind::Burst, 0), 0, &Map::empty(), &s, &[enemy], Some(2));
        assert_eq!(w.list.len(), s.ships[0].burst_shrapnel as usize);
        assert!(w.list.iter().all(|p| !p.burst_active));
        // In open space they pass through the enemy harmlessly.
        for _ in 0..30 {
            assert!(!w.tick(&Map::empty(), &s, &[enemy], Some(2), &[]).iter().any(|e| matches!(e, WeaponEvent::HitLocal(_))));
        }
        // Next to a wall, the pieces that bounce become live.
        let map = wall_map(515);
        let mut w = Weapons::default();
        w.fire(shooter(0), WeaponData::new(WeaponKind::Burst, 0), 0, &map, &s, &[], None);
        for _ in 0..20 {
            w.tick(&map, &s, &[], None, &[]);
        }
        assert!(w.list.iter().any(|p| p.burst_active));
    }

    #[test]
    fn repel_pushes_enemy_weapons_and_our_ship_away() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let mut w = Weapons::default();
        let mut enemy_gun = shooter(10);
        (enemy_gun.pid, enemy_gun.freq, enemy_gun.x) = (2, 1, 520 * 16);
        // An enemy bullet flying left toward us, and our own ship 3 tiles to the right.
        let mut left = enemy_gun;
        left.direction = 30;
        w.fire(left, WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[], None);
        let me = Target { pid: 5, freq: 1, ship: 0, position: Vec2::new(515.0, 512.0) };
        let events = w.fire(shooter(0), WeaponData::new(WeaponKind::Repel, 0), 0, &map, &s, &[me], Some(5));
        assert!(w.list[0].velocity.x > 0.0, "enemy bullet now flies away from the repel");
        let push = events.iter().find_map(|e| match e {
            WeaponEvent::Repel { pushes_local, .. } => *pushes_local,
            _ => None,
        });
        assert!(push.is_some_and(|v| v.x > 0.0 && (v.length() - s.repel_speed as f32 / 160.0).abs() < 0.01));
    }

    #[test]
    fn decoy_keeps_the_shooters_velocity_and_heading() {
        let s = ArenaSettings::default();
        let mut sh = shooter(12);
        (sh.vel_x, sh.vel_y) = (320, -160);
        let mut w = Weapons::default();
        w.fire(sh, WeaponData::new(WeaponKind::Decoy, 0), 0, &Map::empty(), &s, &[], None);
        let d = &w.list[0];
        assert_eq!((d.velocity.x, d.velocity.y, d.decoy_direction), (2.0, -1.0, 12));
        assert_eq!(d.lifetime, s.decoy_alive_time);
    }

    #[test]
    fn bomb_explodes_into_shrapnel() {
        let s = ArenaSettings::default();
        let map = wall_map(520);
        let mut w = Weapons::default();
        let data = WeaponData::new(WeaponKind::Bomb, 0).with_shrapnel(8, 0, false);
        w.fire(shooter(10), data, 0, &map, &s, &[], None);
        let mut bomb_explosion = false;
        for _ in 0..100 {
            let ev = w.tick(&map, &s, &[], None, &[]);
            bomb_explosion |= ev.iter().any(|e| matches!(e, WeaponEvent::Explosion { kind: ExplosionKind::Bomb, .. }));
            if bomb_explosion {
                break;
            }
        }
        assert!(bomb_explosion);
        assert_eq!(w.list.len(), 8);
        assert!(w.list.iter().all(|b| b.data.kind() == WeaponKind::Bullet));
    }

    #[test]
    fn mine_stays_put() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bomb, 0).with_alternate(true), 0, &map, &s, &[], None);
        for _ in 0..500 {
            w.tick(&map, &s, &[], None, &[]);
        }
        assert_eq!(w.list[0].position, Vec2::new(512.0, 512.0));
    }

    #[test]
    fn weapons_vanish_when_owner_in_safe() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let mut w = Weapons::default();
        w.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 0), 0, &map, &s, &[], None);
        w.tick(&map, &s, &[], None, &[1]);
        assert!(w.list.is_empty());
    }

    #[test]
    fn bullet_damage_scales_with_level() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let me = Target { pid: 2, freq: 1, ship: 0, position: Vec2::new(512.0, 512.0) };
        let mut wpn = Weapons::default();
        wpn.fire(shooter(10), WeaponData::new(WeaponKind::Bullet, 2), 0, &map, &s, &[], None);
        let b = &wpn.list[0];
        // 100 + 50 * 2 with ExactDamage.
        assert_eq!(damage_to_local(b, &me, None, 0, &s, &map, |_| 0), 200);
    }

    #[test]
    fn bomb_damage_falls_off_with_distance() {
        let s = ArenaSettings::default();
        let map = Map::empty();
        let mut wpn = Weapons::default();
        wpn.fire(shooter(10), WeaponData::new(WeaponKind::Bomb, 0), 0, &map, &s, &[], None);
        let mut b = wpn.list[0].clone();
        b.position = Vec2::new(600.0, 600.0);
        let at = |dx: f32| Target { pid: 2, freq: 1, ship: 0, position: Vec2::new(600.0 + dx / 16.0, 600.0) };
        let shooter_far = Some(Vec2::new(0.0, 0.0));
        let center = damage_to_local(&b, &at(0.0), shooter_far, 0, &s, &map, |_| 0);
        let edge = damage_to_local(&b, &at(70.0), shooter_far, 0, &s, &map, |_| 0);
        let outside = damage_to_local(&b, &at(81.0), shooter_far, 0, &s, &map, |_| 0);
        assert_eq!(center, 750);
        assert!(edge > 0 && edge < 200);
        assert_eq!(outside, 0);
    }
}
