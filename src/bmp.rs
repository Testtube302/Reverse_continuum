//! Decoder for the 8-bit Windows bitmaps SubSpace uses (`.bm2`, embedded LVL tilesets).
//! Supports uncompressed (BI_RGB) and RLE8 (BI_RLE8) palettized images.

use std::fmt;

#[derive(Debug)]
pub struct BmpError(pub String);

impl fmt::Display for BmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bmp: {}", self.0)
    }
}

impl std::error::Error for BmpError {}

fn err<T>(msg: impl Into<String>) -> Result<T, BmpError> {
    Err(BmpError(msg.into()))
}

/// A palettized image stored top-down, one palette index per pixel.
#[derive(Clone, Debug)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub palette: Vec<[u8; 3]>,
}

impl IndexedImage {
    /// Expands to RGBA8. Pure black is the transparent colour for SubSpace sprites.
    pub fn to_rgba(&self, black_transparent: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &i in &self.pixels {
            let [r, g, b] = self.palette.get(i as usize).copied().unwrap_or([0, 0, 0]);
            let a = if black_transparent && r == 0 && g == 0 && b == 0 { 0 } else { 255 };
            out.extend_from_slice(&[r, g, b, a]);
        }
        out
    }
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, BmpError> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| BmpError(format!("truncated at {o}")))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32, BmpError> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| BmpError(format!("truncated at {o}")))
}

/// Total file size recorded in the BITMAPFILEHEADER (used by LVL files to find tile data).
pub fn file_size_field(data: &[u8]) -> Option<u32> {
    if data.starts_with(b"BM") { u32_at(data, 2).ok() } else { None }
}

pub fn decode(data: &[u8]) -> Result<IndexedImage, BmpError> {
    if !data.starts_with(b"BM") {
        return err("missing BM signature");
    }
    let bits_offset = u32_at(data, 10)? as usize;
    let header_size = u32_at(data, 14)? as usize;
    let width = u32_at(data, 18)? as i32;
    let raw_height = u32_at(data, 22)? as i32;
    let bpp = u16_at(data, 28)?;
    let compression = u32_at(data, 30)?;
    let colors_used = u32_at(data, 46)?;

    if bpp != 8 {
        return err(format!("unsupported bit depth {bpp}"));
    }
    if width <= 0 || raw_height == 0 {
        return err("bad dimensions");
    }
    let width = width as u32;
    let top_down = raw_height < 0;
    let height = raw_height.unsigned_abs();

    let palette_count = if colors_used == 0 { 256 } else { colors_used.min(256) } as usize;
    let pal_start = 14 + header_size;
    let mut palette = Vec::with_capacity(256);
    for i in 0..palette_count {
        let o = pal_start + i * 4;
        let e = data.get(o..o + 4).ok_or_else(|| BmpError("truncated palette".into()))?;
        palette.push([e[2], e[1], e[0]]);
    }
    palette.resize(256, [0, 0, 0]);

    let bits = data.get(bits_offset..).ok_or_else(|| BmpError("bits offset past end".into()))?;
    // Rows as stored in the file (bottom-up unless top_down).
    let stored = match compression {
        0 => decode_raw(bits, width, height)?,
        1 => decode_rle8(bits, width, height),
        c => return err(format!("unsupported compression {c}")),
    };

    let w = width as usize;
    let mut pixels = vec![0u8; stored.len()];
    for row in 0..height as usize {
        let src = if top_down { row } else { height as usize - 1 - row };
        pixels[row * w..(row + 1) * w].copy_from_slice(&stored[src * w..(src + 1) * w]);
    }

    Ok(IndexedImage { width, height, pixels, palette })
}

fn decode_raw(bits: &[u8], width: u32, height: u32) -> Result<Vec<u8>, BmpError> {
    let w = width as usize;
    let stride = (w + 3) & !3;
    let mut out = vec![0u8; w * height as usize];
    for row in 0..height as usize {
        let src = bits
            .get(row * stride..row * stride + w)
            .ok_or_else(|| BmpError("truncated pixel data".into()))?;
        out[row * w..(row + 1) * w].copy_from_slice(src);
    }
    Ok(out)
}

