//! Heads-up display drawn with the original graphics: bitmap font text, energy digits and bar,
//! item/weapon icons, radar and chat. Layout follows the nullspace client's reconstruction of
//! Continuum's screen positions.

use aicontinuum::assets::AssetDir;
use aicontinuum::game::Game;
use aicontinuum::help::{self, HelpColor, HelpPage};
use aicontinuum::lvl::{FIRST_DOOR, LAST_DOOR, TILE_GOAL, TILE_SAFE};
use aicontinuum::sim::{Ship, Vec2};
use macroquad::prelude::*;

pub const FONT_W: f32 = 8.0;
pub const FONT_H: f32 = 12.0;
const RADAR_BORDER: f32 = 6.0;

/// The font's eight colour blocks, in sheet order.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColor {
    White = 0,
    Green,
    Blue,
    DarkRed,
    Yellow,
    Fuchsia,
    Red,
    Pink,
}

fn load_texture(assets: &AssetDir, name: &str) -> Texture2D {
    let img = assets.graphic(name).unwrap_or_else(|e| panic!("{e}"));
    let t = Texture2D::from_rgba8(img.width as u16, img.height as u16, &img.pixels);
    t.set_filter(FilterMode::Nearest);
    t
}

fn blit(tex: &Texture2D, src: Rect, x: f32, y: f32) {
    draw_texture_ex(tex, x.floor(), y.floor(), WHITE, DrawTextureParams { source: Some(src), ..Default::default() });
}

/// Draws `src` stretched to `w`×`h` (used for 1-pixel colour swatches and gradients).
fn blit_scaled(tex: &Texture2D, src: Rect, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    draw_texture_ex(
        tex,
        x.floor(),
        y.floor(),
        WHITE,
        DrawTextureParams { source: Some(src), dest_size: Some(vec2(w, h)), ..Default::default() },
    );
}

pub struct Hud {
    font: Texture2D,
    energy_font: Texture2D,
    healthbar: Texture2D,
    disp: Texture2D,
    led: Texture2D,
    spectate: Texture2D,
    /// F2 stat box view (0 names, 1 points, 2 point sort, 3 team sort, 4 W/L, 5 teams), or none.
    pub statbox: Option<u8>,
    /// The "ticked" player in the stat box (Page Up/Down), target of F7 attach. None = us.
    pub selected: Option<u16>,
    gradient: Texture2D,
    icons: Texture2D,
    dropflag: Texture2D,
    icon_counts: Texture2D,
    colors: Texture2D,
    /// Radar window texture and its size.
    radar: Option<(Texture2D, u32, usize)>,
    /// Whole-map radar image for zooms where it's under 1024 px, keyed by (map, size).
    radar_small: Option<((usize, u32), Vec<[u8; 3]>)>,
    pub chat_input: Option<String>,
    /// F1 help pages: the install's Commands.hlp, or our own if it has none.
    pub help: Vec<HelpPage>,
}

/// Rows of `colors.png` used for radar and borders (column 0 = static colour).
#[derive(Clone, Copy)]
enum ColorRow {
    Background = 16,
    Border1 = 1,
    Border2 = 2,
    Border3 = 3,
    RadarPrize = 23,
    RadarSelf = 27,
    RadarTeam = 29,
    RadarEnemy = 34,
    RadarGrid = 17,
}

impl Hud {
    pub fn load(assets: &AssetDir) -> Self {
        Self {
            font: load_texture(assets, "tallfont"),
            energy_font: load_texture(assets, "engyfont"),
            healthbar: load_texture(assets, "hlthbar"),
            disp: load_texture(assets, "disp"),
            led: load_texture(assets, "led"),
            spectate: load_texture(assets, "spectate"),
            statbox: None,
            selected: None,
            gradient: load_texture(assets, "gradient"),
            icons: load_texture(assets, "Icons"),
            dropflag: load_texture(assets, "dropflag"),
            icon_counts: load_texture(assets, "icondoor"),
            colors: load_texture(assets, "colors"),
            radar: None,
            radar_small: None,
            chat_input: None,
            help: {
                let text = assets.read("Commands.hlp").map(|b| String::from_utf8_lossy(&b).into_owned());
                let pages = help::parse(text.as_deref().unwrap_or(help::FALLBACK));
                if pages.is_empty() { help::parse(help::FALLBACK) } else { pages }
            },
        }
    }

    /// Draws text with the 8×12 bitmap font; returns the width drawn.
    pub fn text(&self, s: &str, color: TextColor, x: f32, y: f32) -> f32 {
        // 48 glyphs per row; each colour is a block of 96 glyphs (space..DEL).
        let mut cx = x;
        for ch in s.chars() {
            let c = ch as u32;
            if (32..127).contains(&c) {
                let i = (c - 32) + 96 * color as u32;
                let src = Rect::new((i % 48) as f32 * FONT_W, (i / 48) as f32 * FONT_H, FONT_W, FONT_H);
                blit(&self.font, src, cx, y);
            }
            cx += FONT_W;
        }
        cx - x
    }

