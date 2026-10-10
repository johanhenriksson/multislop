//! What client and server both run: the ball, the windmills, the wire format. A shot and everything after it is
//! deterministic, so the client predicts its own ball exactly from the same inputs the server gets.

use crate::map::{WINDMILL_HEIGHT, WINDMILL_REACH, course};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub use sdk::client::look_rot;
pub use sdk::{Entity, TICK_HZ, decode, encode};
pub const DT: f32 = 1.0 / TICK_HZ as f32;

pub const BALL_RADIUS: f32 = 0.15;
const GRAVITY: f32 = 9.8;
/// Rolling friction, m/s²: a full-power shot on the flat rolls about 50 m.
const ROLL_FRICTION: f32 = 1.4;
/// Speed kept off the floor and off rails, blocks and windmills.
const FLOOR_BOUNCE: f32 = 0.35;
const WALL_BOUNCE: f32 = 0.7;
/// Slower than this into a surface, the ball stays on it instead of bouncing.
const SETTLE: f32 = 0.8;
/// A full-power shot, and the fastest the ball goes.
pub const MAX_SHOT: f32 = 12.0;
const MAX_SPEED: f32 = 20.0;
/// Longest move per substep: well under the ball's radius and the thinnest rail.
const SUBSTEP: f32 = 0.05;
pub const CUP_RADIUS: f32 = 0.2;
/// Over the cup slower than this, the ball drops in; faster, it rolls on over.
const CUP_SPEED: f32 = 2.0;
/// Under `REST_SPEED` for `REST_TICKS`, the ball is at rest and can be hit again.
const REST_SPEED: f32 = 0.05;
const REST_TICKS: u8 = (TICK_HZ / 4.0) as u8;
/// Out of bounds (the void or water), the ball is seen falling or sinking for this long before it's put back.
const OUT_TICKS: u8 = (TICK_HZ * 0.6) as u8;
/// Strokes on a hole before you're picked up, which is also the most a hole scores.
pub const MAX_STROKES: u8 = 10;
/// Windmill bars: radians per second (neighbors spin opposite ways) and half their thickness.
const WINDMILL_SPIN: f32 = 1.6;
const BAR_HALF: f32 = 0.06;

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct Input {
    /// Stamped by the host, not sent as part of the input.
    #[serde(skip)]
    pub seq: u32,
    /// The camera's: aim is along its yaw.
    pub yaw: f32,
    pub pitch: f32,
    /// A shot this tick, at this share of full power; 0 for none.
    pub shot: f32,
}

/// One player's ball. Position and velocity travel in the `sdk::Entity`, so they're skipped here.
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Ball {
    #[serde(skip)]
    pub pos: Vec3,
    #[serde(skip)]
    pub vel: Vec3,
    #[serde(skip)]
    pub yaw: f32,
    /// Ticks slow enough to be resting.
    pub slow: u8,
    /// Where the last shot was taken from, to come back to after going out.
    pub rest: Vec3,
    pub strokes: u8,
    pub holed: bool,
    /// Ticks left falling or sinking before being put back, 0 when in play.
    pub out: u8,
    /// Before the hole starts and after it ends: no shots. Part of the state so prediction agrees with the server.
    pub frozen: bool,
    /// Ticks since the hole started; the windmills run on it.
    pub clock: u32,
}

impl Ball {
    pub fn on_tee(tee: Vec3, frozen: bool) -> Ball {
        let pos = tee + Vec3::Y * BALL_RADIUS;
        Ball { pos, rest: pos, slow: REST_TICKS, frozen, ..Default::default() }
    }

    pub fn at_rest(&self) -> bool {
        self.slow >= REST_TICKS && self.out == 0 && !self.holed
    }

    pub fn can_shoot(&self) -> bool {
        self.at_rest() && !self.frozen && self.strokes < MAX_STROKES
    }

    pub fn windmill_time(&self) -> f32 {
        self.clock as f32 / TICK_HZ as f32
    }
}

/// A player as clients see it, carried in an `sdk::Entity`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    /// The entity id, filled in by `NetPlayer::from_entity`.
    #[serde(skip)]
    pub id: u32,
    pub ball: Ball,
    /// Strokes per hole this course, 0 for not played yet.
    pub card: Vec<u8>,
    /// Holed or picked up: done with this hole.
    pub done: bool,
    pub bot: bool,
    pub name: String,
    /// sRGB, from their `sdk::Profile`.
    pub color: [u8; 3],
    /// Round trip as the client measures it, for the scorecard. 0 for bots.
    pub ping: u16,
}

