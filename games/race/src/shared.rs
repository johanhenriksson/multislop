//! What client and server both run: movement (Quake style, floaty and fast), the laser sweepers, the wire format.

use crate::map::{WALL_TOP, course};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub use sdk::client::look_rot;
pub use sdk::{Entity, TICK_HZ, decode, encode};
pub const DT: f32 = 1.0 / TICK_HZ as f32;

pub const RADIUS: f32 = 0.4;
pub const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.6;

pub const MAX_SPEED: f32 = 9.0;
const ACCEL: f32 = 12.0;
/// In the air you only add speed up to `AIR_CAP` along where you steer, so you can correct a jump but not turn it.
const AIR_ACCEL: f32 = 12.0;
const AIR_CAP: f32 = 0.9;
const FRICTION: f32 = 5.0;
const STOP_SPEED: f32 = 2.5;
/// About 60% of earth gravity: a jump goes 1.75 m up and hangs for over a second.
pub const GRAVITY: f32 = 12.0;
const JUMP_SPEED: f32 = 6.5;
/// Jump pads throw you about 7 m up, keeping up to this much of your run.
pub const PAD_SPEED: f32 = 13.0;
const PAD_CARRY: f32 = 6.5;
/// Highest rise walked up without jumping (a stair), and how far down you stay glued to the floor.
const STEP: f32 = 0.55;
/// Feet this low are in the lava.
pub const LAVA: f32 = 0.05;

/// Laser bars: each spins about its pivot this far out either side, at this height over the floor.
pub const LASER_REACH: f32 = 6.5;
pub const LASER_HEIGHT: f32 = 0.5;
pub const LASER_RADIUS: f32 = 0.1;
/// Radians per second; neighbors spin opposite ways.
const LASER_SPIN: f32 = 1.3;

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct Input {
    /// Stamped by the host, not sent as part of the input.
    #[serde(skip)]
    pub seq: u32,
    pub forward: f32,
    pub right: f32,
    pub yaw: f32,
    pub pitch: f32,
    /// Held: jumps again on every landing.
    pub jump: bool,
    /// Back to the last checkpoint.
    pub reset: bool,
}

/// Position and view travel in the player's `sdk::Entity`, so they're skipped here.
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct PlayerState {
    /// Feet position.
    #[serde(skip)]
    pub pos: Vec3,
    #[serde(skip)]
    pub vel: Vec3,
    #[serde(skip)]
    pub yaw: f32,
    #[serde(skip)]
    pub pitch: f32,
    pub on_ground: bool,
    /// Waiting for the start, or the race is over: look around, nothing else. Part of the state so prediction agrees
    /// with the server.
    pub frozen: bool,
}

/// A player as clients see it, carried in an `sdk::Entity`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    /// The entity id, filled in by `NetPlayer::from_entity`.
    #[serde(skip)]
    pub id: u32,
    pub state: PlayerState,
    /// Last checkpoint reached this race, 0 for none.
    pub checkpoint: u8,
    /// Place and time once over the line.
    pub finish: Option<(u8, f32)>,
    /// Races won.
    pub wins: u32,
    pub bot: bool,
    pub name: String,
    /// sRGB, from their `sdk::Profile`.
    pub color: [u8; 3],
    /// Round trip as the client measures it, for the scoreboard. 0 for bots.
    pub ping: u16,
}

impl NetPlayer {
    pub fn entity(&self, teleport: u8) -> Entity {
        let s = &self.state;
        Entity { id: self.id, pos: s.pos, vel: s.vel, yaw: s.yaw, pitch: s.pitch, teleport, data: encode(self) }
    }

    pub fn from_entity(e: &Entity) -> Option<NetPlayer> {
        let mut p: NetPlayer = decode(&e.data)?;
        p.id = e.id;
        (p.state.pos, p.state.vel, p.state.yaw, p.state.pitch) = (e.pos, e.vel, e.yaw, e.pitch);
        Some(p)
    }

    /// Sort key for the standings: finishers by place, then whoever is furthest along.
    pub fn standing(&self) -> (u8, std::cmp::Reverse<u8>, i32) {
        match self.finish {
            Some((place, _)) => (place, std::cmp::Reverse(0), 0),
            None => (u8::MAX, std::cmp::Reverse(self.checkpoint), (self.state.pos.z * 10.0) as i32),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// On the start line; the race starts when the clock reaches 0.
    Countdown,
    Racing,
    /// Everyone is in (or out of time); the next race starts after this.
    Results,
}

/// The race as clients see it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Race {
    pub phase: Phase,
    /// Ticks since this race's countdown began; the lasers run on it.
    pub clock: u32,
    /// Seconds since the start while racing; seconds left of the countdown or the results.
    pub time: f32,
    /// Seconds left to finish once someone has.
    pub closing: Option<f32>,
}

impl Race {
    pub fn laser_time(&self) -> f32 {
        self.clock as f32 / TICK_HZ as f32
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Event {
    /// Into the lava, or lasered, and back to their checkpoint from `at`.
    Fell { id: u32, at: Vec3, lasered: bool },
    Checkpoint { id: u32, n: u8 },
    Finish { id: u32, place: u8, secs: f32 },
    Go,
}

/// What happened to a player in one tick, for sounds.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Step {
    pub jumped: bool,
    pub launched: bool,
}

/// Client-side prediction of our own player: one input on top of `me`.
pub fn predict(me: &mut Entity, input: &[u8], seq: u32) -> Step {
    let (Some(mut p), Some(inp)) = (NetPlayer::from_entity(me), decode::<Input>(input)) else { return Step::default() };
    let step = step_player(&mut p.state, &Input { seq, ..inp });
    *me = p.entity(me.teleport);
    step
}

pub fn eye(s: &PlayerState) -> Vec3 {
    s.pos + Vec3::Y * EYE
}

/// Advances one tick of movement.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> Step {
    s.yaw = inp.yaw;
    s.pitch = inp.pitch.clamp(-1.55, 1.55);
    let mut step = Step::default();

