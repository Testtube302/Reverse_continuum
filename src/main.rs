use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use aicontinuum::assets::AssetDir;
use aicontinuum::bmp::IndexedImage;
use aicontinuum::lvl::{Map, TILE_PIXELS};
use aicontinuum::net::session::{Config, Phase, SPECTATOR, Session};
use aicontinuum::settings::{ArenaSettings, SHIP_NAMES};
use aicontinuum::sim::{self, Input, Ship, TICK_SECONDS, Vec2};
use macroquad::prelude::*;

const SHIP_FRAME: f32 = 36.0;
const SHIP_SHEET_COLUMNS: u32 = 10;
const TILESET_COLUMNS: u32 = 19;
const TILESET_COUNT: u8 = 190;
/// Spectator camera speed in tiles per second.
const SPECTATOR_SPEED: f32 = 40.0;

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
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    args
}

fn texture(img: &IndexedImage) -> Texture2D {
    let t = Texture2D::from_rgba8(img.width as u16, img.height as u16, &img.to_rgba(true));
    t.set_filter(FilterMode::Nearest);
    t
}

fn window_conf() -> Conf {
    Conf { window_title: "AIContinuum".into(), window_width: 1024, window_height: 768, ..Default::default() }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

struct World {
    map: Map,
    settings: ArenaSettings,
    tiles_tex: Texture2D,
}

impl World {
    fn new(map: Map, settings: ArenaSettings, assets: &AssetDir) -> Self {
        let img = match &map.tileset {
            Some(t) => t.clone(),
            None => assets.bm2("tiles").unwrap_or_else(|e| die(e)),
        };
        Self { tiles_tex: texture(&img), map, settings }
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let args = parse_args();
    let assets = AssetDir::locate(args.assets.as_deref()).unwrap_or_else(|e| die(e));
    let ships_tex = texture(&assets.bm2("ships").unwrap_or_else(|e| die(e)));

    let mut session = args.connect.map(|server| {
        let zone_dir = format!("{}_{}", server.ip(), server.port()).replace([':', '.'], "_");
        let config = Config {
            server,
            name: args.name.clone(),
            password: args.password.clone(),
            arena: args.arena.clone(),
            map_dir: assets.path("zones").join(zone_dir),
        };
        Session::connect(config).unwrap_or_else(|e| die(format!("connect {server}: {e}")))
    });

    // Offline: load the map now. Online: wait for the server's map.
    let mut world = if session.is_none() {
        let map_path = args.map.clone().or_else(|| {
            let p = assets.path("server/Zone/_bzw.lvl");
            p.is_file().then_some(p)
        });
        let map = match &map_path {
            Some(p) => std::fs::read(p).map_err(|e| e.to_string()).and_then(|d| Map::parse(&d)),
            None => Ok(Map::empty()),
        };
        Some(World::new(map.unwrap_or_else(|e| die(e)), ArenaSettings::default(), &assets))
    } else {
        None
    };

    let spawn = |w: &World, ship_type: u8| {
        let r = w.settings.ships[ship_type as usize].radius_tiles();
        Ship::new(ship_type, sim::find_spawn(&w.map, Vec2::new(512.0, 512.0), r), &w.settings)
    };
    let mut ship: Option<Ship> = world.as_ref().map(|w| spawn(w, args.ship.unwrap_or(0)));
    let mut camera = Vec2::new(512.0, 512.0);
    let mut requested_initial_ship = false;

    let mut accumulator = 0.0f32;
    let mut frame = 0u32;
    let mut scripted_ticks = 0u32;

    loop {
        if is_key_pressed(KeyCode::Escape) {
            break;
        }

        let ship_keys = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];
        let mut wanted_ship = ship_keys.iter().position(|k| is_key_pressed(*k)).map(|i| i as u8);
        if is_key_pressed(KeyCode::S) {
            wanted_ship = Some(SPECTATOR);
        }

        if let Some(s) = session.as_mut() {
            s.update();
            if world.is_none() && s.ready() {
                world = Some(World::new(s.map.clone().unwrap(), s.settings.clone().unwrap(), &assets));
            }
            if s.ready() && !requested_initial_ship {
                requested_initial_ship = true;
                if let Some(n) = args.ship {
                    s.request_ship(n);
                }
            }
            if let Some(n) = wanted_ship {
                s.request_ship(n);
            }
            // The server decides our ship; follow its confirmations.
            if let (Some(me), Some(w)) = (s.me().map(|m| m.ship), world.as_ref()) {
                match (&ship, me) {
                    (_, SPECTATOR) => ship = None,
                    (Some(cur), n) if cur.ship == n => {}
                    (_, n) if n < 8 => ship = Some(spawn(w, n)),
                    _ => {}
                }
            }
        } else if let (Some(n), Some(w)) = (wanted_ship, world.as_ref()) {
            ship = (n < 8).then(|| spawn(w, n));
        }

        let mut input = Input {
            left: is_key_down(KeyCode::Left),
            right: is_key_down(KeyCode::Right),
            forward: is_key_down(KeyCode::Up),
            backward: is_key_down(KeyCode::Down),
            afterburner: is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift),
        };

        if let Some(w) = world.as_ref() {
            if args.screenshot.is_some() && session.is_none() {
                // Deterministic: a fixed number of ticks regardless of frame time.
                while scripted_ticks < args.script_thrust_ticks {
                    input.forward = true;
                    if let Some(sh) = ship.as_mut() {
                        sh.tick(&w.map, &w.settings, input);
                    }
                    scripted_ticks += 1;
                }
            } else {
                accumulator += get_frame_time().min(0.25);
                while accumulator >= TICK_SECONDS {
                    match ship.as_mut() {
                        Some(sh) => {
                            sh.tick(&w.map, &w.settings, input);
                        }
                        None => {
                            let d = SPECTATOR_SPEED * TICK_SECONDS;
                            camera.x += (input.right as i32 - input.left as i32) as f32 * d;
                            camera.y += (input.backward as i32 - input.forward as i32) as f32 * d;
                            camera.x = camera.x.clamp(0.0, 1024.0);
                            camera.y = camera.y.clamp(0.0, 1024.0);
                        }
                    }
                    if let Some(s) = session.as_mut() {
                        s.tick_players();
                    }
                    accumulator -= TICK_SECONDS;
                }
            }
            if let Some(sh) = &ship {
                camera = sh.position;
            }
            if let Some(s) = session.as_mut() {
                s.spectate_at = camera;
                s.send_position(ship.as_ref(), false);
            }
            draw_world(w, &ships_tex, ship.as_ref(), session.as_ref(), camera);
        } else {
            clear_background(BLACK);
            let status = match session.as_ref().map(|s| &s.phase) {
                Some(Phase::Failed(why)) => format!("Connection failed: {why}"),
                Some(p) => format!("{p:?}... {}", session.as_ref().and_then(|s| s.log.last()).cloned().unwrap_or_default()),
                None => String::new(),
            };
            draw_text(&status, 8.0, 16.0, 16.0, GREEN);
        }

        if world.is_some() {
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
    if let Some(s) = session.as_mut() {
        s.disconnect();
    }
}

fn draw_ship(ships_tex: &Texture2D, ship_type: u8, direction: u8, screen_x: f32, screen_y: f32) {
    let frame = ship_type as u32 * 40 + direction.min(39) as u32;
    let src = Rect::new(
        (frame % SHIP_SHEET_COLUMNS) as f32 * SHIP_FRAME,
        (frame / SHIP_SHEET_COLUMNS) as f32 * SHIP_FRAME,
        SHIP_FRAME,
        SHIP_FRAME,
    );
    draw_texture_ex(
        ships_tex,
        screen_x - SHIP_FRAME / 2.0,
        screen_y - SHIP_FRAME / 2.0,
        WHITE,
        DrawTextureParams { source: Some(src), ..Default::default() },
    );
}

fn draw_world(w: &World, ships_tex: &Texture2D, ship: Option<&Ship>, session: Option<&Session>, camera: Vec2) {
    clear_background(BLACK);
    let (sw, sh) = (screen_width(), screen_height());
    let tile = TILE_PIXELS as f32;

    // Camera centred on the ship (or spectator point), snapped to whole pixels.
    let cam_x = (camera.x * tile).floor() - (sw / 2.0).floor();
    let cam_y = (camera.y * tile).floor() - (sh / 2.0).floor();

    let first_tx = (cam_x / tile).floor() as i32;
    let first_ty = (cam_y / tile).floor() as i32;
    let cols = (sw / tile) as i32 + 2;
    let rows = (sh / tile) as i32 + 2;
    for ty in first_ty..first_ty + rows {
        for tx in first_tx..first_tx + cols {
            let id = w.map.tile(tx, ty);
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
                &w.tiles_tex,
                tx as f32 * tile - cam_x,
                ty as f32 * tile - cam_y,
                WHITE,
                DrawTextureParams { source: Some(src), ..Default::default() },
            );
        }
    }

    if let Some(s) = session {
        for p in s.players.values() {
            if Some(p.pid) == s.pid || p.ship >= 8 {
                continue;
            }
            let x = (p.position.x * tile).floor() - cam_x;
            let y = (p.position.y * tile).floor() - cam_y;
            draw_ship(ships_tex, p.ship, p.direction, x, y);
            // Temporary label until the original fonts are implemented.
            draw_text(&p.name, x + SHIP_FRAME / 2.0, y + SHIP_FRAME / 2.0, 14.0, YELLOW);
        }
    }

    if let Some(ship) = ship {
        let x = (ship.position.x * tile).floor() - cam_x;
        let y = (ship.position.y * tile).floor() - cam_y;
        draw_ship(ships_tex, ship.ship, ship.direction(), x, y);
    }

    // Temporary debug readout until the real HUD is implemented.
    let info = match ship {
        Some(ship) => format!(
            "{}  x:{} y:{}  speed:{:.0}  energy:{:.0}",
            SHIP_NAMES[ship.ship as usize],
            ship.position.x as i32,
            ship.position.y as i32,
            ship.velocity.length() * 160.0,
            ship.energy
        ),
        None => format!("Spectating  x:{} y:{}  (1-8 to enter a ship)", camera.x as i32, camera.y as i32),
    };
    draw_text(&info, 8.0, 16.0, 16.0, GREEN);
    if let Some(s) = session {
        let others = s.players.len().saturating_sub(1);
        let arena = s.map_name.clone().unwrap_or_default();
        draw_text(&format!("{} players here  map {arena}  ping {}ms", others + 1, s.transport.rtt * 10), 8.0, 32.0, 16.0, GREEN);
    }
}