impl NetPlayer {
    pub fn entity(&self, teleport: u8) -> Entity {
        let b = &self.ball;
        Entity { id: self.id, pos: b.pos, vel: b.vel, yaw: b.yaw, pitch: 0.0, teleport, data: encode(self) }
    }

    pub fn from_entity(e: &Entity) -> Option<NetPlayer> {
        let mut p: NetPlayer = decode(&e.data)?;
        p.id = e.id;
        (p.ball.pos, p.ball.vel, p.ball.yaw) = (e.pos, e.vel, e.yaw);
        Some(p)
    }

    pub fn total(&self) -> u32 {
        self.card.iter().map(|&s| s as u32).sum()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// On the tee, frozen, while the camera shows the hole.
    Tee,
    Play,
    /// Everyone is done (or out of time): the scorecard, then the next hole.
    Card,
    /// After the last hole: the final card, then a new course.
    Final,
}

/// The course as clients see it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Round {
    pub phase: Phase,
    pub hole: u8,
    /// Seconds left in this phase.
    pub time: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Event {
    Holed { id: u32, strokes: u8 },
    PickedUp { id: u32 },
    /// Into the void or the water at `at`.
    Out { id: u32, at: Vec3, water: bool },
    Go,
}

/// What happened to a ball in one tick, for sounds and the server's bookkeeping.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Step {
    /// Hit at this share of full power.
    pub shot: Option<f32>,
    /// Hardest knock off a rail, block or windmill, m/s.
    pub knock: f32,
    pub holed: bool,
    /// Went out this tick: into water (true) or the void.
    pub out: Option<bool>,
    /// Put back where the last shot was taken from.
    pub back: bool,
}

/// Client-side prediction of our own ball: one input on top of `me`.
pub fn predict(me: &mut Entity, input: &[u8], seq: u32) -> Step {
    let (Some(mut p), Some(inp)) = (NetPlayer::from_entity(me), decode::<Input>(input)) else { return Step::default() };
    let step = step_ball(&mut p.ball, &Input { seq, ..inp });
    *me = p.entity(me.teleport);
    step
}

/// Which way aim at `yaw` sends the ball.
pub fn aim(yaw: f32) -> Vec3 {
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}