    let fwd = Vec3::new(-s.yaw.sin(), 0.0, -s.yaw.cos());
    let right = Vec3::new(s.yaw.cos(), 0.0, -s.yaw.sin());
    let wish = if s.frozen { Vec3::ZERO } else { (fwd * inp.forward + right * inp.right).normalize_or_zero() };

    if s.on_ground && course().on_pad(s.pos) && !s.frozen {
        let carry = s.vel.with_y(0.0).clamp_length_max(PAD_CARRY);
        s.vel = carry + Vec3::Y * PAD_SPEED;
        s.on_ground = false;
        step.launched = true;
    } else if s.on_ground && inp.jump && !s.frozen {
        s.vel.y = JUMP_SPEED;
        s.on_ground = false;
        step.jumped = true;
    }
    if s.on_ground {
        let speed = s.vel.with_y(0.0).length();
        if speed > 0.0 {
            let drop = speed.max(STOP_SPEED) * FRICTION * DT;
            s.vel *= (speed - drop).max(0.0) / speed;
        }
        accelerate(&mut s.vel, wish, MAX_SPEED, ACCEL);
    } else {
        accelerate(&mut s.vel, wish, AIR_CAP, AIR_ACCEL);
    }
    s.vel.y -= GRAVITY * DT;
    move_and_collide(s);
    step
}

/// Quake acceleration: adds speed along `wish` up to `cap`, at a rate set by full run speed.
fn accelerate(vel: &mut Vec3, wish: Vec3, cap: f32, accel: f32) {
    let add = cap - vel.dot(wish);
    if add > 0.0 {
        *vel += wish * (accel * MAX_SPEED * DT).min(add);
    }
}

/// Overlap of the player hull with a box, with a small skin so resting exactly on a face doesn't count.
fn overlaps(p: Vec3, c: Vec3, h: Vec3) -> bool {
    const SKIN: f32 = 1e-3;
    (p.x - c.x).abs() < h.x + RADIUS - SKIN
        && (p.z - c.z).abs() < h.z + RADIUS - SKIN
        && p.y < c.y + h.y - SKIN
        && p.y + HEIGHT > c.y - h.y + SKIN
}

fn blocked(p: Vec3) -> bool {
    course().boxes.iter().any(|&(c, h)| overlaps(p, c, h))
}

/// Highest floor under the hull at or below `pos`: a block top or the lava.
pub fn floor_below(pos: Vec3) -> f32 {
    let under = |&&(c, h): &&(Vec3, Vec3)| {
        (pos.x - c.x).abs() < h.x + RADIUS && (pos.z - c.z).abs() < h.z + RADIUS && c.y + h.y <= pos.y + 1e-3
    };
    course().boxes.iter().filter(under).map(|&(c, h)| c.y + h.y).fold(0.0, f32::max)
}

// ponytail: the hull is an AABB moved axis by axis against the blocks.
fn move_and_collide(s: &mut PlayerState) {
    let was_on_ground = s.on_ground;
    s.on_ground = false;
    for axis in [0, 2, 1] {
        let before = s.pos[axis];
        s.pos[axis] += s.vel[axis] * DT;
        for &(c, h) in &course().boxes {
            let mut prev = s.pos;
            prev[axis] = before;
            // Only resolve overlaps this axis' move caused; a touching contact on another axis must not teleport us.
            if !overlaps(s.pos, c, h) || overlaps(prev, c, h) {
                continue;
            }
            // A stair: walk up onto it.
            let top = c.y + h.y;
            if axis != 1 && was_on_ground && top - s.pos.y <= STEP && !blocked(s.pos.with_y(top)) {
                s.pos.y = top;
                continue;
            }
            let (lo, hi) = if axis == 1 { (c.y - h.y - HEIGHT, top) } else { (c[axis] - h[axis] - RADIUS, c[axis] + h[axis] + RADIUS) };
            if s.pos[axis] < before {
                s.pos[axis] = hi;
                s.on_ground |= axis == 1;
            } else {
                s.pos[axis] = lo;
            }
            s.vel[axis] = 0.0;
        }
    }
    if s.pos.y <= 0.0 {
        s.pos.y = 0.0;
        s.on_ground = true;
    }
    // Walking down a stair or off a ledge no higher than one: stay on the floor rather than hop.
    if !s.on_ground && was_on_ground && s.vel.y <= 0.0 {
        let floor = floor_below(s.pos);
        if s.pos.y - floor < STEP {
            s.pos.y = floor;
            s.on_ground = true;
        }
    }
    if s.on_ground {
        s.vel.y = s.vel.y.max(0.0);
    }
}

/// Which way sweeper `i`'s bar points `time` seconds into the race.
pub fn laser_dir(i: usize, time: f32) -> Vec3 {
    let spin = if i.is_multiple_of(2) { LASER_SPIN } else { -LASER_SPIN };
    let a = i as f32 * 1.7 + time * spin;
    Vec3::new(a.cos(), 0.0, a.sin())
}

/// Whether a player standing at `feet` touches a laser bar.
pub fn lasered(feet: Vec3, time: f32) -> bool {
    course().sweepers.iter().enumerate().any(|(i, &pivot)| {
        let y = pivot.y + LASER_HEIGHT;
        if feet.y > y + LASER_RADIUS || feet.y + HEIGHT < y - LASER_RADIUS {
            return false;
        }
        let d = laser_dir(i, time);
        let rel = (feet - pivot).with_y(0.0);
        rel.distance(d * rel.dot(d).clamp(-LASER_REACH, LASER_REACH)) < RADIUS + LASER_RADIUS
    })
}

/// Inside the course: between the outer walls, above the lava, below the wall tops plus a pad's throw.
pub fn in_bounds(p: Vec3) -> bool {
    let (w, d) = course().size();
    p.is_finite() && p.x.abs() < w / 2.0 && p.z.abs() < d / 2.0 && p.y >= 0.0 && p.y < WALL_TOP + 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut PlayerState, inp: Input, secs: f32) {
        for _ in 0..(secs * TICK_HZ as f32) as usize {
            step_player(s, &inp);
        }
    }