/// RLE8 decoder. Malformed or truncated streams leave remaining pixels at index 0.
fn decode_rle8(bits: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * h];
    let (mut x, mut y) = (0usize, 0usize);
    let mut i = 0usize;
    let put =|x: usize, y: usize, v: u8, out: &mut Vec<u8>| {
        if x < w && y < h {
            out[y * w + x] = v;
        }
    };
    while i + 1 < bits.len() && y < h {
        let (count, value) = (bits[i] as usize, bits[i + 1]);
        i += 2;
        if count > 0 {
            for _ in 0..count {
                put(x, y, value, &mut out);
                x += 1;
            }
            continue;
        }
        match value {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                if i + 1 >= bits.len() {
                    break;
                }
                x += bits[i] as usize;
                y += bits[i + 1] as usize;
                i += 2;
            }
            n => {
                let n = n as usize;
                for k in 0..n {
                    let Some(&v) = bits.get(i + k) else { break };
                    put(x, y, v, &mut out);
                    x += 1;
                }
                i += n + (n & 1);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_bmp(width: i32, height: i32, compression: u32, bits: &[u8]) -> Vec<u8> {
        let header = 40u32;
        let pal = 256 * 4;
        let off = 14 + header + pal;
        let mut d = Vec::new();
        d.extend_from_slice(b"BM");
        d.extend_from_slice(&(off + bits.len() as u32).to_le_bytes());
        d.extend_from_slice(&[0; 4]);
        d.extend_from_slice(&off.to_le_bytes());
        d.extend_from_slice(&header.to_le_bytes());
        d.extend_from_slice(&width.to_le_bytes());
        d.extend_from_slice(&height.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&8u16.to_le_bytes());
        d.extend_from_slice(&compression.to_le_bytes());
        d.extend_from_slice(&[0; 20]);
        for i in 0..256u32 {
            d.extend_from_slice(&[i as u8, (i / 2) as u8, (i / 4) as u8, 0]);
        }
        d.extend_from_slice(bits);
        d
    }

    #[test]
    fn raw_bottom_up_is_flipped_and_stride_padded() {
        // 3x2 image: stored rows padded to 4 bytes, bottom row first.
        let bits = [4, 5, 6, 0, 1, 2, 3, 0];
        let img = decode(&build_bmp(3, 2, 0, &bits)).unwrap();
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(img.pixels, vec![1, 2, 3, 4, 5, 6]);
        // Palette stored as BGR: entry 8 => b=8, g=4, r=2.
        assert_eq!(img.palette[8], [2, 4, 8]);
    }

    #[test]
    fn raw_top_down() {
        let bits = [1, 2, 3, 0, 4, 5, 6, 0];
        let img = decode(&build_bmp(3, -2, 0, &bits)).unwrap();
        assert_eq!(img.pixels, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn rle8_runs_absolute_delta_and_eol() {
        // Row 0 (bottom): run of 2x7, absolute [8,9,10] (padded), EOL.
        // Row 1: delta (+1,0) then run 1x5, end of bitmap.
        let bits = [2, 7, 0, 3, 8, 9, 10, 0, 0, 0, 0, 2, 1, 0, 1, 5, 0, 1];
        let img = decode(&build_bmp(5, 2, 1, &bits)).unwrap();
        assert_eq!(img.pixels, vec![0, 5, 0, 0, 0, 7, 7, 8, 9, 10]);
    }

    #[test]
    fn rgba_black_is_transparent() {
        let img = IndexedImage { width: 2, height: 1, pixels: vec![0, 1], palette: vec![[0, 0, 0], [9, 8, 7]] };
        assert_eq!(img.to_rgba(true), vec![0, 0, 0, 0, 9, 8, 7, 255]);
        assert_eq!(img.to_rgba(false)[3], 255);
    }
}
