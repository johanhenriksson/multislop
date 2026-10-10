//! What client and server both run: movement (Quake style, lighter gravity, faster), the railgun, the wire format.

use crate::map::{MAP, WALL_TOP};
use glam::Vec3;
use sdk::ability::{Ability, Accuracy, Cost, Delivery, Hands, Payload, Trigger};
use serde::{Deserialize, Serialize};

pub use sdk::client::look_rot;
pub use sdk::{Entity, TICK_HZ, decode, encode};
pub const DT: f32 = 1.0 / TICK_HZ as f32;

pub const RADIUS: f32 = 0.4;
pub const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.6;

pub const MAX_SPEED: f32 = 9.0;
const ACCEL: f32 = 12.0;
/// In the air you only add speed up to `AIR_CAP` along where you steer, so speed comes from strafe jumping.
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

pub const RANGE: f32 = 300.0;
pub const MAX_REWIND_TICKS: f32 = (0.4 * TICK_HZ) as f32;

/// Instagib: one rail, one frag, one second to recharge. Hold fire to keep shooting.
pub const RAIL: Ability = Ability {
    trigger: Trigger::Auto,
    cost: Cost::Free,
    cycle: 1.0,
    accuracy: Accuracy { spread: 0.0, moving: 0.0, airborne: 0.0, kick: 0.0 },
    delivery: Delivery::Hitscan { range: RANGE, falloff: 1.0 },
    payload: Payload::Damage { amount: 1000.0, armor_pen: 1.0 },
};

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
    /// Fires the rail; while dead, respawns.
    pub fire: bool,
    /// Server tick the client was rendering remote players at, for lag compensation. Stamped by the host.
    #[serde(skip)]
    pub view_tick: f32,
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
    /// Rail recharge.
    pub hands: Hands,
    /// Intermission: look around, nothing else. Part of the state so prediction agrees with the server.
    pub frozen: bool,
}

/// A player as clients see it, carried in an `sdk::Entity`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    /// The entity id, filled in by `NetPlayer::from_entity`.
    #[serde(skip)]
    pub id: u32,
    pub state: PlayerState,
    pub alive: bool,
    pub frags: i32,
    pub deaths: u32,
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
}

/// The match as clients see it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Match {
    /// Seconds left of the match, or of the intermission once it's over.
    pub time_left: f32,
    pub frag_limit: i32,
    /// The match is over and this player won; the scoreboard shows until the next one.
    pub winner: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Event {
    /// A rail fired, so everyone draws the beam.
    // ponytail: sent once, so a dropped snapshot loses that beam. Resend for a few ticks if that shows on Wi-Fi.
    Rail { shooter: u32, from: Vec3, to: Vec3 },
    Frag { killer: u32, victim: u32 },
}

/// What happened to a player in one tick, for effects and sounds.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Step {
    /// The rail fired, this way.
    pub fired: Option<Vec3>,
    pub jumped: bool,
    pub launched: bool,
}

/// Client-side prediction of our own player: one input on top of `me`.
pub fn predict(me: &mut Entity, input: &[u8], seq: u32) -> Step {
    let (Some(mut p), Some(inp)) = (NetPlayer::from_entity(me), decode::<Input>(input)) else { return Step::default() };
    if !p.alive {
        return Step::default();
    }
    let step = step_player(&mut p.state, &Input { seq, ..inp });
    *me = p.entity(me.teleport);
    step
}

pub fn eye(s: &PlayerState) -> Vec3 {
    s.pos + Vec3::Y * EYE
}

/// The middle of the chest, where bots aim.
pub fn chest(pos: Vec3) -> Vec3 {
    pos + Vec3::Y * 1.2
}

pub fn aim_dir(s: &PlayerState) -> Vec3 {
    look_rot(s.yaw, s.pitch) * Vec3::NEG_Z
}

