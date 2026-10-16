//! End-to-end tests against a running zone server (ASSS / Subspace Server .NET).
//! Skipped unless AICONTINUUM_SERVER is set, e.g. AICONTINUUM_SERVER=127.0.0.1:5000.
//! Set AICONTINUUM_TEST_EXPANDER to test Continuum with native identification.

use std::time::{Duration, Instant};

use aicontinuum::net::session::{Config, Phase, Session};
use aicontinuum::sim::{Input, Ship, Vec2};

fn server() -> Option<std::net::SocketAddr> {
    std::env::var("AICONTINUUM_SERVER").ok().map(|s| s.parse().expect("AICONTINUUM_SERVER must be ip:port"))
}

fn config(name: &str) -> Config {
    Config {
        server: server().unwrap(),
        name: name.into(),
        password: "test".into(),
        arena: None,
        map_dir: std::env::temp_dir().join(format!("aicontinuum-maps-{}", std::process::id())),
    }
}

fn connect(config: Config) -> std::io::Result<Session> {
    if let Some(program) = std::env::var_os("AICONTINUUM_TEST_EXPANDER") {
        let mut expander = aicontinuum::net::continuum_expander::CommandExpander::new(program.into());
        let identity = aicontinuum::net::native_identity::prepare(|nonce| expander.expand_identity(nonce))?;
        Session::connect_continuum_identified(config, move |seed| expander.expand(seed).ok(), identity)
    } else {
        Session::connect(config)
    }
}

