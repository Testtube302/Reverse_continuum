//! Windows dialogs from the install's `menu040.dll` dialog templates, drawn like Wine's classic
//! (unthemed) controls with Wine's MS Sans Serif / Tahoma bitmaps, so they match the original
//! client running under Wine.
//!
//! Positions come from the templates in dialog units: x * base_x / 4, y * base_y / 8 with the
//! dialog font's average width (6) and height (13). A modal frame is 3 px, the caption 19 px.

use crate::launcher::{Canvas, Input};
use crate::skin::Rect;
use crate::winfont::{BitmapFont, MS_SANS_SERIF_8, TAHOMA_8_BOLD};

pub const FACE: [u8; 3] = [212, 208, 200];
pub const HILIGHT: [u8; 3] = [255, 255, 255];
pub const LIGHT: [u8; 3] = [212, 208, 200];
pub const SHADOW: [u8; 3] = [128, 128, 128];
pub const DKSHADOW: [u8; 3] = [64, 64, 64];
pub const WINDOW: [u8; 3] = [255, 255, 255];
pub const TEXT: [u8; 3] = [0, 0, 0];
pub const HIGHLIGHT: [u8; 3] = [10, 36, 106];
const CAPTION_LEFT: [u8; 3] = [10, 36, 106];
const CAPTION_RIGHT: [u8; 3] = [166, 202, 240];

pub const FRAME: i32 = 3;
pub const CAPTION: i32 = 19;
const CHECK_SIZE: i32 = 13;
const SCROLL_W: i32 = 16;

pub const IDOK: i32 = 1;
pub const IDCANCEL: i32 = 2;

const WS_VISIBLE: u32 = 0x1000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const ES_PASSWORD: u32 = 0x20;
const ES_NUMBER: u32 = 0x2000;
const SS_CENTER: u32 = 1;
const SS_RIGHT: u32 = 2;
const ES_MULTILINE: u32 = 4;
const BS_LEFT: u32 = 0x100;
const WS_EX_STATICEDGE: u32 = 0x2_0000;

#[derive(Clone, Debug, PartialEq)]
pub enum Class {
    Button,
    Edit,
    Static,
    ListBox,
    ScrollBar,
    ComboBox,
    Custom(String),
}

#[derive(Clone, Debug)]
pub struct ControlTemplate {
    pub id: i32,
    pub class: Class,
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub cx: i32,
    pub cy: i32,
    pub style: u32,
    pub ex_style: u32,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub id: u32,
    pub title: String,
    pub cx: i32,
    pub cy: i32,
    pub style: u32,
    pub controls: Vec<ControlTemplate>,
}

/// Every dialog template in a PE file.
pub fn templates(dll: &[u8]) -> Vec<Template> {
    crate::pe::resources(dll)
        .unwrap_or_default()
        .iter()
        .filter(|r| r.kind == 5)
        .filter_map(|r| parse_template(r.id, r.data))
        .collect()
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.d.get(self.p..self.p + 2)?.try_into().ok()?);
        self.p += 2;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.d.get(self.p..self.p + 4)?.try_into().ok()?);
        self.p += 4;
        Some(v)
    }
    fn i16(&mut self) -> Option<i32> {
        self.u16().map(|v| v as i16 as i32)
    }
    fn align4(&mut self) {
        self.p = (self.p + 3) & !3;
    }
    /// A string or ordinal field: returns (text, ordinal).
    fn sz_or_ord(&mut self) -> Option<(String, Option<u16>)> {
        let first = self.u16()?;
        if first == 0xFFFF {
            return Some((String::new(), Some(self.u16()?)));
        }
        let mut units = Vec::new();
        let mut c = first;
        while c != 0 {
            units.push(c);
            c = self.u16()?;
        }
        Some((String::from_utf16_lossy(&units), None))
    }
}