/// Advances one tick of movement and the rail.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> Step {
    s.yaw = inp.yaw;
    s.pitch = inp.pitch.clamp(-1.55, 1.55);
    let mut step = Step::default();

    let fwd = Vec3::new(-s.yaw.sin(), 0.0, -s.yaw.cos());
    let right = Vec3::new(s.yaw.cos(), 0.0, -s.yaw.sin());
    let wish = if s.frozen { Vec3::ZERO } else { (fwd * inp.forward + right * inp.right).normalize_or_zero() };

    if s.on_ground && MAP.on_pad(s.pos) && !s.frozen {
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

    let mut hands = s.hands;
    if hands.tick(&RAIL, None, inp.fire, false, s.frozen).is_some() {
        step.fired = Some(aim_dir(s));
    }
    s.hands = hands;
    step
}

/// Quake acceleration: adds speed along `wish` up to `cap`, at a rate set by full run speed. In the air the cap is
/// tiny but the rate isn't, which is what lets strafe jumping build speed.
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
    MAP.boxes.iter().any(|&(c, h)| overlaps(p, c, h))
}

/// Highest floor under the hull at or below `pos`: a block top or the ground.
pub fn floor_below(pos: Vec3) -> f32 {
    let under = |&&(c, h): &&(Vec3, Vec3)| {
        (pos.x - c.x).abs() < h.x + RADIUS && (pos.z - c.z).abs() < h.z + RADIUS && c.y + h.y <= pos.y + 1e-3
    };
    MAP.boxes.iter().filter(under).map(|&(c, h)| c.y + h.y).fold(0.0, f32::max)
}