    fn swatch(&self, row: ColorRow, x: f32, y: f32, w: f32, h: f32) {
        blit_scaled(&self.colors, Rect::new(0.0, row as u32 as f32, 1.0, 1.0), x, y, w, h);
    }

    fn icon(&self, index: u32, x: f32, y: f32) {
        let cols = (self.icons.width() / 26.0) as u32;
        blit(&self.icons, Rect::new((index % cols) as f32 * 26.0, (index / cols) as f32 * 24.0, 26.0, 24.0), x, y);
    }

    /// The 4-pixel "empty slot" strips cut from the bottom row of Icons.png.
    fn empty_icon(&self, right_side: bool, x: f32, y: f32) {
        let sx = if right_side { 78.0 } else { 152.0 };
        blit(&self.icons, Rect::new(sx, 120.0, 4.0, 24.0), x, y);
    }

    pub fn draw(&mut self, game: &Game) {
        let (sw, sh) = (screen_width(), screen_height());
        if let (Some(ship), true) = (&game.ship, game.alive()) {
            let settings = game.world().unwrap().1;
            let max_energy = settings.ships[ship.ship as usize].maximum_energy.max(1);
            self.energy_display(ship, max_energy, sw);
            self.indicators(ship, sw, sh);
        } else if game.ship.is_none() {
            // Spectators get only the X-radar indicator (slot 4 of the right column).
            let icon = if game.xradar_on() { 36 } else { 37 };
            self.icon(icon, sw - 26.0, (sh / 2.0).floor() - 48.0 + 4.0 * 25.0);
        }
        if let Some(secs) = game.flag_drop_seconds() {
            // Animated flag icon at (sw-16, 109) with the seconds left in dark red to its left
            // (measured; the icon's animation speed is a guess).
            let frame = ((get_time() * 10.0) as u32 % 10) as f32;
            blit(&self.dropflag, Rect::new(frame * 16.0, 0.0, 16.0, 16.0), sw - 16.0, 109.0);
            let text = format!("{secs:.1}");
            self.text(&text, TextColor::DarkRed, sw - 16.0 - text.len() as f32 * FONT_W, 113.0);
        }
        if let Some((h, m, sec)) = game.safety_time_left() {
            // Starts at the screen centre, 48 px above it (measured).
            let (x, y) = ((sw / 2.0).floor(), (sh / 2.0).floor() - 48.0);
            self.text(&format!("Safety Time Left: {h}:{m:02}:{sec:02}"), TextColor::Yellow, x, y);
        }
        self.radar(game, sw, sh);
        self.chat(game, sw, sh);
        // Centre message area: arena enter/leave lines (blue) and kills (yellow) at (sw/6, sh*0.6), newest at the bottom.
        let (cx, cy) = ((sw / 6.0).floor(), (sh * 0.6).floor());
        for (i, m) in game.center_messages.iter().enumerate() {
            let color = if m.yellow { TextColor::Yellow } else { TextColor::Blue };
            self.text(&m.text, color, cx, cy + i as f32 * FONT_H);
        }
    }

    /// Top-right status panel and energy digits, and the energy bar under the health-bar frame.
    /// Positions measured from the original client at 1024x768.
    fn energy_display(&self, ship: &Ship, max_energy: u32, sw: f32) {
        blit(&self.disp, Rect::new(0.0, 0.0, 84.0, 68.0), sw - 84.0, 5.0);

        // Energy: 16x24 digits right-aligned to 14 px from the screen edge.
        let digits = (ship.energy.max(0.0) as u32).to_string();
        let mut x = sw - 14.0 - digits.len() as f32 * 16.0;
        for c in digits.bytes() {
            let d = (c - b'0') as f32;
            blit(&self.energy_font, Rect::new(d * 16.0, 0.0, 16.0, 24.0), x, 0.0);
            x += 16.0;
        }

        // Three LED counters (7x10 digits) right-aligned in the panel windows.
        // TODO: rows 1 and 2 are always 0 for us; identify what the original shows there.
        self.led(0, sw - 12.0, 33.0);
        self.led(0, sw - 33.0, 45.0);
        self.led(0, sw - 12.0, 45.0);
        self.led(ship.bounty, sw - 12.0, 57.0);

        // Energy bar: a 2 px track and a 5 px animated fill, mirrored around the centre.
        // The fill is scaled by current energy over the arena's MaximumEnergy.
        const HALF: f32 = 182.0;
        let (bar_w, bar_h) = (116.0, 28.0);
        let cx = (sw / 2.0).floor();
        let frac = (ship.energy / max_energy as f32).clamp(0.0, 1.0);
        let view = (HALF * frac).round();
        let health = ship.energy / ship.max_energy.max(1) as f32;
        let row = if health > 0.5 { 0.0 } else if health > 0.25 { 1.0 } else { 2.0 };
        let anim = ((get_time() * 10.0) as u32 % 14) as f32;
        // Dim full-length track = MaximumEnergy; bright inner segment = this ship's current capacity.
        let track = Rect::new(5.0, 0.0, 1.0, 1.0);
        // The dim track starts one pixel further left than the fill: x = cx-183 ..= cx+181 (measured).
        blit_scaled(&self.gradient, track, cx - HALF - 1.0, 9.0, HALF * 2.0 + 1.0, 2.0);
        let capacity = (HALF * (ship.max_energy as f32 / max_energy as f32).min(1.0)).round();
        blit_scaled(&self.gradient, Rect::new(0.0, 0.0, 1.0, 1.0), cx - capacity, 9.0, capacity * 2.0, 2.0);
        let fill = Rect::new(anim, row, 1.0, 1.0);
        blit_scaled(&self.gradient, fill, cx - view, 14.0, view * 2.0, 5.0);
        blit(&self.healthbar, Rect::new(0.0, 0.0, bar_w, bar_h), cx - bar_w / 2.0, 0.0);
    }

