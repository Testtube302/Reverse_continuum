//! Headless test pilot: joins a zone and flies circles near the map centre.
//! Usage: cargo run --example bot -- 127.0.0.1:5000 [name] [seconds] [ship] [fire|still]
//!   fire  - also hold the gun
//!   still - sit still (a target)

use std::time::{Duration, Instant};

use aicontinuum::game::Game;
use aicontinuum::net::session::{Config, Phase, Session};
use aicontinuum::sim::Input;

fn main() {
    let mut args = std::env::args().skip(1);
    let server = args.next().expect("server ip:port").parse().expect("ip:port");
    let name = args.next().unwrap_or_else(|| "AIC-Bot".into());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let ship_type: u8 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mode = args.next().unwrap_or_default();

    let session = Session::connect(Config {
        server,
        name: name.clone(),
        password: "bot".into(),
        arena: None,
        map_dir: std::env::temp_dir().join("aicontinuum-bot-maps"),
    })
    .expect("connect");
    let mut game = Game::online(session);

    let end = Instant::now() + Duration::from_secs(seconds);
    let mut requested = false;
    let mut next_tick = Instant::now();
    let input = match mode.as_str() {
        "still" => Input::default(),
        "fire" => Input { forward: true, right: true, gun: true, ..Default::default() },
        _ => Input { forward: true, right: true, ..Default::default() },
    };
    while Instant::now() < end {
        game.frame();
        if let Some(Phase::Failed(why)) = game.session.as_ref().map(|s| &s.phase) {
            eprintln!("{name}: {why}");
            std::process::exit(1);
        }
        if game.world().is_some() && !requested {
            requested = true;
            game.request_ship(ship_type);
            println!("{name}: in arena, requesting ship {ship_type}");
        }
        while Instant::now() >= next_tick {
            let fired = game.tick(input);
            if fired.0 != 0 {
                game.send_updates(fired);
            }
            next_tick += Duration::from_millis(10);
        }
        game.send_updates(Default::default());
        for n in game.notices.drain(..) {
            println!("{name}: {n}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    println!("{name}: done, kills {} deaths {}", game.kills, game.deaths);
    if let Some(s) = game.session.as_mut() {
        s.disconnect();
    }
}
