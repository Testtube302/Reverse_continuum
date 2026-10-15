//! The startup window's Windows menu bar and its drop-down menus, drawn like Wine's classic
//! menus: Tahoma 8, 17 px items, 9 px separators, a raised 3 px frame, navy highlight.
//!
//! Item texts come from the install's `menu040.dll` menu resource 101; the original fills the
//! Resolution, Skin, Web Links and Help menus at run time (captured from it). Header boxes, popup
//! widths and shortcut columns were measured on the original under Wine.

use crate::dialog::{DKSHADOW, FACE, HIGHLIGHT, HILIGHT, LIGHT, SHADOW, TEXT};
use crate::launcher::Canvas;
use crate::skin::Rect;
use crate::winfont::BitmapFont;

pub const BAR_H: i32 = 19;
const ITEM_H: i32 = 17;
const SEP_H: i32 = 9;
const TEXT_X: i32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Play,
    Quit,
    Profiles,
    Macros,
    KeyDefs,
    Zones,
    Trace,
    Options,
    AdvancedOptions,
    Proxy,
    Banners,
    PrevProfile,
    NextProfile,
    PrevShip,
    NextShip,
    PrevZone,
    NextZone,
    Resolution(u16, u16),
    Windowed,
    Skin(usize),
    Url(&'static str),
    ZoneNews,
    Tip,
    About,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub text: String,
    pub accel: String,
    pub cmd: Option<Cmd>,
    pub checked: bool,
}

impl Item {
    fn new(text: &str, cmd: Cmd) -> Self {
        let (text, accel) = text.split_once('\t').unwrap_or((text, ""));
        Self { text: text.into(), accel: accel.into(), cmd: Some(cmd), checked: false }
    }
    fn separator() -> Self {
        Self { text: String::new(), accel: String::new(), cmd: None, checked: false }
    }
    fn is_separator(&self) -> bool {
        self.cmd.is_none() && self.text.is_empty()
    }
}

pub struct Menu {
    pub title: &'static str,
    /// Where the title's text starts and the header's pressed box (x0..=x1).
    pub text_x: i32,
    pub box_x: (i32, i32),
    /// Popup width and shortcut column (None: computed from the texts).
    pub width: Option<i32>,
    pub accel_x: Option<i32>,
    pub items: Vec<Item>,
}

pub const RESOLUTIONS: [(u16, u16); 10] =
    [(512, 384), (640, 400), (640, 480), (768, 576), (800, 600), (1024, 768), (1280, 1024), (1600, 1200), (1920, 1440), (2048, 1536)];

pub struct State<'a> {
    pub resolution: (u16, u16),
    pub windowed: bool,
    pub skins: &'a [String],
    pub skin: usize,
}

