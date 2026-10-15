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
use crate::dialog::{self, Dialog, Event, IDCANCEL, IDOK, Item, Template};
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Play { addr: SocketAddr, zone: String, name: String, password: String, ship: u8 },
    Quit,
}

/// Height of the Windows menu bar above the skin.
pub const MENU_H: i32 = 19;
/// The original's menu bar items and where Wine puts their text.
const MENU_ITEMS: [(&str, i32); 7] =
    [("&Game", 7), ("&View", 49), ("&Navigate", 87), ("&Resolution", 147), ("&Skin", 213), ("&Web Links && News", 250), ("&Help", 354)];

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
const PROF_MOVE_UP: i32 = 1202;
const PROF_MOVE_DOWN: i32 = 1203;
const PASS_EDIT: i32 = 1023;
const PASS_SAVE: i32 = 1201;

const ZONE_TEXT_GREY: [u8; 3] = [128, 128, 128];
const ZONE_TEXT_SHOWN: [u8; 3] = [0, 128, 0];
const ZONE_TEXT_SELECTED: [u8; 3] = [128, 255, 128];
const ZONE_SELECTION: [u8; 3] = [0, 0, 128];

enum Kind {
    Zones(ZonesDialog),
    Profile,
    Password,
}

struct Open {
    kind: Kind,
    d: Dialog,
}

struct ZonesDialog {
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
    pub skin: Skin,
    spectate_icon: RgbaImage,
    ships: RgbaImage,
    font: MenuFont,
    pub profile: Profile,
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
    /// Directory servers for the zones dialog.
    pub servers: String,
    /// Set whenever profile or zones change; the frontend saves and clears it.
    pub dirty: bool,
}

impl Launcher {
    pub fn new(
        skin: Skin,
        spectate_icon: RgbaImage,
        ships: RgbaImage,
        font: RgbaImage,
        templates: Vec<Template>,
        profile: Profile,
        zones: Vec<Zone>,
    ) -> Self {
        let mut l = Self {
            skin,
            spectate_icon,
            ships,
            font: MenuFont { image: font },
            profile,
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
            servers: DEFAULT_DIRECTORY_SERVERS.into(),
            dirty: false,
        };
        let want = l.profile.get("zone").to_string();
        l.selected = l.shown().iter().position(|&i| l.zones[i].name == want).or(if l.shown().is_empty() { None } else { Some(0) });
        l
    }

    /// Window size: the menu bar plus the skin's main region.
    pub fn size(&self) -> (i32, i32) {
        (self.skin.main.w(), self.skin.main.h() + MENU_H)
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
        self.skin.hotspot("zonetext").map_or(0, |h| {
            let r = h.regions[0].rect;
            ((r.h() - GLYPH_H) / ROW_PITCH + 1).max(0) as usize
        })
    }

    fn rect(&self, name: &str) -> Option<Rect> {
        self.skin.hotspot(name).map(|h| h.regions[0].rect)
    }