    /// Draws `value` in red LED digits ending at `right`.
    fn led(&self, value: u32, right: f32, y: f32) {
        let digits = value.to_string();
        let mut x = right - digits.len() as f32 * 7.0;
        for c in digits.bytes() {
            blit(&self.led, Rect::new((c - b'0') as f32 * 7.0, 0.0, 7.0, 10.0), x, y);
            x += 7.0;
        }
    }

    fn indicators(&self, ship: &Ship, sw: f32, sh: f32) {
        let top = (sh / 2.0).floor() - 48.0;
        // Left column: items (burst, repel, decoy, thor, brick, rocket, portal) with counts.
        // ship.items order: repel, burst, decoy, thor, brick, rocket, portal.
        let left = [(ship.items[1], 30), (ship.items[0], 31), (ship.items[2], 40), (ship.items[3], 41), (ship.items[4], 42), (ship.items[5], 43), (ship.items[6], 46)];
        for (i, (count, icon)) in left.iter().enumerate() {
            let y = top + i as f32 * 25.0;
            if *count > 0 {
                self.icon(*icon, 0.0, y);
                if *count > 1 {
                    let c = (*count).min(10);
                    // Count box from icondoor.png row 0 at (24, y+6), matched against the original.
                    blit(&self.icon_counts, Rect::new(c as f32 * 11.0, 0.0, 11.0, 13.0), 24.0, y + 6.0);
                }
            } else {
                self.empty_icon(false, 0.0, y);
            }
        }

        // Right column: guns, bombs, stealth, cloak, x-radar, antiwarp.
        let x = sw - 26.0;
        let gun = (ship.guns > 0).then(|| {
            let mut start = 0;
            if ship.caps.multifire {
                start = if ship.multifire { 3 } else { 6 };
            }
            if ship.caps.bouncing_bullets {
                start += 9;
            }
            start + ship.guns - 1
        });
        let bomb = (ship.bombs > 0).then(|| {
            let start = match (ship.caps.proximity, ship.shrapnel > 0) {
                (false, false) => 18,
                (true, false) => 21,
                (false, true) => 24,
                (true, true) => 27,
            };
            start + ship.bombs - 1
        });
        // Abilities: icon 32/34/36/38 while on, the next one while available but off.
        use aicontinuum::sim::{STATUS_ANTIWARP, STATUS_CLOAK, STATUS_STEALTH, STATUS_XRADAR};
        let state = |cap: bool, bit: u8, on: u32| cap.then_some(if ship.status & bit != 0 { on } else { on + 1 });
        let abilities = [
            state(ship.caps.stealth, STATUS_STEALTH, 32),
            state(ship.caps.cloak, STATUS_CLOAK, 34),
            state(ship.caps.xradar, STATUS_XRADAR, 36),
            state(ship.caps.antiwarp, STATUS_ANTIWARP, 38),
        ];
        for (i, icon) in [gun, bomb].into_iter().chain(abilities).enumerate() {
            let y = top + i as f32 * 25.0;
            match icon {
                Some(index) => self.icon(index, x, y),
                None => self.empty_icon(true, x + 22.0, y),
            }
        }
    }

    fn radar_color(id: u8) -> [u8; 3] {
        match id {
            TILE_SAFE => [0x18, 0x52, 0x18],
            TILE_GOAL => [0xFF, 0x39, 0x08],
            FIRST_DOOR..=LAST_DOOR => [0xAD, 0xAD, 0xAD],
            0 | 242.. => [0x0A, 0x19, 0x0A],
            _ => [0x5A, 0x5A, 0x5A],
        }
    }