/// The seven menus with their current checks.
pub fn menus(s: &State) -> Vec<Menu> {
    use Cmd::*;
    let mut res: Vec<Item> = RESOLUTIONS
        .iter()
        .map(|&(w, h)| {
            let mut i = Item::new(&format!("{w} x {h} x 16"), Resolution(w, h));
            i.checked = (w, h) == s.resolution;
            i
        })
        .collect();
    res.push(Item::separator());
    let mut win = Item::new("&Windowed", Windowed);
    win.checked = s.windowed;
    res.push(win);
    let skins: Vec<Item> = std::iter::once("<default>".to_string())
        .chain(s.skins.iter().cloned())
        .enumerate()
        .map(|(k, name)| {
            let mut i = Item::new(&name, Skin(k));
            i.checked = k == s.skin;
            i
        })
        .collect();
    vec![
        Menu {
            title: "&Game",
            text_x: 7,
            box_x: (0, 41),
            width: Some(110),
            accel_x: Some(60),
            items: vec![Item::new("&Play      \tEnter", Play), Item::new("&Quit\tAlt-F4", Quit)],
        },
        Menu {
            title: "&View",
            text_x: 49,
            box_x: (42, 78),
            width: Some(209),
            accel_x: Some(158),
            items: vec![
                Item::new("&Profiles...\tCtrl-P", Profiles),
                Item::new("&Macros...\tCtrl-M", Macros),
                Item::new("&Keyboard/Controller Defs...\tCtrl-K", KeyDefs),
                Item::separator(),
                Item::new("&Zones...\tCtrl-Z", Zones),
                Item::new("&Trace...\tCtrl-T", Trace),
                Item::separator(),
                Item::new("&Options...\tCtrl-O", Options),
                Item::new("&Advanced Options...\tCtrl-A", AdvancedOptions),
                Item::new("Pro&xy Options...\tCtrl-X", Proxy),
                Item::separator(),
                Item::new("Create/&Edit Banners...\tCtrl-E", Banners),
            ],
        },
        Menu {
            title: "&Navigate",
            text_x: 87,
            box_x: (79, 138),
            width: Some(145),
            accel_x: Some(97),
            items: vec![
                Item::new("Previous Profile\tHome", PrevProfile),
                Item::new("Next Profile\tEnd", NextProfile),
                Item::separator(),
                Item::new("Previous Ship\tLeft", PrevShip),
                Item::new("Next Ship\tRight", NextShip),
                Item::separator(),
                Item::new("Previous Zone\tUp", PrevZone),
                Item::new("Next Zone\tDown", NextZone),
            ],
        },
        Menu { title: "&Resolution", text_x: 147, box_x: (139, 205), width: Some(125), accel_x: None, items: res },
        Menu { title: "&Skin", text_x: 213, box_x: (206, 239), width: None, accel_x: None, items: skins },
        Menu {
            title: "&Web Links && News",
            text_x: 250,
            box_x: (240, 346),
            width: Some(185),
            accel_x: None,
            items: vec![
                Item::new("Subspace Central", Url("https://www.subspacehq.com/")),
                Item::new("Subspace Downloads", Url("https://www.getcontinuum.com/")),
                Item::new("Player Scores", Url("https://www.trenchwars.org/")),
                Item::separator(),
                Item::new("News For Selected Zone Ctrl-N", ZoneNews),
                Item::separator(),
            ],
        },
        Menu {
            title: "&Help",
            text_x: 354,
            box_x: (347, 382),
            width: Some(135),
            accel_x: None,
            items: vec![
                Item::new("Player's Guide", Url("https://wiki.minegoboom.com/")),
                Item::new("Report Bug", Url("https://github.com/Testtube302/Reverse_continuum/issues")),
                Item::new("Tech Support FAQ", Url("https://wiki.minegoboom.com/")),
                Item::new("Tech Support Forum", Url("https://forums.minegoboom.com/")),
                Item::separator(),
                Item::new("&Tip Of The Day...", Tip),
                Item::new("&About...", About),
            ],
        },
    ]
}

impl Menu {
    fn popup_width(&self, f: &BitmapFont) -> i32 {
        self.width.unwrap_or_else(|| {
            let text = self.items.iter().map(|i| f.width(&strip(&i.text))).max().unwrap_or(0);
            let accel = self.items.iter().map(|i| f.width(&i.accel)).max().unwrap_or(0);
            if accel > 0 { text + accel + 43 } else { text + 38 }
        })
    }

    fn popup_height(&self) -> i32 {
        6 + self.items.iter().map(|i| if i.is_separator() { SEP_H } else { ITEM_H }).sum::<i32>()
    }

    /// The popup's outer rectangle, kept inside a window `win_w` wide.
    pub fn popup(&self, f: &BitmapFont, win_w: i32) -> Rect {
        let w = self.popup_width(f);
        let x0 = self.box_x.0.min(win_w - w).max(0);
        Rect { x0, y0: BAR_H, x1: x0 + w - 1, y1: BAR_H + self.popup_height() - 1 }
    }

    /// Item index under (x, y) in the popup, separators excluded.
    pub fn item_at(&self, f: &BitmapFont, win_w: i32, x: i32, y: i32) -> Option<usize> {
        let p = self.popup(f, win_w);
        if x < p.x0 + 3 || x > p.x1 - 3 {
            return None;
        }
        let mut top = p.y0 + 3;
        for (k, it) in self.items.iter().enumerate() {
            let h = if it.is_separator() { SEP_H } else { ITEM_H };
            if y >= top && y < top + h {
                return (!it.is_separator()).then_some(k);
            }
            top += h;
        }
        None
    }
}

