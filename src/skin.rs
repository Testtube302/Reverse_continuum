//! Continuum startup-screen skins, as described in the Continuum skins guide (`skins/skins.txt`
//! in every install): a bitmap holding the main region plus overlay regions, and a text file of
//! `HOTSPOT = (x1,y1,x2,y2) [NORM ...] [+ ALT ...]` and `COLORDEF = (r,g,b)` lines.
//!
//! `.skn` files are the bitmap with the text appended. The built-in `<default>` skin is read
//! from the install's `menures.dll` resources.

use crate::assets::RgbaImage;
use crate::bmp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    /// Inclusive, as in the skin file.
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
    pub fn w(&self) -> i32 {
        self.x1 - self.x0 + 1
    }
    pub fn h(&self) -> i32 {
        self.y1 - self.y0 + 1
    }
}

/// One step of an overlay animation: shown for `ticks` interface ticks (~200 ms each);
/// `at` is the overlay's top-left in the skin bitmap, or None for `(*)` (draw nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub ticks: u32,
    pub at: Option<(i32, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub rect: Rect,
    pub norm: Vec<Step>,
    pub alt: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotspot {
    pub name: String,
    pub regions: Vec<Region>,
}

impl Hotspot {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.regions.iter().any(|r| r.rect.contains(x, y))
    }
}

pub struct Skin {
    pub image: RgbaImage,
    pub main: Rect,
    pub hotspots: Vec<Hotspot>,
    pub colors: Vec<(String, [u8; 3])>,
}

impl Skin {
    pub fn hotspot(&self, name: &str) -> Option<&Hotspot> {
        self.hotspots.iter().find(|h| h.name == name)
    }

    pub fn color(&self, name: &str) -> Option<[u8; 3]> {
        self.colors.iter().find(|(n, _)| n == name).map(|(_, c)| *c)
    }

    /// Builds a skin from a bitmap (with BM header) and its description.
    pub fn from_parts(bitmap: &[u8], text: &str) -> Result<Self, String> {
        let img = bmp::decode(bitmap).map_err(|e| e.to_string())?;
        let image = RgbaImage { width: img.width, height: img.height, pixels: img.to_rgba(false) };
        let (main, hotspots, colors) = parse(text)?;
        if main.x1 as u32 >= image.width || main.y1 as u32 >= image.height {
            return Err("main region is larger than the skin bitmap".into());
        }
        Ok(Self { image, main, hotspots, colors })
    }

    /// A `.skn` file: a BMP followed by the description text.
    pub fn from_skn(data: &[u8]) -> Result<Self, String> {
        let size = bmp::file_size_field(data).ok_or("not a skin file")? as usize;
        let text = data.get(size..).ok_or("truncated skin file")?;
        Self::from_parts(&data[..size], &String::from_utf8_lossy(text))
    }

    /// The built-in skin and the spectate icon from the install's `menures.dll`.
    pub fn from_menures(dll: &[u8]) -> Result<(Self, RgbaImage), String> {
        let resources = pe_resources(dll)?;
        let text = resources
            .iter()
            .filter(|r| r.kind != RT_BITMAP)
            .map(|r| String::from_utf8_lossy(r.data.split(|&b| b == 0).next().unwrap_or(&[])).into_owned())
            .find(|t| parse(t).is_ok())
            .ok_or("menures.dll: no skin description")?;
        let (main, _, _) = parse(&text)?;
        let bitmaps: Vec<Vec<u8>> =
            resources.iter().filter(|r| r.kind == RT_BITMAP).filter_map(|r| with_file_header(r.data)).collect();
        let dims = |b: &Vec<u8>| bmp::decode(b).map(|i| (i.width as i32, i.height as i32)).unwrap_or((0, 0));
        let sheet = bitmaps
            .iter()
            .find(|b| {
                let (w, h) = dims(b);
                w > main.x1 && h > main.y1
            })
            .ok_or("menures.dll: no skin bitmap")?;
        let icon = bitmaps.iter().find(|b| dims(b) == (36, 36)).ok_or("menures.dll: no spectate icon")?;
        let icon = bmp::decode(icon).map_err(|e| e.to_string())?;
        let icon = RgbaImage { width: icon.width, height: icon.height, pixels: icon.to_rgba(true) };
        Ok((Self::from_parts(sheet, &text)?, icon))
    }
}