/// Advances a ball one tick: a shot if there is one and it's allowed, then the ball rolls, bounces, drops into the
/// cup or goes out.
pub fn step_ball(b: &mut Ball, inp: &Input) -> Step {
    b.clock += 1;
    b.yaw = inp.yaw;
    let mut step = Step::default();
    if b.holed {
        return step;
    }
    let course = course();
    let hole = course.hole_at(b.pos);
    if b.out > 0 {
        b.out -= 1;
        if b.out == 0 {
            (b.pos, b.vel, b.slow, step.back) = (b.rest, Vec3::ZERO, REST_TICKS, true);
            b.strokes = (b.strokes + 1).min(MAX_STROKES);
        } else if let Some(surface) = hole.water(b.pos) {
            b.vel = Vec3::Y * -0.4;
            b.pos.y = (b.pos.y + b.vel.y * DT).max(surface - 0.4);
        } else {
            b.vel.y -= GRAVITY * DT;
            b.pos += b.vel * DT;
        }
        return step;
    }
    if inp.shot > 0.0 && b.can_shoot() {
        let power = inp.shot.min(1.0);
        (b.vel, b.rest, b.slow) = (aim(inp.yaw) * power * MAX_SHOT, b.pos, 0);
        b.strokes += 1;
        step.shot = Some(power);
    }

    b.vel.y -= GRAVITY * DT;
    b.vel = b.vel.clamp_length_max(MAX_SPEED);
    let steps = (b.vel.length() * DT / SUBSTEP).ceil().max(1.0);
    let mut grounded = None;
    for _ in 0..steps as usize {
        b.pos += b.vel * DT / steps;
        if let Some((h, n)) = hole.floor(b.pos) {
            // Resting on a slope puts the center further up than the radius.
            let rest = h + BALL_RADIUS / n.y;
            if b.pos.y < rest + 1e-3 {
                b.pos.y = b.pos.y.max(rest);
                let into = b.vel.dot(n);
                if into < 0.0 {
                    let keep = if -into < SETTLE { 0.0 } else { FLOOR_BOUNCE };
                    b.vel -= n * into * (1.0 + keep);
                }
                grounded = Some(n);
            }
        }
        for &(c, h) in &hole.boxes {
            let closest = b.pos.clamp(c - h, c + h);
            step.knock = step.knock.max(bounce(&mut b.pos, &mut b.vel, closest, Vec3::ZERO));
        }
        for (i, &pivot) in hole.windmills.iter().enumerate() {
            if b.pos.y - BALL_RADIUS > pivot.y + WINDMILL_HEIGHT || b.pos.y + BALL_RADIUS < pivot.y {
                continue;
            }
            let (dir, spin) = windmill(i, b.windmill_time());
            let rel = (b.pos - pivot).with_y(0.0);
            let along = rel.dot(dir).clamp(-WINDMILL_REACH, WINDMILL_REACH);
            let axis = pivot.with_y(b.pos.y) + dir * along;
            let off = b.pos - axis;
            // The bar moves under the ball as it turns.
            let bar = Vec3::new(-dir.z, 0.0, dir.x) * along * spin;
            let closest = axis + off.normalize_or_zero() * off.length().min(BAR_HALF);
            step.knock = step.knock.max(bounce(&mut b.pos, &mut b.vel, closest, bar));
        }
    }

    let speed = b.vel.length();
    if let Some(n) = grounded {
        if speed > 0.0 {
            b.vel *= (speed - ROLL_FRICTION * DT).max(0.0) / speed;
        }
        // Slow on a slope too gentle to roll it: friction holds it.
        let pull = GRAVITY * (1.0 - n.y * n.y).max(0.0).sqrt();
        if b.vel.length() < REST_SPEED && pull < ROLL_FRICTION {
            b.vel = Vec3::ZERO;
        }
    }
    b.slow = if grounded.is_some() && b.vel.length() < REST_SPEED { b.slow.saturating_add(1) } else { 0 };

    let flat = Vec2::new(b.pos.x - hole.cup.x, b.pos.z - hole.cup.z).length();
    if flat < CUP_RADIUS && b.vel.length() < CUP_SPEED && b.pos.y < hole.cup.y + BALL_RADIUS + 0.05 {
        (b.holed, b.pos, b.vel, step.holed) = (true, hole.cup + Vec3::Y * (BALL_RADIUS - 0.12), Vec3::ZERO, true);
        return step;
    }
    let water = hole.water(b.pos).is_some_and(|surface| b.pos.y < surface);
    if water || b.pos.y < hole.low || !b.pos.is_finite() {
        if !b.pos.is_finite() {
            (b.pos, b.vel) = (b.rest, Vec3::ZERO);
        }
        b.out = OUT_TICKS;
        step.out = Some(water);
    }
    step
}

/// Pushes the ball out of a surface whose nearest point to its center is `closest`, moving at `surface`, bouncing
/// it if it was moving into it. Returns how hard it hit.
fn bounce(pos: &mut Vec3, vel: &mut Vec3, closest: Vec3, surface: Vec3) -> f32 {
    let d = *pos - closest;
    let dist = d.length();
    if dist >= BALL_RADIUS || dist < 1e-5 {
        return 0.0;
    }
    let n = d / dist;
    *pos += n * (BALL_RADIUS - dist);
    let into = (*vel - surface).dot(n);
    if into >= 0.0 {
        return 0.0;
    }
    let keep = if -into < SETTLE { 0.0 } else { WALL_BOUNCE };
    *vel -= n * into * (1.0 + keep);
    -into
}

