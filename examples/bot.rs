//! Headless test pilot: joins a zone and flies circles near the map centre.
//! Usage: cargo run --example bot -- 127.0.0.1:5000 [name] [seconds] [ship] [modes]
//!   modes: comma-separated; fire = hold the gun, still = sit still (a target),
//!          chat = say something in public chat every 3 seconds,
//!          chase:NAME = steer toward player NAME, keep ~10 tiles away and fire when lined up,
//!          dist:N = chase distance in tiles (default 10),
//!          every:MS = at most one shot per MS milliseconds, bomb = fire bombs instead of guns,
//!          flags = collect dropped flags first (then chase),
//!          ball = fetch the free powerball, carry it to the chase target and throw it (warp key),
//!          arena:NAME = join that arena,
//!          attach:NAME = attach to that player as a turret ~3 s after getting a ship,
//!          watch = report the chase target but don't move,
//!          freq:N = switch to team N after getting a ship

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
        arena: mode.split(',').find_map(|m| m.strip_prefix("arena:")).map(str::to_string),
        map_dir: std::env::temp_dir().join("aicontinuum-bot-maps"),
    })
    .expect("connect");
    let mut game = Game::online(session);

    let end = Instant::now() + Duration::from_secs(seconds);
    let mut requested = false;
    let mut next_tick = Instant::now();
    let modes: Vec<&str> = mode.split(',').collect();
    let input = if modes.contains(&"still") {
        Input::default()
    } else {
        Input { forward: true, right: true, gun: modes.contains(&"fire"), ..Default::default() }
    };
    let chat = modes.contains(&"chat");
    let chase = modes.iter().find_map(|m| m.strip_prefix("chase:")).map(str::to_string);
    let freq: Option<u16> = modes.iter().find_map(|m| m.strip_prefix("freq:")).and_then(|f| f.parse().ok());
    let mut freq_sent = false;
    let dist: f32 = modes.iter().find_map(|m| m.strip_prefix("dist:")).and_then(|d| d.parse().ok()).unwrap_or(10.0);
    let every: Option<u64> = modes.iter().find_map(|m| m.strip_prefix("every:")).and_then(|d| d.parse().ok());
    let bomb = modes.contains(&"bomb");
    let want_flags = modes.contains(&"flags");
    let want_ball = modes.contains(&"ball");
    let mut attach_to = modes.iter().find_map(|m| m.strip_prefix("attach:")).map(str::to_string);
    let mut attach_at: Option<Instant> = None;
    let mut next_shot = Instant::now();
    let mut next_report = Instant::now() + Duration::from_secs(2);
    let mut next_chat = Instant::now() + Duration::from_secs(3);
    let mut said = 0u32;
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
        if let (Some(f), false, true) = (freq, freq_sent, game.ship.is_some()) {
            freq_sent = true;
            if let Some(s) = game.session.as_mut() {
                s.request_freq(f);
            }
        }
        if let (Some(name), true) = (attach_to.clone(), game.alive()) {
            let when = *attach_at.get_or_insert(Instant::now() + Duration::from_secs(3));
            let pid = game.session.as_ref().and_then(|s| s.players.values().find(|p| p.name == name).map(|p| p.pid));
            if let (true, Some(pid)) = (Instant::now() >= when, pid) {
                game.request_attach(Some(pid));
                println!("{name}: attach requested; notices {:?}", game.center_messages.last().map(|m| m.text.clone()));
                attach_to = None;
            }
        }
        if let (Some(target), true) = (&chase, Instant::now() >= next_report) {
            next_report += Duration::from_secs(2);
            report_chase(&game, target);
            if want_flags {
                let o = game.ship.as_ref().map(|s| (s.orientation, s.velocity.x, s.velocity.y));
                println!("flag input {:?} ship (orient, vx, vy) {o:?}", flag_input(&game));
            }
        }
        let watch = modes.contains(&"watch");
        let flag_input = if want_flags {
            flag_input(&game)
        } else if want_ball {
            ball_input(&game, chase.as_deref(), dist)
        } else {
            None
        };
        let input = match (&chase, flag_input.or_else(|| chase_input(&game, chase.as_deref(), dist))) {
            (Some(_), Some(i)) => i,
            (Some(_), None) => Input::default(),
            _ => input,
        };
        let input = if watch { Input::default() } else { input };
        while Instant::now() >= next_tick {
            let mut input = input;
            if bomb {
                (input.bomb, input.gun) = (input.gun, false);
            }
            if every.is_some() && Instant::now() < next_shot {
                (input.gun, input.bomb) = (false, false);
            }
            let fired = game.tick(input);
            if fired.0 != 0 {
                game.send_updates(fired);
                if let Some(ms) = every {
                    next_shot = Instant::now() + Duration::from_millis(ms);
                    println!("{name}: shot at {:?}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap());
                }
            }
            next_tick += Duration::from_millis(10);
        }
        game.send_updates(Default::default());
        if chat && game.world().is_some() && Instant::now() >= next_chat {
            said += 1;
            if let Some(s) = game.session.as_mut() {
                s.send_public_chat(&format!("hello from {name}, message {said}"));
            }
            next_chat += Duration::from_secs(3);
        }
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

/// One status line for chase mode: where we and the target are.
fn report_chase(game: &Game, target: &str) {
    let Some(s) = game.session.as_ref() else { return };
    let me = game.ship.as_ref().map(|m| (m.position.x as i32, m.position.y as i32));
    match s.players.values().find(|p| p.name == target) {
        Some(t) => println!(
            "chase: me {me:?} target ship {} dead {} updates {} at ({}, {}) px ({}, {}) dir {}",
            t.ship,
            t.dead,
            t.position_updates,
            t.position.x as i32,
            t.position.y as i32,
            (t.position.x * 16.0).round(),
            (t.position.y * 16.0).round(),
            t.direction
        ),
        None => println!("chase: me {me:?}, no player named {target}"),
    }
    let dropped: Vec<_> = s.flags.values().filter(|f| f.dropped).map(|f| (f.id, f.x, f.y, f.owner)).collect();
    println!(
        "flags: known {} dropped {:?} carrying {}",
        s.flags.len(),
        dropped,
        s.me().map_or(0, |m| m.flags_carried)
    );
}

/// Seek steering toward `target` (tiles): wanted velocity points at it and slows on approach.
fn seek(game: &Game, target: (f32, f32)) -> Option<Input> {
    let ship = game.ship.as_ref()?;
    let (dx, dy) = (target.0 - ship.position.x, target.1 - ship.position.y);
    let dist = (dx * dx + dy * dy).sqrt().max(0.01);
    let speed = (dist * 1.2).min(6.0);
    let (ex, ey) = (dx / dist * speed - ship.velocity.x, dy / dist * speed - ship.velocity.y);
    let err = (ex * ex + ey * ey).sqrt();
    let want = (ex.atan2(-ey) / std::f32::consts::TAU).rem_euclid(1.0);
    let diff = (want - ship.orientation + 1.5).rem_euclid(1.0) - 0.5;
    Some(Input {
        right: diff > 0.01 && diff < 0.45,
        left: diff < -0.01 && diff > -0.45,
        forward: err > 0.3 && diff.abs() < 0.12,
        backward: err > 0.3 && diff.abs() > 0.38,
        ..Default::default()
    })
}

/// Powerball errand: fetch the free ball, then fly at the target and throw (warp) when close.
fn ball_input(game: &Game, target: Option<&str>, keep: f32) -> Option<Input> {
    let ship = game.ship.as_ref()?;
    let s = game.session.as_ref()?;
    let me = s.pid?;
    if game.soccer.carried_by(me).is_some() {
        let t = s.players.values().find(|p| Some(p.name.as_str()) == target && p.ship < 8 && p.position_updates > 0)?;
        let (dx, dy) = (t.position.x - ship.position.x, t.position.y - ship.position.y);
        let mut input = seek(game, (t.position.x, t.position.y))?;
        input.warp = (dx * dx + dy * dy).sqrt() < keep;
        return Some(input);
    }
    let ball = game.soccer.balls.iter().find(|b| b.state == aicontinuum::soccer::BallState::World)?;
    let at = ball.position();
    seek(game, (at.x, at.y))
}

/// Steering toward the nearest dropped flag, while there is one and we carry fewer than 3.
fn flag_input(game: &Game) -> Option<Input> {
    let ship = game.ship.as_ref()?;
    let s = game.session.as_ref()?;
    if s.me().is_some_and(|m| m.flags_carried >= 3) {
        return None;
    }
    let near = s.flags.values().filter(|f| f.dropped).min_by(|a, b| {
        let d = |f: &&aicontinuum::net::session::Flag| {
            let (dx, dy) = (f.x as f32 + 0.5 - ship.position.x, f.y as f32 + 0.5 - ship.position.y);
            dx * dx + dy * dy
        };
        d(a).total_cmp(&d(b))
    })?;
    let (dx, dy) = (near.x as f32 + 0.5 - ship.position.x, near.y as f32 + 0.5 - ship.position.y);
    // Seek: wanted velocity points at the flag and slows on approach; steer to close the gap
    // between it and our velocity (velocity is in tiles/s).
    let dist = (dx * dx + dy * dy).sqrt().max(0.01);
    let speed = (dist * 1.2).min(6.0);
    let (ex, ey) = (dx / dist * speed - ship.velocity.x, dy / dist * speed - ship.velocity.y);
    let err = (ex * ex + ey * ey).sqrt();
    let want = (ex.atan2(-ey) / std::f32::consts::TAU).rem_euclid(1.0);
    let diff = (want - ship.orientation + 1.5).rem_euclid(1.0) - 0.5;
    Some(Input {
        right: diff > 0.01 && diff < 0.45,
        left: diff < -0.01 && diff > -0.45,
        forward: err > 0.3 && diff.abs() < 0.12,
        backward: err > 0.3 && diff.abs() > 0.38,
        ..Default::default()
    })
}

/// Steering toward a named player: turn to face them, thrust when far, back off when close,
/// and fire when roughly lined up.
fn chase_input(game: &Game, target: Option<&str>, keep: f32) -> Option<Input> {
    let target = target?;
    let ship = game.ship.as_ref()?;
    let s = game.session.as_ref()?;
    let t = s.players.values().find(|p| p.name == target && p.ship < 8 && !p.dead && p.position_updates > 0)?;
    let (dx, dy) = (t.position.x - ship.position.x, t.position.y - ship.position.y);
    let dist = (dx * dx + dy * dy).sqrt();
    // Guns don't fire in a safe zone: close in until we're out of it.
    let in_safe = game.world().is_some_and(|(map, _)| {
        map.tile(ship.position.x as i32, ship.position.y as i32) == aicontinuum::lvl::TILE_SAFE
    });
    // Orientation 0 = up, increasing clockwise, in turns.
    let want = (dx.atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0);
    let diff = (want - ship.orientation + 1.5).rem_euclid(1.0) - 0.5;
    Some(Input {
        right: diff > 0.01,
        left: diff < -0.01,
        forward: dist > keep + 2.0 || (in_safe && dist > 2.0),
        backward: dist < keep - 3.0,
        gun: diff.abs() < 0.04 && dist < 30.0,
        ..Default::default()
    })
}