    fn radar(&mut self, game: &Game, sw: f32, sh: f32) {
        let Some((map, settings)) = game.world() else { return };
        let width = sw as u32;
        // The radar is a 1:1 window onto a virtual whole-map image `full_dim` pixels across,
        // centred on the camera and clamped to the map edges.
        let full_dim = width * 8 / settings.map_zoom_factor.max(1);
        let view = (width / 6 / 4) * 4;
        let dim = view as f32;
        let center = game.camera;
        let limit = full_dim as i32 - view as i32;
        let min_x = ((center.x * full_dim as f32 / 1024.0) as i32 - view as i32 / 2).clamp(0, limit);
        let min_y = ((center.y * full_dim as f32 / 1024.0) as i32 - view as i32 / 2).clamp(0, limit);

        let v = view as usize;
        let mut px = vec![0u8; v * v * 4];
        if full_dim < 1024 {
            // Several tiles share a pixel: walls overwrite empty space so thin walls survive.
            // The whole small image is built once per map/zoom and windowed each frame.
            let key = (map as *const _ as usize, full_dim);
            if self.radar_small.as_ref().is_none_or(|(k, _)| *k != key) {
                let d = full_dim as usize;
                let mut img = vec![Self::radar_color(0); d * d];
                for y in 0..1024i32 {
                    let dy = (y as f32 / 1024.0 * full_dim as f32) as usize;
                    for x in 0..1024i32 {
                        let id = map.tile(x, y);
                        if id != 0 && id <= 241 {
                            let dx = (x as f32 / 1024.0 * full_dim as f32) as usize;
                            img[dy * d + dx] = Self::radar_color(id);
                        }
                    }
                }
                self.radar_small = Some((key, img));
            }
            let img = &self.radar_small.as_ref().unwrap().1;
            let d = full_dim as usize;
            for (i, p) in px.chunks_exact_mut(4).enumerate() {
                let (gx, gy) = (min_x as usize + i % v, min_y as usize + i / v);
                let [r, g, b] = img.get(gy * d + gx).copied().unwrap_or(Self::radar_color(0));
                p.copy_from_slice(&[r, g, b, 255]);
            }
        } else {
            for (i, p) in px.chunks_exact_mut(4).enumerate() {
                let gx = (min_x as u64 + (i % v) as u64) * 1024 / full_dim as u64;
                let gy = (min_y as u64 + (i / v) as u64) * 1024 / full_dim as u64;
                let [r, g, b] = Self::radar_color(map.tile(gx as i32, gy as i32));
                p.copy_from_slice(&[r, g, b, 255]);
            }
        }
        let reuse = self.radar.as_ref().is_some_and(|(_, w, _)| *w == view);
        if !reuse {
            let t = Texture2D::from_rgba8(view as u16, view as u16, &px);
            t.set_filter(FilterMode::Nearest);
            self.radar = Some((t, view, 0));
        } else {
            self.radar.as_ref().unwrap().0.update(&Image { bytes: px, width: view as u16, height: view as u16 });
        }
        let tex = &self.radar.as_ref().unwrap().0;
        let pos = vec2(sw - dim - RADAR_BORDER, sh - dim - RADAR_BORDER);
        draw_texture(tex, pos.x, pos.y, WHITE);

        // Sector grid: the map is 20x20 sectors (A-T across, 1-20 down).
        for k in 1..20u32 {
            let line = (k * full_dim) as f32 / 20.0;
            let gx = (line - min_x as f32).floor();
            if (0.0..dim).contains(&gx) {
                self.swatch(ColorRow::RadarGrid, pos.x + gx, pos.y, 1.0, dim);
            }
            let gy = (line - min_y as f32).floor();
            if (0.0..dim).contains(&gy) {
                self.swatch(ColorRow::RadarGrid, pos.x, pos.y + gy, dim, 1.0);
            }
        }

        // Clock and current sector above the radar, right-aligned to its edge.
        let sector_x = ((center.x / 51.2) as u32).min(19);
        let sector_y = ((center.y / 51.2) as u32).min(19);
        let sector = format!("{}{}", (b'A' + sector_x as u8) as char, sector_y + 1);
        let label = format!("{}  {sector}", clock_text());
        // 16 px above the radar's inner top edge (re-measured in round 12: y=578 at 1024x768).
        self.text(&label, TextColor::White, pos.x + dim - label.len() as f32 * FONT_W, pos.y - 16.0);

        let world_min = vec2(min_x as f32, min_y as f32) * 1024.0 / full_dim as f32;
        let world_dim = view as f32 * 1024.0 / full_dim as f32;
        let dot = |hud: &Self, p: Vec2, row: ColorRow, size: f32| {
            let (u, v) = ((p.x - world_min.x) / world_dim, (p.y - world_min.y) / world_dim);
            if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
                hud.swatch(row, pos.x + dim * u, pos.y + dim * v, size, size);
            }
        };
        for g in &game.greens.list {
            dot(self, Vec2::new(g.x as f32, g.y as f32), ColorRow::RadarPrize, 2.0);
        }
        if let Some(s) = &game.session {
            let my_freq = s.me().map(|m| m.freq);
            for p in s.players.values() {
                if Some(p.pid) == s.pid || p.ship >= 8 || p.dead || p.position_updates == 0 {
                    continue;
                }
                // Stealthed enemies are off the radar unless we have X-radar on.
                let stealth = p.status & aicontinuum::sim::STATUS_STEALTH != 0;
                if stealth && Some(p.freq) != my_freq && !game.xradar_on() {
                    continue;
                }
                let row = if Some(p.freq) == my_freq { ColorRow::RadarTeam } else { ColorRow::RadarEnemy };
                dot(self, p.position, row, 2.0);
            }
        }
        // Our own dot blinks: 16 ticks on, 16 off (measured ~160 ms each).
        let blink_on = (get_time() * 100.0) as u64 % 32 < 16;
        if let (Some(ship), true, true) = (&game.ship, game.alive(), blink_on) {
            dot(self, ship.position, ColorRow::RadarSelf, 2.0);
        }