    fn standing(pos: Vec3) -> PlayerState {
        PlayerState { pos, on_ground: true, ..Default::default() }
    }

    #[test]
    fn never_leaves_the_course() {
        for (i, sp) in course().spawns.iter().enumerate() {
            let mut s = PlayerState { pos: *sp, ..Default::default() };
            assert!(!blocked(s.pos), "spawn {i} inside a block");
            let mut x = 12345u32 + i as u32;
            for t in 0..20000 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let inp = Input {
                    forward: (x % 3) as f32 - 1.0,
                    right: ((x >> 3) % 3) as f32 - 1.0,
                    yaw: (x >> 8) as f32 * 1e-6,
                    jump: x & 64 != 0,
                    ..Default::default()
                };
                step_player(&mut s, &inp);
                assert!(in_bounds(s.pos) && !blocked(s.pos), "spawn {i} tick {t}: {:?}", s.pos);
            }
        }
    }

    #[test]
    fn running_off_a_ledge_drops_into_the_lava() {
        // North off the start platform, no jump: the stepping stones are too far to walk to.
        let mut s = standing(course().spawns[0]);
        run(&mut s, Input { forward: 1.0, ..Default::default() }, 2.0);
        assert!(s.pos.y < LAVA, "{:?}", s.pos);
    }

    #[test]
    fn a_running_jump_clears_a_two_cell_gap() {
        // North off the start platform to a stepping stone three cells on, steering in the air to land on it.
        let stone = course().ground(7, course().cell(course().spawns[0]).1 - 4);
        let mut s = standing(stone + Vec3::Z * 6.9);
        run(&mut s, Input { forward: 1.0, ..Default::default() }, 0.1);
        step_player(&mut s, &Input { forward: 1.0, jump: true, ..Default::default() });
        while !s.on_ground {
            let forward = (s.pos.z - stone.z).signum();
            step_player(&mut s, &Input { forward, ..Default::default() });
        }
        assert!((s.pos.y - stone.y).abs() < 1e-3, "landed on the stone: {:?}", s.pos);
    }

    #[test]
    fn the_jump_pads_throw_onto_the_ledge() {
        let mut s = standing(course().pads[1] + Vec3::Z * 4.0);
        while !step_player(&mut s, &Input { forward: 1.0, ..Default::default() }).launched {}
        while !s.on_ground {
            step_player(&mut s, &Input { forward: 1.0, ..Default::default() });
        }
        assert!((s.pos.y - 7.0).abs() < 1e-3, "up on the ledge: {:?}", s.pos);
    }

    #[test]
    fn laser_bars_sweep_but_can_be_jumped() {
        let pivot = course().sweepers[0];
        let at = pivot + laser_dir(0, 0.0) * 4.0;
        assert!(lasered(at, 0.0));
        assert!(!lasered(at, 1.0), "the bar moved on");
        assert!(!lasered(at + Vec3::Y * 0.7, 0.0), "jumped over it");
        assert!(!lasered(pivot + laser_dir(0, 0.0) * (LASER_REACH + 1.0), 0.0), "past its end");
    }
}
