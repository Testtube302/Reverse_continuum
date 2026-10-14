//! End-to-end tests against a running zone server (ASSS / Subspace Server .NET).
//! Skipped unless AICONTINUUM_SERVER is set, e.g. AICONTINUUM_SERVER=127.0.0.1:5000.

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

fn pump_until(sessions: &mut [&mut Session], secs: u64, mut done: impl FnMut(&[&mut Session]) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        for s in sessions.iter_mut() {
            s.update();
            s.send_position(None, false);
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
    let mut s = Session::connect(config("AIC-Test1")).unwrap();
    let ok = pump_until(&mut [&mut s], 15, |s| s[0].ready());
    assert!(ok, "did not become ready; log: {:#?}", s.log);

    let settings = s.settings.as_ref().unwrap();
    assert!(settings.ships.iter().all(|sh| sh.initial_speed > 0), "ship settings parsed");
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
    let mut s = Session::connect(config("AIC-Sec")).unwrap();
    let ok = pump_until(&mut [&mut s], 75, |s| s[0].ready() && s[0].security_replies > 0);
    assert!(ok, "no security exchange; log: {:#?}", s.log);
    s.disconnect();
}

#[test]
fn two_clients_see_each_other_fly() {
    if server().is_none() {
        return;
    }
    let mut a = Session::connect(config("AIC-Pilot")).unwrap();
    let mut b = Session::connect(config("AIC-Watcher")).unwrap();
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
    b.send_position(None, true);
    let mut ship = Ship::new(0, start, &settings);
    let input = Input { forward: true, ..Default::default() };
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        ship.tick(&map, &settings, input);
        a.update();
        a.send_position(Some(&ship), false);
        b.update();
        b.tick_players();
        std::thread::sleep(Duration::from_millis(10));
    }
    a.send_position(Some(&ship), true);
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
