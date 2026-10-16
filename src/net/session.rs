//! Game session: login, arena entry, settings, map download, players and positions.
//! VIE by default, with an opt-in Continuum transport and login format.

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use super::transport::{Event, State, Transport, now_tick};
use crate::lvl::{Map, crc32};
use crate::settings::ArenaSettings;
use crate::sim::{Ship, Vec2};
use crate::net::vie::VieRng;
use crate::weapons::{Shooter, WeaponData};

pub const SPECTATOR: u8 = 8;
const POSITION_INTERVAL_TICKS: u32 = 10;
const SPECTATOR_POSITION_INTERVAL_TICKS: u32 = 100;

fn u16_at(p: &[u8], o: usize) -> Option<u16> {
    p.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn i16_at(p: &[u8], o: usize) -> Option<i16> {
    u16_at(p, o).map(|v| v as i16)
}
fn u32_at(p: &[u8], o: usize) -> Option<u32> {
    p.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}
fn fixed_str(s: &str, len: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s.bytes().take(len - 1).collect();
    v.resize(len, 0);
    v
}

pub const LOGIN_RESPONSES: [&str; 19] = [
    "OK",
    "new user",
    "invalid password",
    "zone is full",
    "locked out",
    "permission only",
    "spectate only",
    "too many points",
    "connection too slow",
    "permission only",
    "no new connections",
    "invalid name",
    "obscene name",
    "billing server down (OK)",
    "server busy",
    "restricted",
    "new-user questionnaire requested",
    "too many demo users",
    "demo versions not allowed",
];

#[derive(Clone, Debug)]
pub struct Config {
    pub server: SocketAddr,
    pub name: String,
    pub password: String,
    /// Arena name; None joins any public arena.
    pub arena: Option<String>,
    /// Where downloaded maps are cached.
    pub map_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Connecting,
    LoggingIn,
    EnteringArena,
    /// In the arena; `ready` once settings and map are loaded.
    InArena,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Player {
    pub pid: u16,
    pub name: String,
    pub squad: String,
    pub ship: u8,
    pub freq: u16,
    pub position: Vec2,
    pub velocity: Vec2,
    pub direction: u8,
    /// Stealth/cloak/X-radar/antiwarp/flash bits from their last position packet.
    pub status: u8,
    /// Score: kill and flag points, kills (wins) and deaths (losses).
    pub kill_points: u32,
    pub flag_points: u32,
    pub wins: u16,
    pub losses: u16,
    /// Sum of the bounties of the players this one killed while we watched (for F2's "Ave").
    pub kill_bounty_sum: u32,
    /// The player this one is attached to as a turret.
    pub attached_to: Option<u16>,
    pub bounty: u16,
    pub energy: u16,
    /// Latency in 10 ms units, as relayed in position packets.
    pub ping: u8,
    /// Local tick when `energy` last arrived as extra position data.
    pub energy_tick: Option<u32>,
    /// Low 16 bits of the server timestamp of the last position update.
    last_timestamp: Option<u16>,
    pub position_updates: u32,
    /// Flags this player carries.
    pub flags_carried: u16,
    /// Killed and not yet seen again.
    pub dead: bool,
}

/// A shot fired by another player, to be spawned by the weapon simulation.
#[derive(Clone, Copy, Debug)]
pub struct RemoteFire {
    pub shooter: Shooter,
    pub data: WeaponData,
    /// Ticks the shot has already been in flight (latency compensation).
    pub catch_up: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Death {
    pub killer: u16,
    pub killed: u16,
    pub bounty: u16,
    /// Prize left where the victim died (0 = none).
    pub green: i8,
    /// The victim and killer as they were when the packet arrived: a ship change or departure
    /// handled later in the same frame must not change the explosion or the kill message.
    pub victim_name: String,
    pub victim_ship: u8,
    pub victim_position: Vec2,
    pub victim_velocity: Vec2,
    pub killer_name: String,
}

/// A flag on the map (or carried, when `dropped` is false).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub id: u16,
    pub x: u16,
    pub y: u16,
    /// Owning team; 0xFFFF = neutral.
    pub owner: u16,
    pub dropped: bool,
}

/// Prize traffic from the server for the game to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrizeEvent {
    /// New shared prize seed; restart green spawning in lockstep.
    Seed(u32),
    /// Someone picked up the green on this tile.
    Taken { x: u16, y: u16, prize: i16, pid: u16 },
    /// The server granted us `count` of a prize.
    Granted { prize: i16, count: u16 },
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub kind: u8,
    pub pid: u16,
    pub text: String,
}

pub struct Session {
    pub transport: Transport,
    continuum_id: Option<[u8; 64]>,
    login_machine_id: Option<u32>,
    login_timezone_bias: i16,
    pub config: Config,
    pub phase: Phase,
    pub pid: Option<u16>,
    pub players: BTreeMap<u16, Player>,
    pub settings: Option<Arc<ArenaSettings>>,
    pub map: Option<Arc<Map>>,
    pub map_name: Option<String>,
    pub chat: Vec<Chat>,
    /// Shots and deaths received since the game last drained them.
    pub remote_fires: Vec<RemoteFire>,
    pub deaths: Vec<Death>,
    pub prize_events: Vec<PrizeEvent>,
    pub flags: BTreeMap<u16, Flag>,
    /// Door RNG and the local tick of its last step; set from the server's security packet.
    doors: Option<(VieRng, u32)>,
    /// Bumped whenever doors change, so frontends can react (e.g. a door closing on a ship).
    pub door_generation: u32,
    /// "X entered arena" / "X left arena" lines for players arriving after we joined.
    pub arena_messages: Vec<String>,
    /// Status bits to send with our next position (0x10 = warp flash), then cleared.
    pub pending_status: u8,
    /// Our stealth/cloak/X-radar/antiwarp bits, sent with every position.
    pub status_bits: u8,
    /// Players seen warping (their position packet had the flash bit), to animate.
    pub warp_flashes: Vec<Vec2>,
    /// Powerball position packets (S2C 0x2E) for the game to apply.
    pub ball_packets: Vec<crate::soccer::BallPacket>,
    /// Bricks standing on the map: (tile x, tile y, team, local tick they expire).
    pub bricks: Vec<(u16, u16, u16, u32)>,
    /// Human-readable protocol log for diagnostics.
    pub log: Vec<String>,
    pub joined: bool,
    /// Where our camera is while spectating. Servers only relay ships near this point.
    pub spectate_at: Vec2,
    pending_security_key: Option<u32>,
    pub security_replies: u32,
    received_packet_types: [u32; 256],
    expected_map_checksum: u32,
    last_position_sent: u32,
    last_position_timestamp: u32,
}

impl Session {
    pub fn connect(config: Config) -> io::Result<Self> {
        let transport = Transport::connect(config.server)?;
        Ok(Self::with_transport(config, transport, None))
    }

    /// Connect using Continuum transport and its 165-byte login format.
    /// The identity field is caller-supplied; its billing-server interpretation
    /// is not part of the public zone protocol. No integrity response is forged.
    pub fn connect_continuum(
        config: Config,
        expand: impl FnMut(u32) -> Option<[u32; 20]> + Send + 'static,
        identity: [u8; 64],
    ) -> io::Result<Self> {
        let transport = Transport::connect_continuum(config.server, expand)?;
        Ok(Self::with_transport(config, transport, Some(identity)))
    }

    /// Connect with freshly encoded local identification and native timezone.
    pub fn connect_continuum_identified(
        config: Config,
        expand: impl FnMut(u32) -> Option<[u32; 20]> + Send + 'static,
        identity: super::continuum_identity::LoginIdentity,
    ) -> io::Result<Self> {
        let mut session = Self::connect_continuum(config, expand, identity.billing)?;
        session.login_machine_id = Some(identity.machine_id);
        session.login_timezone_bias = identity.timezone_bias;
        Ok(session)
    }

    fn with_transport(config: Config, transport: Transport, continuum_id: Option<[u8; 64]>) -> Self {
        Self {
            transport,
            continuum_id,
            login_machine_id: None,
            login_timezone_bias: 0,
            config,
            phase: Phase::Connecting,
            pid: None,
            players: BTreeMap::new(),
            settings: None,
            map: None,
            map_name: None,
            chat: Vec::new(),
            remote_fires: Vec::new(),
            deaths: Vec::new(),
            prize_events: Vec::new(),
            flags: BTreeMap::new(),
            doors: None,
            door_generation: 0,
            arena_messages: Vec::new(),
            pending_status: 0,
            status_bits: 0,
            warp_flashes: Vec::new(),
            bricks: Vec::new(),
            ball_packets: Vec::new(),
            log: Vec::new(),
            joined: false,
            spectate_at: Vec2::new(512.0, 512.0),
            pending_security_key: None,
            security_replies: 0,
            received_packet_types: [0; 256],
            expected_map_checksum: 0,
            last_position_sent: 0,
            last_position_timestamp: 0,
        }
    }

    /// True once we're in the arena with settings and map loaded.
    pub fn ready(&self) -> bool {
        self.phase == Phase::InArena && self.joined && self.settings.is_some() && self.map.is_some()
    }

    pub fn me(&self) -> Option<&Player> {
        self.pid.and_then(|p| self.players.get(&p))
    }

    fn note(&mut self, msg: String) {
        self.log.push(msg);
    }

    fn fail(&mut self, why: String) {
        eprintln!("[Session] {why}");
        eprintln!("[Session] disconnect state: phase {:?}, joined {}, map loaded {}, settings loaded {}, security pending {}, security replies {}",
            self.phase, self.joined, self.map.is_some(), self.settings.is_some(),
            self.pending_security_key.is_some(), self.security_replies);
        let counts: Vec<_> = self.received_packet_types.iter().enumerate()
            .filter(|(_, count)| **count != 0)
            .map(|(kind, count)| format!("{kind:02x}:{count}"))
            .collect();
        eprintln!("[Session] received game packet counts: {}", counts.join(", "));
        // Preserve only recent server notices, not player/private conversations.
        for chat in self.chat.iter().rev().filter(|c| matches!(c.kind, 0 | 6 | 8)).take(8).collect::<Vec<_>>().into_iter().rev() {
            eprintln!("[Session] server notice (type {}): {:?}", chat.kind, chat.text);
        }
        self.note(format!("failed: {why}"));
        self.phase = Phase::Failed(why);
    }

    /// Processes network traffic. Call every frame.
    pub fn update(&mut self) {
        for ev in self.transport.poll() {
            // Keep the original failure visible if other packets arrived in the same batch.
            if matches!(self.phase, Phase::Failed(_)) {
                break;
            }
            match ev {
                Event::Connected => {
                    eprintln!("[Session] encryption established");
                    self.note("encryption established".into());
                    self.send_login(false);
                    self.phase = Phase::LoggingIn;
                }
                Event::Packet(p) => self.handle(&p),
                Event::Disconnected(why) => {
                    let stage = match self.phase {
                        Phase::LoggingIn => " during login",
                        _ => "",
                    };
                    let mut reason = format!("{why}{stage}");
                    if let Some(notice) = self.chat.iter().rev().take(8).find(|c|
                        c.kind == 0 && c.text.starts_with("You are banned from SSC network")
                    ) {
                        reason.push_str(&format!("; server reported: {}", notice.text));
                    }
                    self.fail(reason);
                }
            }
        }
        self.update_doors();
        self.expire_bricks();
    }

    /// Detaches `pid` from its carrier and any turrets from it.
    fn detach_all(&mut self, pid: u16) {
        for pl in self.players.values_mut() {
            if pl.pid == pid || pl.attached_to == Some(pid) {
                pl.attached_to = None;
            }
        }
    }

    /// Asks to attach to `pid` as a turret (0xFFFF detaches).
    pub fn send_attach_request(&mut self, pid: u16) {
        let mut p = vec![0x10];
        p.extend_from_slice(&pid.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// As a carrier, shakes off all turrets.
    pub fn send_attach_drop(&mut self) {
        self.transport.send_reliable(&[0x14]);
    }

    /// Server tick now (for ball timestamps).
    pub fn server_now(&self) -> u32 {
        self.transport.server_tick()
    }

    /// Asks to pick up ball `id` (its last timestamp).
    pub fn send_ball_pickup(&mut self, id: u8, timestamp: u32) {
        let mut p = vec![0x20, id];
        p.extend_from_slice(&timestamp.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Throws ball `id` from pixel (x, y) with velocity in pixels per 10 s.
    pub fn send_ball_fire(&mut self, id: u8, x: u16, y: u16, vx: i16, vy: i16, pid: u16, timestamp: u32) {
        let mut p = vec![0x1F, id];
        for v in [x, y, vx as u16, vy as u16, pid] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&timestamp.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Reports our thrown ball reaching an enemy goal tile.
    pub fn send_ball_goal(&mut self, id: u8, x: i16, y: i16) {
        let mut p = vec![0x21, id];
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Asks the zone to drop a brick at our tile.
    pub fn send_drop_brick(&mut self, x: u16, y: u16) {
        let mut p = vec![0x1C];
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// S2C 0x21: a brick wall from (x1,y1) to (x2,y2) for a team, standing BrickTime ticks.
    fn on_brick(&mut self, p: &[u8]) {
        let (Some(x1), Some(y1), Some(x2), Some(y2), Some(team)) =
            (u16_at(p, 1), u16_at(p, 3), u16_at(p, 5), u16_at(p, 7), u16_at(p, 9))
        else {
            return;
        };
        let life = self.settings.as_ref().map_or(500, |s| s.brick_time);
        let until = now_tick().wrapping_add(life);
        let (dx, dy) = (x2 as f32 - x1 as f32, y2 as f32 - y1 as f32);
        let dist = (dx * dx + dy * dy).sqrt();
        let (sx, sy) = if dist > 0.0 { (dx / dist, dy / dist) } else { (0.0, 0.0) };
        let (mut x, mut y) = (x1 as f32, y1 as f32);
        let mut i = 0.0;
        while i <= dist {
            let (tx, ty) = (x as u16, y as u16);
            self.bricks.push((tx, ty, team, until));
            if let Some(map) = self.map.as_mut() {
                Arc::make_mut(map).set_tile(tx as i32, ty as i32, crate::lvl::TILE_BRICK);
            }
            x += sx;
            y += sy;
            i += 1.0;
        }
    }

    fn expire_bricks(&mut self) {
        let now = now_tick();
        let (gone, keep): (Vec<_>, Vec<_>) =
            self.bricks.drain(..).partition(|b| now.wrapping_sub(b.3) as i32 >= 0);
        self.bricks = keep;
        if let Some(map) = self.map.as_mut() {
            for (x, y, _, _) in gone {
                Arc::make_mut(map).set_tile(x as i32, y as i32, 0);
            }
        }
    }

    /// Steps the shared door RNG for every `DoorDelay` that has passed and applies the result.
    fn update_doors(&mut self) {
        let (Some(settings), Some((rng, last))) = (self.settings.as_ref(), self.doors.as_mut()) else { return };
        let Some(map) = self.map.as_mut() else { return };
        let delay = settings.door_delay.max(1);
        let elapsed = now_tick().wrapping_sub(*last) as i32;
        if elapsed < 0 {
            return;
        }
        let steps = elapsed as u32 / delay;
        if steps == 0 {
            return;
        }
        let mut seed = rng.seed as u8;
        for _ in 0..steps {
            seed = match settings.door_mode {
                -2 => rng.next() as u8,
                -1 => {
                    // Seven draws, each contributing one bit (bit 0 also sets bit 4).
                    let t: [u32; 7] = std::array::from_fn(|_| rng.next());
                    let bit = |v: u32, mask: u32, out: u8| if (v as i32 & mask as i32) != 0 { out } else { 0 };
                    bit(t[6], 0xF, 0x80)
                        + bit(t[5], 7, 0x40)
                        + bit(t[4], 3, 0x20)
                        + bit(t[3], 0xF, 0x08)
                        + bit(t[2], 7, 0x04)
                        + bit(t[1], 3, 0x02)
                        + bit(t[0], 1, 0x11)
                }
                m if m >= 0 => m as u8,
                _ => seed,
            };
            *last = last.wrapping_add(delay);
        }
        Arc::make_mut(map).seed_doors(seed);
        self.door_generation = self.door_generation.wrapping_add(1);
    }

    /// Advances remote players by one 10 ms tick, extrapolating from their last update.
    pub fn tick_players(&mut self) {
        for p in self.players.values_mut() {
            if Some(p.pid) != self.pid {
                p.position.x += p.velocity.x * crate::sim::TICK_SECONDS;
                p.position.y += p.velocity.y * crate::sim::TICK_SECONDS;
            }
        }
    }

    fn send_login(&mut self, new_user: bool) {
        let mut p = vec![if self.continuum_id.is_some() { 0x24 } else { 0x09 }, new_user as u8];
        p.extend(fixed_str(&self.config.name, 32));
        p.extend(fixed_str(&self.config.password, 32));
        let machine_id = self.login_machine_id.unwrap_or_else(|| crc32(self.config.name.as_bytes()) | 1);
        p.extend_from_slice(&machine_id.to_le_bytes());
        p.push(0x04); // connect type
        p.extend_from_slice(&self.login_timezone_bias.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        let version = if self.continuum_id.is_some() { 40u16 } else { 0x86 };
        p.extend_from_slice(&version.to_le_bytes());
        p.extend_from_slice(&444u16.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes()); // no optional client features
        p.extend_from_slice(&[0; 8]);
        // Original 0.40 and SS.NET LoginPacket report the actual destination,
        // encoded as a little-endian IPv4 numeric value (zero for IPv6).
        let remote_ip = match self.config.server.ip() {
            std::net::IpAddr::V4(ip) => u32::from(ip),
            std::net::IpAddr::V6(_) => 0,
        };
        p.extend_from_slice(&remote_ip.to_le_bytes());
        let port = self.transport.local_addr().map(|a| a.port()).unwrap_or(0);
        p.extend_from_slice(&port.to_le_bytes());
        p.extend_from_slice(&[0; 4]); // login ID (no zone redirect)
        p.extend_from_slice(&[0; 2]); // reserved
        debug_assert_eq!(p.len(), 101);
        if let Some(identity) = self.continuum_id {
            p.extend_from_slice(&identity);
        }
        eprintln!("[Session] sending login: packet type {:02x}, length {}, new user {new_user}", p[0], p.len());
        self.transport.send_reliable(&p);
    }

    fn send_arena_login(&mut self) {
        let (number, name) = match &self.config.arena {
            Some(a) => (0xFFFDu16, a.clone()),
            None => (0xFFFFu16, String::new()),
        };
        let mut p = vec![0x01, SPECTATOR];
        p.extend_from_slice(&0u16.to_le_bytes()); // voice
        p.extend_from_slice(&1024u16.to_le_bytes());
        p.extend_from_slice(&768u16.to_le_bytes());
        p.extend_from_slice(&number.to_le_bytes());
        p.extend(fixed_str(&name, 16));
        if self.continuum_id.is_some() {
            // Continuum 0.40 capture and SS.NET C2S_GoArenaContinuum: byte 26
            // selects optional LVZ downloads. We currently load only the map.
            p.push(0);
        }
        self.transport.send_reliable(&p);
        self.phase = Phase::EnteringArena;
        self.joined = false;
        self.players.clear();
        self.map = None;
        self.settings = None;
    }

    /// Requests a ship (0-7) or spectator mode (8).
    pub fn request_ship(&mut self, ship: u8) {
        self.transport.send_reliable(&[0x18, ship]);
    }

    /// Asks the server to send us `pid`'s positions wherever they are (spectator follow).
    pub fn send_spectate_request(&mut self, pid: u16) {
        let mut p = vec![0x08];
        p.extend_from_slice(&pid.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Asks to pick up a flag we're touching.
    pub fn send_flag_request(&mut self, id: u16) {
        let mut p = vec![0x13];
        p.extend_from_slice(&id.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Reports picking up the green at (x, y).
    pub fn send_take_green(&mut self, x: u16, y: u16, prize: i16) {
        let mut p = vec![0x07];
        p.extend_from_slice(&self.transport.server_tick().to_le_bytes());
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        p.extend_from_slice(&prize.to_le_bytes());
        if self.settings.as_ref().is_some_and(|s| s.take_prize_reliable) {
            self.transport.send_reliable(&p);
        } else {
            self.transport.send(&p);
        }
    }

    /// Requests a change of team (frequency).
    pub fn request_freq(&mut self, freq: u16) {
        let mut p = vec![0x0F];
        p.extend_from_slice(&freq.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    /// Tells the server we died, and who killed us.
    pub fn send_death(&mut self, killer: u16, bounty: u16) {
        let mut p = vec![0x05];
        p.extend_from_slice(&killer.to_le_bytes());
        p.extend_from_slice(&bounty.to_le_bytes());
        self.transport.send_reliable(&p);
    }

    pub fn send_public_chat(&mut self, text: &str) {
        let mut p = vec![0x06, 0x02, 0x00];
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend(text.bytes().take(240));
        p.push(0);
        self.transport.send_reliable(&p);
    }

    /// Sends our position if due. `ship` is None while spectating. A non-empty `weapon` is
    /// always sent immediately (that's how shots reach other players). `dead` hides the ship
    /// during the post-death enter delay.
    pub fn send_position(&mut self, ship: Option<&Ship>, weapon: WeaponData, dead: bool, force: bool) {
        if !self.ready() {
            return;
        }
        let now = now_tick();
        let interval = if ship.is_some() { POSITION_INTERVAL_TICKS } else { SPECTATOR_POSITION_INTERVAL_TICKS };
        if !force && weapon.0 == 0 && now.wrapping_sub(self.last_position_sent) < interval {
            return;
        }
        self.last_position_sent = now;

        let mut ts = self.transport.server_tick();
        // The server drops position packets whose timestamp isn't newer than the last one.
        if ts <= self.last_position_timestamp {
            ts = self.last_position_timestamp.wrapping_add(1);
        }
        self.last_position_timestamp = ts;

        let (dir, mut x, mut y, vx, vy, energy, bounty) = match ship {
            Some(s) => (
                s.direction(),
                (s.position.x * 16.0) as u16,
                (s.position.y * 16.0) as u16,
                (s.velocity.x * 160.0) as i16,
                (s.velocity.y * 160.0) as i16,
                s.energy as u16,
                s.bounty as u16,
            ),
            None => (
                0,
                (self.spectate_at.x * 16.0) as u16,
                (self.spectate_at.y * 16.0) as u16,
                0,
                0,
                0,
                0,
            ),
        };
        if dead && ship.is_some() {
            (x, y) = (0xFFFF, 0xFFFF);
        }
        let mut p = Vec::with_capacity(22);
        p.push(0x03);
        p.push(dir);
        p.extend_from_slice(&ts.to_le_bytes());
        p.extend_from_slice(&vx.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        p.push(0); // checksum, filled below
        p.push(self.status_bits | std::mem::take(&mut self.pending_status)); // status (0x10 = flash)
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&vy.to_le_bytes());
        p.extend_from_slice(&bounty.to_le_bytes());
        p.extend_from_slice(&energy.to_le_bytes());
        p.extend_from_slice(&weapon.0.to_le_bytes());
        p[10] = p.iter().fold(0, |a, b| a ^ b);
        self.transport.send(&p);
    }

    fn handle(&mut self, p: &[u8]) {
        let Some(&kind) = p.first() else { return };
        self.received_packet_types[kind as usize] = self.received_packet_types[kind as usize].saturating_add(1);
        match kind {
            0x0A => {
                let code = p.get(1).copied().unwrap_or(0xFF);
                let text = LOGIN_RESPONSES.get(code as usize).copied().unwrap_or("unknown");
                eprintln!("[Session] login response {code}: {text}");
                self.note(format!("login response {code}: {text}"));
                // Public nullspace PasswordResponse and SS.NET S2C_LoginResponse:
                // byte 19 requests demographics independently of the login result.
                let demographics = p.get(19).copied();
                eprintln!("[Session] questionnaire flag: {}", match demographics {
                    Some(0) => "not requested",
                    Some(_) => "requested",
                    None => "absent (short login response)",
                });
                if code == 0x10 || (matches!(code, 0x00 | 0x06 | 0x0D)
                    && demographics.is_some_and(|value| value != 0))
                {
                    self.fail("New-user questionnaire required.\nAIContinuum cannot submit it yet.\nComplete it in the original Continuum client with this same account\nand server, then reconnect.".into());
                    self.transport.disconnect();
                    return;
                }
                match code {
                    0x00 | 0x0D => self.send_arena_login(),
                    0x01 => self.send_login(true),
                    _ => self.fail(format!("login refused: {text}")),
                }
            }
            0x01 => {
                if let Some(pid) = u16_at(p, 1) {
                    self.pid = Some(pid);
                    self.note(format!("assigned player id {pid}"));
                }
            }
            0x02 => {
                self.joined = true;
                self.phase = Phase::InArena;
                eprintln!("[Session] joined arena");
                self.note("joined arena".into());
                self.try_answer_security();
            }
            0x03 => {
                for rec in p.chunks_exact(64) {
                    let pl = Player {
                        ship: rec[1],
                        name: cstr(&rec[3..23]),
                        squad: cstr(&rec[23..43]),
                        pid: u16_at(rec, 51).unwrap_or(0),
                        freq: u16_at(rec, 53).unwrap_or(0),
                        position: Vec2::default(),
                        velocity: Vec2::default(),
                        direction: 0,
                        status: 0,
                        kill_points: u32_at(rec, 43).unwrap_or(0),
                        flag_points: u32_at(rec, 47).unwrap_or(0),
                        wins: u16_at(rec, 55).unwrap_or(0),
                        losses: u16_at(rec, 57).unwrap_or(0),
                        bounty: 0,
                        energy: 0,
                        ping: 0,
                        energy_tick: None,
                        last_timestamp: None,
                        position_updates: 0,
                        dead: false,
                        flags_carried: u16_at(rec, 61).unwrap_or(0),
                        attached_to: u16_at(rec, 59).filter(|&a| a != 0xFFFF),
                        kill_bounty_sum: 0,
                    };
                    self.note(format!("player entered: {} (pid {}, ship {})", pl.name, pl.pid, pl.ship));
                    if self.joined {
                        self.arena_messages.push(format!("{} entered arena", pl.name));
                    }
                    self.players.insert(pl.pid, pl);
                }
            }
            0x04 => {
                if let Some(pid) = u16_at(p, 1) {
                    if let Some(pl) = self.players.remove(&pid) {
                        self.detach_all(pid);
                        self.note(format!("player left: {}", pl.name));
                        self.arena_messages.push(format!("{} left arena", pl.name));
                    }
                }
            }
            0x21 => self.on_brick(p),
            0x2E => {
                if let Some(b) = crate::soccer::BallPacket::parse(p) {
                    self.ball_packets.push(b);
                }
            }
            // Goal scored (team, points): the original's message for it isn't measured yet.
            0x0B => self.note(format!("goal by freq {:?}", u16_at(p, 1))),
            // Turret link: requester attached to destination (0xFFFF = detached); a short packet
            // detaches us.
            0x0E => {
                let Some(requester) = u16_at(p, 1) else { return };
                self.note(format!("turret link: {requester} -> {:?}", u16_at(p, 3)));
                match u16_at(p, 3) {
                    None => {
                        if let Some(me) = self.pid.and_then(|pid| self.players.get_mut(&pid)) {
                            me.attached_to = None;
                        }
                    }
                    Some(0xFFFF) => {
                        if let Some(pl) = self.players.get_mut(&requester) {
                            pl.attached_to = None;
                        }
                    }
                    Some(dest) => {
                        let at = self.players.get(&dest).map(|d| d.position);
                        if let Some(pl) = self.players.get_mut(&requester) {
                            pl.attached_to = Some(dest);
                            if let Some(at) = at {
                                pl.position = at;
                            }
                        }
                    }
                }
            }
            // A carrier dropped all its turrets.
            0x15 => {
                if let Some(pid) = u16_at(p, 1) {
                    if self.me().is_some_and(|m| m.attached_to == Some(pid)) {
                        self.send_attach_request(0xFFFF);
                    }
                    for pl in self.players.values_mut().filter(|pl| pl.attached_to == Some(pid)) {
                        pl.attached_to = None;
                    }
                }
            }
            // Score update: pid, kill points, flag points, kills, deaths.
            0x09 => {
                if let (Some(pid), Some(kp), Some(fp), Some(w), Some(l)) =
                    (u16_at(p, 1), u32_at(p, 3), u32_at(p, 7), u16_at(p, 11), u16_at(p, 13))
                {
                    if let Some(pl) = self.players.get_mut(&pid) {
                        (pl.kill_points, pl.flag_points, pl.wins, pl.losses) = (kp, fp, w, l);
                    }
                }
            }
            0x05 => self.on_large_position(p),
            0x28 => self.on_small_position(p),
            0x06 => {
                if let (Some(killer), Some(killed), Some(bounty)) = (u16_at(p, 2), u16_at(p, 4), u16_at(p, 6)) {
                    self.detach_all(killed);
                    if let Some(pl) = self.players.get_mut(&killed) {
                        pl.dead = true;
                        pl.losses = pl.losses.wrapping_add(1);
                    }
                    if let Some(pl) = self.players.get_mut(&killer) {
                        pl.wins = pl.wins.wrapping_add(1);
                        pl.kill_bounty_sum += bounty as u32;
                    }
                    let name = |pid| self.players.get(&pid).map(|p| p.name.clone()).unwrap_or_default();
                    let (victim_name, killer_name) = (name(killed), name(killer));
                    self.note(format!("{victim_name} killed by {killer_name} ({bounty})"));
                    let victim = self.players.get(&killed);
                    self.deaths.push(Death {
                        killer,
                        killed,
                        bounty,
                        green: p[1] as i8,
                        victim_name,
                        victim_ship: victim.map_or(SPECTATOR, |v| v.ship),
                        victim_position: victim.map_or_else(Vec2::default, |v| v.position),
                        victim_velocity: victim.map_or_else(Vec2::default, |v| v.velocity),
                        killer_name,
                    });
                }
            }
            0x07 => {
                if p.len() >= 5 {
                    let c = Chat { kind: p[1], pid: u16_at(p, 3).unwrap_or(0), text: cstr(&p[5..]) };
                    self.chat.push(c);
                }
            }
            0x12 => {
                if let (Some(id), Some(x), Some(y), Some(owner)) = (u16_at(p, 1), u16_at(p, 3), u16_at(p, 5), u16_at(p, 7)) {
                    self.flags.insert(id, Flag { id, x, y, owner, dropped: true });
                }
            }
            0x13 => {
                if let (Some(id), Some(pid)) = (u16_at(p, 1), u16_at(p, 3)) {
                    let freq = self.players.get(&pid).map(|p| p.freq);
                    let carry = self.settings.as_ref().map_or(0, |s| s.carry_flags);
                    if let Some(f) = self.flags.get_mut(&id) {
                        if carry > 0 {
                            f.dropped = false;
                            if let Some(pl) = self.players.get_mut(&pid) {
                                pl.flags_carried += 1;
                            }
                        } else if let Some(freq) = freq {
                            f.owner = freq;
                        }
                    }
                }
            }
            0x16 => {
                if let Some(pid) = u16_at(p, 1) {
                    if let Some(pl) = self.players.get_mut(&pid) {
                        pl.flags_carried = 0;
                    }
                }
            }
            0x08 => {
                if let (Some(x), Some(y), Some(prize), Some(pid)) = (u16_at(p, 5), u16_at(p, 7), i16_at(p, 9), u16_at(p, 11)) {
                    self.prize_events.push(PrizeEvent::Taken { x, y, prize, pid });
                }
            }
            0x20 => {
                if let (Some(count), Some(prize)) = (u16_at(p, 1), i16_at(p, 3)) {
                    self.prize_events.push(PrizeEvent::Granted { prize, count });
                }
            }
            0x0F => match ArenaSettings::from_packet(p) {
                Some(s) => {
                    self.settings = Some(Arc::new(s));
                    eprintln!("[Session] received arena settings");
                    self.note("received arena settings".into());
                    self.try_answer_security();
                }
                None => self.note(format!("malformed settings packet ({} bytes)", p.len())),
            },
            0x1D => {
                if let (Some(&ship), Some(pid), Some(freq)) = (p.get(1), u16_at(p, 2), u16_at(p, 4)) {
                    if let Some(pl) = self.players.get_mut(&pid) {
                        pl.ship = ship;
                        pl.freq = freq;
                    }
                    self.note(format!("pid {pid} -> ship {ship} freq {freq}"));
                }
            }
            0x0D => {
                if let (Some(pid), Some(freq)) = (u16_at(p, 1), u16_at(p, 3)) {
                    if let Some(pl) = self.players.get_mut(&pid) {
                        pl.freq = freq;
                    }
                }
            }
            0x18 => {
                if let Some(seed) = u32_at(p, 1) {
                    self.prize_events.push(PrizeEvent::Seed(seed));
                }
                if let (Some(door_seed), Some(ts)) = (u32_at(p, 5), u32_at(p, 9)) {
                    let local = (ts as i64 - self.transport.time_diff as i64) as u32 & 0x7FFF_FFFF;
                    self.doors = Some((VieRng::new(door_seed), local));
                    self.update_doors();
                }
                let key = u32_at(p, 13).unwrap_or(0);
                if key != 0 {
                    eprintln!("[Session] received map/settings security request");
                    self.pending_security_key = Some(key);
                    self.try_answer_security();
                }
            }
            0x29 => self.on_map_info(p),
            0x2A => self.on_compressed_map(p),
            _ => {}
        }
    }

    fn on_map_info(&mut self, p: &[u8]) {
        let (Some(name_bytes), Some(checksum)) = (p.get(1..17), u32_at(p, 17)) else { return };
        let name = cstr(name_bytes);
        self.expected_map_checksum = checksum;
        self.map_name = Some(name.clone());
        let cached = self.config.map_dir.join(&name);
        if let Ok(data) = std::fs::read(&cached) {
            if crc32(&data) == checksum {
                self.note(format!("map {name} loaded from cache"));
                self.load_map(&data);
                return;
            }
        }
        eprintln!("[Session] requesting map download");
        self.note(format!("downloading map {name}"));
        // Continuum file requests include a little-endian index (zero is the map).
        // See nullspace FileRequester::SendNextRequest.
        let request: &[u8] = if self.continuum_id.is_some() { &[0x0C, 0, 0] } else { &[0x0C] };
        self.transport.send_reliable(request);
    }

    fn on_compressed_map(&mut self, p: &[u8]) {
        let Some(name_bytes) = p.get(1..17) else { return };
        let name = cstr(name_bytes);
        let data = match miniz_oxide::inflate::decompress_to_vec_zlib(&p[17..]) {
            Ok(d) => d,
            Err(e) => return self.fail(format!("map {name} failed to decompress: {e:?}")),
        };
        if crc32(&data) != self.expected_map_checksum {
            return self.fail(format!("map {name} checksum mismatch"));
        }
        if std::fs::create_dir_all(&self.config.map_dir).is_ok() {
            let _ = std::fs::write(self.config.map_dir.join(&name), &data);
        }
        self.note(format!("map {name} downloaded ({} bytes)", data.len()));
        self.load_map(&data);
    }

    fn load_map(&mut self, data: &[u8]) {
        match Map::parse(data) {
            Ok(m) => {
                eprintln!("[Session] map loaded");
                self.map = Some(Arc::new(m));
                self.try_answer_security();
            }
            Err(e) => self.fail(format!("map parse error: {e}")),
        }
    }

    /// Replies to the server's security request once settings and map are known.
    /// The exe checksum is sent as 0: AIContinuum is not SubSpace 1.35 and doesn't claim to be.
    fn try_answer_security(&mut self) {
        let Some(key) = self.pending_security_key else { return };
        let (Some(settings), Some(map)) = (&self.settings, &self.map) else { return };
        let mut p = vec![0x1A];
        p.extend_from_slice(&0u32.to_le_bytes()); // weapons received
        p.extend_from_slice(&settings.checksum(key).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // exe checksum
        p.extend_from_slice(&map.checksum(key).to_le_bytes());
        p.extend_from_slice(&[0; 8]); // S2C slow/fast totals
        p.extend_from_slice(&[0; 6]); // slow/fast current, reliable out
        let ping = (self.transport.rtt * 10 / 2 / 10) as u16;
        for _ in 0..4 {
            p.extend_from_slice(&ping.to_le_bytes());
        }
        p.push(0); // slow frames
        self.transport.send_reliable(&p);
        self.pending_security_key = None;
        self.security_replies += 1;
        eprintln!("[Session] sent map/settings security reply (executable checksum unspecified)");
    }

    fn accept_timestamp(pl: &mut Player, ts: u16) -> bool {
        let newer = match pl.last_timestamp {
            None => true,
            Some(prev) => (ts.wrapping_sub(prev) as i16) > 0,
        };
        if newer {
            pl.last_timestamp = Some(ts);
        }
        newer
    }

    fn on_large_position(&mut self, p: &[u8]) {
        if p.len() < 21 {
            return;
        }
        let pid = u16_at(p, 8).unwrap();
        let timestamp = u16_at(p, 2).unwrap();
        let weapon = WeaponData(u16_at(p, 19).unwrap());
        let Some(pl) = self.players.get_mut(&pid) else { return };
        // Shots are processed even when the packet's position is stale.
        if weapon.0 != 0 && pl.ship < 8 {
            let server_now = self.transport.server_tick();
            let sent = (server_now & 0x7FFF_0000) | timestamp as u32;
            let their_ping = p[14] as i32;
            let my_ping = (self.transport.rtt / 2) as i32;
            let elapsed = (server_now.wrapping_sub(sent) as i32) + their_ping + my_ping;
            self.remote_fires.push(RemoteFire {
                shooter: Shooter {
                    pid,
                    freq: pl.freq,
                    ship: pl.ship,
                    direction: p[1],
                    x: u16_at(p, 4).unwrap() as u32,
                    y: u16_at(p, 15).unwrap() as u32,
                    vel_x: i16_at(p, 10).unwrap() as i32,
                    vel_y: i16_at(p, 6).unwrap() as i32,
                },
                data: weapon,
                catch_up: elapsed.clamp(0, 100) as u32,
            });
        }
        if !Self::accept_timestamp(pl, timestamp) {
            return;
        }
        pl.dead = false;
        pl.direction = p[1];
        pl.position = Vec2::new(u16_at(p, 4).unwrap() as f32 / 16.0, u16_at(p, 15).unwrap() as f32 / 16.0);
        pl.status = p.get(13).copied().unwrap_or(0);
        if p.get(13).is_some_and(|st| st & 0x10 != 0) {
            self.warp_flashes.push(pl.position);
        }
        pl.velocity = Vec2::new(i16_at(p, 10).unwrap() as f32 / 160.0, i16_at(p, 6).unwrap() as f32 / 160.0);
        pl.bounty = u16_at(p, 17).unwrap();
        pl.ping = p[14];
        pl.position_updates += 1;
        if p.len() >= 23 {
            pl.energy = u16_at(p, 21).unwrap();
            pl.energy_tick = Some(now_tick());
        }
    }

    fn on_small_position(&mut self, p: &[u8]) {
        if p.len() < 16 {
            return;
        }
        let pid = p[8] as u16;
        let Some(pl) = self.players.get_mut(&pid) else { return };
        if !Self::accept_timestamp(pl, u16_at(p, 2).unwrap()) {
            return;
        }
        pl.dead = false;
        pl.direction = p[1];
        pl.bounty = p[7] as u16;
        pl.ping = p[6];
        pl.position = Vec2::new(u16_at(p, 4).unwrap() as f32 / 16.0, u16_at(p, 12).unwrap() as f32 / 16.0);
        pl.status = p.get(9).copied().unwrap_or(0);
        if p.get(9).is_some_and(|st| st & 0x10 != 0) {
            self.warp_flashes.push(pl.position);
        }
        pl.velocity = Vec2::new(i16_at(p, 14).unwrap() as f32 / 160.0, i16_at(p, 10).unwrap() as f32 / 160.0);
        pl.position_updates += 1;
        if p.len() >= 18 {
            pl.energy = u16_at(p, 16).unwrap();
            pl.energy_tick = Some(now_tick());
        }
    }

    pub fn disconnect(&mut self) {
        self.transport.disconnect();
    }

    pub fn is_connected(&self) -> bool {
        self.transport.state == State::Connected
    }
}