type Parsed = (Rect, Vec<Hotspot>, Vec<(String, [u8; 3])>);

/// Parses a skin description. Names are lower-cased; regions of the same hotspot are merged.
pub fn parse(text: &str) -> Result<Parsed, String> {
    let mut main = None;
    let mut hotspots: Vec<Hotspot> = Vec::new();
    let mut colors = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line: String = raw.split(';').next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
        if line.is_empty() {
            continue;
        }
        let bad = || format!("skin line {}: {raw}", n + 1);
        let (name, rest) = line.split_once('=').ok_or_else(bad)?;
        let name = name.to_ascii_lowercase();
        let groups = tuples(rest).ok_or_else(bad)?;
        if name.contains("color") {
            let [r, g, b] = groups.first().and_then(|(_, t)| <[i32; 3]>::try_from(t.clone()).ok()).ok_or_else(bad)?;
            colors.push((name, [r as u8, g as u8, b as u8]));
            continue;
        }
        let (_, first) = groups.first().ok_or_else(bad)?;
        let [x0, y0, x1, y1] = <[i32; 4]>::try_from(first.clone()).map_err(|_| bad())?;
        let rect = Rect { x0, y0, x1, y1 };
        if name == "main" {
            main = Some(rect);
            continue;
        }
        let mut region = Region { rect, norm: Vec::new(), alt: Vec::new() };
        let mut in_alt = false;
        for (prefix, t) in &groups[1..] {
            let (count, plus) = parse_prefix(prefix).ok_or_else(bad)?;
            in_alt |= plus;
            let at = match t.as_slice() {
                [x, y] => Some((*x, *y)),
                [] => None,
                _ => return Err(bad()),
            };
            let step = Step { ticks: count, at };
            if in_alt { region.alt.push(step) } else { region.norm.push(step) }
        }
        match hotspots.iter_mut().find(|h| h.name == name) {
            Some(h) => h.regions.push(region),
            None => hotspots.push(Hotspot { name, regions: vec![region] }),
        }
    }
    Ok((main.ok_or("skin has no main region")?, hotspots, colors))
}

/// Splits `+2(1,2)(*)(3,4)` into (prefix, numbers) pairs: [("+2",[1,2]), ("",[]), ("",[3,4])].
fn tuples(s: &str) -> Option<Vec<(String, Vec<i32>)>> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest[open..].find(')')? + open;
        let prefix = rest[..open].to_string();
        let inner = &rest[open + 1..close];
        let nums = if inner == "*" {
            Vec::new()
        } else {
            inner.split(',').map(|v| v.parse().ok()).collect::<Option<Vec<i32>>>()?
        };
        out.push((prefix, nums));
        rest = &rest[close + 1..];
    }
    Some(out)
}

/// `"+3"` -> (3, true); `""` -> (1, false).
fn parse_prefix(p: &str) -> Option<(u32, bool)> {
    let (plus, digits) = match p.strip_prefix('+') {
        Some(d) => (true, d),
        None => (false, p),
    };
    let count = if digits.is_empty() { 1 } else { digits.parse().ok()? };
    Some((count.max(1), plus))
}

const RT_BITMAP: u32 = 2;

struct Resource<'a> {
    kind: u32,
    data: &'a [u8],
}