        // Three-pixel bevelled border.
        let (x0, y0) = (pos.x, pos.y);
        self.swatch(ColorRow::Border1, x0 - 1.0, y0 - 1.0, 1.0, dim + 2.0);
        self.swatch(ColorRow::Border1, x0 + dim, y0 - 1.0, 1.0, dim + 2.0);
        self.swatch(ColorRow::Border1, x0 - 1.0, y0 - 1.0, dim + 2.0, 1.0);
        self.swatch(ColorRow::Border1, x0 - 1.0, y0 + dim, dim + 2.0, 1.0);
        self.swatch(ColorRow::Border2, x0 - 2.0, y0 - 2.0, 1.0, dim + 4.0);
        self.swatch(ColorRow::Border2, x0 + dim + 1.0, y0 - 2.0, 1.0, dim + 4.0);
        self.swatch(ColorRow::Border2, x0 - 2.0, y0 - 2.0, dim + 4.0, 1.0);
        self.swatch(ColorRow::Border2, x0 - 2.0, y0 + dim + 1.0, dim + 4.0, 1.0);
        self.swatch(ColorRow::Border3, x0 - 3.0, y0 - 1.0, 1.0, dim + 2.0);
        self.swatch(ColorRow::Border3, x0 + dim + 2.0, y0 - 1.0, 1.0, dim + 2.0);
        self.swatch(ColorRow::Border3, x0 - 1.0, y0 - 3.0, dim + 2.0, 1.0);
        self.swatch(ColorRow::Border3, x0 - 1.0, y0 + dim + 2.0, dim + 2.0, 1.0);
    }

    fn chat(&self, game: &Game, sw: f32, sh: f32) {
        let radar_size = sw * 0.165 + 11.0;
        let max_chars = ((sw - radar_size) / FONT_W) as usize;
        let shown = (sh / 100.0) as usize + 1;
        let mut lines: Vec<(String, TextColor)> = Vec::new();
        if let Some(s) = &game.session {
            for c in s.chat.iter() {
                let sender = s.players.get(&c.pid).map(|p| p.name.as_str()).unwrap_or("");
                let (text, color) = match c.kind {
                    0 => (c.text.clone(), TextColor::Green),              // arena
                    2 | 1 => (format!("{sender:>10}> {}", c.text), TextColor::Blue), // public
                    3 => (format!("{sender:>10}> {}", c.text), TextColor::Yellow),   // team
                    5 | 7 => (format!("{sender:>10}> {}", c.text), TextColor::Green), // private
                    6 | 8 => (c.text.clone(), TextColor::Red),
                    _ => (c.text.clone(), TextColor::White),
                };
                lines.push((text, color));
            }
        }
        lines.extend(game.notices.iter().map(|n| (n.clone(), TextColor::Green)));
        let mut y = sh;
        if let Some(input) = &self.chat_input {
            y -= FONT_H;
            let shown_input: String = input.chars().rev().take(max_chars.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
            self.text(&format!("{shown_input}_"), TextColor::White, 0.0, y);
        }
        let start = lines.len().saturating_sub(shown);
        for (text, color) in lines[start..].iter().rev() {
            y -= FONT_H;
            let clipped: String = text.chars().take(max_chars).collect();
            self.text(&clipped, *color, 0.0, y);
        }
    }

    /// "name(bounty)[ping ms]" 14 px right/down of the ship's centre (team yellow, enemies blue),
    /// plus the ship's energy right-aligned to its left when the zone has sent it recently.
    /// Flag carriers show "name(bounty:flags)[ping]", dark red for enemies (measured; a team
    /// carrier's colour hasn't been seen yet and stays yellow).
    #[allow(clippy::too_many_arguments)]
    pub fn name_label(&self, name: &str, bounty: u16, flags: u16, ping_ms: u32, team: bool, energy: Option<(u16, u32)>, center: (f32, f32)) {
        let color = match (team, flags > 0) {
            (true, _) => TextColor::Yellow,
            (false, true) => TextColor::DarkRed,
            (false, false) => TextColor::Blue,
        };
        let bounty = if flags > 0 { format!("{bounty}:{flags}") } else { bounty.to_string() };
        self.text(&format!("{name}({bounty})[{ping_ms}]"), color, center.0 + 14.0, center.1 + 14.0);
        if let Some((energy, initial)) = energy {
            let text = energy.to_string();
            let color = energy_color(energy as u32, initial).unwrap_or(TextColor::Blue);
            self.text(&text, color, center.0 - 22.0 - text.len() as f32 * FONT_W, center.1 + 14.0);
        }
    }

    /// Our own ship's energy, 14 px right/down of its centre, only while below half
    /// (measured from the original: yellow below 1/2, dark red below 1/4).
    pub fn own_energy_label(&self, energy: u32, max_energy: u32, center: (f32, f32)) {
        if let Some(color) = energy_color(energy, max_energy) {
            self.text(&energy.to_string(), color, center.0 + 14.0, center.1 + 14.0);
        }
    }
}

