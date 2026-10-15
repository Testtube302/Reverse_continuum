//! Powerballs: positions from the zone (S2C 0x2E), local simulation with friction and wall
//! bounces, carrying, and goal sides. Rules follow the MIT-licensed nullspace client.

use crate::lvl::Map;
use crate::sim::Vec2;

/// Sentinel for "no player" in ball packets.
pub const NO_PLAYER: u16 = 0xFFFF;

/// One S2C 0x2E ball packet: id, pixel position, velocity (pixels per 10 s), owner, timestamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallPacket {
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub vel_x: i16,
    pub vel_y: i16,
    pub owner: u16,
    /// Server tick; 0 with a valid owner means "carried".
    pub timestamp: u32,
}

impl BallPacket {
    pub fn parse(p: &[u8]) -> Option<Self> {
        let u16_at = |o: usize| p.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let ts = p.get(12..16).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & 0x7FFF_FFFF)?;
        Some(Self {
            id: *p.get(1)?,
            x: u16_at(2)?,
            y: u16_at(4)?,
            vel_x: u16_at(6)? as i16,
            vel_y: u16_at(8)? as i16,
            owner: u16_at(10)?,
            timestamp: ts,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallState {
    World,
    Carried,
    Goal,
}

#[derive(Clone, Copy, Debug)]
pub struct Ball {
    pub id: u8,
    /// Position in thousandths of a pixel (pixels * 1000), velocity in pixels per 10 s.
    pub x: i64,
    pub y: i64,
    pub vel_x: i32,
    pub vel_y: i32,
    pub carrier: u16,
    pub state: BallState,
    pub timestamp: u32,
    friction: i64,
    friction_delta: i64,
    /// Server ticks of the last throw and of the last time someone was close enough to touch it.
    pub last_fire: u32,
    pub last_touch: u32,
    trail_delay: i32,
}

impl Ball {
    /// Position in tiles.
    pub fn position(&self) -> Vec2 {
        Vec2::new(self.x as f32 / 16000.0, self.y as f32 / 16000.0)
    }
}

/// What a tick of simulation produced for the frontend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BallEvent {
    /// Drop a trail puff (powerb.png row 2) here.
    Trail(Vec2),
    /// The ball we last threw entered an enemy goal: send C2S 0x21.
    Goal { id: u8, x: i16, y: i16 },
}

#[derive(Default)]
pub struct Soccer {
    pub balls: Vec<Ball>,
}

impl Soccer {
    pub fn get(&self, id: u8) -> Option<&Ball> {
        self.balls.iter().find(|b| b.id == id)
    }

    fn get_mut(&mut self, id: u8) -> Option<&mut Ball> {
        self.balls.iter_mut().find(|b| b.id == id)
    }

    /// The ball we carry, if any.
    pub fn carried_by(&self, pid: u16) -> Option<&Ball> {
        self.balls.iter().find(|b| b.state == BallState::Carried && b.carrier == pid)
    }

    /// Applies a 0x2E packet. `friction_of` gives SoccerBallFriction for the owner's ship, `map`
    /// is used to catch the ball up to now (`server_now`).
    pub fn apply(&mut self, p: BallPacket, server_now: u32, friction_of: impl Fn(u16) -> i64, map: &Map, me: u16) {
        if self.get(p.id).is_none() {
            self.balls.push(Ball {
                id: p.id,
                x: 0,
                y: 0,
                vel_x: 0,
                vel_y: 0,
                carrier: NO_PLAYER,
                state: BallState::World,
                timestamp: 0,
                friction: 0,
                friction_delta: 0,
                last_fire: server_now,
                last_touch: server_now,
                trail_delay: 0,
            });
            // A brand-new ball takes any packet.
            self.get_mut(p.id).unwrap().timestamp = p.timestamp.wrapping_sub(1);
        }
        let ball = self.get_mut(p.id).unwrap();
        let newer = (p.timestamp.wrapping_sub(ball.timestamp) as i32) > 0
            || ball.state == BallState::Goal
            || (ball.state == BallState::Carried && p.timestamp != 0);
        if newer && p.timestamp != 0 {
            ball.x = p.x as i64 * 1000;
            ball.y = p.y as i64 * 1000;
            ball.vel_x = p.vel_x as i32;
            ball.vel_y = p.vel_y as i32;
            ball.state = BallState::World;
            ball.last_fire = p.timestamp;
            if p.owner == me {
                ball.last_touch = server_now.wrapping_sub(100);
            }
            if ball.vel_x != 0 || ball.vel_y != 0 {
                ball.friction_delta = friction_of(p.owner);
                ball.friction = 1_000_000;
            } else {
                ball.friction = 0;
            }
            ball.carrier = p.owner;
            let catch_up = (server_now.wrapping_sub(p.timestamp) as i32).clamp(0, 6000);
            for _ in 0..catch_up {
                simulate(ball, map, None);
            }
            ball.timestamp = p.timestamp;
        } else if p.timestamp == 0 {
            if p.owner != NO_PLAYER {
                // Carried.
                ball.timestamp = 0;
                ball.carrier = p.owner;
                ball.vel_x = 0;
                ball.vel_y = 0;
                ball.state = BallState::Carried;
            } else {
                // Gone.
                self.balls.retain(|b| b.id != p.id);
            }
        }
    }

    /// One 10 ms tick for every ball in play. `me` is our pid: a ball we threw that reaches an
    /// enemy goal tile reports a goal.
    pub fn tick(&mut self, map: &Map, me: u16, is_team_goal: impl Fn(Vec2) -> bool) -> Vec<BallEvent> {
        let mut events = Vec::new();
        for ball in &mut self.balls {
            if ball.state == BallState::Carried {
                continue;
            }
            simulate(ball, map, Some(&mut events));
            if ball.state != BallState::Goal && ball.carrier == me {
                let at = ball.position();
                if map.tile(at.x as i32, at.y as i32) == crate::lvl::TILE_GOAL && !is_team_goal(at) {
                    events.push(BallEvent::Goal { id: ball.id, x: at.x as i16, y: at.y as i16 });
                    ball.state = BallState::Goal;
                }
            }
        }
        events
    }
}

fn simulate(ball: &mut Ball, map: &Map, events: Option<&mut Vec<BallEvent>>) {
    if ball.friction <= 0 {
        return;
    }
    let axis = |pos: &mut i64, vel: &mut i32, other: i64, x_axis: bool| {
        let before = *pos;
        *pos += *vel as i64;
        let (tx, ty) = if x_axis { (*pos / 16000, other / 16000) } else { (other / 16000, *pos / 16000) };
        if map.is_solid(tx as i32, ty as i32) {
            *pos = before;
            *vel = -*vel;
        }
    };
    let y = ball.y;
    axis(&mut ball.x, &mut ball.vel_x, y, true);
    let x = ball.x;
    axis(&mut ball.y, &mut ball.vel_y, x, false);
    if let Some(events) = events {
        if ball.vel_x != 0 || ball.vel_y != 0 {
            ball.trail_delay -= 1;
            if ball.trail_delay <= 0 {
                events.push(BallEvent::Trail(ball.position()));
                ball.trail_delay = 5;
            }
        }
    }
    let f = ball.friction / 1000;
    ball.vel_x = (ball.vel_x as i64 * f / 1000) as i32;
    ball.vel_y = (ball.vel_y as i64 * f / 1000) as i32;
    ball.friction -= ball.friction_delta;
}

/// Whether `at` is on our own team's side for goal purposes (SoccerMode 0-6, reference client).
pub fn is_team_goal(at: Vec2, freq: u16, mode: u32) -> bool {
    let (x, y) = (at.x, at.y);
    let corner = |f: u16| match f % 4 {
        0 => x < 512.0 && y < 512.0,
        1 => x >= 512.0 && y < 512.0,
        2 => x < 512.0 && y >= 512.0,
        _ => x >= 512.0 && y >= 512.0,
    };
    let side5 = |f: u16| match f % 4 {
        0 => if y < 512.0 { x < y } else { x + y < 1024.0 },
        1 => if x < 512.0 { x + y >= 1024.0 } else { x < y },
        2 => if x < 512.0 { x >= y } else { x + y < 1024.0 },
        _ => if y <= 512.0 { x + y >= 1024.0 } else { x >= y },
    };
    match mode {
        0 => false,
        1 => if freq & 1 != 0 { x >= 512.0 } else { x < 512.0 },
        2 => if freq & 1 != 0 { y >= 512.0 } else { y < 512.0 },
        3 => corner(freq),
        4 => !corner(freq),
        5 => side5(freq),
        6 => !side5(freq),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(ts: u32, owner: u16, vx: i16) -> BallPacket {
        BallPacket { id: 0, x: 512 * 16, y: 512 * 16, vel_x: vx, vel_y: 0, owner, timestamp: ts }
    }

    #[test]
    fn thrown_ball_slows_down_and_carried_ball_follows_packets() {
        let map = Map::empty();
        let mut s = Soccer::default();
        s.apply(packet(1000, 3, 3000), 1000, |_| 1000, &map, 9);
        let b = s.get(0).unwrap();
        assert_eq!((b.state, b.carrier), (BallState::World, 3));
        let start = b.x;
        for _ in 0..100 {
            s.tick(&map, 9, |_| false);
        }
        let b = s.get(0).unwrap();
        assert!(b.x > start && b.vel_x < 3000, "moved and slowed: x {} v {}", b.x - start, b.vel_x);
        // Carried: timestamp 0 with an owner.
        s.apply(packet(0, 4, 0), 1200, |_| 1000, &map, 9);
        assert_eq!(s.carried_by(4).map(|b| b.id), Some(0));
        // Gone: timestamp 0 without an owner.
        s.apply(packet(0, NO_PLAYER, 0), 1300, |_| 1000, &map, 9);
        assert!(s.get(0).is_none());
    }

    #[test]
    fn parses_a_16_byte_ball_packet() {
        let mut p = vec![0x2E, 2];
        for v in [8162u16, 8191, 100, (-50i16) as u16, 7] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&1234u32.to_le_bytes());
        let b = BallPacket::parse(&p).unwrap();
        assert_eq!((b.id, b.x, b.y, b.vel_x, b.vel_y, b.owner, b.timestamp), (2, 8162, 8191, 100, -50, 7, 1234));
    }

    #[test]
    fn goal_sides_by_mode() {
        let left = Vec2::new(100.0, 500.0);
        assert!(is_team_goal(left, 0, 1) && !is_team_goal(left, 1, 1));
        assert!(!is_team_goal(left, 0, 0));
    }
}
