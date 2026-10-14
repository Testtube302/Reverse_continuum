//! Checks decoders against a real Continuum install. Skipped unless AICONTINUUM_ASSETS is set.

use aicontinuum::assets::{ASSETS_ENV, AssetDir};
use aicontinuum::lvl::Map;

fn assets() -> Option<AssetDir> {
    std::env::var(ASSETS_ENV).ok()?;
    Some(AssetDir::locate(None).expect("AICONTINUUM_ASSETS does not contain graphics/"))
}

#[test]
fn decodes_ship_sheet_uncompressed() {
    let Some(a) = assets() else { return };
    let img = a.bm2("ships").unwrap();
    assert_eq!((img.width, img.height), (360, 1152));
    // 8 ships x 40 rotations of 36x36 frames.
    assert_eq!((img.width / 36) * (img.height / 36), 8 * 40);
    assert!(img.pixels.iter().any(|&p| p != 0));
}

#[test]
fn decodes_rle8_tileset() {
    let Some(a) = assets() else { return };
    let img = a.bm2("tiles").unwrap();
    assert_eq!((img.width, img.height), (304, 160));
    assert_eq!(img.pixels.len(), 304 * 160);
    // Tile 1 (top-left 16x16) must contain drawn pixels.
    let lit = (0..16).flat_map(|y| (0..16).map(move |x| y * 304 + x)).filter(|&i| img.pixels[i] != 0).count();
    assert!(lit > 0);
}

#[test]
fn decodes_every_bm2() {
    let Some(a) = assets() else { return };
    let mut n = 0;
    for e in std::fs::read_dir(a.path("graphics")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bm2")) {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            a.bm2(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
            n += 1;
        }
    }
    assert!(n >= 3);
}

#[test]
fn loads_sample_map() {
    let Some(a) = assets() else { return };
    let map = Map::parse(&a.read("server/Zone/_bzw.lvl").unwrap()).unwrap();
    assert!(map.tileset.is_none());
    assert_eq!(map.records.len(), 123612 / 4);
    // First records in the file: (0,0)=0x12, (1,0)=0x02, (2,0)=0x96.
    assert_eq!([map.tile(0, 0), map.tile(1, 0), map.tile(2, 0)], [0x12, 0x02, 0x96]);
    assert!(map.is_solid(-1, 0) && map.is_solid(0, 1024));
}
