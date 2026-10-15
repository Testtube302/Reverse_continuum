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
use crate::skin::{Hotspot, Rect, Skin, Step};

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
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Self { w, h, px: vec![0; (w * h * 4) as usize] }
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
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

#[derive(Clone, Debug, PartialEq)]
enum Field {
    Name,
    Password,
}

enum Dialog {
    /// Name and password; `then_play` continues to the zone afterwards.
    Profile { name: String, password: String, save: bool, focus: Field, then_play: bool },
    Zones(ZonesDialog),
}

struct ZonesDialog {
    zones: Vec<Zone>,
    selected: Option<usize>,
    scroll: usize,
    servers: String,
    status: String,
    download: Option<std::sync::mpsc::Receiver<Result<Vec<DirZone>, String>>>,
}

impl ZonesDialog {
    /// Downloads from every listed directory server on a worker thread, merging what answers.
    fn start_download(&mut self) {
        let servers: Vec<String> = self.servers.split_whitespace().map(String::from).collect();
        let (tx, rx) = std::sync::mpsc::channel();
        self.status = "Downloading zone list...".into();
        self.download = Some(rx);
        std::thread::spawn(move || {
            let mut all: Vec<DirZone> = Vec::new();
            let mut err = String::from("no directory server answered");
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
    }
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
    dialog: Option<Dialog>,
    /// Set whenever profile or zones change; the frontend saves and clears it.
    pub dirty: bool,
}

impl Launcher {
    pub fn new(skin: Skin, spectate_icon: RgbaImage, ships: RgbaImage, font: RgbaImage, profile: Profile, zones: Vec<Zone>) -> Self {
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
            dirty: false,
        };
        let want = l.profile.get("zone").to_string();
        l.selected = l.shown().iter().position(|&i| l.zones[i].name == want).or(if l.shown().is_empty() { None } else { Some(0) });
        l
    }

    pub fn size(&self) -> (i32, i32) {
        (self.skin.main.w(), self.skin.main.h())
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
        self.mouse = input.mouse;
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
        let (mx, my) = input.mouse;
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
            "profile" | "proftext" | "profbox" => self.open_profile(false),
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

    /// Opens a dialog by name ("zones", "profile", "password"); used by test aids.
    pub fn open(&mut self, which: &str) {
        match which {
            "zones" => self.open_zones(),
            "zones-download" => {
                self.open_zones();
                if let Some(Dialog::Zones(d)) = &mut self.dialog {
                    d.start_download();
                }
            }
            "profile" => self.open_profile(false),
            "password" => self.open_profile(true),
            _ => {}
        }
    }

    fn set_ship(&mut self, ship: u8) {
        self.profile.set("ship", &ship.to_string());
        self.dirty = true;
    }

    fn open_profile(&mut self, then_play: bool) {
        let name = self.profile.get("name").to_string();
        let password = self.profile.get("password").to_string();
        let save = !password.is_empty();
        let focus = if then_play { Field::Password } else { Field::Name };
        self.dialog = Some(Dialog::Profile { name, password, save, focus, then_play });
    }

    fn open_zones(&mut self) {
        self.dialog = Some(Dialog::Zones(ZonesDialog {
            zones: self.zones.clone(),
            selected: None,
            scroll: 0,
            servers: DEFAULT_DIRECTORY_SERVERS.into(),
            status: String::new(),
            download: None,
        }));
    }

    /// Same as clicking PLAY (test aid).
    pub fn press_play(&mut self) -> Option<Action> {
        self.play()
    }

    fn play(&mut self) -> Option<Action> {
        let shown = self.shown();
        let zone = self.zones.get(*shown.get(self.selected?)?)?.clone();
        let password = self.profile.get("password").to_string();
        if password.is_empty() {
            self.open_profile(true);
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

    fn update_dialog(&mut self, input: &Input) -> Option<Action> {
        let (mx, my) = input.mouse;
        let layout = DialogLayout::new(self.size());
        match self.dialog.as_mut()? {
            Dialog::Profile { name, password, save, focus, then_play } => {
                let field = if *focus == Field::Name { &mut *name } else { &mut *password };
                for &ch in &input.chars {
                    if (' '..='~').contains(&ch) && field.len() < 23 {
                        field.push(ch);
                    }
                }
                if input.backspace {
                    field.pop();
                }
                if input.tab {
                    *focus = if *focus == Field::Name { Field::Password } else { Field::Name };
                }
                let p = &layout.profile;
                let mut ok = input.enter;
                let mut cancel = input.escape;
                if input.left_pressed {
                    if p.name.contains(mx, my) {
                        *focus = Field::Name;
                    } else if p.password.contains(mx, my) {
                        *focus = Field::Password;
                    } else if p.save.contains(mx, my) {
                        *save = !*save;
                    } else if p.ok.contains(mx, my) {
                        ok = true;
                    } else if p.cancel.contains(mx, my) {
                        cancel = true;
                    }
                }
                if cancel {
                    self.dialog = None;
                } else if ok && !name.trim().is_empty() && (!*then_play || !password.is_empty()) {
                    let (name, password, save, then_play) = (name.trim().to_string(), password.clone(), *save, *then_play);
                    self.dialog = None;
                    self.profile.set("name", &name);
                    self.profile.set("password", if save { &password } else { "" });
                    self.dirty = true;
                    if then_play {
                        let shown = self.shown();
                        let zone = self.zones.get(*shown.get(self.selected?)?)?.clone();
                        return self.play_with(&zone, password);
                    }
                }
            }
            Dialog::Zones(d) => {
                if let Some(rx) = &d.download {
                    if let Ok(result) = rx.try_recv() {
                        d.download = None;
                        match result {
                            Ok(found) => {
                                let before = d.zones.len();
                                merge_directory(&mut d.zones, &found);
                                d.status = format!("Downloaded {} zones ({} new).", found.len(), d.zones.len() - before);
                            }
                            Err(e) => d.status = e,
                        }
                    }
                }
                let z = &layout.zones;
                let rows = z.rows();
                if input.wheel != 0 && z.list.contains(mx, my) {
                    let max = d.zones.len().saturating_sub(rows) as i32;
                    d.scroll = (d.scroll as i32 - input.wheel * 3).clamp(0, max.max(0)) as usize;
                }
                let mut ok = input.enter;
                let cancel = input.escape;
                if input.left_pressed {
                    if let Some(row) = z.row_at(mx, my).map(|r| r + d.scroll).filter(|&r| r < d.zones.len()) {
                        d.selected = Some(row);
                        if input.double_click {
                            d.zones[row].shown = !d.zones[row].shown;
                        }
                    } else if z.download.contains(mx, my) && d.download.is_none() {
                        d.start_download();
                    } else if z.remove.contains(mx, my) {
                        if let Some(i) = d.selected.take().filter(|&i| i < d.zones.len()) {
                            d.zones.remove(i);
                        }
                    } else if z.ok.contains(mx, my) {
                        ok = true;
                    } else if z.cancel.contains(mx, my) {
                        self.dialog = None;
                        return None;
                    }
                }
                if cancel {
                    self.dialog = None;
                } else if ok {
                    let mut zones = std::mem::take(&mut d.zones);
                    zones.sort_by_key(|z| z.name.to_ascii_lowercase());
                    let keep = self.shown().get(self.selected.unwrap_or(usize::MAX)).map(|&i| self.zones[i].name.clone());
                    self.zones = zones;
                    self.dialog = None;
                    self.dirty = true;
                    self.scroll = 0;
                    let shown = self.shown();
                    self.selected = keep
                        .and_then(|n| shown.iter().position(|&i| self.zones[i].name == n))
                        .or(if shown.is_empty() { None } else { Some(0) });
                }
            }
        }
        None
    }

    fn tick(&self) -> u32 {
        (self.started.elapsed().as_millis() / TICK.as_millis()) as u32
    }

    /// Draws the whole screen.
    pub fn draw(&self, c: &mut Canvas) {
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
        if let Some(d) = &self.dialog {
            self.draw_dialog(c, d);
        }
    }

    fn draw_dialog(&self, c: &mut Canvas, d: &Dialog) {
        let layout = DialogLayout::new(self.size());
        let f = &self.font;
        match d {
            Dialog::Profile { name, password, save, focus, then_play } => {
                let p = &layout.profile;
                let title = if *then_play { "Password Required" } else { "Profile" };
                window(c, f, p.frame, title);
                f.draw_flat(c, "Name:", BLACK, p.frame.x0 + 12, p.name.y0 + 3);
                f.draw_flat(c, "Password:", BLACK, p.frame.x0 + 12, p.password.y0 + 3);
                let stars: String = "*".repeat(password.len());
                edit(c, f, p.name, name, *focus == Field::Name);
                edit(c, f, p.password, &stars, *focus == Field::Password);
                checkbox(c, f, p.save, "Save password", *save);
                button(c, f, p.ok, if *then_play { "Play" } else { "OK" });
                button(c, f, p.cancel, "Cancel");
            }
            Dialog::Zones(d) => {
                let z = &layout.zones;
                window(c, f, z.frame, "Add/Update Zones");
                f.draw_flat(c, "# Players  Zone Name", BLACK, z.list.x0, z.list.y0 - 14);
                sunken(c, z.list, WHITE);
                for (row, zone) in d.zones.iter().enumerate().skip(d.scroll).take(z.rows()) {
                    let y = z.list.y0 + 2 + (row - d.scroll) as i32 * 14;
                    if d.selected == Some(row) {
                        c.fill(z.list.x0 + 2, y - 1, z.list.w() - 4, 14, NAVY);
                    }
                    let color = if d.selected == Some(row) { WHITE } else if zone.shown { GREEN } else { GREY_TEXT };
                    let players = zone.players.map_or(String::new(), |p| p.to_string());
                    let line: String =
                        format!("{players:>4}  {}", zone.name).chars().take(((z.list.w() - 8) / GLYPH_W) as usize).collect();
                    f.draw_flat(c, &line, color, z.list.x0 + 4, y);
                }
                group(c, f, z.desc, "Zone Description");
                if let Some(zone) = d.selected.and_then(|i| d.zones.get(i)) {
                    let mut text = vec![
                        format!("Players: {}", zone.players.map_or("?".into(), |p| p.to_string())),
                        format!("IP: {}", zone.ip),
                        format!("Port: {}", zone.port),
                        String::new(),
                    ];
                    text.extend(wrap(&zone.description, ((z.desc.w() - 16) / GLYPH_W) as usize));
                    for (i, line) in text.iter().enumerate() {
                        let y = z.desc.y0 + 18 + i as i32 * 14;
                        if y + GLYPH_H > z.desc.y1 {
                            break;
                        }
                        f.draw_flat(c, line, BLACK, z.desc.x0 + 8, y);
                    }
                }
                f.draw_flat(c, "Double-click a zone to show or hide it.", BLACK, z.list.x0, z.list.y1 + 6);
                button(c, f, z.remove, "Remove Zone");
                group(c, f, z.dl_group, "Download New Zones");
                button(c, f, z.download, if d.download.is_some() { "Working..." } else { "Download" });
                f.draw_flat(c, &d.status, BLACK, z.download.x1 + 10, z.download.y0 + 6);
                f.draw_flat(c, "Directory servers:", BLACK, z.dl_group.x0 + 10, z.download.y1 + 8);
                let servers: String = d.servers.chars().take(((z.dl_group.w() - 20) / GLYPH_W) as usize).collect();
                f.draw_flat(c, &servers, BLACK, z.dl_group.x0 + 10, z.download.y1 + 22);
                button(c, f, z.ok, "OK");
                button(c, f, z.cancel, "Cancel");
            }
        }
    }
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

// ---------------------------------------------------------------------------------------------
// Dialog look: Windows "classic" controls (grey faces, white/dark 3D edges, navy title).

const FACE: [u8; 3] = [212, 208, 200];
const LIGHT: [u8; 3] = [255, 255, 255];
const SHADOW: [u8; 3] = [128, 128, 128];
const DARK: [u8; 3] = [64, 64, 64];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const NAVY: [u8; 3] = [10, 36, 106];
const GREEN: [u8; 3] = [0, 128, 0];
const GREY_TEXT: [u8; 3] = [96, 96, 96];

fn r(x0: i32, y0: i32, w: i32, h: i32) -> Rect {
    Rect { x0, y0, x1: x0 + w - 1, y1: y0 + h - 1 }
}

fn bevel(c: &mut Canvas, b: Rect, tl: [u8; 3], br: [u8; 3]) {
    c.fill(b.x0, b.y0, b.w(), 1, tl);
    c.fill(b.x0, b.y0, 1, b.h(), tl);
    c.fill(b.x0, b.y1, b.w(), 1, br);
    c.fill(b.x1, b.y0, 1, b.h(), br);
}

fn inset(b: Rect, n: i32) -> Rect {
    Rect { x0: b.x0 + n, y0: b.y0 + n, x1: b.x1 - n, y1: b.y1 - n }
}

fn window(c: &mut Canvas, f: &MenuFont, b: Rect, title: &str) {
    c.fill(b.x0, b.y0, b.w(), b.h(), FACE);
    bevel(c, b, FACE, DARK);
    bevel(c, inset(b, 1), LIGHT, SHADOW);
    c.fill(b.x0 + 3, b.y0 + 3, b.w() - 6, 18, NAVY);
    f.draw_flat(c, title, WHITE, b.x0 + 7, b.y0 + 6);
}

fn sunken(c: &mut Canvas, b: Rect, fill: [u8; 3]) {
    c.fill(b.x0, b.y0, b.w(), b.h(), fill);
    bevel(c, b, SHADOW, LIGHT);
    bevel(c, inset(b, 1), DARK, FACE);
}

fn button(c: &mut Canvas, f: &MenuFont, b: Rect, label: &str) {
    c.fill(b.x0, b.y0, b.w(), b.h(), FACE);
    bevel(c, b, LIGHT, DARK);
    bevel(c, inset(b, 1), FACE, SHADOW);
    let x = (b.x0 + b.x1 + 1) / 2 - label.len() as i32 * GLYPH_W / 2;
    f.draw_flat(c, label, BLACK, x, (b.y0 + b.y1 + 1) / 2 - GLYPH_H / 2);
}

fn edit(c: &mut Canvas, f: &MenuFont, b: Rect, text: &str, focused: bool) {
    sunken(c, b, WHITE);
    f.draw_flat(c, text, BLACK, b.x0 + 4, b.y0 + 3);
    if focused {
        c.fill(b.x0 + 4 + text.len() as i32 * GLYPH_W, b.y0 + 3, 1, GLYPH_H, BLACK);
    }
}

fn checkbox(c: &mut Canvas, f: &MenuFont, b: Rect, label: &str, on: bool) {
    let bx = r(b.x0, b.y0 + 1, 13, 13);
    sunken(c, bx, WHITE);
    if on {
        // Classic tick: a short down-stroke then a longer up-stroke, 3 px thick.
        for (i, top) in [5, 6, 7, 6, 5, 4, 3].iter().enumerate() {
            c.fill(bx.x0 + 3 + i as i32, bx.y0 + top, 1, 3, BLACK);
        }
    }
    f.draw_flat(c, label, BLACK, b.x0 + 18, b.y0 + 2);
}

fn group(c: &mut Canvas, f: &MenuFont, b: Rect, label: &str) {
    bevel(c, b, SHADOW, LIGHT);
    bevel(c, Rect { x0: b.x0 + 1, y0: b.y0 + 1, x1: b.x1 - 1, y1: b.y1 - 1 }, LIGHT, SHADOW);
    c.fill(b.x0 + 6, b.y0 - 6, label.len() as i32 * GLYPH_W + 4, 13, FACE);
    f.draw_flat(c, label, BLACK, b.x0 + 8, b.y0 - 6);
}

struct ProfileLayout {
    frame: Rect,
    name: Rect,
    password: Rect,
    save: Rect,
    ok: Rect,
    cancel: Rect,
}

struct ZonesLayout {
    frame: Rect,
    list: Rect,
    desc: Rect,
    remove: Rect,
    dl_group: Rect,
    download: Rect,
    ok: Rect,
    cancel: Rect,
}

impl ZonesLayout {
    fn rows(&self) -> usize {
        ((self.list.h() - 4) / 14) as usize
    }
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let row = (y - self.list.y0 - 1) / 14;
        (self.list.contains(x, y) && row >= 0 && (row as usize) < self.rows()).then_some(row as usize)
    }
}

struct DialogLayout {
    profile: ProfileLayout,
    zones: ZonesLayout,
}

impl DialogLayout {
    fn new((w, h): (i32, i32)) -> Self {
        let pf = r((w - 300) / 2, (h - 150) / 2, 300, 150);
        let profile = ProfileLayout {
            frame: pf,
            name: r(pf.x0 + 100, pf.y0 + 34, 186, 20),
            password: r(pf.x0 + 100, pf.y0 + 62, 186, 20),
            save: r(pf.x0 + 100, pf.y0 + 90, 150, 16),
            ok: r(pf.x0 + 120, pf.y0 + 116, 80, 24),
            cancel: r(pf.x0 + 208, pf.y0 + 116, 80, 24),
        };
        let zf = r((w - 600) / 2, (h - 480) / 2, 600, 480);
        let zones = ZonesLayout {
            frame: zf,
            list: r(zf.x0 + 12, zf.y0 + 44, 300, 272),
            desc: r(zf.x0 + 324, zf.y0 + 38, 264, 278),
            remove: r(zf.x0 + 12, zf.y0 + 340, 120, 24),
            dl_group: r(zf.x0 + 12, zf.y0 + 380, 576, 56),
            download: r(zf.x0 + 22, zf.y0 + 390, 96, 24),
            ok: r(zf.x0 + 404, zf.y0 + 446, 88, 24),
            cancel: r(zf.x0 + 500, zf.y0 + 446, 88, 24),
        };
        Self { profile, zones }
    }
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
