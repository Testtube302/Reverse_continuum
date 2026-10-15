//! The startup screen: profile, ship choice, zone list with ping/players, the zone download
//! dialog and the password prompt. Drawn into an RGBA framebuffer the size of the skin's main
//! region, so the frontend only uploads and shows it 1:1.
//!
//! Layout and text measured against the original at 16 bpp (see NOTES.md, "Startup screen"):
//! zone rows 14 px apart from the top of ZONETEXT, ping right-aligned in 3 cells from x+4, name
//! from x+40, players right-aligned ending 4 px short of the right edge, selection bar 2 px above
//! the text and 15 px tall; INFOBOX text in font band 4, word-wrapped to the box, each line
//! centred on the box's middle, 14 px apart, lines that don't fit are dropped.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use crate::assets::RgbaImage;
use crate::dialog::{Dialog, Event, IDCANCEL, IDOK, Item, Template};
use crate::menubar::{self, Cmd};
use crate::skin::{Hotspot, Rect, Skin, Step};
use crate::winfont::{BitmapFont, TAHOMA_8};


pub const GLYPH_W: i32 = 8;
pub const GLYPH_H: i32 = 12;
const ROW_PITCH: i32 = 14;
const INFO_PITCH: i32 = 14;
/// Interface animation tick (skins guide: ~200 ms).
const TICK: Duration = Duration::from_millis(200);
const PING_EVERY: Duration = Duration::from_secs(3);
const PING_TIMEOUT: Duration = Duration::from_secs(8);

// Font bands in graphics/menutext.bm2 (8 colours x 96 glyphs, two rows of 48 per band).
const BAND_GREEN: u8 = 1;
const BAND_RED: u8 = 3;
const BAND_YELLOW: u8 = 4;

const SEL_FILL: [u8; 3] = [66, 65, 132];
const SEL_EDGE: [u8; 3] = [99, 97, 165];

pub const SHIP_NAMES: [&str; 8] = ["Warbird", "Javelin", "Spider", "Leviathan", "Terrier", "Weasel", "Lancaster", "Shark"];

const INFO_DEFAULT: &str = "Move the mouse over any item and read the description here.";
const INFO_ZONES_LIST: &str = "Select a zone to play in. Red zones may be down. Hold right mouse button down on a zone for its description.";

fn info_for(hotspot: &str) -> Option<String> {
    let s = match hotspot {
        "profile" => "View, edit, and select a player profile. This includes name, password, ship, resolution, zone, macros, and keyboard definitions.",
        "profbox" | "proftext" | "prevprof" | "nextprof" => "Information about your selected player profile.",
        "spectate" => "Spectate (watch the game).",
        "shipbox" => "Select one of eight ships to fly, or be a spectator. You can always change your mind while in the game.",
        "zones" => "View and edit the displayed zone list. Download a list of new zones.",
        "trace" => "View a real-time graph of your network connection to the selected zone.",
        "zoneup" => "Scroll the zone list up.",
        "zonedown" => "Scroll the zone list down.",
        "zonebox" | "zonetext" => INFO_ZONES_LIST,
        "play" => "Connect to the selected zone.",
        "quit" => "Leave the game. Why would you ever do this?",
        "infobox" => INFO_DEFAULT,
        s if s.starts_with("ship") => {
            let n: usize = s[4..].parse().ok()?;
            return SHIP_NAMES.get(n.checked_sub(1)?).map(|name| format!("Select ship: {name}."));
        }
        _ => return None,
    };
    Some(s.to_string())
}

/// Word-wraps to `width` characters.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ---------------------------------------------------------------------------------------------
// zone.dat and profile.dat

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub dns: String,
    /// Shown on the main screen (green in the zones dialog).
    pub shown: bool,
    pub description: String,
    pub players: Option<u32>,
    pub ping_ms: Option<u32>,
}

impl Zone {
    pub fn new(name: &str, ip: &str, port: u16) -> Self {
        Self {
            name: name.into(),
            ip: ip.into(),
            port,
            dns: String::new(),
            shown: false,
            description: String::new(),
            players: None,
            ping_ms: None,
        }
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        let host = if self.ip.is_empty() { &self.dns } else { &self.ip };
        (host.as_str(), self.port).to_socket_addrs().ok()?.next()
    }
}

/// `name,ip,port,dns,shown` lines, each optionally followed by a `#description` line.
pub fn parse_zone_dat(text: &str) -> Vec<Zone> {
    let mut zones: Vec<Zone> = Vec::new();
    for line in text.lines().map(|l| l.trim_end_matches('\r')) {
        if let Some(desc) = line.strip_prefix('#') {
            if let Some(z) = zones.last_mut() {
                z.description = desc.to_string();
            }
            continue;
        }
        // Names may contain commas: the last four fields are fixed.
        let mut f: Vec<&str> = line.rsplitn(5, ',').collect();
        if f.len() != 5 {
            continue;
        }
        f.reverse();
        let Ok(port) = f[2].trim().parse() else { continue };
        let mut z = Zone::new(f[0], f[1].trim(), port);
        z.dns = f[3].trim().to_string();
        z.shown = f[4].trim() == "1";
        zones.push(z);
    }
    zones
}

pub fn write_zone_dat(zones: &[Zone]) -> String {
    let mut out = String::new();
    for z in zones {
        out += &format!("{},{},{},{},{}\r\n", z.name, z.ip, z.port, z.dns, z.shown as u8);
        if !z.description.is_empty() {
            out += &format!("#{}\r\n", z.description);
        }
    }
    out
}

/// `key = value` lines, kept in order so unknown keys survive a save.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub entries: Vec<(String, String)>,
}

impl Profile {
    pub fn parse(text: &str) -> Self {
        let entries = text
            .lines()
            .filter(|l| !l.trim_start().starts_with(';'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        Self { entries }
    }

    pub fn defaults() -> Self {
        let mut p = Self::default();
        for (k, v) in [("profile", "Default"), ("name", "Name"), ("ship", "8"), ("zone", ""), ("macros", "Default"), ("keyboard", "Default")] {
            p.set(k, v);
        }
        p
    }

    pub fn get(&self, key: &str) -> &str {
        self.entries.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v)
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = value.to_string(),
            None => self.entries.push((key.to_string(), value.to_string())),
        }
    }

    /// 0-7 for a ship, 8 for spectator.
    pub fn ship(&self) -> u8 {
        self.get("ship").parse().ok().filter(|s| *s <= 8).unwrap_or(8)
    }

    pub fn write(&self) -> String {
        let mut out = String::from("; This file is auto-generated. DO NOT EDIT.\r\n\r\n");
        for (k, v) in &self.entries {
            out += &format!("{k:<11}= {v}\r\n");
        }
        out + "\r\n"
    }
}

