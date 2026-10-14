//! Game session: login, arena entry, settings, map download, players and positions.
//! Speaks the VIE (SubSpace 1.35) client protocol on top of [`Transport`].

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use super::transport::{Event, State, Transport, now_tick};
use crate::lvl::{Map, crc32};
use crate::settings::ArenaSettings;
use crate::sim::{Ship, Vec2};

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
    "demo version",
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
    pub bounty: u16,
    pub energy: u16,
    /// Low 16 bits of the server timestamp of the last position update.
    last_timestamp: Option<u16>,
    pub position_updates: u32,
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub kind: u8,
    pub pid: u16,
    pub text: String,
}

pub struct Session {
    pub transport: Transport,
    pub config: Config,
    pub phase: Phase,
    pub pid: Option<u16>,
    pub players: BTreeMap<u16, Player>,
    pub settings: Option<ArenaSettings>,
    pub map: Option<Map>,
    pub map_name: Option<String>,
    pub chat: Vec<Chat>,
    /// Human-readable protocol log for diagnostics.
    pub log: Vec<String>,
    pub joined: bool,
    /// Where our camera is while spectating. Servers only relay ships near this point.
    pub spectate_at: Vec2,
    pending_security_key: Option<u32>,
    pub security_replies: u32,
    expected_map_checksum: u32,
    last_position_sent: u32,
    last_position_timestamp: u32,
}

impl Session {
    pub fn connect(config: Config) -> io::Result<Self> {
        let transport = Transport::connect(config.server)?;
        Ok(Self {
            transport,
            config,
            phase: Phase::Connecting,
            pid: None,
            players: BTreeMap::new(),
            settings: None,
            map: None,
            map_name: None,
            chat: Vec::new(),
            log: Vec::new(),
            joined: false,
            spectate_at: Vec2::new(512.0, 512.0),
            pending_security_key: None,
            security_replies: 0,
            expected_map_checksum: 0,
            last_position_sent: 0,
            last_position_timestamp: 0,
        })
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
        self.note(format!("failed: {why}"));
        self.phase = Phase::Failed(why);
    }

