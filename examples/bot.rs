//! Headless test pilot: joins a zone and flies circles near the map centre.
//! Usage: cargo run --example bot -- 127.0.0.1:5000 [name] [seconds] [ship]

use std::time::{Duration, Instant};

use aicontinuum::net::session::{Config, Phase, Session};
use aicontinuum::sim::{self, Input, Ship, Vec2};

fn main() {
    let mut args = std::env::args().skip(1);
    let server = args.next().expect("server ip:port").parse().expect("ip:port");
    let name = args.next().unwrap_or_else(|| "AIC-Bot".into());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let ship_type: u8 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut s = Session::connect(Config {
        server,
        name: name.clone(),
        password: "bot".into(),
        arena: None,
        map_dir: std::env::temp_dir().join("aicontinuum-bot-maps"),
    })
    .expect("connect");

    let end = Instant::now() + Duration::from_secs(seconds);
    let mut ship: Option<Ship> = None;
    let mut requested = false;
    let mut next_tick = Instant::now();
    while Instant::now() < end {
        s.update();
        if let Phase::Failed(why) = &s.phase {
            eprintln!("{name}: {why}");
            std::process::exit(1);
        }
        if s.ready() && !requested {
            requested = true;
            s.request_ship(ship_type);
            println!("{name}: in arena, requesting ship {ship_type}");
        }
        if ship.is_none() && s.me().is_some_and(|m| m.ship == ship_type) {
            let (map, settings) = (s.map.as_ref().unwrap(), s.settings.as_ref().unwrap());
            let r = settings.ships[ship_type as usize].radius_tiles();
            ship = Some(Ship::new(ship_type, sim::find_spawn(map, Vec2::new(512.0, 512.0), r), settings));
            println!("{name}: flying");
        }
        while Instant::now() >= next_tick {
            if let (Some(sh), Some(map), Some(settings)) = (ship.as_mut(), s.map.as_ref(), s.settings.as_ref()) {
                sh.tick(map, settings, Input { forward: true, right: true, ..Default::default() });
            }
            next_tick += Duration::from_millis(10);
        }
        s.send_position(ship.as_ref(), false);
        std::thread::sleep(Duration::from_millis(5));
    }
    s.disconnect();
}