impl Trace {
    /// Sends one ping a second and records replies; true when the lines changed.
    fn tick(&mut self) -> bool {
        let (Some(socket), Some(target)) = (&self.socket, self.target) else {
            if self.lines.is_empty() {
                self.lines.push("No zone selected.".into());
                return true;
            }
            return false;
        };
        let mut changed = false;
        let now_ms = self.started.elapsed().as_millis() as u32;
        if self.last_send.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            // A cycle with no reply by the next send is a lost packet.
            if self.last_send.is_some() && self.lines.last().is_some_and(|l| l.starts_with("Sent")) {
                let n = self.lines.len() - 1;
                self.lines[n] = format!("Cycle {}: no reply (lost)", self.cycles);
            }
            self.cycles += 1;
            self.last_send = Some(Instant::now());
            let _ = socket.send_to(&now_ms.to_le_bytes(), target);
            self.lines.push(format!("Sent cycle {}", self.cycles));
            changed = true;
        }
        let mut buf = [0u8; 64];
        while let Ok((n, from)) = socket.recv_from(&mut buf) {
            if n >= 8 && from == target {
                let players = u32::from_le_bytes(buf[0..4].try_into().unwrap()) & 0xFFFF;
                let sent = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                let rtt = now_ms.wrapping_sub(sent);
                if let Some(last) = self.lines.last_mut().filter(|l| l.starts_with("Sent")) {
                    *last = format!("Cycle {}: {} ms, {} players", self.cycles, rtt, players);
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Every profile in profile.dat and which one is selected. The original writes one profile;
/// we append the others, each starting at its `profile =` line, and note the selection in a
/// `selected =` line before them (both still read as a single profile by a plain parser).
pub fn parse_profiles(text: &str) -> (Vec<Profile>, usize) {
    let mut selected = String::new();
    let mut out: Vec<Profile> = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with(';')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if k == "selected" && out.is_empty() {
            selected = v.to_string();
            continue;
        }
        if k == "profile" || out.is_empty() {
            out.push(Profile::default());
        }
        out.last_mut().unwrap().entries.push((k.to_string(), v.to_string()));
    }
    if out.is_empty() {
        out.push(Profile::defaults());
    }
    let active = out.iter().position(|p| p.get("profile") == selected).unwrap_or(0);
    (out, active)
}

pub fn write_profiles(profiles: &[Profile], active: usize) -> String {
    let mut out = String::from("; This file is auto-generated. DO NOT EDIT.\r\n\r\n");
    if profiles.len() > 1 {
        out += &format!("{:<11}= {}\r\n\r\n", "selected", profiles.get(active).map_or("", |p| p.get("profile")));
    }
    for p in profiles {
        for (k, v) in &p.entries {
            out += &format!("{k:<11}= {v}\r\n");
        }
        out += "\r\n";
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Zone ping (UDP to game port + 1): send a 4-byte timestamp, get back <players u32><timestamp>.

pub struct Pinger {
    socket: Option<UdpSocket>,
    start: Instant,
    last_round: Option<Instant>,
    /// Per zone: address pinged and when it last answered.
    targets: Vec<(SocketAddr, Option<Instant>)>,
    /// Replies stamped on arrival by a receiver thread: (from, players, echoed ms, arrival ms).
    replies: Option<std::sync::mpsc::Receiver<(SocketAddr, u32, u32, u32)>>,
}

impl Pinger {
    pub fn new() -> Self {
        let start = Instant::now();
        let socket = UdpSocket::bind("0.0.0.0:0").ok();
        let replies = socket.as_ref().and_then(|s| s.try_clone().ok()).map(|rx_socket| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = rx_socket.set_read_timeout(Some(Duration::from_millis(250)));
            std::thread::spawn(move || {
                let mut buf = [0u8; 64];
                loop {
                    match rx_socket.recv_from(&mut buf) {
                        Ok((n, from)) if n >= 8 => {
                            let players = u32::from_le_bytes(buf[0..4].try_into().unwrap()) & 0xFFFF;
                            let sent = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                            if tx.send((from, players, sent, start.elapsed().as_millis() as u32)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                            // Exit once the Pinger is gone.
                            if tx.send((([0, 0, 0, 0], 0).into(), 0, 0, 0)).is_err() {
                                return;
                            }
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
            });
            rx
        });
        Self { socket, start, last_round: None, targets: Vec::new(), replies }
    }

    fn now_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    /// Sends a round of pings when due and folds replies into `zones` (ping, players).
    pub fn update(&mut self, zones: &mut [Zone]) {
        let Some(socket) = &self.socket else { return };
        if self.targets.len() != zones.len() {
            self.targets = zones
                .iter()
                .map(|z| (z.addr().map(|a| SocketAddr::new(a.ip(), a.port().wrapping_add(1))).unwrap_or(([0, 0, 0, 0], 0).into()), None))
                .collect();
            self.last_round = None;
        }
        if self.last_round.is_none_or(|t| t.elapsed() >= PING_EVERY) {
            self.last_round = Some(Instant::now());
            let stamp = self.now_ms().to_le_bytes();
            for (addr, _) in &self.targets {
                if addr.port() != 0 {
                    let _ = socket.send_to(&stamp, addr);
                }
            }
        }
        if let Some(rx) = &self.replies {
            while let Ok((from, players, sent, arrived)) = rx.try_recv() {
                if from.port() == 0 {
                    continue;
                }
                for (i, (addr, seen)) in self.targets.iter_mut().enumerate() {
                    if *addr == from {
                        *seen = Some(Instant::now());
                        zones[i].players = Some(players);
                        zones[i].ping_ms = Some(arrived.wrapping_sub(sent).min(999));
                    }
                }
            }
        }
        for (i, (_, seen)) in self.targets.iter().enumerate() {
            if seen.is_none_or(|t| t.elapsed() > PING_TIMEOUT) {
                zones[i].ping_ms = None;
                zones[i].players = None;
            }
        }
    }
}

impl Default for Pinger {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// Directory servers: core-protocol connection to UDP 4990, request `01 <min players u32>`,
// reply `01` then entries of ip[4] port u16 players u16 billing u16 version u32 name[64]
// description NUL-terminated.

#[derive(Clone, Debug, PartialEq)]
pub struct DirZone {
    pub ip: String,
    pub port: u16,
    pub players: u16,
    pub name: String,
    pub description: String,
}

pub fn parse_directory_list(p: &[u8]) -> Vec<DirZone> {
    let mut out = Vec::new();
    let mut o = 1;
    while o + 78 <= p.len() {
        let ip = format!("{}.{}.{}.{}", p[o], p[o + 1], p[o + 2], p[o + 3]);
        let port = u16::from_le_bytes([p[o + 4], p[o + 5]]);
        let players = u16::from_le_bytes([p[o + 6], p[o + 7]]);
        let name_bytes = &p[o + 14..o + 78];
        let name = String::from_utf8_lossy(name_bytes.split(|&b| b == 0).next().unwrap_or(&[])).trim().to_string();
        let rest = &p[o + 78..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let description = String::from_utf8_lossy(&rest[..end]).trim().to_string();
        o += 78 + end + 1;
        if !name.is_empty() {
            out.push(DirZone { ip, port, players, name, description });
        }
    }
    out
}

/// Downloads the zone list from one directory server (blocking, gives up after `timeout`).
pub fn download_directory(host: &str, timeout: Duration) -> Result<Vec<DirZone>, String> {
    use crate::net::transport::{Event, Transport};
    let (host, port) = match host.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().unwrap()),
        _ => (host, 4990),
    };
    let addr = (host, port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.next().ok_or(format!("{host}: no address"))?;
    let mut t = Transport::connect(addr).map_err(|e| format!("{host}: {e}"))?;
    let start = Instant::now();
    let mut requested = false;
    while start.elapsed() < timeout {
        for ev in t.poll() {
            match ev {
                Event::Connected if !requested => {
                    requested = true;
                    t.send_reliable(&[0x01, 0, 0, 0, 0]);
                }
                Event::Packet(p) if p.first() == Some(&0x01) => {
                    t.disconnect();
                    return Ok(parse_directory_list(&p));
                }
                Event::Disconnected(why) => return Err(format!("{host}: {why}")),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    t.disconnect();
    Err(format!("{host}: no answer"))
}

/// Folds a downloaded list into zone.dat entries: known names get fresh address, players and
/// description; new zones are added unselected.
pub fn merge_directory(zones: &mut Vec<Zone>, found: &[DirZone]) {
    for d in found {
        match zones.iter_mut().find(|z| z.name.eq_ignore_ascii_case(&d.name)) {
            Some(z) => {
                z.ip = d.ip.clone();
                z.port = d.port;
                z.description = d.description.clone();
                z.players = Some(d.players as u32);
            }
            None => {
                let mut z = Zone::new(&d.name, &d.ip, d.port);
                z.description = d.description.clone();
                z.players = Some(d.players as u32);
                zones.push(z);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
    /// Added to every coordinate (the skin is drawn below the menu bar).
    pub origin: (i32, i32),
    /// Optional clip rectangle (x0, y0, x1, y1 inclusive) in canvas coordinates.
    pub clip: Option<(i32, i32, i32, i32)>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Self { w, h, px: vec![0; (w * h * 4) as usize], origin: (0, 0), clip: None }
    }

    /// Inverts a pixel (XOR with white), as Windows draws focus rectangles.
    pub fn invert(&mut self, x: i32, y: i32) {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        if self.clipped(x, y) {
            return;
        }
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let o = ((y * self.w + x) * 4) as usize;
            for k in 0..3 {
                self.px[o + k] = 255 - self.px[o + k];
            }
        }
    }

    fn clipped(&self, x: i32, y: i32) -> bool {
        self.clip.is_some_and(|(x0, y0, x1, y1)| x < x0 || y < y0 || x > x1 || y > y1)
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        if self.clipped(x, y) {
            return;
        }
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let o = ((y * self.w + x) * 4) as usize;
            self.px[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }

    pub fn fill(&mut self, x0: i32, y0: i32, w: i32, h: i32, c: [u8; 3]) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.put(x, y, c);
            }
        }
    }

    /// Copies a `w`x`h` block of `src` at (sx, sy) to (dx, dy); pixels with alpha 0 are skipped.
    pub fn blit(&mut self, src: &RgbaImage, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
        for y in 0..h {
            for x in 0..w {
                let (ix, iy) = (sx + x, sy + y);
                if ix < 0 || iy < 0 || ix >= src.width as i32 || iy >= src.height as i32 {
                    continue;
                }
                let o = ((iy * src.width as i32 + ix) * 4) as usize;
                if src.pixels[o + 3] != 0 {
                    self.put(dx + x, dy + y, [src.pixels[o], src.pixels[o + 1], src.pixels[o + 2]]);
                }
            }
        }
    }
}

/// graphics/menutext.bm2: 8x12 glyphs for chars 32..127, 48 per row, two rows per colour band.
pub struct MenuFont {
    pub image: RgbaImage,
}

impl MenuFont {
    pub fn draw(&self, c: &mut Canvas, text: &str, band: u8, x: i32, y: i32) {
        for (i, ch) in text.chars().enumerate() {
            let code = ch as i32 - 32;
            if !(0..96).contains(&code) {
                continue;
            }
            let (row, col) = (band as i32 * 2 + code / 48, code % 48);
            c.blit(&self.image, col * GLYPH_W, row * GLYPH_H, GLYPH_W, GLYPH_H, x + i as i32 * GLYPH_W, y);
        }
    }

    /// Draws using only the glyph shapes, in a flat colour (dialog text).
    pub fn draw_flat(&self, c: &mut Canvas, text: &str, color: [u8; 3], x: i32, y: i32) {
        for (i, ch) in text.chars().enumerate() {
            let code = ch as i32 - 32;
            if !(0..96).contains(&code) {
                continue;
            }
            let (sx, sy) = ((code % 48) * GLYPH_W, (code / 48) * GLYPH_H);
            for gy in 0..GLYPH_H {
                for gx in 0..GLYPH_W {
                    let o = (((sy + gy) * self.image.width as i32 + sx + gx) * 4) as usize;
                    if self.image.pixels[o + 3] != 0 {
                        c.put(x + i as i32 * GLYPH_W + gx, y + gy, color);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The screen

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub mouse: (i32, i32),
    pub left_pressed: bool,
    pub right_down: bool,
    pub double_click: bool,
    pub wheel: i32,
    pub chars: Vec<char>,
    pub enter: bool,
    pub escape: bool,
    pub backspace: bool,
    pub tab: bool,
    /// A letter pressed with Ctrl held (menu shortcuts).
    pub ctrl_letter: Option<char>,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub home: bool,
    pub end: bool,
    pub alt_f4: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Play { addr: SocketAddr, zone: String, name: String, password: String, ship: u8, fullscreen: bool, resolution: (u16, u16) },
    Quit,
    /// Open a web page.
    Url(String),
}

/// Height of the Windows menu bar above the skin.
pub const MENU_H: i32 = 19;
// Dialog template ids and control ids in menu040.dll.
const DLG_ZONES: u32 = 104;
const DLG_PROFILE: u32 = 118;
const DLG_PASSWORD: u32 = 142;
const ZONES_LIST: i32 = 1004;
const ZONES_SORT: i32 = 1215;
const ZONES_ADD_REMOVE: i32 = 1020;
const ZONES_DOWNLOAD: i32 = 1019;
const ZONES_MIN_PLAYERS: i32 = 1002;
const ZONES_SERVERS: i32 = 1111;
const ZONES_DESCRIPTION: i32 = 1005;
const ZONES_STATUS: i32 = 1165;
const PROF_LIST: i32 = 1020;
const PROF_NAME: i32 = 1021;
const PROF_PLAYER: i32 = 1022;
const PROF_PASSWORD: i32 = 1023;
const PROF_SHIP: i32 = 1024;
const PROF_RESOLUTION: i32 = 1031;
const PROF_REFRESH: i32 = 1194;
const PROF_WINDOW: i32 = 1032;
const PROF_ZONE: i32 = 1004;
const PROF_MACROS: i32 = 1026;
const PROF_KEYS: i32 = 1027;
const PROF_TARGET: i32 = 1043;
const PROF_CHATS: i32 = 1042;
const PROF_AUTO: i32 = 1044;
const PROF_MOVE_UP: i32 = 1202;
const PROF_MOVE_DOWN: i32 = 1203;
const PROF_NEW: i32 = 1028;
const PROF_COPY: i32 = 1030;
const PROF_DELETE: i32 = 1029;
const PASS_EDIT: i32 = 1023;
const PASS_SAVE: i32 = 1201;
const TIP_TEXT: i32 = 1227;
const DLG_CUSTOM: u32 = 176;
const ZONES_ADD_CUSTOM: i32 = 1021;
const CUSTOM_NAME: i32 = 1007;
const CUSTOM_IP: i32 = 1008;
const CUSTOM_PORT: i32 = 1260;
const DLG_TRACE: u32 = 107;
const TRACE_ZONE: i32 = 1007;
const TRACE_IP: i32 = 1008;
const TRACE_LIST: i32 = 1209;
const TRACE_EXPORT: i32 = 1223;
const TRACE_CYCLES: i32 = 1010;
const DLG_PROXY: u32 = 151;
const PROXY_ON: i32 = 1216;
const PROXY_HOST: i32 = 1217;
const PROXY_PORT: i32 = 1218;
const PROXY_AUTH: i32 = 1219;
const PROXY_USER: i32 = 1038;
const PROXY_PASS: i32 = 1039;
const TIP_NEXT: i32 = 1226;

const ZONE_TEXT_GREY: [u8; 3] = [128, 128, 128];
const ZONE_TEXT_SHOWN: [u8; 3] = [0, 128, 0];
const ZONE_TEXT_SELECTED: [u8; 3] = [128, 255, 128];
const ZONE_SELECTION: [u8; 3] = [0, 0, 128];

enum Kind {
    Zones(ZonesDialog),
    /// The profiles being edited and which list row is shown in the fields.
    Profile(Vec<Profile>, usize),
    Password,
    /// Any other dialog: shown, closed by its buttons.
    Other,
    Tip(usize),
    Trace(Trace),
    Proxy,
}

/// The Trace dialog's live pings to one zone.
struct Trace {
    target: Option<SocketAddr>,
    socket: Option<UdpSocket>,
    started: Instant,
    last_send: Option<Instant>,
    cycles: u32,
    lines: Vec<String>,
}

struct Open {
    kind: Kind,
    d: Dialog,
}

struct ZonesDialog {
    /// "Add Custom Zone", open on top of this dialog.
    custom: Option<Dialog>,
    zones: Vec<Zone>,
    download: Option<std::sync::mpsc::Receiver<Result<Vec<DirZone>, String>>>,
}

/// Downloads from every listed directory server on a worker thread, merging what answers.
fn start_download(servers: &str) -> std::sync::mpsc::Receiver<Result<Vec<DirZone>, String>> {
    let servers: Vec<String> = servers.split_whitespace().map(String::from).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut all: Vec<DirZone> = Vec::new();
        let mut err = String::from("No directory server answered.");
        for s in &servers {
            match download_directory(s, Duration::from_secs(6)) {
                Ok(found) => {
                    for f in found {
                        if !all.iter().any(|a| a.name.eq_ignore_ascii_case(&f.name)) {
                            all.push(f);
                        }
                    }
                }
                Err(e) => err = e,
            }
        }
        let _ = tx.send(if all.is_empty() { Err(err) } else { Ok(all) });
    });
    rx
}

pub const DEFAULT_DIRECTORY_SERVERS: &str =
    "sscentral.sscuservers.net sscentral.trenchwars.org directory.poid.be subspace.gamespec.org";

pub struct Launcher {
    /// Index 0 is the built-in skin, then the install's skins/*.skn.
    skins: Vec<(String, Skin)>,
    skin_idx: usize,
    spectate_icon: RgbaImage,
    ships: RgbaImage,
    font: MenuFont,
    /// All profiles; `active` is the one in use.
    pub profiles: Vec<Profile>,
    pub active: usize,
    /// Every zone in zone.dat; the main list shows the `shown` ones.
    pub zones: Vec<Zone>,
    pub selected: Option<usize>,
    scroll: usize,
    pinger: Pinger,
    started: Instant,
    mouse: (i32, i32),
    right_down: bool,
    dialog: Option<Open>,
    templates: Vec<Template>,
    menu_font: BitmapFont,
    menu_open: Option<usize>,
    menu_hot: Option<usize>,
    tips: Vec<String>,
    /// Directory servers for the zones dialog.
    pub servers: String,
    /// Where Export writes files (the install folder).
    pub data_dir: Option<std::path::PathBuf>,
    /// Set whenever profile or zones change; the frontend saves and clears it.
    pub dirty: bool,
}

impl Launcher {
    pub fn new(
        skin: Skin,
        extra_skins: Vec<(String, Skin)>,
        tips: Vec<String>,
        spectate_icon: RgbaImage,
        ships: RgbaImage,
        font: RgbaImage,
        templates: Vec<Template>,
        profiles: (Vec<Profile>, usize),
        zones: Vec<Zone>,
    ) -> Self {
        let mut l = Self {
            skins: std::iter::once(("<default>".to_string(), skin)).chain(extra_skins).collect(),
            skin_idx: 0,
            spectate_icon,
            ships,
            font: MenuFont { image: font },
            active: profiles.1.min(profiles.0.len().saturating_sub(1)),
            profiles: profiles.0,
            zones,
            selected: None,
            scroll: 0,
            pinger: Pinger::new(),
            started: Instant::now(),
            mouse: (-1, -1),
            right_down: false,
            dialog: None,
            templates,
            menu_font: BitmapFont::parse(TAHOMA_8).expect("built-in font"),
            menu_open: None,
            menu_hot: None,
            tips,
            servers: DEFAULT_DIRECTORY_SERVERS.into(),
            data_dir: None,
            dirty: false,
        };
        let want = l.profiles[l.active].get("zone").to_string();
        l.selected = l.shown().iter().position(|&i| l.zones[i].name == want).or(if l.shown().is_empty() { None } else { Some(0) });
        l
    }

    fn skin(&self) -> &Skin {
        &self.skins[self.skin_idx].1
    }

    /// Picks the skin named in the profile, if installed.
    pub fn select_skin_from_profile(&mut self) {
        let want = self.profiles[self.active].get("skin").to_string();
        self.skin_idx = self.skins.iter().position(|(n, _)| *n == want).unwrap_or(0);
    }

    /// Window size: the menu bar plus the skin's main region.
    pub fn size(&self) -> (i32, i32) {
        (self.skin().main.w(), self.skin().main.h() + MENU_H)
    }

    /// The original's whole window (frame, title and menu) in our coordinates; dialogs centre on it.
    fn parent_frame(&self) -> Rect {
        let (w, h) = self.size();
        Rect { x0: -2, y0: -21, x1: w, y1: h }
    }

    /// Indexes into `zones` of the entries on the main list.
    fn shown(&self) -> Vec<usize> {
        (0..self.zones.len()).filter(|&i| self.zones[i].shown).collect()
    }

    fn rows(&self) -> usize {
        self.skin().hotspot("zonetext").map_or(0, |h| {
            let r = h.regions[0].rect;
            ((r.h() - GLYPH_H) / ROW_PITCH + 1).max(0) as usize
        })
    }

    fn rect(&self, name: &str) -> Option<Rect> {
        self.skin().hotspot(name).map(|h| h.regions[0].rect)
    }

    fn hit(&self, x: i32, y: i32) -> Option<&Hotspot> {
        // Buttons first, then the boxes that contain them.
        let boxes = ["profbox", "shipbox", "zonebox", "infobox", "proftext", "zonetext"];
        self.skin()
            .hotspots
            .iter()
            .filter(|h| !boxes.contains(&h.name.as_str()))
            .find(|h| h.contains(x, y))
            .or_else(|| ["zonetext", "proftext", "profbox", "shipbox", "zonebox", "infobox"].iter().filter_map(|n| self.skin().hotspot(n)).find(|h| h.contains(x, y)))
    }

    fn zone_row_at(&self, x: i32, y: i32) -> Option<usize> {
        let r = self.rect("zonetext")?;
        if !r.contains(x, y) {
            return None;
        }
        let row = ((y - r.y0 + 2) / ROW_PITCH) as usize;
        (row < self.rows()).then_some(self.scroll + row)
    }

    /// Handles one frame of input; returns what the frontend should do.
    pub fn update(&mut self, input: &Input) -> Option<Action> {
        // Main-screen coordinates are relative to the skin, below the menu bar.
        self.mouse = (input.mouse.0, input.mouse.1 - MENU_H);
        self.right_down = input.right_down;
        let shown = self.shown();
        // Ping only what's on the main list.
        let mut list: Vec<Zone> = shown.iter().map(|&i| self.zones[i].clone()).collect();
        self.pinger.update(&mut list);
        for (k, &i) in shown.iter().enumerate() {
            self.zones[i].ping_ms = list[k].ping_ms;
            self.zones[i].players = list[k].players;
        }
        if self.dialog.is_some() {
            return self.update_dialog(input);
        }
        let (used, action) = self.update_menus(input);
        if used {
            return action;
        }
        if let Some(action) = self.shortcut(input) {
            return action;
        }
        let (mx, my) = self.mouse;
        let rows = self.rows();
        let max_scroll = shown.len().saturating_sub(rows);
        if input.wheel != 0 && self.rect("zonebox").is_some_and(|r| r.contains(mx, my)) {
            self.scroll = (self.scroll as i32 - input.wheel).clamp(0, max_scroll as i32) as usize;
        }
        if input.enter {
            return self.play();
        }
        if input.escape {
            return Some(Action::Quit);
        }
        if !input.left_pressed {
            return None;
        }
        let name = self.hit(mx, my).map(|h| h.name.clone())?;
        match name.as_str() {
            "play" => return self.play(),
            "quit" => return Some(Action::Quit),
            "spectate" => self.set_ship(8),
            "profile" => self.open_profile(),
            "zones" => self.open_zones(),
            "zoneup" => self.scroll = self.scroll.saturating_sub(1),
            "zonedown" => self.scroll = (self.scroll + 1).min(max_scroll),
            "zonetext" => {
                if let Some(row) = self.zone_row_at(mx, my).filter(|&r| r < shown.len()) {
                    self.selected = Some(row);
                    self.profiles[self.active].set("zone", &self.zones[shown[row]].name.clone());
                    self.dirty = true;
                    if input.double_click {
                        return self.play();
                    }
                }
            }
            s if s.starts_with("ship") => {
                if let Some(n) = s[4..].parse::<u8>().ok().filter(|n| (1..=8).contains(n)) {
                    self.set_ship(n - 1);
                }
            }
            _ => {}
        }
        None
    }

    /// Opens a dialog by name ("zones", "zones-download", "profile", "password"); test aid.
    pub fn open(&mut self, which: &str) {
        match which {
            "zones" => self.open_zones(),
            "zones-download" => {
                self.open_zones();
                self.zones_command(ZONES_DOWNLOAD);
            }
            "profile" => self.open_profile(),
            "password" => self.open_password(),
            "profile-ship" => {
                self.open_profile();
                if let Some(c) = self.dialog.as_mut().and_then(|o| o.d.control_mut(PROF_SHIP)) {
                    c.dropped = true;
                    c.hot = c.sel;
                }
            }
            other => {
                if let Some(k) = other.strip_prefix("menu:").and_then(|k| k.parse().ok()) {
                    self.menu_open = Some(k);
                }
            }
        }
    }

    /// After switching or editing profiles: its skin and zone take effect.
    fn profile_changed(&mut self) {
        self.select_skin_from_profile();
        let want = self.profiles[self.active].get("zone").to_string();
        if let Some(row) = self.shown().iter().position(|&i| self.zones[i].name == want) {
            self.selected = Some(row);
        }
    }

    fn set_ship(&mut self, ship: u8) {
        self.profiles[self.active].set("ship", &ship.to_string());
        self.dirty = true;
    }

    fn new_dialog(&self, id: u32) -> Option<Dialog> {
        let t = self.templates.iter().find(|t| t.id == id)?;
        let mut d = Dialog::new(t, self.parent_frame());
        let (w, h) = self.size();
        d.clamp_into(w, h);
        Some(d)
    }

    fn open_password(&mut self) {
        let Some(mut d) = self.new_dialog(DLG_PASSWORD) else { return };
        d.focus_id(PASS_EDIT);
        self.dialog = Some(Open { kind: Kind::Password, d });
    }

    fn open_profile(&mut self) {
        let Some(mut d) = self.new_dialog(DLG_PROFILE) else { return };
        let list = self.profiles.clone();
        let cur = self.active;
        refresh_profile_list(&mut d, &list, cur);
        fill_profile_fields(&mut d, &list[cur], &self.zones);
        d.focus_id(PROF_LIST);
        self.dialog = Some(Open { kind: Kind::Profile(list, cur), d });
    }

    fn open_zones(&mut self) {
        let Some(mut d) = self.new_dialog(DLG_ZONES) else { return };
        d.set_text(ZONES_MIN_PLAYERS, "0");
        d.set_text(ZONES_SERVERS, &self.servers);
        let zones = self.zones.clone();
        let current = self.shown().get(self.selected.unwrap_or(usize::MAX)).map(|&i| self.zones[i].name.clone());
        let sel = current.and_then(|n| zones.iter().position(|z| z.name == n)).or(if zones.is_empty() { None } else { Some(0) });
        if let Some(c) = d.control_mut(ZONES_LIST) {
            c.sel = sel;
        }
        d.focus_id(IDOK);
        let mut open = Open { kind: Kind::Zones(ZonesDialog { custom: None, zones, download: None }), d };
        refresh_zones(&mut open);
        self.dialog = Some(open);
    }

    /// Same as clicking PLAY (test aid).
    pub fn press_play(&mut self) -> Option<Action> {
        self.play()
    }

    fn selected_zone(&self) -> Option<Zone> {
        let shown = self.shown();
        self.zones.get(*shown.get(self.selected?)?).cloned()
    }

    fn play(&mut self) -> Option<Action> {
        let zone = self.selected_zone()?;
        let password = self.profiles[self.active].get("password").to_string();
        if password.is_empty() {
            self.open_password();
            return None;
        }
        self.play_with(&zone, password)
    }

    fn play_with(&mut self, zone: &Zone, password: String) -> Option<Action> {
        let addr = zone.addr()?;
        Some(Action::Play {
            addr,
            zone: zone.name.clone(),
            name: self.profiles[self.active].get("name").to_string(),
            password,
            ship: self.profiles[self.active].ship(),
            fullscreen: self.profiles[self.active].get("windowed") != "1",
            resolution: self
                .profiles[self.active]
                .get("resolution")
                .split_once('x')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                .unwrap_or((1024, 768)),
        })
    }

    fn zones_command(&mut self, id: i32) {
        let custom = if id == ZONES_ADD_CUSTOM { self.new_dialog(DLG_CUSTOM) } else { None };
        let Some(Open { kind: Kind::Zones(z), d }) = self.dialog.as_mut() else { return };
        match id {
            ZONES_ADD_CUSTOM => {
                if let Some(mut c) = custom {
                    c.set_text(CUSTOM_PORT, "5000");
                    c.focus_id(CUSTOM_NAME);
                    z.custom = Some(c);
                }
            }
            ZONES_DOWNLOAD if z.download.is_none() => {
                self.servers = d.text(ZONES_SERVERS);
                z.download = Some(start_download(&self.servers));
                d.set_text(ZONES_STATUS, "Downloading...");
                if let Some(c) = d.control_mut(ZONES_DOWNLOAD) {
                    c.enabled = false;
                }
            }
            ZONES_ADD_REMOVE => {
                if let Some(i) = d.control(ZONES_LIST).and_then(|c| c.sel) {
                    if let Some(zone) = z.zones.get_mut(i) {
                        zone.shown = !zone.shown;
                    }
                }
            }
            _ => {}
        }
        if let Some(open) = self.dialog.as_mut() {
            refresh_zones(open);
        }
    }

    fn update_dialog(&mut self, input: &Input) -> Option<Action> {
        let open = self.dialog.as_mut()?;
        if let Kind::Trace(t) = &mut open.kind {
            if t.tick() {
                let lines: Vec<Item> = t.lines.iter().map(|l| Item::text(l)).collect();
                if let Some(c) = open.d.control_mut(TRACE_LIST) {
                    let rows = ((c.rect.h() - 4) / 13).max(1) as usize;
                    c.top = lines.len().saturating_sub(rows);
                    c.items = lines;
                }
                open.d.set_text(TRACE_CYCLES, &format!("Cycles: {}", t.cycles));
            }
        }
        // Add Custom Zone sits on top of the zones dialog and takes the input while open.
        if let Kind::Zones(z) = &mut open.kind {
            if let Some(events) = z.custom.as_mut().map(|c| c.handle(input)) {
                for ev in events {
                    match ev {
                        Event::Command(IDCANCEL) => {
                            z.custom = None;
                            break;
                        }
                        Event::Command(IDOK) => {
                            let Some(c) = z.custom.as_ref() else { break };
                            let name = c.text(CUSTOM_NAME).trim().to_string();
                            let ip = c.text(CUSTOM_IP).trim().to_string();
                            let port = c.text(CUSTOM_PORT).trim().parse::<u16>().unwrap_or(0);
                            if name.is_empty() || ip.is_empty() || port == 0 {
                                continue;
                            }
                            let mut zone = Zone::new(&name, &ip, port);
                            zone.shown = true;
                            z.zones.retain(|old| !old.name.eq_ignore_ascii_case(&name));
                            z.zones.push(zone);
                            let row = z.zones.len() - 1;
                            z.custom = None;
                            if let Some(list) = open.d.control_mut(ZONES_LIST) {
                                list.sel = Some(row);
                            }
                            refresh_zones(open);
                            break;
                        }
                        _ => {}
                    }
                }
                return None;
            }
        }
        // Finished downloads.
        if let Kind::Zones(z) = &mut open.kind {
            if let Some(result) = z.download.as_ref().and_then(|rx| rx.try_recv().ok()) {
                z.download = None;
                let min: u32 = open.d.text(ZONES_MIN_PLAYERS).parse().unwrap_or(0);
                let status = match result {
                    Ok(found) => {
                        let found: Vec<DirZone> = found.into_iter().filter(|f| f.players as u32 >= min).collect();
                        let before = z.zones.len();
                        merge_directory(&mut z.zones, &found);
                        if open.d.checked(ZONES_SORT) {
                            z.zones.sort_by_key(|z| z.name.to_ascii_lowercase());
                        }
                        format!("{} zones downloaded, {} new.", found.len(), z.zones.len() - before)
                    }
                    Err(e) => e,
                };
                open.d.set_text(ZONES_STATUS, &status);
                if let Some(c) = open.d.control_mut(ZONES_DOWNLOAD) {
                    c.enabled = true;
                }
                refresh_zones(open);
            }
        }
        let events = open.d.handle(input);
        let keep = self.selected_zone().map(|z| z.name);
        for ev in events {
            let Some(open) = self.dialog.as_mut() else { break };
            match (&mut open.kind, ev) {
                (_, Event::Command(IDCANCEL)) => {
                    self.dialog = None;
                }
                (Kind::Other | Kind::Trace(_), Event::Command(IDOK)) => {
                    self.dialog = None;
                }
                (Kind::Trace(t), Event::Command(TRACE_EXPORT)) => {
                    if let Some(dir) = &self.data_dir {
                        let path = dir.join("trace.txt");
                        let ok = std::fs::write(&path, t.lines.join("\r\n") + "\r\n").is_ok();
                        t.lines.push(if ok { format!("Exported to {}", path.display()) } else { "Export failed.".into() });
                        t.last_send = None;
                    }
                }
                (Kind::Proxy, Event::Command(IDOK)) => {
                    let d = &open.d;
                    let p = &mut self.profiles[self.active];
                    p.set("proxy", if d.checked(PROXY_ON) { "1" } else { "0" });
                    p.set("proxyhost", d.text(PROXY_HOST).trim());
                    p.set("proxyport", d.text(PROXY_PORT).trim());
                    p.set("proxyauth", if d.checked(PROXY_AUTH) { "1" } else { "0" });
                    p.set("proxyuser", d.text(PROXY_USER).trim());
                    p.set("proxypass", &d.text(PROXY_PASS));
                    self.dirty = true;
                    self.dialog = None;
                }
                (Kind::Tip(k), Event::Command(TIP_NEXT)) => {
                    let next = (*k + 1) % self.tips.len().max(1);
                    *k = next;
                    let text = self.tips.get(next).cloned().unwrap_or_default();
                    open.d.set_text(TIP_TEXT, &text);
                }
                (Kind::Password, Event::Command(IDOK)) => {
                    let password = open.d.text(PASS_EDIT);
                    if password.is_empty() {
                        continue;
                    }
                    if open.d.checked(PASS_SAVE) {
                        self.profiles[self.active].set("password", &password);
                        self.dirty = true;
                    }
                    self.dialog = None;
                    let zone = self.selected_zone()?;
                    return self.play_with(&zone, password);
                }
                (Kind::Profile(list, cur), Event::Command(IDOK)) => {
                    read_profile_fields(&open.d, &mut list[*cur]);
                    self.profiles = std::mem::take(list);
                    self.active = *cur;
                    self.dirty = true;
                    self.dialog = None;
                    self.profile_changed();
                }
                (Kind::Profile(list, cur), Event::Select(PROF_LIST, row)) => {
                    read_profile_fields(&open.d, &mut list[*cur]);
                    *cur = row.min(list.len() - 1);
                    fill_profile_fields(&mut open.d, &list[*cur], &self.zones);
                    refresh_profile_list(&mut open.d, list, *cur);
                }
                (Kind::Profile(list, cur), Event::Command(id @ (PROF_NEW | PROF_COPY | PROF_DELETE | PROF_MOVE_UP | PROF_MOVE_DOWN))) => {
                    read_profile_fields(&open.d, &mut list[*cur]);
                    let unique = |list: &[Profile], base: &str| {
                        (1..).map(|n| if n == 1 { base.to_string() } else { format!("{base} {n}") }).find(|c| !list.iter().any(|p| p.get("profile") == c)).unwrap()
                    };
                    match id {
                        PROF_NEW => {
                            let mut p = Profile::defaults();
                            p.set("profile", &unique(list, "New Profile"));
                            list.insert(*cur + 1, p);
                            *cur += 1;
                        }
                        PROF_COPY => {
                            let mut p = list[*cur].clone();
                            p.set("profile", &unique(list, &format!("Copy of {}", list[*cur].get("profile"))));
                            list.insert(*cur + 1, p);
                            *cur += 1;
                        }
                        PROF_DELETE if list.len() > 1 => {
                            list.remove(*cur);
                            *cur = (*cur).min(list.len() - 1);
                        }
                        PROF_MOVE_UP if *cur > 0 => {
                            list.swap(*cur, *cur - 1);
                            *cur -= 1;
                        }
                        PROF_MOVE_DOWN if *cur + 1 < list.len() => {
                            list.swap(*cur, *cur + 1);
                            *cur += 1;
                        }
                        _ => {}
                    }
                    fill_profile_fields(&mut open.d, &list[*cur], &self.zones);
                    refresh_profile_list(&mut open.d, list, *cur);
                }
                (Kind::Zones(z), Event::Command(IDOK)) => {
                    let mut zones = std::mem::take(&mut z.zones);
                    zones.sort_by_key(|z| z.name.to_ascii_lowercase());
                    let keep = keep.clone();
                    self.servers = open.d.text(ZONES_SERVERS);
                    self.zones = zones;
                    self.dialog = None;
                    self.dirty = true;
                    self.scroll = 0;
                    let shown = self.shown();
                    self.selected = keep
                        .and_then(|n| shown.iter().position(|&i| self.zones[i].name == n))
                        .or(if shown.is_empty() { None } else { Some(0) });
                }
                (Kind::Zones(z), Event::DoubleClick(ZONES_LIST, row)) => {
                    if let Some(zone) = z.zones.get_mut(row) {
                        zone.shown = !zone.shown;
                    }
                    refresh_zones(open);
                }
                (Kind::Zones(_), Event::Select(ZONES_LIST, _)) => refresh_zones(open),
                (Kind::Zones(z), Event::Command(ZONES_SORT)) => {
                    if open.d.checked(ZONES_SORT) {
                        let sel = open.d.control(ZONES_LIST).and_then(|c| c.sel).and_then(|i| z.zones.get(i)).map(|z| z.name.clone());
                        z.zones.sort_by_key(|z| z.name.to_ascii_lowercase());
                        if let Some(c) = open.d.control_mut(ZONES_LIST) {
                            c.sel = sel.and_then(|n| z.zones.iter().position(|z| z.name == n));
                        }
                    }
                    refresh_zones(open);
                }
                (Kind::Zones(_), Event::Command(id)) => self.zones_command(id),
                _ => {}
            }
        }
        None
    }

    fn tick(&self) -> u32 {
        (self.started.elapsed().as_millis() / TICK.as_millis()) as u32
    }

    /// Draws the whole screen.
    pub fn draw(&self, c: &mut Canvas) {
        c.origin = (0, 0);
        self.draw_menu_bar(c);
        c.origin = (0, MENU_H);
        let main = self.skin().main;
        c.blit(&self.skin().image, main.x0, main.y0, main.w(), main.h(), 0, 0);
        let (mx, my) = self.mouse;
        let hover = if self.dialog.is_none() { self.hit(mx, my).map(|h| h.name.clone()) } else { None };
        let shown = self.shown();
        let rows = self.rows();
        let ship = self.profiles[self.active].ship();
        for h in &self.skin().hotspots {
            let n = h.name.as_str();
            let alt = match n {
                "spectate" => ship == 8,
                s if s.starts_with("ship") => s[4..].parse::<u8>().is_ok_and(|k| k >= 1 && k - 1 == ship),
                "zoneup" => self.scroll > 0,
                "zonedown" => self.scroll + rows < shown.len(),
                _ => hover.as_deref() == Some(n),
            };
            for r in &h.regions {
                let steps = if alt && !r.alt.is_empty() { &r.alt } else { &r.norm };
                if let Some((sx, sy)) = step_at(steps, self.tick()) {
                    c.blit(&self.skin().image, sx, sy, r.rect.w(), r.rect.h(), r.rect.x0, r.rect.y0);
                }
            }
        }
        // Ship pictures: frame 5 of each ship in graphics/ships.bm2, centred on the button
        // (2 px inside the default skin's 40x40 buttons).
        for k in 0..8 {
            if let Some(r) = self.rect(&format!("ship{}", k + 1)) {
                c.blit(&self.ships, 5 * 36, k * 4 * 36, 36, 36, r.x0 + (r.w() - 36) / 2, r.y0 + (r.h() - 36) / 2);
            }
        }
        if let Some(r) = self.rect("spectate") {
            c.blit(&self.spectate_icon, 0, 0, 36, 36, r.x0 + (r.w() - 36) / 2, r.y0 + (r.h() - 36) / 2);
        }
        if let Some(r) = self.rect("proftext") {
            for (i, key) in ["profile", "name", "macros", "keyboard"].iter().enumerate() {
                let v: String = self.profiles[self.active].get(key).chars().take(23).collect();
                self.font.draw(c, &v, BAND_YELLOW, r.x0, r.y0 + i as i32 * 16);
            }
        }
        if let Some(r) = self.rect("zonetext") {
            for (row, &i) in shown.iter().skip(self.scroll).take(rows).enumerate() {
                let z = &self.zones[i];
                let y = r.y0 + row as i32 * ROW_PITCH;
                if self.selected == Some(self.scroll + row) {
                    c.fill(r.x0, y - 2, r.w() - 1, 15, SEL_EDGE);
                    c.fill(r.x0 + 1, y - 1, r.w() - 3, 13, SEL_FILL);
                }
                let band = match z.ping_ms {
                    None => BAND_RED,
                    Some(p) if p < 200 => BAND_GREEN,
                    Some(p) if p < 400 => BAND_YELLOW,
                    Some(_) => BAND_RED,
                };
                if let Some(p) = z.ping_ms {
                    let s = p.to_string();
                    self.font.draw(c, &s, band, r.x0 + 28 - s.len() as i32 * GLYPH_W, y);
                }
                let players_x = r.x1 - 4 - 3 * GLYPH_W;
                let max_name = ((players_x - GLYPH_W - (r.x0 + 40)) / GLYPH_W).max(0) as usize;
                let name: String = z.name.chars().take(max_name).collect();
                self.font.draw(c, &name, band, r.x0 + 40, y);
                if let (Some(n), Some(_)) = (z.players, z.ping_ms) {
                    let s = n.min(999).to_string();
                    self.font.draw(c, &s, BAND_YELLOW, r.x1 - 4 - s.len() as i32 * GLYPH_W, y);
                }
            }
        }
        // INFOBOX: the zone's description while the right button is held on it, else help.
        let info = match (self.right_down, self.zone_row_at(mx, my)) {
            (true, Some(row)) if self.dialog.is_none() && row < shown.len() => {
                let z = &self.zones[shown[row]];
                if z.description.is_empty() { z.name.clone() } else { z.description.clone() }
            }
            _ => hover.as_deref().and_then(info_for).unwrap_or_else(|| INFO_DEFAULT.into()),
        };
        if let Some(r) = self.rect("infobox") {
            let mid = (r.x0 + r.x1) / 2;
            // The original leaves one cell spare: 224 px wraps at 27 characters.
            for (i, line) in wrap(&info, (r.w() / GLYPH_W - 1) as usize).iter().enumerate() {
                let y = r.y0 + i as i32 * INFO_PITCH;
                if y + GLYPH_H - 1 > r.y1 {
                    break;
                }
                self.font.draw(c, line, BAND_YELLOW, mid - line.len() as i32 * GLYPH_W / 2, y);
            }
        }
        c.origin = (0, 0);
        if let Some(open) = &self.dialog {
            open.d.draw(c);
            if let Kind::Zones(ZonesDialog { custom: Some(cd), .. }) = &open.kind {
                cd.draw(c);
            }
        }
        self.draw_menu_popup(c);
    }

    fn menus(&self) -> Vec<menubar::Menu> {
        let res = self.profiles[self.active].get("resolution");
        let resolution = res
            .split_once('x')
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
            .unwrap_or((1024, 768));
        let names: Vec<String> = self.skins.iter().skip(1).map(|(n, _)| n.clone()).collect();
        menubar::menus(&menubar::State {
            resolution,
            windowed: self.profiles[self.active].get("windowed") == "1",
            skins: &names,
            skin: self.skin_idx,
        })
    }

    /// The Windows menu bar, and the open drop-down on top of everything.
    fn draw_menu_bar(&self, c: &mut Canvas) {
        let menus = self.menus();
        menubar::draw_bar(c, &self.menu_font, &menus, self.size().0, self.menu_open);
    }

    fn draw_menu_popup(&self, c: &mut Canvas) {
        if let Some(m) = self.menu_open.and_then(|k| self.menus().into_iter().nth(k)) {
            menubar::draw_popup(c, &self.menu_font, &m, self.size().0, self.menu_hot);
        }
    }

    /// Mouse and keys for the menu bar; returns true when the input was used.
    fn update_menus(&mut self, input: &Input) -> (bool, Option<Action>) {
        let menus = self.menus();
        let (w, _) = self.size();
        let (mx, my) = input.mouse;
        let f = &self.menu_font;
        if let Some(open) = self.menu_open {
            if let Some(h) = menubar::header_at(&menus, mx, my) {
                if h != open {
                    self.menu_open = Some(h);
                    self.menu_hot = None;
                }
            }
            let m = &menus[self.menu_open.unwrap_or(open)];
            let hot = m.item_at(f, w, mx, my);
            if hot.is_some() || !m.popup(f, w).contains(mx, my) {
                self.menu_hot = hot;
            }
            if input.escape {
                self.menu_open = None;
                return (true, None);
            }
            if input.up || input.down {
                let n = m.items.len();
                let step = if input.down { 1 } else { n - 1 };
                let mut k = self.menu_hot.unwrap_or(if input.down { n - 1 } else { 0 });
                for _ in 0..n {
                    k = (k + step) % n;
                    if m.items[k].cmd.is_some() {
                        break;
                    }
                }
                self.menu_hot = Some(k);
                return (true, None);
            }
            if input.left || input.right {
                let n = menus.len();
                let cur = self.menu_open.unwrap_or(open);
                self.menu_open = Some(if input.right { (cur + 1) % n } else { (cur + n - 1) % n });
                self.menu_hot = None;
                return (true, None);
            }
            let chosen = if input.left_pressed {
                if menubar::header_at(&menus, mx, my) == Some(open) {
                    self.menu_open = None;
                    return (true, None);
                }
                hot
            } else if input.enter {
                self.menu_hot
            } else {
                return (true, None);
            };
            let cmd = chosen.and_then(|k| m.items[k].cmd);
            self.menu_open = None;
            self.menu_hot = None;
            return (true, cmd.and_then(|c| self.run_command(c)));
        }
        if input.left_pressed {
            if let Some(h) = menubar::header_at(&menus, mx, my) {
                self.menu_open = Some(h);
                self.menu_hot = None;
                return (true, None);
            }
        }
        (false, None)
    }

    /// Menu commands and their keyboard shortcuts.
    fn run_command(&mut self, cmd: Cmd) -> Option<Action> {
        let shown = self.shown();
        match cmd {
            Cmd::Play => return self.play(),
            Cmd::Quit => return Some(Action::Quit),
            Cmd::Profiles => self.open_profile(),
            Cmd::Zones => self.open_zones(),
            Cmd::Macros => self.open_other(120),
            Cmd::KeyDefs => self.open_other(122),
            Cmd::Trace => self.open_trace(),
            Cmd::Options => self.open_other(159),
            Cmd::AdvancedOptions => self.open_other(113),
            Cmd::Proxy => self.open_proxy(),
            Cmd::Banners => self.open_other(161),
            Cmd::About => self.open_other(121),
            Cmd::Tip => self.open_tip(),
            Cmd::PrevProfile | Cmd::NextProfile => {
                let n = self.profiles.len();
                let next = if cmd == Cmd::NextProfile { (self.active + 1).min(n - 1) } else { self.active.saturating_sub(1) };
                if next != self.active {
                    self.active = next;
                    self.dirty = true;
                    self.profile_changed();
                }
            }
            Cmd::PrevShip | Cmd::NextShip => {
                // Spectator (8) sits before the Warbird on the ship panel.
                let order = [8u8, 0, 1, 2, 3, 4, 5, 6, 7];
                let k = order.iter().position(|&s| s == self.profiles[self.active].ship()).unwrap_or(0);
                let k = if cmd == Cmd::NextShip { (k + 1) % 9 } else { (k + 8) % 9 };
                self.set_ship(order[k]);
            }
            Cmd::PrevZone | Cmd::NextZone => {
                if !shown.is_empty() {
                    let cur = self.selected.unwrap_or(0);
                    let next = if cmd == Cmd::NextZone { (cur + 1).min(shown.len() - 1) } else { cur.saturating_sub(1) };
                    self.selected = Some(next);
                    self.profiles[self.active].set("zone", &self.zones[shown[next]].name.clone());
                    self.dirty = true;
                    let rows = self.rows();
                    if next < self.scroll {
                        self.scroll = next;
                    } else if next >= self.scroll + rows {
                        self.scroll = next + 1 - rows;
                    }
                }
            }
            Cmd::Resolution(w, h) => {
                self.profiles[self.active].set("resolution", &format!("{w}x{h}"));
                self.dirty = true;
            }
            Cmd::Windowed => {
                let on = self.profiles[self.active].get("windowed") == "1";
                self.profiles[self.active].set("windowed", if on { "0" } else { "1" });
                self.dirty = true;
            }
            Cmd::Skin(k) => {
                if k < self.skins.len() {
                    self.skin_idx = k;
                    self.profiles[self.active].set("skin", &self.skins[k].0.clone());
                    self.dirty = true;
                    self.scroll = 0;
                }
            }
            Cmd::Url(u) => return Some(Action::Url(u.to_string())),
            Cmd::ZoneNews => {}
        }
        None
    }

    fn shortcut(&mut self, input: &Input) -> Option<Option<Action>> {
        let cmd = match input.ctrl_letter {
            Some('p') => Cmd::Profiles,
            Some('m') => Cmd::Macros,
            Some('k') => Cmd::KeyDefs,
            Some('z') => Cmd::Zones,
            Some('t') => Cmd::Trace,
            Some('o') => Cmd::Options,
            Some('a') => Cmd::AdvancedOptions,
            Some('x') => Cmd::Proxy,
            Some('e') => Cmd::Banners,
            _ if input.home => Cmd::PrevProfile,
            _ if input.end => Cmd::NextProfile,
            _ if input.left => Cmd::PrevShip,
            _ if input.right => Cmd::NextShip,
            _ if input.up => Cmd::PrevZone,
            _ if input.down => Cmd::NextZone,
            _ if input.alt_f4 => Cmd::Quit,
            _ => return None,
        };
        Some(self.run_command(cmd))
    }

    fn open_other(&mut self, id: u32) {
        if let Some(d) = self.new_dialog(id) {
            self.dialog = Some(Open { kind: Kind::Other, d });
        }
    }

    fn open_trace(&mut self) {
        let Some(mut d) = self.new_dialog(DLG_TRACE) else { return };
        let zone = self.selected_zone();
        d.set_text(TRACE_ZONE, zone.as_ref().map_or("", |z| z.name.as_str()));
        d.set_text(TRACE_IP, &zone.as_ref().map_or(String::new(), |z| format!("{}:{}", z.ip, z.port)));
        d.set_text(TRACE_CYCLES, "Cycles: 0");
        d.focus_id(IDOK);
        let target = zone.and_then(|z| z.addr()).map(|a| SocketAddr::new(a.ip(), a.port().wrapping_add(1)));
        let socket = UdpSocket::bind("0.0.0.0:0").ok().filter(|s| s.set_nonblocking(true).is_ok());
        let trace = Trace { target, socket, started: Instant::now(), last_send: None, cycles: 0, lines: Vec::new() };
        self.dialog = Some(Open { kind: Kind::Trace(trace), d });
    }

    fn open_proxy(&mut self) {
        let Some(mut d) = self.new_dialog(DLG_PROXY) else { return };
        let p = &self.profiles[self.active];
        for (id, key) in [(PROXY_ON, "proxy"), (PROXY_AUTH, "proxyauth")] {
            if let Some(c) = d.control_mut(id) {
                c.checked = p.get(key) == "1";
            }
        }
        for (id, key) in [(PROXY_HOST, "proxyhost"), (PROXY_PORT, "proxyport"), (PROXY_USER, "proxyuser"), (PROXY_PASS, "proxypass")] {
            d.set_text(id, p.get(key));
        }
        self.dialog = Some(Open { kind: Kind::Proxy, d });
    }

    fn open_tip(&mut self) {
        let Some(mut d) = self.new_dialog(160) else { return };
        let k = (self.started.elapsed().as_nanos() as usize) % self.tips.len().max(1);
        d.set_text(TIP_TEXT, self.tips.get(k).map_or("", |s| s.as_str()));
        d.focus_id(IDCANCEL);
        self.dialog = Some(Open { kind: Kind::Tip(k), d });
    }
}

/// Profile names in the dialog's list, with Move Up/Down and Delete enabled when they apply.
fn refresh_profile_list(d: &mut Dialog, list: &[Profile], cur: usize) {
    if let Some(c) = d.control_mut(PROF_LIST) {
        c.items = list.iter().map(|p| Item::text(p.get("profile"))).collect();
        c.sel = Some(cur);
    }
    for (id, on) in [(PROF_MOVE_UP, cur > 0), (PROF_MOVE_DOWN, cur + 1 < list.len()), (PROF_DELETE, list.len() > 1)] {
        if let Some(c) = d.control_mut(id) {
            c.enabled = on;
        }
    }
}

fn set_combo(d: &mut Dialog, id: i32, items: &[String], sel: usize) {
    if let Some(c) = d.control_mut(id) {
        c.items = items.iter().map(|s| Item::text(s)).collect();
        c.sel = Some(sel.min(items.len().saturating_sub(1)));
    }
}

fn combo_text(d: &Dialog, id: i32) -> Option<String> {
    d.control(id).and_then(|c| c.sel.and_then(|s| c.items.get(s))).map(|i| i.runs[0].1.clone())
}

fn resolution_names() -> Vec<String> {
    menubar::RESOLUTIONS.iter().map(|(w, h)| format!("{w} x {h} x 16")).collect()
}

/// Shows one profile in the Edit Selected Profile fields.
fn fill_profile_fields(d: &mut Dialog, p: &Profile, zones: &[Zone]) {
    let or = |k: &str, default: &str| if p.get(k).is_empty() { default.to_string() } else { p.get(k).to_string() };
    d.set_text(PROF_NAME, &or("profile", "Default"));
    d.set_text(PROF_PLAYER, p.get("name"));
    d.set_text(PROF_PASSWORD, p.get("password"));
    let ships: Vec<String> = std::iter::once("<default>").chain(SHIP_NAMES.iter().copied()).chain(["Spectate"]).map(String::from).collect();
    set_combo(d, PROF_SHIP, &ships, p.ship() as usize + 1);
    let res = resolution_names();
    let want = p.get("resolution").replace('x', " x ") + " x 16";
    set_combo(d, PROF_RESOLUTION, &res, res.iter().position(|r| *r == want).unwrap_or(5));
    set_combo(d, PROF_REFRESH, &["<default>".to_string()], 0);
    set_combo(d, PROF_WINDOW, &["Full Screen".to_string(), "Window".to_string()], (p.get("windowed") == "1") as usize);
    let zone_names: Vec<String> =
        std::iter::once("<default>".to_string()).chain(zones.iter().filter(|z| z.shown).map(|z| z.name.clone())).collect();
    let zsel = zone_names.iter().position(|n| n == p.get("zone")).unwrap_or(0);
    set_combo(d, PROF_ZONE, &zone_names, zsel);
    set_combo(d, PROF_MACROS, &[or("macros", "Default")], 0);
    set_combo(d, PROF_KEYS, &[or("keyboard", "Default")], 0);
    d.set_text(PROF_TARGET, p.get("target").trim_start_matches("-1"));
    d.set_text(PROF_CHATS, p.get("chat"));
    d.set_text(PROF_AUTO, p.get("auto"));
}

/// Copies the Edit Selected Profile fields back into a profile.
fn read_profile_fields(d: &Dialog, p: &mut Profile) {
    let sel = |id: i32| d.control(id).and_then(|c| c.sel).unwrap_or(0);
    let name = d.text(PROF_NAME).trim().to_string();
    if !name.is_empty() {
        p.set("profile", &name);
    }
    let player = d.text(PROF_PLAYER).trim().to_string();
    if !player.is_empty() {
        p.set("name", &player);
    }
    p.set("password", &d.text(PROF_PASSWORD));
    let ship = sel(PROF_SHIP);
    if ship > 0 {
        p.set("ship", &(ship - 1).to_string());
    }
    if let Some(&(w, h)) = menubar::RESOLUTIONS.get(sel(PROF_RESOLUTION)) {
        p.set("resolution", &format!("{w}x{h}"));
    }
    p.set("windowed", &sel(PROF_WINDOW).to_string());
    if let Some(z) = combo_text(d, PROF_ZONE) {
        p.set("zone", if z == "<default>" { "" } else { &z });
    }
    let target = d.text(PROF_TARGET);
    p.set("target", if target.is_empty() { "-1" } else { &target });
    p.set("chat", &d.text(PROF_CHATS));
    p.set("auto", &d.text(PROF_AUTO));
}

/// Rebuilds the zones dialog's owner-drawn list, its description panel and the add/remove
/// button label from the current state.
fn refresh_zones(open: &mut Open) {
    let Kind::Zones(z) = &open.kind else { return };
    let d = &mut open.d;
    let sel = d.control(ZONES_LIST).and_then(|c| c.sel);
    let items: Vec<Item> = z
        .zones
        .iter()
        .map(|zone| {
            let players = zone.players.map_or(String::new(), |p| p.to_string());
            let color = if zone.shown { ZONE_TEXT_SHOWN } else { ZONE_TEXT_GREY };
            let px = 30 - dialog_font_width(&players);
            let runs = vec![(px, players.clone(), color), (43, zone.name.clone(), color)];
            let selected_runs = vec![(px, players, ZONE_TEXT_SELECTED), (43, zone.name.clone(), ZONE_TEXT_SELECTED)];
            Item { runs, selected_runs }
        })
        .collect();
    if let Some(c) = d.control_mut(ZONES_LIST) {
        c.items = items;
        c.highlight = Some(ZONE_SELECTION);
        c.bold = z.zones.iter().map(|z| z.shown).collect();
    }
    let desc = sel.and_then(|i| z.zones.get(i)).map(|zone| {
        let players = zone.players.map_or("?".to_string(), |p| p.to_string());
        let mut s = format!("Players: {players}\nIP: {}\nPort: {}\n", zone.ip, zone.port);
        if zone.players.is_none() {
            s += "Updated: zone may be down or no longer exist\n";
        }
        s + "\n" + &zone.description
    });
    d.set_text(ZONES_DESCRIPTION, &desc.unwrap_or_default());
    let shown = sel.and_then(|i| z.zones.get(i)).is_some_and(|z| z.shown);
    d.set_text(ZONES_ADD_REMOVE, if shown { "Remove &Zone(s)" } else { "Add &Zone" });
}

fn dialog_font_width(s: &str) -> i32 {
    thread_local! {
        static FONT: BitmapFont = BitmapFont::parse(crate::winfont::MS_SANS_SERIF_8).expect("built-in font");
    }
    FONT.with(|f| f.width(s))
}

fn step_at(steps: &[Step], tick: u32) -> Option<(i32, i32)> {
    let total: u32 = steps.iter().map(|s| s.ticks).sum();
    if total == 0 {
        return None;
    }
    let mut t = tick % total;
    for s in steps {
        if t < s.ticks {
            return s.at;
        }
        t -= s.ticks;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_dat_round_trips() {
        let text = "CS Extreme Games [Backup],198.23.202.76,5000,,0\r\n#Backup zone\r\nA, B,1.2.3.4,7000,zone.example,1\r\n";
        let zones = parse_zone_dat(text);
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].description, "Backup zone");
        assert_eq!((zones[1].name.as_str(), zones[1].port, zones[1].dns.as_str(), zones[1].shown), ("A, B", 7000, "zone.example", true));
        assert_eq!(parse_zone_dat(&write_zone_dat(&zones)), zones);
    }

    #[test]
    fn profile_keeps_unknown_keys() {
        let mut p = Profile::parse("; c\r\nprofile    = Default\r\nname       = Name\r\nship       = 8\r\nchat       = \r\n");
        let (set, active) = parse_profiles(&write_profiles(&[p.clone(), Profile::defaults()], 1));
        assert_eq!((set.len(), active), (2, 0), "both named Default: the first matches");
        let mut other = Profile::defaults();
        other.set("profile", "Other");
        let (set, active) = parse_profiles(&write_profiles(&[p.clone(), other], 1));
        assert_eq!((set.len(), active, set[1].get("profile")), (2, 1, "Other"));
        assert_eq!(parse_profiles("profile = A\r\nname = X\r\n").0[0].get("name"), "X");
        assert_eq!((p.get("name"), p.ship()), ("Name", 8));
        p.set("ship", "2");
        let again = Profile::parse(&p.write());
        assert_eq!(again.ship(), 2);
        assert_eq!(again.entries.len(), 4);
    }

    #[test]
    fn wraps_like_the_original_infobox() {
        assert_eq!(
            wrap(INFO_ZONES_LIST, 27),
            ["Select a zone to play in.", "Red zones may be down. Hold", "right mouse button down on", "a zone for its description."]
        );
        assert_eq!(wrap(INFO_DEFAULT, 27), ["Move the mouse over any", "item and read the", "description here."]);
    }

    #[test]
    fn parses_directory_entries() {
        let mut p = vec![0x01];
        for (ip, port, players, name, desc) in [([47, 207, 117, 75], 5000u16, 3u16, "Testtube Test Zone", "Test zone"), ([1, 2, 3, 4], 7000, 0, "Other", "")] {
            p.extend_from_slice(&ip);
            p.extend_from_slice(&port.to_le_bytes());
            p.extend_from_slice(&players.to_le_bytes());
            p.extend_from_slice(&1u16.to_le_bytes());
            p.extend_from_slice(&134u32.to_le_bytes());
            let mut n = name.as_bytes().to_vec();
            n.resize(64, 0);
            p.extend_from_slice(&n);
            p.extend_from_slice(desc.as_bytes());
            p.push(0);
        }
        let z = parse_directory_list(&p);
        assert_eq!(z.len(), 2);
        assert_eq!((z[0].ip.as_str(), z[0].port, z[0].players, z[0].name.as_str()), ("47.207.117.75", 5000, 3, "Testtube Test Zone"));
        assert_eq!(z[1].description, "");
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// Talks to the live directory servers: `cargo test --lib -- --ignored live_directory`.
    #[test]
    #[ignore]
    fn live_directory() {
        for host in DEFAULT_DIRECTORY_SERVERS.split_whitespace() {
            match download_directory(host, Duration::from_secs(6)) {
                Ok(z) => {
                    println!("{host}: {} zones", z.len());
                    for d in &z {
                        println!("  {:>4} {}:{} {}", d.players, d.ip, d.port, d.name);
                    }
                }
                Err(e) => println!("{host}: {e}"),
            }
        }
    }
}
