use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use aicontinuum::assets::AssetDir;
use aicontinuum::game::{EffectKind, Game};
use aicontinuum::lvl::{Map, TILE_PIXELS};
use aicontinuum::net::session::{Config, Phase, SPECTATOR, Session};
use aicontinuum::settings::{ArenaSettings, SHIP_NAMES};
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
    mute: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        assets: None,
        map: None,
        connect: None,
        name: "AIContinuum".into(),
        password: "aicontinuum".into(),
        arena: None,
        ship: None,
        screenshot: None,
        script_thrust_ticks: 0,
        script_fire: false,
        mute: false,
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
            "--mute" => args.mute = true,
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    args
}

fn window_conf() -> Conf {
    Conf { window_title: "AIContinuum".into(), window_width: 1024, window_height: 768, ..Default::default() }
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
}

impl Sheet {
    fn load(assets: &AssetDir, name: &str, frame_w: u32, frame_h: u32) -> Self {
        let img = assets.graphic(name).unwrap_or_else(|e| die(e));
        let tex = Texture2D::from_rgba8(img.width as u16, img.height as u16, &img.pixels);
        tex.set_filter(FilterMode::Nearest);
        Self { tex, frame_w: frame_w as f32, frame_h: frame_h as f32, columns: (img.width / frame_w).max(1) }
    }

    fn frame_count(&self) -> u32 {
        self.columns * (self.tex.height() / self.frame_h) as u32
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
    prizes: Sheet,
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
            prizes: Sheet::load(assets, "prizes", 16, 16),
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

struct Tiles {
    tex: Texture2D,
}

impl Tiles {
    fn for_map(map: &Map, assets: &AssetDir) -> Self {
        let rgba = match &map.tileset {
            Some(t) => (t.width, t.height, t.to_rgba(true)),
            None => {
                let img = assets.graphic("tiles").unwrap_or_else(|e| die(e));
                (img.width, img.height, img.pixels)
            }
        };
        let tex = Texture2D::from_rgba8(rgba.0 as u16, rgba.1 as u16, &rgba.2);
        tex.set_filter(FilterMode::Nearest);
        Self { tex }
    }
}

fn read_input() -> Input {
    let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    let tab = is_key_down(KeyCode::Tab);
    Input {
        left: is_key_down(KeyCode::Left),
        right: is_key_down(KeyCode::Right),
        forward: is_key_down(KeyCode::Up),
        backward: is_key_down(KeyCode::Down),
        afterburner: shift && !tab,
        gun: is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl),
        bomb: tab && !shift,
        mine: tab && shift,
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let args = parse_args();
    let assets = AssetDir::locate(args.assets.as_deref()).unwrap_or_else(|e| die(e));
    let gfx = Graphics::load(&assets);
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
            Game::online(Session::connect(config).unwrap_or_else(|e| die(format!("connect {server}: {e}"))))
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
            Game::offline(map.unwrap_or_else(|e| die(e)), ArenaSettings::default(), args.ship.unwrap_or(0))
        }
    };

    let mut tiles: Option<Tiles> = None;
    let mut requested_initial_ship = false;
    let mut accumulator = 0.0f32;
    let mut frame = 0u32;
    let mut scripted_ticks = 0u32;
    let mut fire_held = false;

    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }
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
        let ship_keys = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];
        if let Some(i) = ship_keys.iter().position(|k| is_key_pressed(*k)) {
            game.request_ship(i as u8);
        }
        if is_key_pressed(KeyCode::S) && !is_key_down(KeyCode::Tab) {
            game.request_ship(SPECTATOR);
        }
        if is_key_pressed(KeyCode::Delete) {
            if let Some(ship) = game.ship.as_mut() {
                ship.multifire = !ship.multifire;
                game.notices.push(format!("Multifire {}", if ship.multifire { "on" } else { "off" }));
            }
        }

        let mut fired_any = aicontinuum::weapons::WeaponData::default();
        if ready {
            if args.screenshot.is_some() && game.session.is_none() {
                // Deterministic offline script: a fixed number of ticks regardless of frame time.
                while scripted_ticks < args.script_thrust_ticks {
                    let input = Input {
                        forward: true,
                        gun: args.script_fire && scripted_ticks > args.script_thrust_ticks.saturating_sub(40),
                        bomb: args.script_fire && scripted_ticks == args.script_thrust_ticks.saturating_sub(60),
                        ..Default::default()
                    };
                    game.tick(input);
                    scripted_ticks += 1;
                }
            } else {
                let mut input = read_input();
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
            draw_world(&game, tiles.as_ref().unwrap(), &gfx);
        } else {
            clear_background(BLACK);
            let status = match game.session.as_ref().map(|s| &s.phase) {
                Some(Phase::Failed(why)) => format!("Connection failed: {why}"),
                Some(p) => format!("{p:?}... {}", game.session.as_ref().and_then(|s| s.log.last()).cloned().unwrap_or_default()),
                None => String::new(),
            };
            draw_text(&status, 8.0, 16.0, 16.0, GREEN);
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
        _ => {}
    }
}

