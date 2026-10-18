//! SubSpace `.lvl` map loader.
//!
//! Layout: an optional BMP tileset (whose BITMAPFILEHEADER size field marks where tile
//! data begins), followed by 4-byte little-endian tile records: x in bits 0-11, y in bits
//! 12-23, tile id in bits 24-31. The map is a 1024x1024 grid of 16x16 pixel tiles.

use crate::bmp::{self, RgbImage};

pub const MAP_SIZE: usize = 1024;
pub const TILE_PIXELS: u32 = 16;

pub const FIRST_DOOR: u8 = 162;
pub const LAST_DOOR: u8 = 169;
pub const TILE_FLAG: u8 = 170;
pub const TILE_SAFE: u8 = 171;
pub const TILE_GOAL: u8 = 172;
pub const TILE_WORMHOLE: u8 = 220;
pub const TILE_BRICK: u8 = 250;
/// Id reported for coordinates outside the map (treated as a wall).
pub const OUT_OF_BOUNDS: u8 = 20;

/// Multi-tile objects whose single record covers an NxN area, as (id, size).
pub const LARGE_OBJECTS: [(u8, usize); 3] = [(217, 2), (219, 6), (220, 5)];

pub fn tile_is_solid(id: u8) -> bool {
    match id {
        0 => false,
        1..=169 => true,
        TILE_WORMHOLE => false,
        192..=240 => true,
        242..=252 => true,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRecord {
    pub x: u16,
    pub y: u16,
    pub id: u8,
}

impl TileRecord {
    pub fn from_u32(v: u32) -> Self {
        Self { x: (v & 0xFFF) as u16, y: ((v >> 12) & 0xFFF) as u16, id: (v >> 24) as u8 }
    }
}

#[derive(Clone)]
pub struct Map {
    tiles: Vec<u8>,
    /// Tile records exactly as stored in the file.
    pub records: Vec<TileRecord>,
    pub doors: Vec<TileRecord>,
    /// Tileset embedded in the map, if any (otherwise `graphics/tiles.bm2` is used).
    pub tileset: Option<RgbImage>,
}

impl Map {
    pub fn empty() -> Self {
        Self { tiles: vec![0; MAP_SIZE * MAP_SIZE], records: Vec::new(), doors: Vec::new(), tileset: None }
    }

    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let mut map = Self::empty();
        let mut pos = 0usize;
        if let Some(size) = bmp::file_size_field(data) {
            pos = size as usize;
            if pos > data.len() {
                return Err(format!("tileset size {pos} exceeds file length {}", data.len()));
            }
            map.tileset = Some(bmp::decode_rgb(&data[..pos]).map_err(|e| e.to_string())?);
        }

        for chunk in data[pos..].chunks_exact(4) {
            let rec = TileRecord::from_u32(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            if rec.x as usize >= MAP_SIZE || rec.y as usize >= MAP_SIZE {
                continue;
            }
            let size = LARGE_OBJECTS.iter().find(|(id, _)| *id == rec.id).map_or(1, |(_, s)| *s);
            for dy in 0..size {
                for dx in 0..size {
                    map.set(rec.x as usize + dx, rec.y as usize + dy, rec.id);
                }
            }
            if (FIRST_DOOR..=LAST_DOOR).contains(&rec.id) {
                map.doors.push(rec);
            }
            map.records.push(rec);
        }
        Ok(map)
    }

    fn set(&mut self, x: usize, y: usize, id: u8) {
        if x < MAP_SIZE && y < MAP_SIZE {
            self.tiles[y * MAP_SIZE + x] = id;
        }
    }

    /// Changes one tile (bricks); out-of-map coordinates are ignored.
    pub fn set_tile(&mut self, x: i32, y: i32, id: u8) {
        if x >= 0 && y >= 0 && (x as usize) < MAP_SIZE && (y as usize) < MAP_SIZE {
            self.tiles[y as usize * MAP_SIZE + x as usize] = id;
        }
    }

    pub fn tile(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as usize >= MAP_SIZE || y as usize >= MAP_SIZE {
            return OUT_OF_BOUNDS;
        }
        self.tiles[y as usize * MAP_SIZE + x as usize]
    }

    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        tile_is_solid(self.tile(x, y))
    }

    /// Sets every door from an 8-bit pattern: door type 162+n is closed when bit n is set,
    /// otherwise it becomes tile 170 (open, passable, not drawn).
    pub fn seed_doors(&mut self, seed: u8) {
        let b = seed;
        let table: [u8; 8] = [
            if b & 1 != 0 { 0xA2 } else { 0xAA },
            if b & 2 != 0 { 0xA3 } else { 0xAA },
            if b & 4 != 0 { 0xA4 } else { 0xAA },
            if b & 8 != 0 { 0xA5 } else { 0xAA },
            if b & 0x10 != 0 { 0xA6 } else { 0xAA },
            if b & 0x20 != 0 { 0xA7 } else { 0xAA },
            if b & 0x40 != 0 { 0xA8 } else { 0xAA },
            if b & 0x80 != 0 { 0xA9 } else { 0xAA },
        ];
        for i in 0..self.doors.len() {
            let d = self.doors[i];
            self.set(d.x as usize, d.y as usize, table[(d.id - FIRST_DOOR) as usize]);
        }
    }

    /// Security checksum: samples every 31st column of every 32nd row starting at offsets
    /// derived from the key, summing `key ^ tile` for wall tiles (1..=160) and safe zones.
    /// Bricks count as empty. Coordinates wrap like the reference's 16-bit casts.
    pub fn checksum(&self, key: u32) -> u32 {
        let base = key as i32;
        let mut sum = key;
        let mut y = base % 32;
        while y < 1024 {
            let mut x = base % 31;
            while x < 1024 {
                let mut tile = self.tile(x as u16 as i32, y as u16 as i32);
                if tile == TILE_BRICK {
                    tile = 0;
                }
                if (1..=160).contains(&tile) || tile == TILE_SAFE {
                    sum = sum.wrapping_add((base ^ tile as i32) as u32);
                }
                x += 31;
            }
            y += 32;
        }
        sum
    }

    /// Whether a ship of `radius` (in tiles) centred at (x, y) overlaps no solid tile.
    pub fn can_fit(&self, x: f32, y: f32, radius: f32) -> bool {
        let mut oy = -radius;
        while oy < radius {
            let mut ox = -radius;
            while ox < radius {
                if self.is_solid((x + ox) as i32, (y + oy) as i32) {
                    return false;
                }
                ox += 1.0;
            }
            oy += 1.0;
        }
        true
    }
}

/// Standard (zlib) CRC-32, used to verify downloaded map files.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doors_open_and_close_by_pattern_bit() {
        let mut d = Vec::new();
        d.extend_from_slice(&record(10, 10, 162));
        d.extend_from_slice(&record(11, 10, 169));
        let mut map = Map::parse(&d).unwrap();
        map.seed_doors(0x01);
        assert_eq!((map.tile(10, 10), map.tile(11, 10)), (162, 170));
        assert!(map.is_solid(10, 10) && !map.is_solid(11, 10));
        map.seed_doors(0x80);
        assert_eq!((map.tile(10, 10), map.tile(11, 10)), (170, 169));
    }

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn checksum_counts_sampled_walls() {
        let empty = Map::empty();
        assert_eq!(empty.checksum(0), 0);
        // Key 0 samples (0,0), (31,0), ...; a wall at (31,32) adds 0 ^ id.
        let mut d = Vec::new();
        d.extend_from_slice(&record(31, 32, 5));
        d.extend_from_slice(&record(30, 32, 7)); // not sampled
        let map = Map::parse(&d).unwrap();
        assert_eq!(map.checksum(0), 5);
        // Key 1 starts at x=1,y=1: (1,1) sampled.
        let map = Map::parse(&record(1, 1, 9)).unwrap();
        assert_eq!(map.checksum(1), 1 + (1 ^ 9));
    }

    fn record(x: u32, y: u32, id: u32) -> [u8; 4] {
        (x | (y << 12) | (id << 24)).to_le_bytes()
    }

    #[test]
    fn decodes_bitfields() {
        let r = TileRecord::from_u32(u32::from_le_bytes([0x01, 0x00, 0x00, 0x02]));
        assert_eq!(r, TileRecord { x: 1, y: 0, id: 2 });
        assert_eq!(TileRecord::from_u32(0xAB_456_123), TileRecord { x: 0x123, y: 0x456, id: 0xAB });
    }

    #[test]
    fn parses_records_doors_and_large_objects() {
        let mut d = Vec::new();
        d.extend_from_slice(&record(5, 6, 1));
        d.extend_from_slice(&record(10, 10, 163));
        d.extend_from_slice(&record(100, 100, 219));
        let map = Map::parse(&d).unwrap();
        assert_eq!(map.tile(5, 6), 1);
        assert_eq!(map.doors.len(), 1);
        assert_eq!(map.tile(105, 105), 219);
        assert_eq!(map.tile(106, 105), 0);
        assert!(map.tileset.is_none());
    }

    #[test]
    fn out_of_bounds_is_solid() {
        let map = Map::empty();
        assert_eq!(map.tile(-1, 0), OUT_OF_BOUNDS);
        assert!(map.is_solid(1024, 5));
        assert!(!map.is_solid(512, 512));
    }

    #[test]
    fn solidity_rules() {
        assert!(!tile_is_solid(0));
        assert!(tile_is_solid(1) && tile_is_solid(162) && tile_is_solid(169));
        assert!(!tile_is_solid(170) && !tile_is_solid(TILE_SAFE) && !tile_is_solid(TILE_GOAL));
        assert!(!tile_is_solid(TILE_WORMHOLE) && tile_is_solid(216) && tile_is_solid(TILE_BRICK));
        assert!(!tile_is_solid(241) && !tile_is_solid(253));
    }
}