/// Lists the leaf resources of a PE file (type id and data), enough to find bitmaps and text.
fn pe_resources(d: &[u8]) -> Result<Vec<Resource<'_>>, String> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let bad = || "not a PE file".to_string();
    let pe = u32_at(0x3C).ok_or_else(bad)?;
    if d.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(bad());
    }
    let sections = u16_at(pe + 6).ok_or_else(bad)?;
    let opt = pe + 24;
    let opt_size = u16_at(pe + 20).ok_or_else(bad)?;
    let dirs = opt + if u16_at(opt) == Some(0x20B) { 112 } else { 96 };
    let res_rva = u32_at(dirs + 2 * 8).ok_or_else(bad)?;
    let mut table = Vec::new();
    for i in 0..sections {
        let s = opt + opt_size + i * 40;
        let (vsize, va, raw_size, raw) =
            (u32_at(s + 8).ok_or_else(bad)?, u32_at(s + 12).ok_or_else(bad)?, u32_at(s + 16).ok_or_else(bad)?, u32_at(s + 20).ok_or_else(bad)?);
        table.push((va, vsize.max(raw_size), raw));
    }
    let to_off = |rva: usize| table.iter().find(|(va, size, _)| rva >= *va && rva < va + size).map(|(va, _, raw)| rva - va + raw);
    let root = to_off(res_rva).ok_or("no resource section")?;
    let mut out = Vec::new();
    // Walk type -> name -> language; offsets are relative to the resource root.
    let entries = |dir: usize| -> Vec<(usize, usize)> {
        let n = u16_at(root + dir + 12).unwrap_or(0) + u16_at(root + dir + 14).unwrap_or(0);
        (0..n).filter_map(|i| Some((u32_at(root + dir + 16 + i * 8)?, u32_at(root + dir + 20 + i * 8)?))).collect()
    };
    for (kind, sub) in entries(0) {
        if sub & 0x8000_0000 == 0 {
            continue;
        }
        for (_, sub2) in entries(sub & 0x7FFF_FFFF) {
            let leaves = if sub2 & 0x8000_0000 != 0 { entries(sub2 & 0x7FFF_FFFF) } else { vec![(0, sub2)] };
            for (_, leaf) in leaves {
                let Some(rva) = u32_at(root + leaf) else { continue };
                let Some(size) = u32_at(root + leaf + 4) else { continue };
                if let Some(data) = to_off(rva).and_then(|o| d.get(o..o + size)) {
                    out.push(Resource { kind: if kind & 0x8000_0000 != 0 { 0 } else { kind as u32 }, data });
                }
            }
        }
    }
    Ok(out)
}

/// RT_BITMAP resources omit the BITMAPFILEHEADER; add one so the BMP decoder can read them.
fn with_file_header(dib: &[u8]) -> Option<Vec<u8>> {
    let hs = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
    let bpp = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let used = u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize;
    let colors = if used != 0 { used } else if bpp <= 8 { 1 << bpp } else { 0 };
    let bits = 14 + hs + colors * 4;
    let mut out = Vec::with_capacity(14 + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(bits as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_guide_examples() {
        let (main, hs, colors) = parse(
            "; comment\nmain = (0,0,639,479) ; c\n\
             profile=(0,0,20,20)(800,0)(810,0)+(900,0)2(910,0)\n\
             trace = (670,25,702,45) 2(0,750) 5(*)\n\
             zones = (400,0,500,30) + (20,800)\nzones=(500,0,600,35)\n\
             zoneselcolor1 = (255,0,255)\n",
        )
        .unwrap();
        assert_eq!(main, Rect { x0: 0, y0: 0, x1: 639, y1: 479 });
        let p = &hs[0].regions[0];
        assert_eq!(p.norm, vec![Step { ticks: 1, at: Some((800, 0)) }, Step { ticks: 1, at: Some((810, 0)) }]);
        assert_eq!(p.alt, vec![Step { ticks: 1, at: Some((900, 0)) }, Step { ticks: 2, at: Some((910, 0)) }]);
        assert_eq!(hs[1].regions[0].norm[1], Step { ticks: 5, at: None });
        assert_eq!(hs[2].regions.len(), 2);
        assert_eq!(colors, vec![("zoneselcolor1".to_string(), [255, 0, 255])]);
    }
}
