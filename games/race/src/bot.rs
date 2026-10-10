//! Server-side bots: they run the course one cell at a time down `Map::to_finish`, jumping at ledges and over laser
//! bars that are about to reach them, and steering in the air to land. They produce an `Input` per tick like a client
//! would.

use crate::map::MAP;
use crate::shared::*;
use glam::Vec3;

/// Turn speed, radians per second.
const TURN_SPEED: f32 = 8.0;
/// Chance of not reacting to a laser bar in time, per bar. The main difficulty knob.
const LASER_MISS: f32 = 0.15;
/// How far off a cell's center a bot aims, at most, so they don't all run the same line.
const WANDER: f32 = 0.5;

pub struct Bot {
    seq: u32,
    rng: u32,
    yaw: f32,
    target: Option<(usize, usize)>,
    offset: Vec3,
    /// Decided whether to jump the laser bar coming up (true) or not.
    laser: Option<bool>,
    last_pos: Vec3,
    stuck_ticks: u32,
}

impl Bot {
    pub fn new(seed: u32) -> Bot {
        let rng = seed.wrapping_mul(0x9E37_79B9) | 1;
        Bot { seq: 0, rng, yaw: 0.0, target: None, offset: Vec3::ZERO, laser: None, last_pos: Vec3::ZERO, stuck_ticks: 0 }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    pub fn think(&mut self, me: &PlayerState, time: f32) -> Input {
        self.seq += 1;
        let mut inp = Input { seq: self.seq, yaw: self.yaw, ..Default::default() };
        if me.frozen {
            (self.target, self.yaw) = (None, me.yaw);
            return Input { yaw: me.yaw, ..inp };
        }
        self.stuck_ticks = if me.pos.distance(self.last_pos) < 0.01 { self.stuck_ticks + 1 } else { 0 };
        self.last_pos = me.pos;

        let here = MAP.cell(me.pos);
        let w = MAP.width();
        if me.on_ground && MAP.is_floor(here.0, here.1) {
            let dist = |&(x, z): &(usize, usize)| MAP.to_finish[z * w + x];
            let next = MAP.moves(here.0, here.1).into_iter().filter(|c| dist(c) < dist(&here)).min_by_key(dist);
            if next != self.target {
                let (a, r) = (self.random() * std::f32::consts::TAU, self.random() * WANDER);
                self.offset = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            }
            self.target = next;
        }
        let Some(target) = self.target else { return inp };
        let to = (MAP.ground(target.0, target.1) + self.offset - me.pos).with_y(0.0);
        let dir = to.normalize_or_zero();

        if me.on_ground && dir != Vec3::ZERO {
            let turn = TURN_SPEED * DT;
            self.yaw += wrap(f32::atan2(-dir.x, -dir.z) - self.yaw).clamp(-turn, turn);
        }
        let mut push = dir;
        if me.on_ground && MAP.is_jump(here, target) {
            let adjacent = here.0.abs_diff(target.0) <= 1 && here.1.abs_diff(target.1) <= 1;
            // Up onto a block: jump now. Over a gap: at the edge, running straight at the target, or else back off
            // and take the run again.
            let ahead = MAP.cell(me.pos + dir * (RADIUS + 0.3));
            let edge = !MAP.is_floor(ahead.0, ahead.1) || MAP.height(ahead.0, ahead.1) < MAP.height(here.0, here.1) - 0.25;
            let speed = me.vel.with_y(0.0).length();
            let lined_up = speed > 5.0 && me.vel.dot(dir) > 0.97 * speed;
            inp.jump = adjacent || (edge && lined_up);
            if edge && !lined_up && !adjacent {
                push = (MAP.ground(here.0, here.1) - me.pos).with_y(0.0).normalize_or_zero();
            }
        } else if !me.on_ground && to.length() < 0.3 {
            // Over the target: stop pushing so we don't sail past.
            push = Vec3::ZERO;
        }
        // Run (or steer in the air) whichever way we face.
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        (inp.yaw, inp.forward, inp.right) = (self.yaw, push.dot(fwd), push.dot(right));
        let bar_coming = [0.1, 0.18, 0.26].iter().any(|&k| lasered(me.pos + me.vel * k, time + k));
        if bar_coming && self.laser.is_none() {
            self.laser = Some(self.random() > LASER_MISS);
        } else if !bar_coming && me.on_ground {
            self.laser = None;
        }
        inp.jump |= me.on_ground && bar_coming && self.laser == Some(true);
        inp.jump |= self.stuck_ticks > TICK_HZ as u32 / 2;
        inp
    }
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
