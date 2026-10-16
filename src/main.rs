use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

mod hud;
mod menu;

use aicontinuum::assets::AssetDir;
use aicontinuum::game::{EffectKind, Game};
use aicontinuum::keys::{self, Key, KeySet, MacroSet};
use aicontinuum::launcher::Action;
use aicontinuum::lvl::{Map, TILE_PIXELS};
use aicontinuum::net::session::{Config, Phase, SPECTATOR, Session};
use aicontinuum::settings::ArenaSettings;
use aicontinuum::sim::{Input, TICK_SECONDS, Vec2};
use aicontinuum::weapons::{Weapon, WeaponKind};
use macroquad::audio::{PlaySoundParams, Sound, load_sound_from_bytes, play_sound};
use macroquad::prelude::*;

const TILESET_COLUMNS: u32 = 19;
const TILESET_COUNT: u8 = 190;
/// Sounds fade out over this many tiles from the camera.
const HEARING_RANGE: f32 = 50.0;

struct Args {
    assets: Option<PathBuf>,
    map: Option<PathBuf>,
    connect: Option<SocketAddr>,
    continuum_expander: Option<PathBuf>,
    name: String,
    password: String,
    arena: Option<String>,
    /// Ship to request once in the arena (online) or to start in (offline).
    ship: Option<u8>,
    /// Render this many frames (after joining, when online), save a PNG and exit.
    screenshot: Option<(PathBuf, u32)>,
    /// Scripted input for screenshot runs: ticks of forward thrust.
    script_thrust_ticks: u32,
    /// Scripted input for screenshot runs: hold the gun and bomb keys.
    script_fire: bool,
    /// Scripted input: fire a rocket on the first scripted tick (one is granted).
    script_rocket: bool,
    mute: bool,
    /// Offline: start the ship at this tile position.
    at: Option<Vec2>,
    /// Start with the Esc menu open (for screenshots).
    menu: bool,
    /// Start with this F2 stat box view shown (0-5).
    statbox: Option<u8>,
    /// Start with this F1 help page open.
    help: Option<usize>,
    /// Spectate following this player's ship.
    follow: Option<String>,
    /// Ask for this team after getting the first ship (online).
    freq: Option<u16>,
    /// Test aid: attach to this player as a turret ~5 s after getting a ship (like F7).
    attach_to: Option<String>,
    /// Test aid: press warp (throws a carried ball) once on this frame.
    throw_after: Option<u32>,
    /// Test aid: once we carry a ball, put our ship at (x, y) facing direction d, then throw.
    carry_to: Option<(f32, f32, u8)>,
    /// Startup-screen test aids: a fixed mouse position, the right button held, a dialog to open.
    mouse: Option<(i32, i32)>,
    right_held: bool,
    open: Option<String>,
    autoplay: bool,
    /// Startup-screen test aid: left clicks at these points, one every 15 frames from frame 30.
    clicks: Vec<(i32, i32)>,
    /// Key definition and macro set names (the profile's keyboard / macros).
    keyset: String,
    macroset: String,
    /// Game test aid: act as if a key definition was pressed at a frame ("Macro1@300").
    press: Vec<(String, u32)>,
    /// Startup-screen test aid: typed after the clicks, one character a frame (\t Tab, \n Enter).
    typed: String,
}