/// Which way windmill `i`'s bar points `time` seconds into the hole, and how fast it turns (radians per second).
pub fn windmill(i: usize, time: f32) -> (Vec3, f32) {
    let spin = if i.is_multiple_of(2) { WINDMILL_SPIN } else { -WINDMILL_SPIN };
    let a = i as f32 * 1.7 + time * spin;
    (Vec3::new(a.cos(), 0.0, a.sin()), spin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{self, HOLES};

    /// Runs a ball until it rests, drops or goes out, at most `secs`.
    fn settle(b: &mut Ball, secs: f32) -> Step {
        let mut last = Step::default();
        for _ in 0..(secs * TICK_HZ as f32) as usize {
            let s = step_ball(b, &Input::default());
            if s.holed || s.out.is_some() {
                return s;
            }
            last.knock = last.knock.max(s.knock);
            if b.at_rest() {
                break;
            }
        }
        last
    }

    #[test]
    fn a_ball_on_the_tee_stays_put() {
        map::load("random-3");
        for h in &course().holes {
            let mut b = Ball::on_tee(h.tee, false);
            for _ in 0..TICK_HZ as usize {
                step_ball(&mut b, &Input::default());
            }
            assert!(b.at_rest() && b.pos.distance(h.tee + Vec3::Y * BALL_RADIUS) < 0.01, "{:?} vs {:?}", b.pos, h.tee);
        }
    }

    #[test]
    fn a_shot_rolls_out_and_stops_and_never_leaves_the_rails() {
        map::load("random-11");
        for h in &course().holes {
            for k in 0..16 {
                let mut b = Ball::on_tee(h.tee, false);
                let yaw = k as f32 * 0.4;
                let s = step_ball(&mut b, &Input { yaw, shot: 0.3 + (k % 4) as f32 * 0.23, ..Default::default() });
                assert!(s.shot.is_some() && b.strokes == 1);
                let end = settle(&mut b, 60.0);
                if end.out.is_none() && !end.holed {
                    assert!(b.at_rest(), "still rolling after a minute: {:?}", b);
                    assert!(course().hole_at(b.pos).kind_at(b.pos).inside(), "rests outside: {:?}", b.pos);
                }
            }
        }
    }

    #[test]
    fn a_slow_ball_over_the_cup_drops_and_a_fast_one_rolls_over() {
        map::load("random-2");
        let h = &course().holes[0];
        let mut b = Ball::on_tee(h.cup, false);
        b.pos += Vec3::X * 0.5;
        b.vel = Vec3::X * -1.2;
        b.slow = 0;
        assert!(settle(&mut b, 5.0).holed && b.holed);

        let mut b = Ball::on_tee(h.cup, false);
        b.pos += Vec3::X * 0.5;
        b.vel = Vec3::X * -6.0;
        b.slow = 0;
        for _ in 0..30 {
            step_ball(&mut b, &Input::default());
        }
        assert!(!b.holed, "too fast to drop");
    }

    #[test]
    fn going_out_costs_a_stroke_and_puts_the_ball_back() {
        map::load("random-4");
        let h = &course().holes[1];
        let mut b = Ball::on_tee(h.tee, false);
        b.pos = h.world(-5.0, -5.0, 0.0);
        b.slow = 0;
        let mut back = false;
        let mut went = false;
        for _ in 0..(TICK_HZ as usize * 3) {
            let s = step_ball(&mut b, &Input::default());
            went |= s.out == Some(false);
            back |= s.back;
        }
        assert!(went && back);
        assert_eq!((b.pos, b.strokes), (h.tee + Vec3::Y * BALL_RADIUS, 1));
    }

    #[test]
    fn rails_bounce_the_ball_back() {
        map::load("random-6");
        let h = &course().holes[0];
        let mut b = Ball::on_tee(h.tee, false);
        // The tee faces north with rails either side: straight east hits one.
        step_ball(&mut b, &Input { yaw: -std::f32::consts::FRAC_PI_2, shot: 0.4, ..Default::default() });
        let end = settle(&mut b, 20.0);
        assert!(end.knock > 1.0, "{end:?}");
        assert!(b.pos.x < h.tee.x + 1.6, "stayed inside: {:?}", b.pos);
    }

    #[test]
    fn no_shots_while_rolling_frozen_or_picked_up() {
        map::load("random-1");
        let h = &course().holes[HOLES - 1];
        let shot = Input { shot: 0.5, ..Default::default() };
        let mut b = Ball::on_tee(h.tee, true);
        assert!(step_ball(&mut b, &shot).shot.is_none(), "frozen");
        b.frozen = false;
        assert!(step_ball(&mut b, &shot).shot.is_some());
        assert!(step_ball(&mut b, &shot).shot.is_none(), "still rolling");
        let mut b = Ball { strokes: MAX_STROKES, ..Ball::on_tee(h.tee, false) };
        assert!(step_ball(&mut b, &shot).shot.is_none(), "picked up");
    }
}