    fn hit(&self, x: i32, y: i32) -> Option<&Hotspot> {
        // Buttons first, then the boxes that contain them.
        let boxes = ["profbox", "shipbox", "zonebox", "infobox", "proftext", "zonetext"];
        self.skin
            .hotspots
            .iter()
            .filter(|h| !boxes.contains(&h.name.as_str()))
            .find(|h| h.contains(x, y))
            .or_else(|| ["zonetext", "proftext", "profbox", "shipbox", "zonebox", "infobox"].iter().filter_map(|n| self.skin.hotspot(n)).find(|h| h.contains(x, y)))
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
                    self.profile.set("zone", &self.zones[shown[row]].name.clone());
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
            _ => {}
        }
    }

    fn set_ship(&mut self, ship: u8) {
        self.profile.set("ship", &ship.to_string());
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
        let p = &self.profile;
        let name = |k: &str, default: &str| if p.get(k).is_empty() { default.to_string() } else { p.get(k).to_string() };
        if let Some(c) = d.control_mut(PROF_LIST) {
            c.items = vec![Item::text(&name("profile", "Default"))];
            c.sel = Some(0);
        }
        d.set_text(PROF_NAME, &name("profile", "Default"));
        d.set_text(PROF_PLAYER, p.get("name"));
        d.set_text(PROF_PASSWORD, p.get("password"));
        let combo = |d: &mut Dialog, id: i32, items: &[&str], sel: usize| {
            if let Some(c) = d.control_mut(id) {
                c.items = items.iter().map(|s| Item::text(s)).collect();
                c.sel = Some(sel.min(items.len().saturating_sub(1)));
            }
        };
        let ships: Vec<&str> = SHIP_NAMES.iter().copied().chain(["Spectate"]).collect();
        combo(&mut d, PROF_SHIP, &ships, p.ship() as usize);
        combo(&mut d, PROF_RESOLUTION, &["1024 x 768 x 16"], 0);
        combo(&mut d, PROF_REFRESH, &["<default>"], 0);
        combo(&mut d, PROF_WINDOW, &["Full Screen", "Window"], if p.get("windowed") == "1" { 1 } else { 0 });
        let zone_names: Vec<String> = std::iter::once("<default>".to_string()).chain(self.zones.iter().filter(|z| z.shown).map(|z| z.name.clone())).collect();
        let zone_refs: Vec<&str> = zone_names.iter().map(|s| s.as_str()).collect();
        let zsel = zone_names.iter().position(|n| n == p.get("zone")).unwrap_or(0);
        combo(&mut d, PROF_ZONE, &zone_refs, zsel);
        combo(&mut d, PROF_MACROS, &[&name("macros", "Default")], 0);
        combo(&mut d, PROF_KEYS, &[&name("keyboard", "Default")], 0);
        d.set_text(PROF_TARGET, p.get("target").trim_start_matches("-1"));
        d.set_text(PROF_CHATS, p.get("chat"));
        for id in [PROF_MOVE_UP, PROF_MOVE_DOWN] {
            if let Some(c) = d.control_mut(id) {
                c.enabled = false;
            }
        }
        d.focus_id(PROF_LIST);
        self.dialog = Some(Open { kind: Kind::Profile, d });
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
        let mut open = Open { kind: Kind::Zones(ZonesDialog { zones, download: None }), d };
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
        let password = self.profile.get("password").to_string();
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
            name: self.profile.get("name").to_string(),
            password,
            ship: self.profile.ship(),
        })
    }

    fn zones_command(&mut self, id: i32) {
        let Some(Open { kind: Kind::Zones(z), d }) = self.dialog.as_mut() else { return };
        match id {
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
                (Kind::Password, Event::Command(IDOK)) => {
                    let password = open.d.text(PASS_EDIT);
                    if password.is_empty() {
                        continue;
                    }
                    if open.d.checked(PASS_SAVE) {
                        self.profile.set("password", &password);
                        self.dirty = true;
                    }
                    self.dialog = None;
                    let zone = self.selected_zone()?;
                    return self.play_with(&zone, password);
                }
                (Kind::Profile, Event::Command(IDOK)) => {
                    let d = &open.d;
                    let sel = |id: i32| d.control(id).and_then(|c| c.sel).unwrap_or(0);
                    let player = d.text(PROF_PLAYER).trim().to_string();
                    let profile = d.text(PROF_NAME).trim().to_string();
                    let password = d.text(PROF_PASSWORD);
                    let ship = sel(PROF_SHIP);
                    let windowed = sel(PROF_WINDOW);
                    let zone = d.control(PROF_ZONE).and_then(|c| c.sel.and_then(|s| c.items.get(s))).map(|i| i.runs[0].1.clone());
                    let chats = d.text(PROF_CHATS);
                    let target = d.text(PROF_TARGET);
                    let p = &mut self.profile;
                    if !profile.is_empty() {
                        p.set("profile", &profile);
                    }
                    if !player.is_empty() {
                        p.set("name", &player);
                    }
                    p.set("password", &password);
                    p.set("ship", &ship.to_string());
                    p.set("windowed", &windowed.to_string());
                    p.set("chat", &chats);
                    p.set("target", if target.is_empty() { "-1" } else { &target });
                    if let Some(z) = zone {
                        p.set("zone", if z == "<default>" { "" } else { &z });
                    }
                    self.dirty = true;
                    self.dialog = None;
                    // The zone choice also selects it on the main list.
                    let want = self.profile.get("zone").to_string();
                    if let Some(row) = self.shown().iter().position(|&i| self.zones[i].name == want) {
                        self.selected = Some(row);
                    }
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
        let main = self.skin.main;
        c.blit(&self.skin.image, main.x0, main.y0, main.w(), main.h(), 0, 0);
        let (mx, my) = self.mouse;
        let hover = if self.dialog.is_none() { self.hit(mx, my).map(|h| h.name.clone()) } else { None };
        let shown = self.shown();
        let rows = self.rows();
        let ship = self.profile.ship();
        for h in &self.skin.hotspots {
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
                    c.blit(&self.skin.image, sx, sy, r.rect.w(), r.rect.h(), r.rect.x0, r.rect.y0);
                }
            }
        }
        // Ship pictures: frame 5 of each ship in graphics/ships.bm2, 2 px inside the button.
        for k in 0..8 {
            if let Some(r) = self.rect(&format!("ship{}", k + 1)) {
                c.blit(&self.ships, 5 * 36, k * 4 * 36, 36, 36, r.x0 + 2, r.y0 + 2);
            }
        }
        if let Some(r) = self.rect("spectate") {
            c.blit(&self.spectate_icon, 0, 0, 36, 36, r.x0 + 2, r.y0 + 2);
        }
        if let Some(r) = self.rect("proftext") {
            for (i, key) in ["profile", "name", "macros", "keyboard"].iter().enumerate() {
                let v: String = self.profile.get(key).chars().take(23).collect();
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
        }
    }

    /// The Windows menu bar: face colour, Tahoma 8 items with underlined mnemonics.
    fn draw_menu_bar(&self, c: &mut Canvas) {
        let (w, _) = self.size();
        c.fill(0, 0, w, MENU_H, dialog::FACE);
        let f = &self.menu_font;
        for (label, x) in MENU_ITEMS {
            let mut pen = x;
            let mut chars = label.chars();
            while let Some(ch) = chars.next() {
                let (ch, underline) = match ch {
                    '&' => match chars.next() {
                        Some('&') => ('&', false),
                        next => (next.unwrap_or(' '), true),
                    },
                    _ => (ch, false),
                };
                let s = ch.to_string();
                let next = f.draw(&s, pen, 6, |px, py| c.put(px, py, dialog::TEXT));
                if underline {
                    c.fill(pen, 6 + f.ascent + 1, next - pen - 1, 1, dialog::TEXT);
                }
                pen = next;
            }
        }
    }
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