fn parse_template(id: u32, d: &[u8]) -> Option<Template> {
    let mut r = Reader { d, p: 0 };
    let ex = d.get(0..4)? == [1, 0, 0xFF, 0xFF];
    let (style, count);
    if ex {
        r.p = 8;
        let _ex_style = r.u32()?;
        style = r.u32()?;
    } else {
        style = r.u32()?;
        let _ex_style = r.u32()?;
    }
    count = r.u16()?;
    let (_x, _y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
    r.sz_or_ord()?; // menu
    r.sz_or_ord()?; // class
    let (title, _) = r.sz_or_ord()?;
    if style & 0x40 != 0 {
        r.u16()?; // point size
        if ex {
            r.u16()?;
            r.u16()?;
        }
        r.sz_or_ord()?; // face name
    }
    let mut controls = Vec::new();
    for _ in 0..count {
        r.align4();
        let (style, ex_style, x, y, cx, cy, id);
        if ex {
            let _help = r.u32()?;
            ex_style = r.u32()?;
            style = r.u32()?;
            (x, y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            id = r.u32()? as i32;
        } else {
            style = r.u32()?;
            ex_style = r.u32()?;
            (x, y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            id = r.u16()? as i16 as i32;
        }
        let class = match r.sz_or_ord()? {
            (_, Some(0x80)) => Class::Button,
            (_, Some(0x81)) => Class::Edit,
            (_, Some(0x82)) => Class::Static,
            (_, Some(0x83)) => Class::ListBox,
            (_, Some(0x84)) => Class::ScrollBar,
            (_, Some(0x85)) => Class::ComboBox,
            (name, _) => Class::Custom(name),
        };
        let (text, _) = r.sz_or_ord()?;
        let extra = r.u16()? as usize;
        r.p += extra;
        controls.push(ControlTemplate { id, class, text, x, y, cx, cy, style, ex_style });
    }
    Some(Template { id, title, cx, cy, style, controls })
}

// ---------------------------------------------------------------------------------------------

/// One list row: text runs (x offset from the row's left, text, colour). Plain lists use one run.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub runs: Vec<(i32, String, [u8; 3])>,
    /// Runs drawn instead while the row is selected (owner-drawn lists); empty = same runs in
    /// highlight text colour.
    pub selected_runs: Vec<(i32, String, [u8; 3])>,
}

impl Item {
    pub fn text(s: &str) -> Self {
        Self { runs: vec![(0, s.to_string(), TEXT)], selected_runs: Vec::new() }
    }
}

pub struct Control {
    pub t: ControlTemplate,
    /// In dialog client pixels.
    pub rect: Rect,
    pub text: String,
    pub checked: bool,
    pub enabled: bool,
    pub visible: bool,
    pub items: Vec<Item>,
    pub sel: Option<usize>,
    pub top: usize,
    /// List selection colour (owner-drawn lists); None = the system highlight.
    pub highlight: Option<[u8; 3]>,
    /// Rows drawn in (synthesised) bold.
    pub bold: Vec<bool>,
    /// Combo box: list dropped down, and the row under the mouse.
    pub dropped: bool,
    pub hot: Option<usize>,
}

impl Control {
    fn is_button_kind(&self, kind: u32) -> bool {
        self.t.class == Class::Button && self.t.style & 0xF == kind
    }
    fn is_push(&self) -> bool {
        self.is_button_kind(0) || self.is_button_kind(1)
    }
    fn is_check(&self) -> bool {
        matches!(self.t.style & 0xF, 2 | 3 | 5 | 6) && self.t.class == Class::Button
    }
    fn focusable(&self) -> bool {
        self.visible && self.enabled && self.t.style & WS_TABSTOP != 0 && !self.is_button_kind(7)
    }
    /// Combobox: the visible selection field (the template height is the drop-down's).
    fn field(&self, font: &BitmapFont) -> Rect {
        if self.t.class == Class::ComboBox {
            Rect { x1: self.rect.x1, y1: self.rect.y0 + font.height + 7, ..self.rect }
        } else {
            self.rect
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A push button was clicked (or Enter / Esc for IDOK / IDCANCEL), or a checkbox toggled.
    Command(i32),
    Select(i32, usize),
    DoubleClick(i32, usize),
}

pub struct Dialog {
    pub template_id: u32,
    pub title: String,
    /// Outer rectangle in window pixels.
    pub frame: Rect,
    pub controls: Vec<Control>,
    pub focus: Option<usize>,
    font: BitmapFont,
    caption_font: BitmapFont,
    last_click: Option<(std::time::Instant, i32, usize)>,
}

fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    // Windows MulDiv: rounds half away from zero.
    let n = a as i64 * b as i64;
    ((n * 2 + if n >= 0 { c as i64 } else { -(c as i64) }) / (2 * c as i64)) as i32
}

impl Dialog {
    /// Lays out a template centred on a parent window whose outer rectangle is `parent`.
    pub fn new(t: &Template, parent: Rect) -> Self {
        let font = BitmapFont::parse(MS_SANS_SERIF_8).expect("built-in font");
        let caption_font = BitmapFont::parse(TAHOMA_8_BOLD).expect("built-in font");
        let (bx, by) = (font.avg_char_width(), font.height);
        let w = mul_div(t.cx, bx, 4) + 2 * FRAME;
        let h = mul_div(t.cy, by, 8) + 2 * FRAME + CAPTION;
        let x0 = parent.x0 + (parent.w() - w + 1) / 2;
        let y0 = parent.y0 + (parent.h() - h + 1) / 2;
        let controls = t
            .controls
            .iter()
            .map(|c| {
                let (x, y) = (mul_div(c.x, bx, 4), mul_div(c.y, by, 8));
                let (cw, ch) = (mul_div(c.cx, bx, 4), mul_div(c.cy, by, 8));
                Control {
                    rect: Rect { x0: x, y0: y, x1: x + cw - 1, y1: y + ch - 1 },
                    text: c.text.clone(),
                    checked: false,
                    enabled: c.style & WS_DISABLED == 0,
                    visible: c.style & WS_VISIBLE != 0,
                    items: Vec::new(),
                    sel: None,
                    top: 0,
                    highlight: None,
                    bold: Vec::new(),
                    dropped: false,
                    hot: None,
                    t: c.clone(),
                }
            })
            .collect();
        let mut d = Self {
            template_id: t.id,
            title: t.title.clone(),
            frame: Rect { x0, y0, x1: x0 + w - 1, y1: y0 + h - 1 },
            controls,
            focus: None,
            font,
            caption_font,
            last_click: None,
        };
        d.focus = d.controls.iter().position(|c| c.focusable());
        d
    }

    /// Moves the dialog so it lies inside a `w` x `h` window.
    pub fn clamp_into(&mut self, w: i32, h: i32) {
        let dx = (-self.frame.x0).max(0) - (self.frame.x1 - (w - 1)).max(0).min(self.frame.x0.max(0));
        let dy = (-self.frame.y0).max(0) - (self.frame.y1 - (h - 1)).max(0).min(self.frame.y0.max(0));
        self.frame = Rect { x0: self.frame.x0 + dx, y0: self.frame.y0 + dy, x1: self.frame.x1 + dx, y1: self.frame.y1 + dy };
    }

    pub fn client_origin(&self) -> (i32, i32) {
        (self.frame.x0 + FRAME, self.frame.y0 + FRAME + CAPTION)
    }

    pub fn control(&self, id: i32) -> Option<&Control> {
        self.controls.iter().find(|c| c.t.id == id)
    }

    pub fn control_mut(&mut self, id: i32) -> Option<&mut Control> {
        self.controls.iter_mut().find(|c| c.t.id == id)
    }

    pub fn text(&self, id: i32) -> String {
        self.control(id).map(|c| c.text.clone()).unwrap_or_default()
    }

    pub fn set_text(&mut self, id: i32, text: &str) {
        if let Some(c) = self.control_mut(id) {
            c.text = text.to_string();
        }
    }

    pub fn checked(&self, id: i32) -> bool {
        self.control(id).is_some_and(|c| c.checked)
    }

    pub fn focus_id(&mut self, id: i32) {
        self.focus = self.controls.iter().position(|c| c.t.id == id);
    }

    fn list_rows(&self, c: &Control) -> usize {
        ((c.rect.h() - 4) / self.font.height).max(1) as usize
    }

    fn list_needs_scroll(&self, c: &Control) -> bool {
        c.t.style & WS_VSCROLL != 0 && c.items.len() > self.list_rows(c)
    }

    /// A dropped combo box's list rectangle (client pixels) and how many rows it shows.
    fn drop_list(&self, c: &Control) -> (Rect, usize) {
        let field = c.field(&self.font);
        let room = ((c.rect.y1 - field.y1 - 2) / self.font.height).max(1) as usize;
        let rows = c.items.len().min(room).max(1);
        let r = Rect { x0: field.x0, y0: field.y1 + 1, x1: field.x1, y1: field.y1 + rows as i32 * self.font.height + 2 };
        (r, rows)
    }

    pub fn handle(&mut self, input: &Input) -> Vec<Event> {
        let mut out = Vec::new();
        let (ox, oy) = self.client_origin();
        let (mx, my) = (input.mouse.0 - ox, input.mouse.1 - oy);
        // An open combo-box list takes all input until it closes.
        if let Some(i) = self.controls.iter().position(|c| c.dropped) {
            let (list, rows) = self.drop_list(&self.controls[i]);
            let fh = self.font.height;
            let c = &mut self.controls[i];
            let under = (list.contains(mx, my) && my > list.y0 && my < list.y1)
                .then(|| c.top + ((my - list.y0 - 1) / fh) as usize)
                .filter(|&k| k < c.items.len());
            if under.is_some() {
                c.hot = under;
            }
            if input.up || input.down {
                let n = c.items.len();
                let k = c.hot.unwrap_or(0);
                c.hot = Some(if input.down { (k + 1).min(n.saturating_sub(1)) } else { k.saturating_sub(1) });
            }
            if let Some(h) = c.hot {
                if h < c.top {
                    c.top = h;
                } else if h >= c.top + rows {
                    c.top = h + 1 - rows;
                }
            }
            if input.escape {
                c.dropped = false;
            } else if input.enter || (input.left_pressed && under.is_some()) {
                c.dropped = false;
                if let Some(h) = c.hot {
                    c.sel = Some(h);
                    out.push(Event::Select(c.t.id, h));
                }
            } else if input.left_pressed {
                c.dropped = false;
            }
            return out;
        }
        if input.wheel != 0 {
            if let Some(i) = self.controls.iter().position(|c| c.visible && c.t.class == Class::ListBox && c.rect.contains(mx, my)) {
                let rows = self.list_rows(&self.controls[i]);
                let c = &mut self.controls[i];
                let max = c.items.len().saturating_sub(rows) as i32;
                c.top = (c.top as i32 - input.wheel * 3).clamp(0, max.max(0)) as usize;
            }
        }
        if input.left_pressed {
            // Caption close box.
            if self.close_box().contains(input.mouse.0, input.mouse.1) {
                out.push(Event::Command(IDCANCEL));
                return out;
            }
            let font_h = self.font.height;
            let hit = self.controls.iter().position(|c| c.visible && c.enabled && !c.is_button_kind(7) && c.field(&self.font).contains(mx, my));
            if let Some(i) = hit {
                if self.controls[i].focusable() || self.controls[i].t.class == Class::ListBox {
                    self.focus = Some(i);
                }
                let scroll = self.list_needs_scroll(&self.controls[i]);
                let rows = self.list_rows(&self.controls[i]);
                let c = &mut self.controls[i];
                match &c.t.class {
                    Class::Button if c.is_check() => {
                        c.checked = !c.checked;
                        out.push(Event::Command(c.t.id));
                    }
                    Class::Button => out.push(Event::Command(c.t.id)),
                    Class::ListBox => {
                        if scroll && mx > c.rect.x1 - 2 - SCROLL_W {
                            // Scroll bar: top arrow, bottom arrow, or page by trough half.
                            let max = c.items.len().saturating_sub(rows);
                            if my < c.rect.y0 + 2 + SCROLL_W {
                                c.top = c.top.saturating_sub(1);
                            } else if my > c.rect.y1 - 2 - SCROLL_W {
                                c.top = (c.top + 1).min(max);
                            } else if my < (c.rect.y0 + c.rect.y1) / 2 {
                                c.top = c.top.saturating_sub(rows);
                            } else {
                                c.top = (c.top + rows).min(max);
                            }
                        } else {
                            let row = c.top + ((my - c.rect.y0 - 2) / font_h).max(0) as usize;
                            if row < c.items.len() {
                                c.sel = Some(row);
                                let now = std::time::Instant::now();
                                let double = self.last_click.is_some_and(|(t, id, r)| {
                                    id == c.t.id && r == row && now.duration_since(t).as_millis() < 500
                                });
                                self.last_click = if double { None } else { Some((now, c.t.id, row)) };
                                out.push(if double || input.double_click { Event::DoubleClick(c.t.id, row) } else { Event::Select(c.t.id, row) });
                            }
                        }
                    }
                    Class::ComboBox if !c.items.is_empty() => {
                        c.dropped = true;
                        c.hot = c.sel;
                        c.top = 0;
                    }
                    _ => {}
                }
            }
        }
        if input.tab {
            let n = self.controls.len();
            let start = self.focus.unwrap_or(n - 1);
            self.focus = (1..=n).map(|k| (start + k) % n).find(|&i| self.controls[i].focusable()).or(self.focus);
        }
        if let Some(i) = self.focus {
            let c = &mut self.controls[i];
            if c.t.class == Class::Edit {
                for &ch in &input.chars {
                    let ok = if c.t.style & ES_NUMBER != 0 { ch.is_ascii_digit() } else { (' '..='~').contains(&ch) };
                    if ok && c.text.len() < 63 {
                        c.text.push(ch);
                    }
                }
                if input.backspace {
                    c.text.pop();
                }
            } else if c.is_check() && input.chars.contains(&' ') {
                c.checked = !c.checked;
                out.push(Event::Command(c.t.id));
            }
        }
        if input.enter {
            let default = self.controls.iter().find(|c| c.visible && c.is_button_kind(1)).map_or(IDOK, |c| c.t.id);
            out.push(Event::Command(default));
        }
        if input.escape {
            out.push(Event::Command(IDCANCEL));
        }
        out
    }

    fn close_box(&self) -> Rect {
        let x1 = self.frame.x1 - FRAME - 2;
        let y0 = self.frame.y0 + FRAME + 2;
        Rect { x0: x1 - 15, y0, x1, y1: y0 + 13 }
    }

    // -----------------------------------------------------------------------------------------
    // Drawing

    pub fn draw(&self, c: &mut Canvas) {
        let f = self.frame;
        c.fill(f.x0, f.y0, f.w(), f.h(), FACE);
        // Modal frame: raised edge then a face-coloured line.
        edge(c, f, LIGHT, DKSHADOW, HILIGHT, SHADOW);
        // Caption: horizontal gradient, bold title, close box.
        let cap = Rect { x0: f.x0 + FRAME, y0: f.y0 + FRAME, x1: f.x1 - FRAME, y1: f.y0 + FRAME + CAPTION - 2 };
        // The gradient stops short of the caption-button area (18 px); the end colour fills it.
        let span = (cap.w() - 18).max(1);
        for x in cap.x0..=cap.x1 {
            let t = ((x - cap.x0) as f32 / span as f32).min(1.0);
            let col = [0, 1, 2].map(|k| (CAPTION_LEFT[k] as f32 + (CAPTION_RIGHT[k] as f32 - CAPTION_LEFT[k] as f32) * t) as u8);
            c.fill(x, cap.y0, 1, cap.h(), col);
        }
        let ty = cap.y0 + 5;
        self.caption_font.draw(&self.title, cap.x0 + 2, ty, |x, y| c.put(x, y, HILIGHT));
        let cb = self.close_box();
        c.fill(cb.x0, cb.y0, cb.w(), cb.h(), FACE);
        edge(c, cb, HILIGHT, DKSHADOW, LIGHT, SHADOW);
        // The "x" as Wine draws it from Marlett: 1 px diagonals with grey anti-aliasing.
        const X: [&str; 8] = ["dg....gd", "g#g..g#g", ".g#gg#g.", "..g##g..", "..g##g..", ".g#gg#g.", "g#g..g#g", "dg....gd"];
        for (y, row) in X.iter().enumerate() {
            for (x, p) in row.chars().enumerate() {
                let col = match p {
                    '#' => TEXT,
                    'g' => [140, 138, 132],
                    'd' => [82, 85, 82],
                    _ => continue,
                };
                c.put(cb.x0 + 4 + x as i32, cb.y0 + 3 + y as i32, col);
            }
        }
        let (ox, oy) = self.client_origin();
        for (i, ctl) in self.controls.iter().enumerate() {
            if ctl.visible {
                self.draw_control(c, ctl, ox, oy, self.focus == Some(i));
            }
        }
        if let Some(ctl) = self.controls.iter().find(|c| c.dropped) {
            let (l, rows) = self.drop_list(ctl);
            let l = Rect { x0: l.x0 + ox, y0: l.y0 + oy, x1: l.x1 + ox, y1: l.y1 + oy };
            c.fill(l.x0, l.y0, l.w(), l.h(), WINDOW);
            outline(c, l, TEXT);
            let fh = self.font.height;
            for (k, item) in ctl.items.iter().enumerate().skip(ctl.top).take(rows) {
                let y = l.y0 + 1 + (k - ctl.top) as i32 * fh;
                let hot = ctl.hot == Some(k);
                if hot {
                    c.fill(l.x0 + 1, y, l.w() - 2, fh, HIGHLIGHT);
                }
                let text: String = item.runs.iter().map(|r| r.1.as_str()).collect();
                self.text_line(c, &text, l.x0 + 2, y, if hot { HILIGHT } else { TEXT }, false);
            }
        }
    }

    fn text_line(&self, c: &mut Canvas, text: &str, x: i32, y: i32, color: [u8; 3], prefix: bool) {
        let mut pen = x;
        let mut chars = text.chars().peekable();
        let mut buf = [0u8; 4];
        while let Some(ch) = chars.next() {
            let mut underline = false;
            let ch = if prefix && ch == '&' {
                match chars.next() {
                    Some('&') => '&',
                    Some(n) => {
                        underline = true;
                        n
                    }
                    None => break,
                }
            } else {
                ch
            };
            let s = ch.encode_utf8(&mut buf);
            let next = self.font.draw(s, pen, y, |px, py| c.put(px, py, color));
            if underline {
                c.fill(pen, y + self.font.ascent + 1, next - pen, 1, color);
            }
            pen = next;
        }
    }

    /// Draws with `extra` pixels added to each advance (synthetic bold widens glyphs by one).
    fn text_line_spaced(&self, c: &mut Canvas, text: &str, x: i32, y: i32, color: [u8; 3], extra: i32) {
        let mut pen = x;
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            pen = self.font.draw(ch.encode_utf8(&mut buf), pen, y, |px, py| c.put(px, py, color)) + extra;
        }
    }

    fn text_width(&self, text: &str, prefix: bool) -> i32 {
        if prefix {
            self.font.width(&strip_prefix(text))
        } else {
            self.font.width(text)
        }
    }

    fn draw_control(&self, c: &mut Canvas, ctl: &Control, ox: i32, oy: i32, focused: bool) {
        let r = Rect { x0: ctl.rect.x0 + ox, y0: ctl.rect.y0 + oy, x1: ctl.rect.x1 + ox, y1: ctl.rect.y1 + oy };
        let fh = self.font.height;
        match &ctl.t.class {
            Class::Button if ctl.is_button_kind(7) => {
                // Group box: etched frame from mid-text height, label over it.
                let top = r.y0 + fh / 2 - 1;
                let fr = Rect { y0: top, ..r };
                edge(c, fr, SHADOW, HILIGHT, HILIGHT, SHADOW);
                let w = self.text_width(&ctl.text, true);
                if w > 0 {
                    c.fill(r.x0 + 7, r.y0, w + 2, fh, FACE);
                    self.text_line(c, &ctl.text, r.x0 + 8, r.y0, TEXT, true);
                }
            }
            Class::Button if ctl.is_check() => {
                // Measured: the box sits one pixel above the vertically centred label.
                let by = r.y0 + (r.h() - fh) / 2 - 1;
                let b = Rect { x0: r.x0, y0: by, x1: r.x0 + CHECK_SIZE - 1, y1: by + CHECK_SIZE - 1 };
                c.fill(b.x0, b.y0, b.w(), b.h(), if ctl.enabled { WINDOW } else { FACE });
                edge(c, b, SHADOW, HILIGHT, DKSHADOW, LIGHT);
                if ctl.checked {
                    // 7x7 tick, 3 px thick, starting 3 px inside the box.
                    for (i, top) in [2, 3, 4, 3, 2, 1, 0].iter().enumerate() {
                        c.fill(b.x0 + 3 + i as i32, b.y0 + 3 + top, 1, 3, TEXT);
                    }
                }
                let tx = r.x0 + CHECK_SIZE + self.font.width("0") / 2 + 1;
                let ty = r.y0 + (r.h() - fh) / 2;
                self.label(c, &ctl.text, tx, ty, ctl.enabled);
                if focused {
                    let w = self.text_width(&ctl.text, true);
                    focus_rect(c, Rect { x0: tx - 1, y0: ty, x1: tx + w, y1: ty + fh - 1 });
                }
            }
            Class::Button => {
                let mut b = r;
                if ctl.t.ex_style & WS_EX_STATICEDGE != 0 {
                    // One-pixel sunken border around the button.
                    c.fill(b.x0, b.y0, b.w() - 1, 1, SHADOW);
                    c.fill(b.x0, b.y0, 1, b.h() - 1, SHADOW);
                    c.fill(b.x0, b.y1, b.w(), 1, HILIGHT);
                    c.fill(b.x1, b.y0, 1, b.h(), HILIGHT);
                    b = inset(b, 1);
                }
                if ctl.is_button_kind(1) || (focused && ctl.is_push()) {
                    outline(c, b, TEXT);
                    b = inset(b, 1);
                }
                c.fill(b.x0, b.y0, b.w(), b.h(), FACE);
                edge(c, b, HILIGHT, DKSHADOW, LIGHT, SHADOW);
                let w = self.text_width(&ctl.text, true);
                let tx = if ctl.t.style & BS_LEFT != 0 { r.x0 + 2 } else { r.x0 + (r.w() - w) / 2 };
                let ty = r.y0 + (r.h() - fh + 1) / 2;
                self.label(c, &ctl.text, tx, ty, ctl.enabled);
                if focused {
                    focus_rect(c, inset(b, 2));
                }
            }
            Class::Edit => {
                c.fill(r.x0, r.y0, r.w(), r.h(), if ctl.enabled { WINDOW } else { FACE });
                edge(c, r, SHADOW, HILIGHT, DKSHADOW, LIGHT);
                let shown = if ctl.t.style & ES_PASSWORD != 0 { "*".repeat(ctl.text.chars().count()) } else { ctl.text.clone() };
                let tx = r.x0 + 3;
                let ty = r.y0 + 3;
                c.clip = Some((r.x0 + 2, r.y0 + 2, r.x1 - 2, r.y1 - 2));
                if ctl.t.style & ES_MULTILINE != 0 {
                    for (i, line) in wrap_px(&self.font, &shown, r.w() - 8).iter().enumerate() {
                        self.text_line(c, line, tx, ty + i as i32 * fh, TEXT, false);
                    }
                } else {
                    self.text_line(c, &shown, tx, ty, TEXT, false);
                }
                c.clip = None;
                if focused && ctl.t.style & ES_MULTILINE == 0 && ctl.t.style & 0x800 == 0 {
                    c.fill(tx + self.font.width(&shown), ty, 1, fh, TEXT);
                }
            }
            Class::Static => {
                let lines = wrap_px(&self.font, &ctl.text, r.w());
                for (i, line) in lines.iter().enumerate() {
                    let y = r.y0 + i as i32 * fh;
                    if y >= r.y1 {
                        break;
                    }
                    let x = match ctl.t.style & 0xF {
                        SS_CENTER => r.x0 + (r.w() - self.text_width(line, true)) / 2,
                        SS_RIGHT => r.x1 + 1 - self.text_width(line, true),
                        _ => r.x0,
                    };
                    self.text_line(c, line, x, y, TEXT, true);
                }
            }
            Class::ListBox => {
                c.fill(r.x0, r.y0, r.w(), r.h(), WINDOW);
                edge(c, r, SHADOW, HILIGHT, DKSHADOW, LIGHT);
                let scroll = self.list_needs_scroll(ctl);
                let inner_r = if scroll { r.x1 - 2 - SCROLL_W } else { r.x1 - 2 };
                let rows = self.list_rows(ctl);
                c.clip = Some((r.x0 + 2, r.y0 + 2, inner_r, r.y1 - 2));
                for (k, item) in ctl.items.iter().enumerate().skip(ctl.top).take(rows) {
                    let y = r.y0 + 2 + (k - ctl.top) as i32 * fh;
                    let selected = ctl.sel == Some(k);
                    if selected {
                        c.fill(r.x0 + 2, y, inner_r - r.x0 - 1, fh, ctl.highlight.unwrap_or(HIGHLIGHT));
                    }
                    let runs = if selected && !item.selected_runs.is_empty() { &item.selected_runs } else { &item.runs };
                    let bold = ctl.bold.get(k).copied().unwrap_or(false);
                    for (dx, text, col) in runs {
                        let col = if selected && item.selected_runs.is_empty() { HILIGHT } else { *col };
                        // Synthetic bold: the glyphs again one pixel to the right.
                        for extra in 0..=bold as i32 {
                            self.text_line_spaced(c, text, r.x0 + 3 + dx + extra, y, col, bold as i32);
                        }
                    }
                    if selected && focused {
                        focus_rect(c, Rect { x0: r.x0 + 2, y0: y, x1: inner_r, y1: y + fh - 1 });
                    }
                }
                c.clip = None;
                if scroll {
                    let sb = Rect { x0: inner_r + 1, y0: r.y0 + 2, x1: r.x1 - 2, y1: r.y1 - 2 };
                    self.draw_vscroll(c, sb, ctl.top, ctl.items.len(), rows);
                }
            }
            Class::ComboBox => {
                let fr = Rect { x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y0 + fh + 7 };
                c.fill(fr.x0, fr.y0, fr.w(), fr.h(), WINDOW);
                edge(c, fr, SHADOW, HILIGHT, DKSHADOW, LIGHT);
                let arrow = Rect { x0: fr.x1 - 1 - SCROLL_W, y0: fr.y0 + 2, x1: fr.x1 - 2, y1: fr.y1 - 2 };
                c.fill(arrow.x0, arrow.y0, arrow.w(), arrow.h(), FACE);
                edge(c, arrow, LIGHT, DKSHADOW, HILIGHT, SHADOW);
                arrow_glyph(c, arrow, false);
                let text = ctl.sel.and_then(|s| ctl.items.get(s)).map(|i| i.runs.iter().map(|r| r.1.as_str()).collect::<String>());
                let tx = fr.x0 + 4;
                let ty = fr.y0 + 4;
                if focused {
                    c.fill(fr.x0 + 3, fr.y0 + 3, arrow.x0 - fr.x0 - 4, fh, HIGHLIGHT);
                }
                if let Some(t) = text {
                    self.text_line(c, &t, tx, ty, if focused { HILIGHT } else { TEXT }, false);
                }
            }
            _ => {}
        }
    }

    fn label(&self, c: &mut Canvas, text: &str, x: i32, y: i32, enabled: bool) {
        if enabled {
            self.text_line(c, text, x, y, TEXT, true);
        } else {
            // Disabled: embossed (highlight offset by one, shadow on top).
            self.text_line(c, text, x + 1, y + 1, HILIGHT, true);
            self.text_line(c, text, x, y, SHADOW, true);
        }
    }

    fn draw_vscroll(&self, c: &mut Canvas, sb: Rect, top: usize, total: usize, page: usize) {
        // Trough: 50% dither of face and highlight.
        for y in sb.y0..=sb.y1 {
            for x in sb.x0..=sb.x1 {
                c.put(x, y, if (x + y) % 2 == 0 { HILIGHT } else { FACE });
            }
        }
        let up = Rect { x0: sb.x0, y0: sb.y0, x1: sb.x1, y1: sb.y0 + SCROLL_W - 1 };
        let down = Rect { x0: sb.x0, y0: sb.y1 - SCROLL_W + 1, x1: sb.x1, y1: sb.y1 };
        for (b, flip) in [(up, true), (down, false)] {
            c.fill(b.x0, b.y0, b.w(), b.h(), FACE);
            edge(c, b, LIGHT, DKSHADOW, HILIGHT, SHADOW);
            arrow_glyph(c, b, flip);
        }
        let track = (down.y0 - up.y1 - 1).max(0);
        let thumb = (track * page as i32 / total.max(1) as i32).max(8).min(track);
        let max_top = total.saturating_sub(page).max(1) as i32;
        let ty = up.y1 + 1 + (track - thumb) * top as i32 / max_top;
        let t = Rect { x0: sb.x0, y0: ty, x1: sb.x1, y1: ty + thumb - 1 };
        c.fill(t.x0, t.y0, t.w(), t.h(), FACE);
        edge(c, t, LIGHT, DKSHADOW, HILIGHT, SHADOW);
    }
}

pub fn strip_prefix(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Word-wraps to a pixel width (DT_WORDBREAK).
pub fn wrap_px(font: &BitmapFont, text: &str, width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && font.width(&strip_prefix(&candidate)) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

fn inset(r: Rect, n: i32) -> Rect {
    Rect { x0: r.x0 + n, y0: r.y0 + n, x1: r.x1 - n, y1: r.y1 - n }
}

fn outline(c: &mut Canvas, r: Rect, col: [u8; 3]) {
    c.fill(r.x0, r.y0, r.w(), 1, col);
    c.fill(r.x0, r.y1, r.w(), 1, col);
    c.fill(r.x0, r.y0, 1, r.h(), col);
    c.fill(r.x1, r.y0, 1, r.h(), col);
}

/// Two-pixel 3D edge: outer top-left/bottom-right, inner top-left/bottom-right.
fn edge(c: &mut Canvas, r: Rect, otl: [u8; 3], obr: [u8; 3], itl: [u8; 3], ibr: [u8; 3]) {
    for (r, tl, br) in [(r, otl, obr), (inset(r, 1), itl, ibr)] {
        c.fill(r.x0, r.y0, r.w() - 1, 1, tl);
        c.fill(r.x0, r.y0, 1, r.h() - 1, tl);
        c.fill(r.x0, r.y1, r.w(), 1, br);
        c.fill(r.x1, r.y0, 1, r.h(), br);
    }
}

/// Dotted focus rectangle: every other pixel inverted, as DrawFocusRect does.
fn focus_rect(c: &mut Canvas, r: Rect) {
    for x in r.x0..=r.x1 {
        if (x - r.x0) % 2 == 0 {
            c.invert(x, r.y0);
            if r.y1 != r.y0 {
                c.invert(x, r.y1);
            }
        }
    }
    for y in r.y0 + 1..r.y1 {
        if (y - r.y0) % 2 == 0 {
            c.invert(r.x0, y);
            if r.x1 != r.x0 {
                c.invert(r.x1, y);
            }
        }
    }
}

/// Small black triangle (down, or up when `up`), 7 wide, centred in `b`.
fn arrow_glyph(c: &mut Canvas, b: Rect, up: bool) {
    let cx = b.x0 + (b.w() - 7) / 2;
    let cy = b.y0 + (b.h() - 4) / 2;
    for i in 0..4 {
        let row = if up { 3 - i } else { i };
        c.fill(cx + i, cy + row, 7 - 2 * i, 1, TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_dialog_template() {
        // DLGTEMPLATE with DS_SETFONT, one button.
        let mut d = Vec::new();
        d.extend_from_slice(&0x80C8_00C0u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        for v in [0i16, 0, 162, 60] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0, 0, 0, 0]); // menu, class
        for u in "Password Required\0".encode_utf16() {
            d.extend_from_slice(&u.to_le_bytes());
        }
        d.extend_from_slice(&8u16.to_le_bytes());
        for u in "MS Sans Serif\0".encode_utf16() {
            d.extend_from_slice(&u.to_le_bytes());
        }
        while d.len() % 4 != 0 {
            d.push(0);
        }
        d.extend_from_slice(&0x5001_0001u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        for v in [27i16, 39, 50, 14] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&[0xFF, 0xFF, 0x80, 0x00]);
        for u in "Play\0".encode_utf16() {
            d.extend_from_slice(&u.to_le_bytes());
        }
        d.extend_from_slice(&0u16.to_le_bytes());
        let t = parse_template(142, &d).unwrap();
        assert_eq!((t.title.as_str(), t.cx, t.cy), ("Password Required", 162, 60));
        assert_eq!(t.controls[0].class, Class::Button);
        assert_eq!((t.controls[0].text.as_str(), t.controls[0].id), ("Play", 1));
        // The original places this dialog at 249x123 px.
        let dlg = Dialog::new(&t, Rect { x0: 0, y0: 0, x1: 642, y1: 536 });
        assert_eq!((dlg.frame.w(), dlg.frame.h()), (249, 123));
        assert_eq!(dlg.controls[0].rect, Rect { x0: 41, y0: 63, x1: 115, y1: 85 });
    }
}