/// Low-energy warning colour: dark red below a quarter, yellow below half, else none.
fn energy_color(energy: u32, full: u32) -> Option<TextColor> {
    if energy * 4 < full {
        Some(TextColor::DarkRed)
    } else if energy * 2 < full {
        Some(TextColor::Yellow)
    } else {
        None
    }
}

/// Local wall-clock time as Continuum shows it above the radar, e.g. "12:00am".
fn clock_text() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let offset = local_utc_offset_secs();
    let day_secs = (secs as i64 + offset).rem_euclid(86_400);
    let (h, m) = (day_secs / 3600, day_secs / 60 % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{m:02}{}", if h < 12 { "am" } else { "pm" })
}

/// Local timezone offset in seconds east of UTC.
#[cfg(windows)]
fn local_utc_offset_secs() -> i64 {
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: [u16; 8],
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: [u16; 8],
        daylight_bias: i32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTimeZoneInformation(info: *mut TimeZoneInformation) -> u32;
    }
    // SAFETY: the struct matches TIME_ZONE_INFORMATION and is only written by the call.
    let mut info: TimeZoneInformation = unsafe { std::mem::zeroed() };
    let mode = unsafe { GetTimeZoneInformation(&mut info) };
    // Bias is minutes to add to local time to get UTC; mode 2 = daylight saving in effect.
    let bias = info.bias + if mode == 2 { info.daylight_bias } else { info.standard_bias };
    -(bias as i64) * 60
}

/// Local timezone offset in seconds east of UTC, from `date +%z`.
#[cfg(not(windows))]
fn local_utc_offset_secs() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        let z = out.and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
        let z = z.trim();
        if z.len() == 5 {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z[1..3].parse().unwrap_or(0);
            let m: i64 = z[3..5].parse().unwrap_or(0);
            sign * (h * 3600 + m * 60)
        } else {
            0
        }
    })
}