fn pump_until(sessions: &mut [&mut Session], secs: u64, mut done: impl FnMut(&[&mut Session]) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        for s in sessions.iter_mut() {
            s.update();
            s.send_position(None, Default::default(), false, false);
            if let Phase::Failed(why) = &s.phase {
                panic!("{} failed: {why}\nlog: {:#?}", s.config.name, s.log);
            }
        }
        if done(sessions) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn logs_in_enters_arena_and_downloads_map() {
    if server().is_none() {
        return;
    }
    let mut s = connect(config("AIC-Test1")).unwrap();
    let ok = pump_until(&mut [&mut s], 15, |s| s[0].ready());
    assert!(ok, "did not become ready; log: {:#?}", s.log);

    let settings = s.settings.as_ref().unwrap();
    assert!(settings.ships.iter().all(|sh| sh.initial_speed > 0), "ship settings parsed");
    // Values from the test zone's conf/svs (server scales damage by 1000).
    let w = &settings.ships[0];
    println!("arena: {settings:?}");
    assert_eq!((w.bullet_speed, w.bomb_speed, w.bullet_fire_delay, w.bomb_fire_delay), (2000, 2000, 25, 150));
    assert_eq!((w.bullet_fire_energy, w.bomb_fire_energy, w.multi_fire_angle, w.max_mines), (20, 300, 500, 5));
    assert_eq!((w.shrapnel_max, w.shrapnel_rate, w.max_guns, w.max_bombs, w.initial_guns), (8, 2, 3, 2, 1));
    assert_eq!((settings.bullet_alive_time, settings.bomb_alive_time, settings.mine_alive_time), (550, 6000, 12000));
    assert_eq!((settings.bomb_explode_pixels, settings.bomb_explode_delay, settings.enter_delay), (80, 150, 200));
    assert_eq!((settings.proximity_distance, settings.shrapnel_speed, settings.team_max_mines), (3, 3000, 12));
    assert_eq!(settings.bullet_damage_level, 100_000);
    assert_eq!(settings.bomb_damage_level, 750_000);
    assert_eq!(settings.bullet_damage_upgrade, 50_000);
    assert!(settings.exact_damage && settings.bomb_safety);
    assert_eq!(
        settings.prize_weights,
        [80, 110, 50, 25, 30, 30, 10, 50, 45, 45, 50, 80, 30, 0, 40, 45, 5, 5, 60, 30, 20, 10, 25, 20, 80, 10, 25, 25]
    );
    // arenas/(default)/arena.conf overrides PrizeFactor, PrizeDelay and MinimumVirtual.
    assert_eq!((settings.prize_factor, settings.prize_delay, settings.prize_hide_count), (10000, 700, 10));
    assert_eq!((settings.minimum_virtual, settings.upgrade_virtual), (10, 6));
    assert_eq!((settings.prize_min_exist, settings.prize_max_exist, settings.prize_negative_factor), (4000, 8000, 300));
    assert_eq!((settings.multi_prize_count, settings.death_prize_time, settings.engine_shutdown_time), (10, 800, 700));
    assert!(!settings.take_prize_reliable);
    assert_eq!(settings.map_zoom_factor, 10);
    assert_eq!(settings.carry_flags, 1);
    assert_eq!((settings.door_delay, settings.door_mode), (400, -1));
    assert!(s.pid.is_some());
    assert!(s.me().is_some(), "we appear in our own player list");
    let map = s.map.as_ref().unwrap();
    assert!(!map.records.is_empty());
    println!("log: {:#?}", s.log);
    s.disconnect();
}

/// Servers send checksum requests periodically (Subspace Server .NET: every 60 s).
#[test]
fn answers_periodic_security_check() {
    if server().is_none() {
        return;
    }
    let mut s = connect(config("AIC-Sec")).unwrap();
    let ok = pump_until(&mut [&mut s], 75, |s| s[0].ready() && s[0].security_replies > 0);
    assert!(ok, "no security exchange; log: {:#?}", s.log);
    s.disconnect();
}

#[test]
fn two_clients_see_each_other_fly() {
    if server().is_none() {
        return;
    }
    let mut a = connect(config("AIC-Pilot")).unwrap();
    let mut b = connect(config("AIC-Watcher")).unwrap();
    assert!(
        pump_until(&mut [&mut a, &mut b], 15, |s| s.iter().all(|s| s.ready())),
        "not ready\nA: {:#?}\nB: {:#?}",
        a.log,
        b.log
    );

    // A enters a Warbird and waits for the server to confirm the ship change.
    a.request_ship(0);
    let a_pid = a.pid.unwrap();
    assert!(
        pump_until(&mut [&mut a, &mut b], 10, |s| s[0].me().is_some_and(|m| m.ship == 0)
            && s[1].players.get(&a_pid).is_some_and(|p| p.ship == 0)),
        "ship change not seen\nA: {:#?}\nB: {:#?}",
        a.log,
        b.log
    );

    // A flies upward for 2 seconds with the real simulation, sending positions.
    let settings = a.settings.clone().unwrap();
    println!("server Warbird settings: {:?}", settings.ships[0]);
    let map = a.map.clone().unwrap();
    // Find a spot with a clear 40-tile runway upward so the flight isn't cut short by a wall.
    let r = settings.ships[0].radius_tiles();
    let start = (0..1024 * 1024)
        .map(|i| Vec2::new((i % 1024) as f32 + 0.5, (i / 1024) as f32 + 0.5))
        .find(|p| p.y > 50.0 && (0..40).all(|dy| map.can_fit(p.x, p.y - dy as f32, r + 1.0)))
        .expect("map has an open runway");
    // Servers only relay ships within view, so B watches the middle of A's flight path.
    b.spectate_at = Vec2::new(start.x, start.y - 15.0);
    b.send_position(None, Default::default(), false, true);
    let mut ship = Ship::new(0, start, &settings);
    let input = Input { forward: true, ..Default::default() };
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        ship.tick(&map, &settings, input);
        a.update();
        a.send_position(Some(&ship), Default::default(), false, false);
        b.update();
        b.tick_players();
        std::thread::sleep(Duration::from_millis(10));
    }
    a.send_position(Some(&ship), Default::default(), false, true);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        a.update();
        b.update();
        std::thread::sleep(Duration::from_millis(10));
    }

    let seen = b.players.get(&a_pid).expect("B knows A");
    println!(
        "A at ({:.2},{:.2}) vel ({:.2},{:.2}); B sees ({:.2},{:.2}) vel ({:.2},{:.2}) after {} updates",
        ship.position.x, ship.position.y, ship.velocity.x, ship.velocity.y,
        seen.position.x, seen.position.y, seen.velocity.x, seen.velocity.y, seen.position_updates
    );
    assert!(seen.position_updates >= 10, "B received A's position updates");
    assert!(ship.position.y < start.y - 5.0, "A actually moved");
    // B's view (extrapolated from the last update) is within a couple of tiles of A's truth.
    assert!((seen.position.x - ship.position.x).abs() < 2.0);
    assert!((seen.position.y - ship.position.y).abs() < 2.0);
    a.disconnect();
    b.disconnect();
}

#[test]
fn public_chat_is_delivered_and_shown_once_to_the_sender() {
    if server().is_none() {
        return;
    }
    let mut sender = connect(config("AIC-ChatSend")).unwrap();
    let mut receiver = connect(config("AIC-ChatRecv")).unwrap();
    assert!(pump_until(&mut [&mut sender, &mut receiver], 15, |s| s.iter().all(|s| s.ready())));
    let pid = sender.pid.unwrap();
    let message = "Local chat delivery regression";
    sender.send_public_chat(message);
    let delivered = pump_until(&mut [&mut sender, &mut receiver], 5, |s| {
        s[1].chat.iter().any(|c| c.kind == 2 && c.pid == pid && c.text == message)
    });
    // Allow a server echo, if any, to arrive before checking for duplication.
    pump_until(&mut [&mut sender, &mut receiver], 1, |_| false);
    let count = |s: &Session| s.chat.iter().filter(|c| c.kind == 2 && c.pid == pid && c.text == message).count();
    let (shown, received) = (count(&sender), count(&receiver));
    sender.disconnect();
    receiver.disconnect();
    assert!(delivered, "public message did not reach the other local client");
    assert_eq!(received, 1, "receiver should show one copy");
    assert_eq!(shown, 1, "sender must show its own message locally");
}