fn draw_world(game: &Game, tiles: &Tiles, gfx: &Graphics) {
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
    for ty in first_ty..first_ty + rows {
        for tx in first_tx..first_tx + cols {
            let id = map.tile(tx, ty);
            if id == 0 || id > TILESET_COUNT {
                continue;
            }
            let idx = (id - 1) as u32;
            let src = Rect::new(
                ((idx % TILESET_COLUMNS) * TILE_PIXELS) as f32,
                ((idx / TILESET_COLUMNS) * TILE_PIXELS) as f32,
                tile,
                tile,
            );
            draw_texture_ex(
                &tiles.tex,
                tx as f32 * tile - cam_x,
                ty as f32 * tile - cam_y,
                WHITE,
                DrawTextureParams { source: Some(src), ..Default::default() },
            );
        }
    }

    // Greens: 10-frame animation, one cycle per second.
    let prize_frame = ((get_time() * 10.0) as u32) % gfx.prizes.frame_count().max(1);
    for g in &game.greens.list {
        let (x, y) = to_screen(Vec2::new(g.x as f32, g.y as f32));
        gfx.prizes.draw(prize_frame, x, y);
    }

    for w in &game.weapons.list {
        let (x, y) = to_screen(w.position);
        draw_weapon(gfx, w, x, y);
    }

    if let Some(s) = &game.session {
        for p in s.players.values() {
            if Some(p.pid) == s.pid || p.ship >= 8 || p.dead {
                continue;
            }
            let (x, y) = to_screen(p.position);
            gfx.ships.draw_centered(p.ship as u32 * 40 + p.direction.min(39) as u32, x, y);
            // Temporary label until the original fonts are implemented.
            draw_text(&p.name, x + 18.0, y + 18.0, 14.0, YELLOW);
        }
    }

    if let (Some(ship), true) = (&game.ship, game.alive()) {
        let (x, y) = to_screen(ship.position);
        gfx.ships.draw_centered(ship.ship as u32 * 40 + ship.direction() as u32, x, y);
    }

    for e in &game.effects {
        let (x, y) = to_screen(e.position);
        let sheet = match e.kind {
            EffectKind::BulletHit => &gfx.explode0,
            EffectKind::Bomb => &gfx.explode2,
            EffectKind::EmpBomb => &gfx.empburst,
            EffectKind::ShipExplosion => &gfx.explode1,
        };
        let n = sheet.frame_count();
        sheet.draw_centered((e.age * n / e.kind.duration()).min(n - 1), x, y);
    }

    // Temporary debug readout until the real HUD is implemented.
    let info = match &game.ship {
        Some(ship) if game.dead_ticks > 0 => format!("{}  respawning in {:.1}s", SHIP_NAMES[ship.ship as usize], game.dead_ticks as f32 / 100.0),
        Some(ship) => format!(
            "{}  x:{} y:{}  speed:{:.0}  energy:{:.0}  guns L{} bombs L{}{}  K/D {}/{}",
            SHIP_NAMES[ship.ship as usize],
            ship.position.x as i32,
            ship.position.y as i32,
            ship.velocity.length() * 160.0,
            ship.energy,
            ship.guns,
            ship.bombs,
            if ship.multifire { " multi" } else { "" },
            game.kills,
            game.deaths
        ),
        None => format!("Spectating  x:{} y:{}  (1-8 to enter a ship)", camera.x as i32, camera.y as i32),
    };
    draw_text(&info, 8.0, 16.0, 16.0, GREEN);
    if let Some(s) = &game.session {
        let arena = s.map_name.clone().unwrap_or_default();
        draw_text(&format!("{} players here  map {arena}  ping {}ms", s.players.len(), s.transport.rtt * 10), 8.0, 32.0, 16.0, GREEN);
    }
    for (i, n) in game.notices.iter().rev().take(4).enumerate() {
        draw_text(n, 8.0, sh - 12.0 - i as f32 * 16.0, 16.0, YELLOW);
    }
}
