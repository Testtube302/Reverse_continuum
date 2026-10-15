//! Monochrome bitmap fonts in the `.bfnt` format made by `tools/fonts/extract_wine_fonts.py`
//! (MS Sans Serif 8 and Tahoma 8 as Wine draws them; see third_party/wine-fonts).

pub struct Glyph {
    pub advance: i32,
    pub x: i32,
    /// Top of the bitmap below the top of the text cell.
    pub y: i32,
    pub w: i32,
    pub h: i32,
    rows: Vec<u8>,
}

impl Glyph {
    pub fn bit(&self, x: i32, y: i32) -> bool {
        let stride = (self.w + 7) / 8;
        self.rows.get((y * stride + x / 8) as usize).is_some_and(|b| b & (0x80 >> (x % 8)) != 0)
    }
}

pub struct BitmapFont {
    pub ascent: i32,
    pub height: i32,
    glyphs: Vec<Option<Glyph>>,
}

pub const MS_SANS_SERIF_8: &[u8] = include_bytes!("../third_party/wine-fonts/mssansserif8.bfnt");
pub const TAHOMA_8: &[u8] = include_bytes!("../third_party/wine-fonts/tahoma8.bfnt");
pub const TAHOMA_8_BOLD: &[u8] = include_bytes!("../third_party/wine-fonts/tahoma8bold.bfnt");

impl BitmapFont {
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.get(0..4)? != b"BFNT" {
            return None;
        }
        let (ascent, height) = (*d.get(4)? as i32, *d.get(5)? as i32);
        let count = u16::from_le_bytes([*d.get(6)?, *d.get(7)?]) as usize;
        let mut glyphs: Vec<Option<Glyph>> = (0..256).map(|_| None).collect();
        let mut p = 8;
        for _ in 0..count {
            let h = d.get(p..p + 7)?;
            let code = u16::from_le_bytes([h[0], h[1]]) as usize;
            let (w, gh) = (h[5] as i32, h[6] as i32);
            let n = ((w + 7) / 8 * gh) as usize;
            let rows = d.get(p + 7..p + 7 + n)?.to_vec();
            p += 7 + n;
            if code < 256 {
                glyphs[code] =
                    Some(Glyph { advance: h[2] as i8 as i32, x: h[3] as i8 as i32, y: h[4] as i8 as i32, w, h: gh, rows });
            }
        }
        Some(Self { ascent, height, glyphs })
    }

    fn glyph(&self, c: char) -> Option<&Glyph> {
        let code = if (c as u32) < 256 { c as usize } else { b'?' as usize };
        self.glyphs[code].as_ref().or(self.glyphs[b'?' as usize].as_ref())
    }

    pub fn width(&self, text: &str) -> i32 {
        text.chars().map(|c| self.glyph(c).map_or(0, |g| g.advance)).sum()
    }

    /// Windows dialog base unit: average width of A-Z and a-z, rounded as GDI does.
    pub fn avg_char_width(&self) -> i32 {
        let total: i32 = ('A'..='Z').chain('a'..='z').map(|c| self.glyph(c).map_or(0, |g| g.advance)).sum();
        (total + 26) / 52
    }

    /// Draws `text` with its cell's top-left at (x, y); `put` receives each set pixel.
    pub fn draw(&self, text: &str, x: i32, y: i32, mut put: impl FnMut(i32, i32)) -> i32 {
        let mut pen = x;
        for c in text.chars() {
            if let Some(g) = self.glyph(c) {
                for gy in 0..g.h {
                    for gx in 0..g.w {
                        if g.bit(gx, gy) {
                            put(pen + g.x + gx, y + g.y + gy);
                        }
                    }
                }
                pen += g.advance;
            }
        }
        pen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_fonts_load_with_dialog_metrics() {
        let ms = BitmapFont::parse(MS_SANS_SERIF_8).unwrap();
        assert_eq!((ms.height, ms.ascent, ms.avg_char_width()), (13, 11, 6));
        let t = BitmapFont::parse(TAHOMA_8_BOLD).unwrap();
        assert!(t.width("Select/Edit Profile") > 0);
    }
}