/// Full combat round trip: B shoots A until A's client registers the damage, dies and tells
/// the server, and the server announces the kill back to B.
#[test]
fn shots_kill_and_server_reports_the_kill() {
    use aicontinuum::game::Game;
    use aicontinuum::lvl::TILE_SAFE;

    if server().is_none() {
        return;
    }
    let mut a = Game::online(connect(config("AIC-Target")).unwrap());
    let mut b = Game::online(connect(config("AIC-Gunner")).unwrap());
    let pump = |a: &mut Game, b: &mut Game, secs: u64, mut done: Box<dyn FnMut(&Game, &Game) -> bool>| {
        let end = Instant::now() + Duration::from_secs(secs);
        let mut next = Instant::now();
        while Instant::now() < end {
            a.frame();
            b.frame();
            while Instant::now() >= next {
                for g in [&mut *a, &mut *b] {
                    let fired = g.tick(Input::default());
                    g.send_updates(fired);
                }
                next += Duration::from_millis(10);
            }
            if done(a, b) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    };
    assert!(pump(&mut a, &mut b, 15, Box::new(|a, b| a.world().is_some() && b.world().is_some())), "join");
    a.request_ship(0);
    b.request_ship(0);
    let (a_pid, b_pid) = (a.local_pid(), b.local_pid());
    assert!(pump(&mut a, &mut b, 10, Box::new(|a, b| a.alive() && b.alive())), "ships");
    // Opposite teams so shots do damage.
    a.session.as_mut().unwrap().request_freq(11);
    b.session.as_mut().unwrap().request_freq(22);
    assert!(
        pump(&mut a, &mut b, 10, Box::new(move |a, b| {
            let freq = |g: &Game, pid| g.session.as_ref().unwrap().players.get(&pid).map(|p| p.freq);
            freq(a, a_pid) == Some(11) && freq(b, b_pid) == Some(22) && a.alive() && b.alive()
        })),
        "freq change\nA: {:#?}",
        a.session.as_ref().unwrap().log
    );

    // Put them in open space outside any safe zone, B 8 tiles left of A facing right.
    let (map, _) = a.world().unwrap();
    let spot = (0..1024 * 1024)
        .map(|i| Vec2::new((i % 1024) as f32 + 0.5, (i / 1024) as f32 + 0.5))
        .find(|p| {
            p.x > 30.0 && (-12..=4).all(|dx| (-3..=3).all(|dy| {
                let (x, y) = (p.x as i32 + dx, p.y as i32 + dy);
                !map.is_solid(x, y) && map.tile(x, y) != TILE_SAFE
            }))
        })
        .expect("open space");
    a.ship.as_mut().unwrap().position = spot;
    let gunner = b.ship.as_mut().unwrap();
    gunner.position = Vec2::new(spot.x - 8.0, spot.y);
    gunner.orientation = 0.25; // facing right
    // Let each side see the other's new position before shooting.
    pump(&mut a, &mut b, 2, Box::new(|_, _| false));

    let end = Instant::now() + Duration::from_secs(20);
    let mut next = Instant::now();
    while Instant::now() < end && b.kills == 0 {
        a.frame();
        b.frame();
        while Instant::now() >= next {
            let fired = a.tick(Input::default());
            a.send_updates(fired);
            let fired = b.tick(Input { gun: true, ..Default::default() });
            b.send_updates(fired);
            // Keep the gunner aimed and in place.
            if let Some(s) = b.ship.as_mut() {
                s.position = Vec2::new(spot.x - 8.0, spot.y);
                s.velocity = Vec2::default();
                s.orientation = 0.25;
            }
            next += Duration::from_millis(10);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    println!("A deaths {} B kills {}; A log tail {:?}", a.deaths, b.kills, a.session.as_ref().unwrap().log.iter().rev().take(5).collect::<Vec<_>>());
    assert_eq!(a.deaths, 1, "target died on its own client");
    assert_eq!(b.kills, 1, "server reported the kill to the gunner");
    for g in [&mut a, &mut b] {
        g.session.as_mut().unwrap().disconnect();
    }
}