fn parse_args() -> Args {
    let mut args = Args {
        assets: None,
        map: None,
        connect: None,
        continuum_expander: None,
        name: "AIContinuum".into(),
        password: "aicontinuum".into(),
        arena: None,
        ship: None,
        screenshot: None,
        script_thrust_ticks: 0,
        script_fire: false,
        script_rocket: false,
        mute: false,
        at: None,
        menu: false,
        statbox: None,
        help: None,
        follow: None,
        freq: None,
        attach_to: None,
        throw_after: None,
        carry_to: None,
        mouse: None,
        right_held: false,
        open: None,
        autoplay: false,
        clicks: Vec::new(),
        typed: String::new(),
        keyset: "Default".into(),
        press: Vec::new(),
        macroset: "Default".into(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--assets" => args.assets = Some(PathBuf::from(value())),
            "--map" => args.map = Some(PathBuf::from(value())),
            "--connect" => {
                let v = value();
                let addr = v.to_socket_addrs().ok().and_then(|mut a| a.next());
                args.connect = Some(addr.unwrap_or_else(|| panic!("cannot resolve {v}")));
            }
            "--continuum-expander" => args.continuum_expander = Some(PathBuf::from(value())),
            "--name" => args.name = value(),
            "--password" => args.password = value(),
            "--arena" => args.arena = Some(value()),
            "--ship" => args.ship = value().parse().ok().filter(|s| *s < 8),
            "--screenshot" => args.screenshot = Some((PathBuf::from(value()), 3)),
            "--frames" => {
                let n = value().parse().expect("--frames <n>");
                if let Some(s) = args.screenshot.as_mut() {
                    s.1 = n;
                }
            }
            "--thrust-ticks" => args.script_thrust_ticks = value().parse().unwrap_or(0),
            "--script-fire" => args.script_fire = true,
            "--script-rocket" => args.script_rocket = true,
            "--mute" => args.mute = true,
            "--fullscreen" | "--offline" | "--launcher" => {}
            "--mouse" => {
                let v: Vec<i32> = value().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x, y] = v[..] {
                    args.mouse = Some((x, y));
                }
            }
            "--right" => args.right_held = true,
            "--autoplay" => args.autoplay = true,
            "--keys" => args.keyset = value(),
            "--press" => {
                let v = value();
                if let Some((a, f)) = v.split_once('@') {
                    args.press.push((a.to_string(), f.parse().unwrap_or(0)));
                }
            }
            "--macros" => args.macroset = value(),
            "--type" => args.typed = value().replace("\\t", "\t").replace("\\n", "\n").replace("\\b", "\u{8}"),
            "--clicks" => {
                args.clicks = value()
                    .split(';')
                    .filter_map(|p| {
                        let (x, y) = p.split_once(',')?;
                        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                    })
                    .collect()
            }
            "--open" => args.open = Some(value()),
            "--menu" => args.menu = true,
            "--statbox" => args.statbox = Some(value().parse().unwrap_or(0)),
            "--help-page" => args.help = value().parse().ok(),
            "--follow" => args.follow = Some(value()),
            "--freq" => args.freq = value().parse().ok(),
            "--attach-to" => args.attach_to = Some(value()),
            "--throw-after" => args.throw_after = value().parse().ok(),
            "--carry-to" => {
                let v: Vec<f32> = value().split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x, y, d] = v[..] {
                    args.carry_to = Some((x, y, d as u8));
                }
            }
            "--at" => {
                let v = value();
                let (x, y) = v.split_once(',').unwrap_or_else(|| panic!("--at x,y"));
                args.at = Some(Vec2::new(x.trim().parse().expect("x"), y.trim().parse().expect("y")));
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    args
}

fn window_conf() -> Conf {
    // Read before macroquad starts: --fullscreen gives an exact 1024x768 surface without decorations.
    let fullscreen = std::env::args().any(|a| a == "--fullscreen");
    // The startup screen opens at the default skin's size; the game resizes to 1024x768.
    let (window_width, window_height) = if launcher_mode() { (640, 496) } else { (1024, 768) };
    Conf { window_title: "AIContinuum".into(), window_width, window_height, fullscreen: fullscreen && !launcher_mode(), ..Default::default() }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// A sprite sheet of equal-sized frames, numbered left to right, top to bottom.
struct Sheet {
    tex: Texture2D,
    frame_w: f32,
    frame_h: f32,
    columns: u32,
    /// Frames up to the last one with any visible pixel (sheets can end in blank cells).
    frames: u32,
}

impl Sheet {
    fn load(assets: &AssetDir, name: &str, frame_w: u32, frame_h: u32) -> Self {
        let img = assets.graphic(name).unwrap_or_else(|e| die(e));
        let tex = Texture2D::from_rgba8(img.width as u16, img.height as u16, &img.pixels);
        tex.set_filter(FilterMode::Nearest);
        let columns = (img.width / frame_w).max(1);
        let cells = columns * (img.height / frame_h);
        let visible = |f: u32| {
            let (x0, y0) = ((f % columns) * frame_w, (f / columns) * frame_h);
            (y0..y0 + frame_h).any(|y| {
                (x0..x0 + frame_w).any(|x| img.pixels[((y * img.width + x) * 4 + 3) as usize] != 0)
            })
        };
        let frames = (0..cells).rev().find(|&f| visible(f)).map_or(cells, |f| f + 1);
        Self { tex, frame_w: frame_w as f32, frame_h: frame_h as f32, columns, frames }
    }

    fn frame_count(&self) -> u32 {
        self.frames
    }

    /// Draws `frame` centred on the screen point (x, y).
    fn draw_centered(&self, frame: u32, x: f32, y: f32) {
        self.draw(frame, (x - self.frame_w / 2.0).floor(), (y - self.frame_h / 2.0).floor());
    }

    fn draw(&self, frame: u32, x: f32, y: f32) {
        let src = Rect::new(
            (frame % self.columns) as f32 * self.frame_w,
            (frame / self.columns) as f32 * self.frame_h,
            self.frame_w,
            self.frame_h,
        );
        draw_texture_ex(&self.tex, x, y, WHITE, DrawTextureParams { source: Some(src), ..Default::default() });
    }
}

struct Graphics {
    ships: Sheet,
    bullets: Sheet,
    bombs: Sheet,
    mines: Sheet,
    shrapnel: Sheet,
    explode0: Sheet,
    explode1: Sheet,
    explode2: Sheet,
    empburst: Sheet,
    warp: Sheet,
    repel: Sheet,
    spark: Sheet,
    turret: Sheet,
    ball: Sheet,
    exhaust: Sheet,
    rocket: Sheet,
    portal: Sheet,
    bricks: Sheet,
    trail: Sheet,
    /// Ship debris, one sheet per ship type: 9 pieces (rows) x 10 rotations.
    junk: Vec<Sheet>,
    prizes: Sheet,
    flags: Sheet,
    goal: Sheet,
    over1: Sheet,
    over2: Sheet,
    over3: Sheet,
    over4: Sheet,
    over5: Sheet,
}

impl Graphics {
    fn load(assets: &AssetDir) -> Self {
        Self {
            ships: Sheet::load(assets, "ships", 36, 36),
            bullets: Sheet::load(assets, "bullets", 5, 5),
            bombs: Sheet::load(assets, "bombs", 16, 16),
            mines: Sheet::load(assets, "mines", 16, 16),
            shrapnel: Sheet::load(assets, "shrapnel", 8, 8),
            explode0: Sheet::load(assets, "explode0", 16, 16),
            explode1: Sheet::load(assets, "explode1", 48, 48),
            explode2: Sheet::load(assets, "explode2", 80, 80),
            empburst: Sheet::load(assets, "empburst", 80, 80),
            warp: Sheet::load(assets, "warp", 48, 48),
            repel: Sheet::load(assets, "repel", 96, 96),
            spark: Sheet::load(assets, "spark", 40, 40),
            turret: Sheet::load(assets, "turret2", 16, 16),
            ball: Sheet::load(assets, "powerb", 16, 16),
            exhaust: Sheet::load(assets, "exhaust", 16, 16),
            rocket: Sheet::load(assets, "rocket", 24, 24),
            portal: Sheet::load(assets, "warppnt", 16, 16),
            bricks: Sheet::load(assets, "wall", 16, 16),
            trail: Sheet::load(assets, "trail", 16, 16),
            junk: ["wb", "jv", "sp", "lv", "te", "we", "nw", "sh"]
                .iter()
                .map(|s| Sheet::load(assets, &format!("junk{s}"), 16, 16))
                .collect(),
            prizes: Sheet::load(assets, "prizes", 16, 16),
            flags: Sheet::load(assets, "flag", 16, 16),
            goal: Sheet::load(assets, "goal", 16, 16),
            over1: Sheet::load(assets, "over1", 16, 16),
            over2: Sheet::load(assets, "over2", 32, 32),
            over3: Sheet::load(assets, "over3", 16, 16),
            over4: Sheet::load(assets, "over4", 96, 96),
            over5: Sheet::load(assets, "over5", 80, 80),
        }
    }
}

struct Sounds {
    clips: HashMap<String, Option<Sound>>,
    muted: bool,
}

impl Sounds {
    async fn play(&mut self, assets: &AssetDir, name: &str, volume: f32) {
        if self.muted || volume <= 0.0 {
            return;
        }
        if !self.clips.contains_key(name) {
            let clip = match assets.sound(name) {
                Ok(bytes) => load_sound_from_bytes(&bytes).await.ok(),
                Err(_) => None,
            };
            self.clips.insert(name.to_string(), clip);
        }
        if let Some(Some(s)) = self.clips.get(name) {
            play_sound(s, PlaySoundParams { looped: false, volume });
        }
    }
}

/// The map tileset twice: opaque (ids up to 172, black drawn black) and with black transparent
/// (ids above 172), matching how the original draws each range.
struct Tiles {
    opaque: Texture2D,
    keyed: Texture2D,
}

impl Tiles {
    fn for_map(map: &Map, assets: &AssetDir) -> Self {
        let (w, h, mut rgba) = match &map.tileset {
            Some(t) => (t.width, t.height, t.to_rgba(false)),
            None => {
                let img = assets.graphic("tiles").unwrap_or_else(|e| die(e));
                (img.width, img.height, img.pixels)
            }
        };
        let tex = |px: &[u8]| {
            let t = Texture2D::from_rgba8(w as u16, h as u16, px);
            t.set_filter(FilterMode::Nearest);
            t
        };
        let keyed = tex(&rgba);
        for p in rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
        Self { opaque: tex(&rgba), keyed }
    }

    fn draw(&self, id: u8, x: f32, y: f32) {
        let idx = (id - 1) as u32;
        let src = Rect::new(
            ((idx % TILESET_COLUMNS) * TILE_PIXELS) as f32,
            ((idx / TILESET_COLUMNS) * TILE_PIXELS) as f32,
            TILE_PIXELS as f32,
            TILE_PIXELS as f32,
        );
        let tex = if id > 172 { &self.keyed } else { &self.opaque };
        draw_texture_ex(tex, x, y, WHITE, DrawTextureParams { source: Some(src), ..Default::default() });
    }
}

/// The tile drawn at (tx, ty). Outside the map the original shows only a one-tile border wall
/// ring (tile 20), not a solid fill, even though everything outside is solid for physics.
fn visible_tile(map: &Map, tx: i32, ty: i32) -> u8 {
    let inside = |v: i32| (0..1024).contains(&v);
    if inside(tx) && inside(ty) {
        return map.tile(tx, ty);
    }
    let ring = |v: i32| v == -1 || v == 1024;
    let span = |v: i32| (-1..=1024).contains(&v);
    if (ring(tx) && span(ty)) || (ring(ty) && span(tx)) { 20 } else { 0 }
}

/// Fly-over tiles are drawn above ships.
fn is_fly_over(id: u8) -> bool {
    (176..=190).contains(&id)
}

/// Tiles drawn from the tileset (flags and goals are animated sprites instead).
fn is_tileset_tile(id: u8) -> bool {
    id != 0 && id != 170 && id != 172 && id <= TILESET_COUNT
}

/// Two parallax layers of single-pixel stars: far (dim, 1/4 speed) and near (bright, 1/2 speed).
/// Star placement is our own deterministic hash; the original's star layout can't be reproduced.
fn draw_stars(cam_px: (f32, f32), sw: f32, sh: f32) {
    const CELL: i64 = 1024;
    const PER_CELL: u64 = 100;
    for (speed, grey, layer) in [(0.25f32, 0x60u8, 1u64), (0.5, 0xB8, 0)] {
        let color = Color::from_rgba(grey, grey, grey, 255);
        let (ox, oy) = ((cam_px.0 * speed) as i64, (cam_px.1 * speed) as i64);
        for cy in (oy.div_euclid(CELL))..=((oy + sh as i64).div_euclid(CELL)) {
            for cx in (ox.div_euclid(CELL))..=((ox + sw as i64).div_euclid(CELL)) {
                let mut h = (cx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (cy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ layer;
                for _ in 0..PER_CELL {
                    h ^= h >> 33;
                    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
                    h ^= h >> 29;
                    let (sx, sy) = ((h % CELL as u64) as i64, ((h >> 20) % CELL as u64) as i64);
                    let x = cx * CELL + sx - ox;
                    let y = cy * CELL + sy - oy;
                    if x >= 0 && y >= 0 && (x as f32) < sw && (y as f32) < sh {
                        draw_rectangle(x as f32, y as f32, 1.0, 1.0, color);
                    }
                }
            }
        }
    }
}

/// Whether a key named in keyboard.dat is held.
fn key_down(k: Key) -> bool {
    use KeyCode as K;
    let any = |codes: &[KeyCode]| codes.iter().any(|c| is_key_down(*c));
    match k {
        Key::Shift => any(&[K::LeftShift, K::RightShift]),
        Key::Ctrl => any(&[K::LeftControl, K::RightControl]),
        Key::Alt => any(&[K::LeftAlt, K::RightAlt]),
        Key::Left => is_key_down(K::Left),
        Key::Right => is_key_down(K::Right),
        Key::Up => is_key_down(K::Up),
        Key::Down => is_key_down(K::Down),
        Key::Tab => is_key_down(K::Tab),
        Key::Space => is_key_down(K::Space),
        Key::Enter => any(&[K::Enter, K::KpEnter]),
        Key::Backspace => is_key_down(K::Backspace),
        Key::Insert => is_key_down(K::Insert),
        Key::Delete => is_key_down(K::Delete),
        Key::Home => is_key_down(K::Home),
        Key::End => is_key_down(K::End),
        Key::PgUp => is_key_down(K::PageUp),
        Key::PgDown => is_key_down(K::PageDown),
        Key::F(n) => key_code_f(n).is_some_and(is_key_down),
        Key::Char(c) => key_code_char(c).is_some_and(is_key_down),
    }
}

pub fn key_code_f(n: u8) -> Option<KeyCode> {
    use KeyCode as K;
    [K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11, K::F12].get((n as usize).checked_sub(1)?).copied()
}

pub fn key_code_char(c: char) -> Option<KeyCode> {
    use KeyCode as K;
    let letters = [
        K::A, K::B, K::C, K::D, K::E, K::F, K::G, K::H, K::I, K::J, K::K, K::L, K::M, K::N, K::O, K::P, K::Q, K::R, K::S, K::T, K::U,
        K::V, K::W, K::X, K::Y, K::Z,
    ];
    let digits = [K::Key0, K::Key1, K::Key2, K::Key3, K::Key4, K::Key5, K::Key6, K::Key7, K::Key8, K::Key9];
    match c.to_ascii_uppercase() {
        l @ 'A'..='Z' => Some(letters[(l as u8 - b'A') as usize]),
        d @ '0'..='9' => Some(digits[(d as u8 - b'0') as usize]),
        _ => None,
    }
}

/// Ship controls from the held key definitions.
fn read_input(active: &[&str]) -> Input {
    let on = |a: &str| active.contains(&a);
    Input {
        left: on("TurnLeft"),
        right: on("TurnRight"),
        forward: on("Forward"),
        backward: on("Reverse"),
        afterburner: on("Afterburners"),
        gun: on("Bullet"),
        bomb: on("Bomb"),
        mine: on("Mine"),
        repel: on("Repel"),
        burst: on("Burst"),
        decoy: on("Decoy"),
        thor: on("Thor"),
        rocket: on("Rocket"),
        brick: on("Brick"),
        portal: on("Portal"),
        warp: on("Warp"),
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let args = parse_args();
    let assets = AssetDir::locate(args.assets.as_deref()).unwrap_or_else(|e| die(e));
    if !launcher_mode() {
        return run_game(args, &assets).await;
    }
    let mut screen = menu::MenuScreen::load(&assets).unwrap_or_else(|e| die(e));
    loop {
        match screen.run(&args).await {
            Some(Action::Play { addr, name, password, ship, fullscreen, resolution, keys, macros, .. }) => {
                let mut game_args = parse_args();
                game_args.keyset = keys;
                game_args.macroset = macros;
                game_args.connect = Some(addr);
                game_args.name = name;
                game_args.password = password;
                game_args.ship = (ship < 8).then_some(ship);
                // The profile's Resolution / Windowed choices, as in the original.
                if fullscreen && args.screenshot.is_none() {
                    set_fullscreen(true);
                } else {
                    request_new_screen_size(resolution.0 as f32, resolution.1 as f32);
                }
                next_frame().await;
                run_game(game_args, &assets).await;
                set_fullscreen(false);
                if args.screenshot.is_some() {
                    break;
                }
            }
            _ => break,
        }
    }
}

/// The startup screen comes first unless the command line already says what to play.
fn launcher_mode() -> bool {
    let a: Vec<String> = std::env::args().collect();
    a.iter().any(|a| a == "--launcher") || !a.iter().any(|a| ["--connect", "--map", "--offline", "--screenshot"].contains(&a.as_str()))
}

async fn run_game(mut args: Args, assets: &AssetDir) {
    let assets = assets;
    let gfx = Graphics::load(&assets);
    let mut hud = hud::Hud::load(&assets);
    hud.statbox = args.statbox;
    let mut sounds = Sounds { clips: HashMap::new(), muted: args.mute || args.screenshot.is_some() };

    let mut game = match args.connect {
        Some(server) => {
            let zone_dir = format!("{}_{}", server.ip(), server.port()).replace([':', '.'], "_");
            let config = Config {
                server,
                name: args.name.clone(),
                password: args.password.clone(),
                arena: args.arena.clone(),
                map_dir: assets.path("zones").join(zone_dir),
            };
            let session = if let Some(program) = &args.continuum_expander {
                let mut expander = aicontinuum::net::continuum_expander::CommandExpander::new(program.clone());
                Session::connect_continuum(config, move |seed| match expander.expand(seed) {
                    Ok(table) => Some(table),
                    Err(error) => { eprintln!("Continuum expansion: {error}"); None }
                }, [0; 64])
            } else {
                Session::connect(config)
            };
            Game::online(session.unwrap_or_else(|e| die(format!("connect {server}: {e}"))))
        }
        None => {
            let map_path = args.map.clone().or_else(|| {
                let p = assets.path("server/Zone/_bzw.lvl");
                p.is_file().then_some(p)
            });
            let map = match &map_path {
                Some(p) => std::fs::read(p).map_err(|e| e.to_string()).and_then(|d| Map::parse(&d)),
                None => Ok(Map::empty()),
            };
            let mut g = Game::offline(map.unwrap_or_else(|e| die(e)), ArenaSettings::default(), args.ship.unwrap_or(0));
            if let (Some(at), Some(ship)) = (args.at, g.ship.as_mut()) {
                ship.position = at;
                g.camera = at;
            }
            g
        }
    };

    game.follow = args.follow.clone();
    if let (Some(at), true) = (args.at, game.session.is_some()) {
        game.camera = at; // online: initial spectator camera
    }
    let mut tiles: Option<Tiles> = None;
    let mut requested_initial_ship = false;
    let mut placed_at = false;
    // Test aid (--carry-to): leave the ship after the throw so we don't catch the ball again.
    let mut spectate_at_frame: Option<u32> = None;
    let mut accumulator = 0.0f32;
    let mut frame = 0u32;
    let mut scripted_ticks = 0u32;
    let mut fire_held = false;
    let mut menu_open = args.menu;
    let mut help_page: Option<usize> = args.help;
    // The profile's key definitions and macros from keyboard.dat / macro.dat.
    let keyset = assets
        .read("keyboard.dat")
        .map(|b| keys::parse_keysets(&String::from_utf8_lossy(&b)))
        .unwrap_or_default()
        .into_iter()
        .find(|k| k.name == args.keyset)
        .unwrap_or_else(|| KeySet::defaults("Default"));
    let macros = assets
        .read("macro.dat")
        .map(|b| keys::parse_macros(&String::from_utf8_lossy(&b)))
        .unwrap_or_default()
        .into_iter()
        .find(|m| m.name == args.macroset)
        .unwrap_or_else(|| MacroSet::empty("Default"));
    let mut held_actions: Vec<String> = Vec::new();
    let mut last_ship: u8 = args.ship.unwrap_or(0);

    loop {
        // Chat entry: Enter opens/sends, Esc cancels; while typing, keys go to the message.
        let typing = hud.chat_input.is_some();
        // Esc menu: Q quits, 1-8/S pick a ship, any other key resumes.
        if menu_open {
            if let Some(key) = get_last_key_pressed() {
                menu_open = false;
                let ships = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];
                if key == KeyCode::Q {
                    break;
                } else if let Some(i) = ships.iter().position(|k| *k == key) {
                    game.request_ship(i as u8);
                } else if key == KeyCode::S {
                    game.request_ship(SPECTATOR);
                }
            }
            while get_char_pressed().is_some() {}
        } else if is_key_pressed(KeyCode::Escape) {
            if typing {
                hud.chat_input = None;
            } else {
                menu_open = true;
                while get_char_pressed().is_some() {}
            }
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
            match hud.chat_input.take() {
                Some(text) if !text.trim().is_empty() => {
                    if let Some(s) = game.session.as_mut() {
                        s.send_public_chat(&text);
                    } else {
                        game.notices.push(text);
                    }
                }
                Some(_) => {}
                None => hud.chat_input = Some(String::new()),
            }
        }
        // Bindings held this frame (none while typing or in the Esc menu) and the newly pressed ones.
        let active: Vec<String> = if typing || menu_open {
            Vec::new()
        } else {
            keyset.active(key_down).into_iter().map(String::from).collect()
        };
        let mut pressed: Vec<String> = active.iter().filter(|a| !held_actions.contains(a)).cloned().collect();
        pressed.extend(args.press.iter().filter(|(_, f)| *f == frame).map(|(a, _)| a.clone()));
        held_actions = active.clone();
        let ctrl_or_alt = key_down(Key::Ctrl) || key_down(Key::Alt);
        while let Some(c) = get_char_pressed() {
            // Without "Require Msg Key", typing a character starts a message, as in the original.
            if hud.chat_input.is_none() && !keyset.req_msg_key && !menu_open && !ctrl_or_alt && c.is_ascii_graphic() {
                hud.chat_input = Some(String::new());
            }
            if let Some(input) = hud.chat_input.as_mut() {
                if (' '..='~').contains(&c) && input.len() < 200 && (typing || input.is_empty()) {
                    input.push(c);
                }
            }
        }
        if pressed.iter().any(|a| a == "Message") && hud.chat_input.is_none() {
            hud.chat_input = Some(String::new());
        }
        // Macros: send the line as chat.
        for a in &pressed {
            if let Some(n) = a.strip_prefix("Macro").and_then(|n| n.parse::<usize>().ok()) {
                let text = macros.lines.get(n.wrapping_sub(1)).cloned().unwrap_or_default();
                if !text.trim().is_empty() {
                    match game.session.as_mut() {
                        Some(s) => s.send_public_chat(&text),
                        None => game.notices.push(text),
                    }
                }
            }
        }
        if hud.chat_input.is_some() && is_key_pressed(KeyCode::Backspace) {
            hud.chat_input.as_mut().unwrap().pop();
        }
        let typing = hud.chat_input.is_some();
        game.frame();

        let ready = game.world().is_some();
        if ready && tiles.is_none() {
            tiles = Some(Tiles::for_map(game.world().unwrap().0, &assets));
        }
        if ready && !requested_initial_ship && game.session.is_some() {
            requested_initial_ship = true;
            if let Some(n) = args.ship {
                game.request_ship(n);
            }
        }
        if let (Some(f), Some(_)) = (args.freq, game.ship.as_ref()) {
            if let Some(s) = game.session.as_mut() {
                s.request_freq(f);
            }
            args.freq = None;
        }
        if let (Some(name), true) = (args.attach_to.clone(), game.alive() && frame > 300) {
            let pid = game.session.as_ref().and_then(|s| s.players.values().find(|p| p.name == name).map(|p| p.pid));
            if pid.is_some() {
                game.request_attach(pid);
                args.attach_to = None;
            }
        }
        // Online --at: place our first ship there (a test aid for reproducible comparisons).
        if let Some((x, y, d)) = args.carry_to {
            let me = game.session.as_ref().and_then(|s| s.pid);
            if me.is_some_and(|pid| game.soccer.carried_by(pid).is_some()) {
                if let Some(ship) = game.ship.as_mut() {
                    ship.position = Vec2::new(x, y);
                    ship.velocity = Vec2::default();
                    ship.orientation = d as f32 / 40.0;
                }
                args.carry_to = None;
                placed_at = true;
                args.throw_after = Some(frame + 300);
                spectate_at_frame = Some(frame + 400);
                eprintln!("carry-to: at ({x},{y}) dir {d}, throwing at frame {}", frame + 300);
            }
        }
        if spectate_at_frame.is_some_and(|f| frame >= f) {
            spectate_at_frame = None;
            game.request_ship(SPECTATOR);
        }
        if std::env::var_os("AIC_DEBUG_BALLS").is_some() && frame % 100 == 0 {
            for b in &game.soccer.balls {
                let at = b.position();
                eprintln!("frame {frame} ball {} at ({:.3},{:.3}) state {:?} vel ({},{})", b.id, at.x, at.y, b.state, b.vel_x, b.vel_y);
            }
        }
        // Keep it there for the first 300 frames: a zone may swap our ship (and respawn us) at first.
        if let (Some(at), Some(ship), false) = (args.at, game.ship.as_mut(), placed_at) {
            ship.position = at;
            ship.velocity = Vec2::default();
            placed_at = frame > 300;
        }
        // 1-8 / S pick a ship directly only when typing doesn't start a message.
        if keyset.req_msg_key {
            let ship_keys = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];
            if let Some(i) = ship_keys.iter().position(|k| is_key_pressed(*k)).filter(|_| !typing && !menu_open) {
                game.request_ship(i as u8);
            }
            if !typing && !menu_open && is_key_pressed(KeyCode::S) && !is_key_down(KeyCode::Tab) {
                game.request_ship(SPECTATOR);
            }
        }
        if let Some(ship) = game.ship.as_ref() {
            last_ship = ship.ship as u8;
        }
        {
            use aicontinuum::sim::{STATUS_ANTIWARP, STATUS_CLOAK, STATUS_STEALTH, STATUS_XRADAR};
            for a in &pressed {
                match a.as_str() {
                    "Stealth" => game.toggle_status(STATUS_STEALTH),
                    "Cloak" => game.toggle_status(STATUS_CLOAK),
                    "XRadar" => game.toggle_status(STATUS_XRADAR),
                    "AntiWarp" => game.toggle_status(STATUS_ANTIWARP),
                    "NextPlayer" => hud::move_selection(&mut hud, &game, true),
                    "PrevPlayer" => hud::move_selection(&mut hud, &game, false),
                    "NextPage" | "PrevPage" => {
                        for _ in 0..10 {
                            hud::move_selection(&mut hud, &game, a == "NextPage");
                        }
                    }
                    "Attach" => game.request_attach(hud.selected),
                    "PlaySpec" => game.request_ship(if game.ship.is_some() { SPECTATOR } else { last_ship }),
                    "Cycle" => {
                        // Cycles the six stat box views, then hides it.
                        hud.statbox = match hud.statbox {
                            None => Some(0),
                            Some(v) if v < 5 => Some(v + 1),
                            Some(_) => None,
                        };
                    }
                    "Multi" => {
                        if let Some(ship) = game.ship.as_mut() {
                            ship.multifire = !ship.multifire;
                            game.notices.push(format!("Multifire {}", if ship.multifire { "on" } else { "off" }));
                        }
                    }
                    _ => {}
                }
            }
        }
        if !typing && !menu_open && is_key_pressed(KeyCode::F1) {
            // F1 pages through help, closing after the last page.
            help_page = match help_page {
                None => Some(0),
                Some(p) if p + 1 < hud.help.len() => Some(p + 1),
                Some(_) => None,
            };
        }

        let mut fired_any = aicontinuum::weapons::WeaponData::default();
        if ready {
            if args.screenshot.is_some() && game.session.is_none() {
                // Deterministic offline script: a fixed number of ticks regardless of frame time.
                if args.script_rocket && scripted_ticks == 0 {
                    if let Some(ship) = game.ship.as_mut() {
                        ship.items[5] = ship.items[5].max(1);
                    }
                }
                while scripted_ticks < args.script_thrust_ticks {
                    let input = Input {
                        rocket: args.script_rocket && scripted_ticks == 0,
                        forward: !args.script_rocket,
                        gun: args.script_fire && scripted_ticks > args.script_thrust_ticks.saturating_sub(40),
                        bomb: args.script_fire && scripted_ticks == args.script_thrust_ticks.saturating_sub(60),
                        ..Default::default()
                    };
                    game.tick(input);
                    scripted_ticks += 1;
                }
            } else {
                let active_refs: Vec<&str> = held_actions.iter().map(|s| s.as_str()).collect();
                let mut input = if typing { Input::default() } else { read_input(&active_refs) };
                // Held for 20 frames: a frame can pass without a game tick at high frame rates.
                if args.throw_after.is_some_and(|f| frame >= f && frame < f + 20) {
                    input.warp = true;
                }
                if args.script_fire {
                    input.gun = true;
                    input.bomb = !fire_held;
                    fire_held = true;
                }
                accumulator += get_frame_time().min(0.25);
                while accumulator >= TICK_SECONDS {
                    let fired = game.tick(input);
                    if fired.0 != 0 {
                        fired_any = fired;
                        // Shots go out immediately so others see them with minimal delay.
                        game.send_updates(fired);
                    }
                    accumulator -= TICK_SECONDS;
                }
            }
            if fired_any.0 == 0 {
                game.send_updates(fired_any);
            }
            draw_world(&game, tiles.as_ref().unwrap(), &gfx, &mut hud);
            if let Some(view) = hud.statbox {
                hud::draw_statbox(&hud, &game, view);
            }
            if let Some(page) = help_page {
                hud::draw_help(&hud, page);
            }
            if menu_open {
                hud::draw_menu(&hud);
            }
        } else {
            clear_background(BLACK);
            let status = match game.session.as_ref().map(|s| &s.phase) {
                Some(Phase::Failed(why)) => format!("Connection failed: {why}"),
                Some(p) => format!("{p:?}... {}", game.session.as_ref().and_then(|s| s.log.last()).cloned().unwrap_or_default()),
                None => String::new(),
            };
            for (line, text) in status.lines().enumerate() {
                draw_text(text, 8.0, 16.0 + line as f32 * 18.0, 16.0, GREEN);
            }
        }

        for cue in std::mem::take(&mut game.sounds) {
            let d = cue.position.sub(game.camera).length();
            sounds.play(&assets, &cue.name, (1.0 - d / HEARING_RANGE).clamp(0.0, 1.0)).await;
        }

        if ready {
            frame += 1;
        }
        if let Some((path, frames)) = &args.screenshot {
            if frame >= *frames {
                if std::env::var_os("AIC_DEBUG_PLAYERS").is_some() {
                    if let Some(s) = &game.session {
                        for p in s.players.values() {
                            eprintln!(
                                "player {} pid={} ship={} freq={} pos=({:.1},{:.1}) updates={} dead={} attached_to={:?}",
                                p.name, p.pid, p.ship, p.freq, p.position.x, p.position.y, p.position_updates, p.dead, p.attached_to
                            );
                        }
                        for b in &game.soccer.balls {
                            let at = b.position();
                            eprintln!("ball {} at ({:.1},{:.1}) state {:?} carrier {} vel ({},{})", b.id, at.x, at.y, b.state, b.carrier, b.vel_x, b.vel_y);
                        }
                        for line in s.log.iter().filter(|l| l.contains("turret")) {
                            eprintln!("{line}");
                        }
                        eprintln!("camera=({:.1},{:.1}) spectate_at=({:.1},{:.1})", game.camera.x, game.camera.y, s.spectate_at.x, s.spectate_at.y);
                    }
                }
                get_screen_data().export_png(&path.to_string_lossy());
                println!("saved {}", path.display());
                break;
            }
        }
        next_frame().await;
    }
    if let Some(s) = game.session.as_mut() {
        s.disconnect();
    }
}

/// Animation frame for something `age` ticks old cycling through `count` frames per `period` ticks.
fn anim_frame(age: u32, count: u32, period: u32) -> u32 {
    (age % period.max(1)) * count / period.max(1)
}

fn draw_weapon(gfx: &Graphics, w: &Weapon, x: f32, y: f32) {
    let level = w.data.level() as u32;
    match w.data.kind() {
        WeaponKind::Bullet | WeaponKind::BouncingBullet if w.data.shrap() > 0 => {
            let row = w.data.shrap_level() as u32 + if w.data.kind() == WeaponKind::BouncingBullet { 3 } else { 0 };
            gfx.shrapnel.draw_centered(row * 10 + anim_frame(w.age, 10, 60), x, y);
        }
        WeaponKind::Bullet => gfx.bullets.draw_centered(level * 4 + anim_frame(w.age, 4, 15), x, y),
        WeaponKind::BouncingBullet => gfx.bullets.draw_centered(20 + level * 4 + anim_frame(w.age, 4, 15), x, y),
        WeaponKind::Bomb | WeaponKind::ProximityBomb if w.data.alternate() => {
            let row = level + if w.emp { 4 } else { 0 };
            gfx.mines.draw_centered(row * 10 + anim_frame(w.age, 10, 100), x, y);
        }
        WeaponKind::Bomb | WeaponKind::ProximityBomb => {
            let row = level + if w.emp { 4 } else if w.bounces_remaining > 0 { 8 } else { 0 };
            gfx.bombs.draw_centered(row * 10 + anim_frame(w.age, 10, 100), x, y);
        }
        WeaponKind::Thor => gfx.bombs.draw_centered(120 + anim_frame(w.age, 10, 100), x, y),
        // Burst pieces: bullets.png frames 16-19 until their first bounce, then the last four.
        WeaponKind::Burst => {
            let first = if w.burst_active { gfx.bullets.frame_count().saturating_sub(4) } else { 16 };
            gfx.bullets.draw_centered(first + anim_frame(w.age, 4, 15), x, y);
        }
        // A decoy looks like the shooter's ship as it was when fired.
        WeaponKind::Decoy => gfx.ships.draw_centered(w.decoy_ship as u32 * 40 + w.decoy_direction.min(39) as u32, x, y),
        _ => {}
    }
}

/// The team things are judged from: ours, or a spectator sees through the followed player.
fn view_freq(game: &Game) -> Option<u16> {
    let s = game.session.as_ref()?;
    let followed = game.follow.as_ref().and_then(|n| s.players.values().find(|p| &p.name == n));
    match (game.ship.is_none(), followed) {
        (true, Some(f)) => Some(f.freq),
        _ => s.me().map(|m| m.freq),
    }
}

/// Turrets riding on `carrier`: turret2.png in each turret's own direction on the carrier, and
/// their name labels stacked 12 px apart under the carrier's (reference client).
fn draw_turrets(gfx: &Graphics, hud: &mut hud::Hud, game: &Game, carrier: u16, at: (f32, f32), view_freq: Option<u16>, first_line: u32) {
    let Some(s) = &game.session else { return };
    let mut riders: Vec<_> = s.players.values().filter(|p| p.attached_to == Some(carrier) && Some(p.pid) != s.pid).collect();
    riders.sort_by_key(|p| p.pid);
    for (k, p) in riders.iter().enumerate() {
        gfx.turret.draw_centered(p.direction.min(39) as u32, at.0, at.1);
        let line = first_line + k as u32;
        let label_at = (at.0, at.1 + 12.0 * line as f32);
        hud.name_label(&p.name, p.bounty, p.flags_carried, game.soccer.carried_by(p.pid).is_some(), p.ping as u32 * 10, view_freq == Some(p.freq), None, label_at);
    }
}

/// Powerballs: powerb.png row 0 (one cycle a second), row 1 while "phased" just after a throw or
/// a touch; a carried ball sits at the carrier's nose (reference client).
fn draw_balls(gfx: &Graphics, game: &Game, to_screen: &dyn Fn(Vec2) -> (f32, f32)) {
    let Some(s) = &game.session else { return };
    let Some((_, settings)) = game.world() else { return };
    let now = s.server_now();
    let frame = ((get_time() * 10.0) as u32) % 10;
    for b in &game.soccer.balls {
        let mut row = 0;
        let at = if b.state == aicontinuum::soccer::BallState::Carried {
            let carrier = if Some(b.carrier) == s.pid {
                game.ship.as_ref().map(|sh| (sh.position, sh.direction(), sh.ship))
            } else {
                s.players.get(&b.carrier).map(|p| (p.position, p.direction, p.ship))
            };
            let Some((pos, dir, ship)) = carrier.filter(|c| c.2 < 8) else { continue };
            let h = aicontinuum::sim::heading(dir);
            let ext = (settings.ships[ship as usize].radius_tiles() - 0.25).max(0.0);
            Vec2::new(pos.x + h.x * ext, pos.y + h.y * ext)
        } else {
            let since_touch = now.wrapping_sub(b.last_touch);
            if since_touch < 40 {
                continue; // About to be picked up: hidden until the zone answers.
            }
            if now.wrapping_sub(b.last_fire) < settings.pass_delay || since_touch < 100 {
                row = 10;
            }
            b.position()
        };
        let (x, y) = to_screen(at);
        gfx.ball.draw_centered(row + frame, x, y);
    }
}

fn draw_world(game: &Game, tiles: &Tiles, gfx: &Graphics, hud: &mut hud::Hud) {
    let (map, _settings) = game.world().unwrap();
    clear_background(BLACK);
    let (sw, sh) = (screen_width(), screen_height());
    let tile = TILE_PIXELS as f32;
    let camera = game.camera;

    // Camera centred on the ship (or spectator point), snapped to whole pixels.
    let cam_x = (camera.x * tile).floor() - (sw / 2.0).floor();
    let cam_y = (camera.y * tile).floor() - (sh / 2.0).floor();
    let to_screen = |p: Vec2| ((p.x * tile).floor() - cam_x, (p.y * tile).floor() - cam_y);

    let first_tx = (cam_x / tile).floor() as i32;
    let first_ty = (cam_y / tile).floor() as i32;
    let cols = (sw / tile) as i32 + 2;
    let rows = (sh / tile) as i32 + 2;
    draw_stars((cam_x, cam_y), sw, sh);

    let visible_tiles = || (first_ty..first_ty + rows).flat_map(move |ty| (first_tx..first_tx + cols).map(move |tx| (tx, ty)));
    for (tx, ty) in visible_tiles() {
        let id = visible_tile(map, tx, ty);
        if is_tileset_tile(id) && !is_fly_over(id) {
            tiles.draw(id, tx as f32 * tile - cam_x, ty as f32 * tile - cam_y);
        }
    }

    // Animated map objects, drawn from their top-left tile. Extend the view for big ones.
    let t = get_time() as f32;
    for r in &map.records {
        let (rx, ry) = (r.x as i32, r.y as i32);
        if rx < first_tx - 6 || rx > first_tx + cols || ry < first_ty - 6 || ry > first_ty + rows {
            continue;
        }
        let (sheet, period, first, count) = match r.id {
            172 => (&gfx.goal, 0.5, gfx.goal.frame_count() / 2, gfx.goal.frame_count() / 2),
            216 => (&gfx.over1, 1.5, 0, gfx.over1.frame_count()),
            217 => (&gfx.over2, 1.5, 0, gfx.over2.frame_count()),
            218 => (&gfx.over3, 1.5, 0, gfx.over3.frame_count()),
            219 => (&gfx.over4, 1.0, 0, gfx.over4.frame_count()),
            220 => (&gfx.over5, 2.5, 0, gfx.over5.frame_count()),
            _ => continue,
        };
        let frame = first + ((t % period) / period * count as f32) as u32 % count.max(1);
        sheet.draw(frame, rx as f32 * tile - cam_x, ry as f32 * tile - cam_y);
    }

    // Greens: 10-frame animation, one cycle per second.
    let prize_frame = ((get_time() * 10.0) as u32) % gfx.prizes.frame_count().max(1);
    for g in &game.greens.list {
        let (x, y) = to_screen(Vec2::new(g.x as f32, g.y as f32));
        gfx.prizes.draw(prize_frame, x, y);
    }

    // Dropped flags: first row of flag.png for enemy/neutral, second for our team's.
    if let Some(s) = &game.session {
        let my_freq = view_freq(game);
        let per_row = gfx.flags.frame_count() / 2;
        let frame = ((get_time() * per_row as f64) as u32) % per_row.max(1);
        for f in s.flags.values().filter(|f| f.dropped) {
            let row = if Some(f.owner) == my_freq { per_row } else { 0 };
            let (x, y) = to_screen(Vec2::new(f.x as f32, f.y as f32));
            gfx.flags.draw(row + frame, x, y);
        }
    }

    // Exhaust sits under the ships (its first frames are hidden by the ship in the original).
    for e in &game.exhaust {
        let (x, y) = to_screen(e.position);
        if e.rocket {
            // rocket.png: 13 frames of 24 px over 0.3 s (reference client).
            gfx.rocket.draw_centered((e.age * 13 / 30).min(12), x, y);
        } else {
            gfx.exhaust.draw_centered(e.frame(), x, y);
        }
    }

    for p in &game.bomb_puffs {
        let (x, y) = to_screen(p.position);
        gfx.trail.draw_centered(p.row as u32 * 10 + p.frame(), x, y);
    }

    for t in &game.trails {
        let (x, y) = to_screen(t.position);
        let [r, g, b] = t.color();
        draw_rectangle(x, y, 1.0, 1.0, Color::from_rgba(r, g, b, 255));
    }

    // Bricks: wall.png, enemy row then team row, 10 frames a second.
    if let Some(s) = &game.session {
        let my_freq = view_freq(game);
        let frame = ((get_time() * 10.0) as u32) % 10;
        for &(bx, by, team, _) in &s.bricks {
            let row = if Some(team) == my_freq { 10 } else { 0 };
            gfx.bricks.draw(row + frame, bx as f32 * tile - cam_x, by as f32 * tile - cam_y);
        }
    }
    // Our portal, while it lasts: warppnt.png cycling once a second.
    if let Some((at, _)) = game.ship.as_ref().and_then(|s| s.portal) {
        let (x, y) = to_screen(at);
        gfx.portal.draw_centered(((get_time() * 10.0) as u32) % 10, x, y);
    }

    for w in &game.weapons.list {
        let (x, y) = to_screen(w.position);
        draw_weapon(gfx, w, x, y);
    }
    draw_balls(gfx, game, &to_screen);

    if let Some(s) = &game.session {
        let view_freq = view_freq(game);
        let now = aicontinuum::net::transport::now_tick();
        for p in s.players.values() {
            if Some(p.pid) == s.pid || p.ship >= 8 || p.dead || p.position_updates == 0 || p.attached_to.is_some() {
                continue;
            }
            // Cloaked enemies stay hidden unless we have X-radar on.
            let cloaked = p.status & aicontinuum::sim::STATUS_CLOAK != 0;
            if cloaked && view_freq != Some(p.freq) && !game.xradar_on() {
                continue;
            }
            let (x, y) = to_screen(p.position);
            gfx.ships.draw_centered(p.ship as u32 * 40 + p.direction.min(39) as u32, x, y);
            let energy = p
                .energy_tick
                .filter(|t| now.wrapping_sub(*t) < 300)
                .map(|_| (p.energy, _settings.ships[p.ship as usize].initial_energy));
            hud.name_label(&p.name, p.bounty, p.flags_carried, game.soccer.carried_by(p.pid).is_some(), p.ping as u32 * 10, view_freq == Some(p.freq), energy, (x, y));
            draw_turrets(gfx, hud, game, p.pid, (x, y), view_freq, 1);
        }
    }

    if let (Some(ship), true) = (&game.ship, game.alive()) {
        let (x, y) = to_screen(ship.position);
        let riding = game.session.as_ref().and_then(|s| s.me()).is_some_and(|m| m.attached_to.is_some());
        if riding {
            // As a turret we're drawn as turret2.png on top of the carrier.
            gfx.turret.draw_centered(ship.direction() as u32, x, y);
        } else {
            gfx.ships.draw_centered(ship.ship as u32 * 40 + ship.direction() as u32, x, y);
        }
        if let Some(pid) = game.session.as_ref().and_then(|s| s.pid) {
            let low = ship.energy * 2.0 < ship.max_energy as f32;
            draw_turrets(gfx, hud, game, pid, (x, y), view_freq(game), if low { 2 } else { 1 });
        }
        // Carrying a ball with a throw timer: seconds left in red where our label would be.
        if let Some(t) = game.carry_ticks {
            hud.text(&format!("{:.1}", t as f32 / 100.0), hud::TextColor::Red, x + 14.0, y + 14.0);
        }
        // Riding as a turret, our energy goes one line below the carrier's label.
        let line = if riding { 12.0 } else { 0.0 };
        hud.own_energy_label(ship.energy as u32, ship.max_energy, (x, y + line));
    }

    for (tx, ty) in visible_tiles() {
        let id = visible_tile(map, tx, ty);
        if is_tileset_tile(id) && is_fly_over(id) {
            tiles.draw(id, tx as f32 * tile - cam_x, ty as f32 * tile - cam_y);
        }
    }

    for d in &game.debris {
        let (x, y) = to_screen(d.position);
        gfx.junk[d.ship.min(7) as usize].draw_centered(d.piece as u32 * 10 + d.frame(), x, y);
    }

    for e in &game.effects {
        let (x, y) = to_screen(e.position);
        let sheet = match e.kind {
            EffectKind::BulletHit => &gfx.explode0,
            EffectKind::Bomb => &gfx.explode2,
            EffectKind::EmpBomb => &gfx.empburst,
            EffectKind::ShipExplosion => &gfx.explode1,
            EffectKind::Warp => &gfx.warp,
            EffectKind::Repel => &gfx.repel,
            EffectKind::BallTrail => {
                // powerb.png row 2, ten frames over the trail's life.
                gfx.ball.draw_centered(20 + (e.age * 10 / e.kind.duration()).min(9), x, y);
                continue;
            }
            EffectKind::EmpSpark => &gfx.spark,
        };
        let n = sheet.frame_count();
        sheet.draw_centered((e.age * n / e.kind.duration()).min(n - 1), x, y);
    }

    hud.draw(game);
}