    /// Processes network traffic. Call every frame.
    pub fn update(&mut self) {
        for ev in self.transport.poll() {
            match ev {
                Event::Connected => {
                    self.note("encryption established".into());
                    self.send_login(false);
                    self.phase = Phase::LoggingIn;
                }
                Event::Packet(p) => self.handle(&p),
                Event::Disconnected(why) => self.fail(why),
            }
        }
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
        let mut p = vec![0x09, new_user as u8];
        p.extend(fixed_str(&self.config.name, 32));
        p.extend(fixed_str(&self.config.password, 32));
        p.extend_from_slice(&(crc32(self.config.name.as_bytes()) | 1).to_le_bytes()); // machine id
        p.push(0x04); // connect type
        p.extend_from_slice(&0u16.to_le_bytes()); // timezone bias
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&0x86u16.to_le_bytes()); // client version (1.34/VIE)
        p.extend_from_slice(&444u16.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes()); // no optional client features
        p.extend_from_slice(&[0; 8]);
        p.extend_from_slice(&[0; 4]); // remote ip (unused)
        let port = self.transport.local_addr().map(|a| a.port()).unwrap_or(0) as u32;
        p.extend_from_slice(&port.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        debug_assert_eq!(p.len(), 101);
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

    pub fn send_public_chat(&mut self, text: &str) {
        let mut p = vec![0x06, 0x02, 0x00];
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend(text.bytes().take(240));
        p.push(0);
        self.transport.send_reliable(&p);
    }

    /// Sends our position if due. `ship` is None while spectating.
    pub fn send_position(&mut self, ship: Option<&Ship>, force: bool) {
        if !self.ready() {
            return;
        }
        let now = now_tick();
        let interval = if ship.is_some() { POSITION_INTERVAL_TICKS } else { SPECTATOR_POSITION_INTERVAL_TICKS };
        if !force && now.wrapping_sub(self.last_position_sent) < interval {
            return;
        }
        self.last_position_sent = now;

        let mut ts = self.transport.server_tick();
        // The server drops position packets whose timestamp isn't newer than the last one.
        if ts <= self.last_position_timestamp {
            ts = self.last_position_timestamp.wrapping_add(1);
        }
        self.last_position_timestamp = ts;

        let (dir, x, y, vx, vy, energy) = match ship {
            Some(s) => (
                s.direction(),
                (s.position.x * 16.0) as u16,
                (s.position.y * 16.0) as u16,
                (s.velocity.x * 160.0) as i16,
                (s.velocity.y * 160.0) as i16,
                s.energy as u16,
            ),
            None => (
                0,
                (self.spectate_at.x * 16.0) as u16,
                (self.spectate_at.y * 16.0) as u16,
                0,
                0,
                0,
            ),
        };
        let mut p = Vec::with_capacity(22);
        p.push(0x03);
        p.push(dir);
        p.extend_from_slice(&ts.to_le_bytes());
        p.extend_from_slice(&vx.to_le_bytes());
        p.extend_from_slice(&y.to_le_bytes());
        p.push(0); // checksum, filled below
        p.push(0); // status flags
        p.extend_from_slice(&x.to_le_bytes());
        p.extend_from_slice(&vy.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes()); // bounty
        p.extend_from_slice(&energy.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes()); // weapon
        p[10] = p.iter().fold(0, |a, b| a ^ b);
        self.transport.send(&p);
    }

    fn handle(&mut self, p: &[u8]) {
        match p[0] {
            0x0A => {
                let code = p.get(1).copied().unwrap_or(0xFF);
                let text = LOGIN_RESPONSES.get(code as usize).copied().unwrap_or("unknown");
                self.note(format!("login response {code}: {text}"));
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
                        bounty: 0,
                        energy: 0,
                        last_timestamp: None,
                        position_updates: 0,
                    };
                    self.note(format!("player entered: {} (pid {}, ship {})", pl.name, pl.pid, pl.ship));
                    self.players.insert(pl.pid, pl);
                }
            }
            0x04 => {
                if let Some(pid) = u16_at(p, 1) {
                    if let Some(pl) = self.players.remove(&pid) {
                        self.note(format!("player left: {}", pl.name));
                    }
                }
            }
            0x05 => self.on_large_position(p),
            0x28 => self.on_small_position(p),
            0x07 => {
                if p.len() >= 5 {
                    let c = Chat { kind: p[1], pid: u16_at(p, 3).unwrap_or(0), text: cstr(&p[5..]) };
                    self.chat.push(c);
                }
            }
            0x0F => match ArenaSettings::from_packet(p) {
                Some(s) => {
                    self.settings = Some(s);
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
                let key = u32_at(p, 13).unwrap_or(0);
                if key != 0 {
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
        self.note(format!("downloading map {name}"));
        self.transport.send_reliable(&[0x0C]);
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
                self.map = Some(m);
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
        let Some(pl) = self.players.get_mut(&pid) else { return };
        if !Self::accept_timestamp(pl, u16_at(p, 2).unwrap()) {
            return;
        }
        pl.direction = p[1];
        pl.position = Vec2::new(u16_at(p, 4).unwrap() as f32 / 16.0, u16_at(p, 15).unwrap() as f32 / 16.0);
        pl.velocity = Vec2::new(i16_at(p, 10).unwrap() as f32 / 160.0, i16_at(p, 6).unwrap() as f32 / 160.0);
        pl.bounty = u16_at(p, 17).unwrap();
        pl.position_updates += 1;
        if p.len() >= 23 {
            pl.energy = u16_at(p, 21).unwrap();
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
        pl.direction = p[1];
        pl.bounty = p[7] as u16;
        pl.position = Vec2::new(u16_at(p, 4).unwrap() as f32 / 16.0, u16_at(p, 12).unwrap() as f32 / 16.0);
        pl.velocity = Vec2::new(i16_at(p, 14).unwrap() as f32 / 160.0, i16_at(p, 10).unwrap() as f32 / 160.0);
        pl.position_updates += 1;
    }

    pub fn disconnect(&mut self) {
        self.transport.disconnect();
    }

    pub fn is_connected(&self) -> bool {
        self.transport.state == State::Connected
    }
}