/// Header index under (x, y) on the bar.
pub fn header_at(menus: &[Menu], x: i32, y: i32) -> Option<usize> {
    if !(0..BAR_H).contains(&y) {
        return None;
    }
    menus.iter().position(|m| x >= m.box_x.0 && x <= m.box_x.1)
}

pub fn strip(s: &str) -> String {
    crate::dialog::strip_prefix(s)
}

/// Draws `label` with `&` mnemonics underlined; returns the pen position.
fn label(c: &mut Canvas, f: &BitmapFont, text: &str, x: i32, y: i32, col: [u8; 3]) -> i32 {
    let mut pen = x;
    let mut chars = text.chars();
    let mut buf = [0u8; 4];
    while let Some(ch) = chars.next() {
        let (ch, under) = match ch {
            '&' => match chars.next() {
                Some('&') => ('&', false),
                Some(n) => (n, true),
                None => break,
            },
            _ => (ch, false),
        };
        let next = f.draw(ch.encode_utf8(&mut buf), pen, y, |px, py| c.put(px, py, col));
        if under {
            c.fill(pen, y + f.ascent + 1, next - pen - 1, 1, col);
        }
        pen = next;
    }
    pen
}

pub fn draw_bar(c: &mut Canvas, f: &BitmapFont, menus: &[Menu], width: i32, open: Option<usize>) {
    c.fill(0, 0, width, BAR_H, FACE);
    for (k, m) in menus.iter().enumerate() {
        if open == Some(k) {
            // Pressed header: one-pixel sunken box.
            let (x0, x1) = m.box_x;
            c.fill(x0, 1, x1 - x0, 1, SHADOW);
            c.fill(x0, 1, 1, BAR_H - 2, SHADOW);
            c.fill(x0, BAR_H - 1, x1 - x0 + 1, 1, HILIGHT);
            c.fill(x1, 1, 1, BAR_H - 1, HILIGHT);
        }
        label(c, f, m.title, m.text_x, 6, TEXT);
    }
}

pub fn draw_popup(c: &mut Canvas, f: &BitmapFont, m: &Menu, win_w: i32, hot: Option<usize>) {
    let p = m.popup(f, win_w);
    c.fill(p.x0, p.y0, p.w(), p.h(), FACE);
    // Raised frame: light/dark outside, highlight/shadow inside.
    for (r, tl, br) in [(p, LIGHT, DKSHADOW), (Rect { x0: p.x0 + 1, y0: p.y0 + 1, x1: p.x1 - 1, y1: p.y1 - 1 }, HILIGHT, SHADOW)] {
        c.fill(r.x0, r.y0, r.w() - 1, 1, tl);
        c.fill(r.x0, r.y0, 1, r.h() - 1, tl);
        c.fill(r.x0, r.y1, r.w(), 1, br);
        c.fill(r.x1, r.y0, 1, r.h(), br);
    }
    let accel_x = m.accel_x.unwrap_or_else(|| TEXT_X + m.items.iter().map(|i| f.width(&strip(&i.text))).max().unwrap_or(0) + 2);
    let mut top = p.y0 + 3;
    for (k, it) in m.items.iter().enumerate() {
        if it.is_separator() {
            c.fill(p.x0 + 4, top + 4, p.w() - 8, 1, SHADOW);
            c.fill(p.x0 + 4, top + 5, p.w() - 8, 1, HILIGHT);
            top += SEP_H;
            continue;
        }
        let col = if hot == Some(k) {
            c.fill(p.x0 + 3, top, p.w() - 6, ITEM_H, HIGHLIGHT);
            HILIGHT
        } else {
            TEXT
        };
        if it.checked {
            const TICK: [&str; 6] = [".....#", "#...##", "##.###", "#####.", ".###..", "..#..."];
            for (y, row) in TICK.iter().enumerate() {
                for (x, ch) in row.chars().enumerate() {
                    if ch == '#' {
                        c.put(p.x0 + 6 + x as i32, top + 6 + y as i32, col);
                    }
                }
            }
        }
        label(c, f, &it.text, p.x0 + TEXT_X, top + 4, col);
        if !it.accel.is_empty() {
            label(c, f, &it.accel, p.x0 + accel_x, top + 4, col);
        }
        top += ITEM_H;
    }
}