/// The Esc menu, laid out as measured from the original client.
pub fn draw_menu(hud: &Hud) {
    const LEFT: [&str; 12] = [
        "Q  = Quit",
        "F1 = Help",
        "F2 = Stat box",
        "F3 = Name tags",
        "F4 = Radar",
        "F5 = Messages",
        "F6 = Help ticker",
        "F8 = Engine sounds",
        " A = Arena List",
        " B = Set Banner",
        " I = Ignore macros",
        "PgUp/PgDn = Adjust stat box",
    ];
    const SHIPS: [&str; 9] =
        ["1 = Warbird", "2 = Javelin", "3 = Spider", "4 = Leviathan", "5 = Terrier", "6 = Weasel", "7 = Lancaster", "8 = Shark", "S = Spectator"];
    let (x0, x1, y0, y1) = (370.0, 659.0, 0.0, 176.0);
    hud.swatch(ColorRow::Background, x0 + 3.0, y0 + 3.0, x1 - x0 - 5.0, y1 - y0 - 5.0);
    // Three bevel rings; the outermost one skips two pixels at each corner.
    for (inset, row, cut) in [(2.0, ColorRow::Border1, 0.0), (1.0, ColorRow::Border2, 0.0), (0.0, ColorRow::Border3, 2.0)] {
        let (l, r, t, b) = (x0 + inset, x1 - inset, y0 + inset, y1 - inset);
        hud.swatch(row, l + cut, t, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l + cut, b, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
        hud.swatch(row, r, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
    }
    hud.swatch(ColorRow::Border1, x0 + 3.0, 16.0, x1 - x0 - 5.0, 1.0);
    hud.text("-= Menu =-", TextColor::Green, 463.0, 4.0);
    for (i, line) in LEFT.iter().enumerate() {
        hud.text(line, TextColor::White, 375.0, 18.0 + i as f32 * 12.0);
    }
    hud.text("Ships", TextColor::DarkRed, 567.0, 18.0);
    for (i, line) in SHIPS.iter().enumerate() {
        hud.text(line, TextColor::White, 551.0, 30.0 + i as f32 * 12.0);
    }
    hud.text("Any other key to resume game", TextColor::Yellow, 407.0, 162.0);
}

/// F2 stat box (player names view), laid out as in the original: a 108 px wide box at the
/// top left with the player count centred in the header and one 12 px row per player.
/// Moves the stat box tick up or down the list (us first, then by name).
pub fn move_selection(hud: &mut Hud, game: &Game, down: bool) {
    let Some(s) = &game.session else { return };
    let Some(me) = s.me() else { return };
    let mut players: Vec<_> = s.players.values().collect();
    players.sort_by_key(|p| (p.pid != me.pid, p.name.to_lowercase()));
    let current = hud.selected.unwrap_or(me.pid);
    let i = players.iter().position(|p| p.pid == current).unwrap_or(0);
    let j = if down { (i + 1).min(players.len().saturating_sub(1)) } else { i.saturating_sub(1) };
    hud.selected = players.get(j).map(|p| p.pid);
}

pub fn draw_statbox(hud: &Hud, game: &Game, view: u8) {
    let Some(s) = &game.session else { return };
    let Some(me) = s.me() else { return };
    let points = |p: &aicontinuum::net::session::Player| p.kill_points + p.flag_points;
    let color = |freq: u16| if freq == me.freq { TextColor::Yellow } else { TextColor::White };
    let mut players: Vec<_> = s.players.values().collect();
    players.sort_by_key(|p| (p.pid != me.pid, p.name.to_lowercase()));

    // Each row: optional marker (spectate.png frame), then text runs (x, colour, right-aligned?).
    struct Row {
        marker: Option<u32>,
        runs: Vec<(f32, TextColor, bool, String)>,
    }
    let ticked = hud.selected.filter(|pid| s.players.contains_key(pid)).unwrap_or(me.pid);
    // spectate.gif: 0 ticked, 1 spectator, 2 ticked spectator, 3 turret, 4 ticked turret.
    let marker = |p: &aicontinuum::net::session::Player| match (p.pid == ticked, p.ship >= 8, p.attached_to.is_some()) {
        (true, false, false) => Some(0),
        (false, true, _) => Some(1),
        (true, true, _) => Some(2),
        (false, false, true) => Some(3),
        (true, false, true) => Some(4),
        _ => None,
    };
    let name = |p: &aicontinuum::net::session::Player| p.name.chars().take(12).collect::<String>();
    let count = players.len().to_string();
    // Box width, header runs, rows: measured from the original's six F2 views.
    let (w, header, rows): (f32, Vec<(f32, TextColor, bool, String)>, Vec<Row>) = match view {
        1..=3 => {
            let title = ["Points", "Point Sort", "Team Sort"][view as usize - 1];
            let header = vec![(49.0, TextColor::Green, false, count.clone()), (173.0, TextColor::Green, true, title.into())];
            let player_row = |p: &&aicontinuum::net::session::Player| Row {
                marker: marker(p),
                runs: vec![(13.0, color(p.freq), false, name(p)), (173.0, color(p.freq), true, points(p).to_string())],
            };
            let rows = match view {
                1 => players.iter().map(player_row).collect(),
                2 => {
                    let mut sorted = players.clone();
                    sorted.sort_by_key(|p| (std::cmp::Reverse(points(p)), p.name.to_lowercase()));
                    sorted.iter().map(player_row).collect()
                }
                _ => {
                    let mut freqs: Vec<u16> = players.iter().filter(|p| p.ship < 8).map(|p| p.freq).collect();
                    freqs.sort();
                    freqs.dedup();
                    let mut rows = Vec::new();
                    for f in freqs {
                        let team: Vec<_> = players.iter().filter(|p| p.ship < 8 && p.freq == f).collect();
                        rows.push(Row {
                            marker: None,
                            runs: vec![
                                (5.0, TextColor::DarkRed, false, format!("{f:04}-------------")),
                                (165.0, TextColor::DarkRed, true, team.len().to_string()),
                            ],
                        });
                        rows.extend(team.into_iter().map(player_row));
                    }
                    rows
                }
            };
            (172.0, header, rows)
        }
        4 => {
            let header = vec![
                (49.0, TextColor::Green, false, count.clone()),
                (173.0, TextColor::Green, true, "Squad".into()),
                (245.0, TextColor::Green, true, "W".into()),
                (285.0, TextColor::Green, true, "L".into()),
                (325.0, TextColor::Green, true, "R".into()),
                (381.0, TextColor::Green, true, "Ave".into()),
            ];
            let rows = players
                .iter()
                .map(|p| {
                    let c = color(p.freq);
                    // Ave = points per kill; R (rating) isn't worked out yet and shows 0.
                    let ave = if p.wins > 0 { points(p) as f32 / p.wins as f32 } else { 0.0 };
                    Row {
                        marker: marker(p),
                        runs: vec![
                            (13.0, c, false, name(p)),
                            (133.0, c, false, p.squad.chars().take(10).collect()),
                            (253.0, c, true, p.wins.to_string()),
                            (293.0, c, true, p.losses.to_string()),
                            (333.0, c, true, "0".into()),
                            (381.0, c, true, format!("{ave:.1}")),
                        ],
                    }
                })
                .collect();
            (380.0, header, rows)
        }
        5 => {
            let header = vec![
                (13.0, TextColor::Green, false, "Freq".into()),
                (125.0, TextColor::Green, true, "Points".into()),
                (173.0, TextColor::Green, true, "Win".into()),
                (221.0, TextColor::Green, true, "Lose".into()),
                (269.0, TextColor::Green, true, "Flag".into()),
            ];
            let mut freqs: Vec<u16> = players.iter().filter(|p| p.ship < 8).map(|p| p.freq).collect();
            freqs.sort();
            freqs.dedup();
            let rows = freqs
                .into_iter()
                .map(|f| {
                    let team: Vec<_> = players.iter().filter(|p| p.ship < 8 && p.freq == f).collect();
                    let sum = |g: &dyn Fn(&aicontinuum::net::session::Player) -> u32| team.iter().map(|p| g(p)).sum::<u32>();
                    let c = color(f);
                    Row {
                        marker: s.players.get(&ticked).is_some_and(|t| t.freq == f).then_some(0),
                        runs: vec![
                            (13.0, c, false, f.to_string()),
                            (125.0, c, true, sum(&|p| points(p)).to_string()),
                            (173.0, c, true, sum(&|p| p.wins as u32).to_string()),
                            (221.0, c, true, sum(&|p| p.losses as u32).to_string()),
                            (269.0, c, true, sum(&|p| p.flags_carried as u32).to_string()),
                        ],
                    }
                })
                .collect();
            (268.0, header, rows)
        }
        _ => {
            let header = vec![(49.0, TextColor::Green, false, count.clone())];
            let rows = players
                .iter()
                .map(|p| Row { marker: marker(p), runs: vec![(13.0, color(p.freq), false, name(p))] })
                .collect();
            (108.0, header, rows)
        }
    };

    let max_rows = ((screen_height() / 2.0 - 60.0) / 12.0).max(1.0) as usize;
    let shown = rows.len().min(max_rows);
    let h = 14.0 + 1.0 + shown as f32 * 12.0;
    hud.swatch(ColorRow::Background, 3.0, 3.0, w, h);
    let draw_run = |(x, c, right, text): &(f32, TextColor, bool, String), y: f32| {
        let x = if *right { x - text.len() as f32 * FONT_W } else if *x == 49.0 { 49.0 - text.len() as f32 * FONT_W / 2.0 } else { *x };
        hud.text(text, *c, x, y);
    };
    for run in &header {
        draw_run(run, 4.0);
    }
    hud.swatch(ColorRow::Border1, 3.0, 16.0, w, 1.0);
    for (i, row) in rows.iter().take(shown).enumerate() {
        let y = 18.0 + i as f32 * 12.0;
        if let Some(m) = row.marker {
            blit(&hud.spectate, Rect::new(m as f32 * 8.0, 0.0, 8.0, 8.0), 3.0, y + 3.0);
        }
        for run in &row.runs {
            draw_run(run, y);
        }
    }
    // Same three-ring bevel as the menu, around (3,3)-(3+w,3+h).
    let (x0, x1, y0, y1) = (0.0, 3.0 + w + 2.0, 0.0, 3.0 + h + 2.0);
    for (inset, row, cut) in [(2.0, ColorRow::Border1, 0.0), (1.0, ColorRow::Border2, 0.0), (0.0, ColorRow::Border3, 2.0)] {
        let (l, r, t, b) = (x0 + inset, x1 - inset, y0 + inset, y1 - inset);
        hud.swatch(row, l + cut, t, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l + cut, b, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
        hud.swatch(row, r, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
    }
}

/// Draws help page `page` in a bevelled box at the top left, sized to its text.
const HELP_MAX_LINES: usize = 36;

pub fn draw_help(hud: &Hud, page: usize) {
    let Some(page) = hud.help.get(page) else { return };
    let w = page.width_chars().max(page.title.chars().count() + 1) as f32 * FONT_W + 4.0;
    // Bottom border at 19 + 12 per line; the original shows at most 36 lines (measured).
    let lines = &page.lines[..page.lines.len().min(HELP_MAX_LINES)];
    let h = 15.0 + lines.len() as f32 * FONT_H;
    let (x0, x1, y0, y1) = (0.0, w + 5.0, 0.0, h + 5.0);
    hud.swatch(ColorRow::Background, x0 + 3.0, y0 + 3.0, x1 - x0 - 5.0, y1 - y0 - 5.0);
    for (inset, row, cut) in [(2.0, ColorRow::Border1, 0.0), (1.0, ColorRow::Border2, 0.0), (0.0, ColorRow::Border3, 2.0)] {
        let (l, r, t, b) = (x0 + inset, x1 - inset, y0 + inset, y1 - inset);
        hud.swatch(row, l + cut, t, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l + cut, b, r - l + 1.0 - 2.0 * cut, 1.0);
        hud.swatch(row, l, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
        hud.swatch(row, r, t + cut, 1.0, b - t + 1.0 - 2.0 * cut);
    }
    hud.text(&page.title, TextColor::Green, 13.0, 4.0);
    hud.swatch(ColorRow::Border1, x0 + 3.0, 16.0, x1 - x0 - 5.0, 1.0);
    for (i, line) in lines.iter().enumerate() {
        let mut x = 5.0;
        for (color, text) in line {
            let color = match color {
                HelpColor::White => TextColor::White,
                HelpColor::Green => TextColor::Green,
                HelpColor::Blue => TextColor::Blue,
                HelpColor::DarkRed => TextColor::DarkRed,
                HelpColor::Yellow => TextColor::Yellow,
                HelpColor::Pink => TextColor::Pink,
            };
            x += hud.text(text, color, x, 18.0 + i as f32 * FONT_H);
        }
    }
}