// ponytail: the hull is an AABB moved axis by axis against the blocks.
fn move_and_collide(s: &mut PlayerState) {
    let was_on_ground = s.on_ground;
    s.on_ground = false;
    for axis in [0, 2, 1] {
        let before = s.pos[axis];
        s.pos[axis] += s.vel[axis] * DT;
        for &(c, h) in &MAP.boxes {
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

/// Distance to the first map surface along the ray (capped at RANGE), and its normal.
pub fn trace_world(o: Vec3, d: Vec3) -> (f32, Vec3) {
    let (mut best, mut normal) = (RANGE, Vec3::Y);
    if d.y < 0.0 {
        best = best.min(o.y / -d.y);
    }
    for &(c, h) in &MAP.boxes {
        let (t1, t2) = ((c - h - o) / d, (c + h - o) / d);
        let near = t1.min(t2);
        let (tn, tf) = (near.max_element(), t1.max(t2).min_element());
        if tn <= tf && tf > 0.0 && tn < best {
            best = tn.max(0.0);
            // The entry face is on the axis that was entered last.
            let axis = if near.x >= near.y && near.x >= near.z { 0 } else if near.y >= near.z { 1 } else { 2 };
            normal = Vec3::ZERO;
            normal[axis] = -d[axis].signum();
        }
    }
    (best, normal)
}

pub fn can_see(from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    trace_world(from, d.normalize_or_zero()).0 >= d.length() - 0.05
}

/// Where a ray enters a player's capsule standing at `feet`.
pub fn trace_capsule(o: Vec3, d: Vec3, feet: Vec3) -> Option<f32> {
    let (a, b) = (feet + Vec3::Y * RADIUS, feet + Vec3::Y * (HEIGHT - RADIUS));
    let e = b - a;
    let r = o - a;
    let (ee, de) = (e.dot(e), d.dot(e));
    let denom = ee - de * de;
    let s = if denom > 1e-6 { ((e.dot(r) - de * d.dot(r)) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let t = d.dot(a + e * s - o).max(0.0);
    let p = a + e * ((o + d * t - a).dot(e) / ee).clamp(0.0, 1.0);
    let dist2 = (o + d * t).distance_squared(p);
    // ponytail: entry point approximated from the closest approach; exact for rays perpendicular to the capsule.
    (dist2 <= RADIUS * RADIUS).then(|| (t - (RADIUS * RADIUS - dist2).sqrt()).max(0.0))
}

/// Inside the map: between the outer walls, above the ground, below the wall tops plus a pad's throw.
pub fn in_bounds(p: Vec3) -> bool {
    let (w, d) = MAP.size();
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

    #[test]
    fn never_leaves_the_map() {
        for map in crate::map::MAPS {
            assert!(crate::map::load(map));
            for (i, sp) in MAP.spawns.iter().enumerate() {
                let mut s = PlayerState { pos: *sp, ..Default::default() };
                assert!(!blocked(s.pos), "{map}: spawn {i} inside a block");
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
                    assert!(in_bounds(s.pos) && !blocked(s.pos), "{map}: spawn {i} tick {t}: {:?}", s.pos);
                }
            }
        }
    }

    #[test]
    fn every_jump_pad_reaches_somewhere_higher() {
        for map in crate::map::MAPS {
            assert!(crate::map::load(map));
            for &pad in &MAP.pads {
                // Run over the pad from a few meters back, in any of 16 directions, and hold forward.
                let lands_high = (0..16).any(|i| {
                    let yaw = i as f32 * std::f32::consts::TAU / 16.0;
                    let fwd = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
                    let mut s = PlayerState { pos: pad - fwd * 3.0, on_ground: true, ..Default::default() };
                    if blocked(s.pos) || (floor_below(s.pos + Vec3::Y * 0.1) - pad.y).abs() > 1e-3 {
                        return false;
                    }
                    let inp = Input { forward: 1.0, yaw, ..Default::default() };
                    (0..3 * TICK_HZ as usize).any(|_| {
                        step_player(&mut s, &inp);
                        s.on_ground && s.pos.y > pad.y + 1.9
                    })
                });
                assert!(lands_high, "{map}: the pad at {pad} throws nowhere");
            }
        }
    }

    #[test]
    fn jumps_are_floaty_and_running_is_fast() {
        let mut s = PlayerState { pos: MAP.center(20, 8), on_ground: true, ..Default::default() };
        run(&mut s, Input { right: 1.0, ..Default::default() }, 1.0);
        assert!((s.vel.length() - MAX_SPEED).abs() < 0.1, "{}", s.vel.length());
        let mut top = 0.0f32;
        let mut airtime = 0;
        step_player(&mut s, &Input { jump: true, ..Default::default() });
        while !s.on_ground {
            step_player(&mut s, &Input::default());
            (top, airtime) = (top.max(s.pos.y), airtime + 1);
        }
        assert!((1.6..1.9).contains(&top) && airtime as f32 / TICK_HZ as f32 > 1.0, "{top} m, {airtime} ticks");
    }

    #[test]
    fn walks_up_the_stairs_to_a_balcony() {
        // The south-west stairs climb east to west along row 7 from the floor at x = 8.
        let mut s = PlayerState { pos: MAP.center(8, 7), on_ground: true, yaw: std::f32::consts::FRAC_PI_2, ..Default::default() };
        run(&mut s, Input { forward: 1.0, yaw: std::f32::consts::FRAC_PI_2, ..Default::default() }, 0.6);
        run(&mut s, Input { forward: 1.0, ..Default::default() }, 0.5);
        assert!((s.pos.y - 2.0).abs() < 1e-3 && s.on_ground, "up on the balcony: {:?}", s.pos);
    }

    #[test]
    fn jump_pad_throws_onto_the_tower() {
        let pad = MAP.pads.iter().copied().find(|p| p.x < 0.0 && p.z < 0.0 && p.x > -20.0).unwrap();
        // Run diagonally at the middle and over the pad.
        let yaw = f32::atan2(-(-pad.x), -(-pad.z));
        let mut s = PlayerState { pos: pad - Vec3::new(1.0, 0.0, 1.0).normalize() * 4.0, on_ground: true, ..Default::default() };
        run(&mut s, Input { forward: 1.0, yaw, ..Default::default() }, 3.0);
        assert!(s.on_ground && (s.pos.y - 4.0).abs() < 1e-3, "landed on the tower: {:?}", s.pos);
    }

    #[test]
    fn rail_recharges_in_a_second() {
        let mut s = PlayerState { pos: MAP.spawns[0], ..Default::default() };
        let fire = Input { fire: true, ..Default::default() };
        let shots = (0..(2.5 * TICK_HZ) as usize).filter(|_| step_player(&mut s, &fire).fired.is_some()).count();
        assert_eq!(shots, 3);
    }

    #[test]
    fn capsule_hits() {
        let feet = Vec3::new(0.0, 0.0, -10.0);
        assert!((trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, feet).unwrap() - 9.6).abs() < 1e-3);
        assert!(trace_capsule(Vec3::new(0.5, 1.0, 0.0), Vec3::NEG_Z, feet).is_none());
        assert!(trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, feet).is_none());
    }
}
