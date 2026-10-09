use bevy::math::{EulerRot, Quat, Vec3};
use crate::map::MAP;
use serde::{Deserialize, Serialize};

/// Bumped by every `tools/release.sh publish`. Server and client must match exactly.
pub const BUILD: &str = env!("FPS16_BUILD");

pub const TICK_HZ: f64 = 128.0;
pub const DT: f32 = 1.0 / TICK_HZ as f32;
/// Snapshots go out every this many ticks (32 Hz). Fewer, slightly bigger packets suit Wi-Fi better.
pub const SNAPSHOT_EVERY: u32 = 4;
/// Clients send inputs every this many ticks (64 Hz); each packet carries several so none are lost.
pub const INPUT_EVERY: u32 = 2;

pub const RADIUS: f32 = 0.4;
pub const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.6;

// GoldSrc values converted from inches to meters.
const MAX_SPEED: f32 = 6.35;
const ACCEL: f32 = 10.0;
const AIR_ACCEL: f32 = 10.0;
const AIR_CAP: f32 = 0.76;
const FRICTION: f32 = 4.0;
const STOP_SPEED: f32 = 2.54;
const GRAVITY: f32 = 20.3;
const JUMP_SPEED: f32 = 6.8;

pub const FIRE_COOLDOWN: u8 = (0.7 * TICK_HZ) as u8;
pub const MAX_HP: i32 = 100;
pub const MAX_REWIND_TICKS: f32 = (0.4 * TICK_HZ) as f32;
pub const RANGE: f32 = 200.0;

pub const FREEZE_SECS: f32 = 5.0;
pub const ROUND_SECS: f32 = 115.0;
pub const ROUND_END_SECS: f32 = 5.0;
pub const PLANT_SECS: f32 = 3.0;
pub const DEFUSE_SECS: f32 = 5.0;
pub const BOMB_SECS: f32 = 35.0;
pub const BOMB_RADIUS: f32 = 20.0;
/// How close you need to be to pick up a dropped bomb or defuse a planted one.
pub const BOMB_REACH: f32 = 1.5;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    T,
    Ct,
}

impl Team {
    pub fn name(self) -> &'static str {
        match self {
            Team::T => "Terrorists",
            Team::Ct => "Counter-Terrorists",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// Round start: everyone is frozen in spawn.
    Freeze,
    Live,
    /// Round decided, short pause before the next one.
    End { winner: Team },
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Bomb {
    None,
    Carried(u32),
    Dropped(Vec3),
    Planted(Vec3),
    Exploded(Vec3),
    Defused(Vec3),
}

/// Round state, sent in every snapshot.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Round {
    pub number: u32,
    pub phase: Phase,
    /// Seconds left in the current phase, or until the bomb goes off once planted.
    pub time_left: f32,
    /// Rounds won by T and CT.
    pub score: [u32; 2],
    pub bomb: Bomb,
    /// Plant or defuse progress 0..1 and who is doing it.
    pub progress: Option<(u32, f32)>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct Input {
    pub seq: u32,
    pub forward: f32,
    pub right: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub jump: bool,
    pub fire: bool,
    /// E: plant or defuse.
    pub use_key: bool,
    /// Server tick the client was rendering remote players at, used for lag compensation.
    pub view_tick: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct PlayerState {
    /// Feet position.
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub fire_cd: u8,
    /// Freeze time: can look around but not move or shoot. Part of the state so prediction agrees with the server.
    pub frozen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    pub id: u32,
    pub state: PlayerState,
    pub hp: i32,
    pub kills: u32,
    pub deaths: u32,
    pub team: Team,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ServerMsg {
    /// Keep as the first variant so its encoding never changes between builds.
    WrongBuild { server: String },
    Snapshot(Snapshot),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub tick: u32,
    pub you: u32,
    /// Last input seq the server processed for the receiving client.
    pub ack: u32,
    pub players: Vec<NetPlayer>,
    pub round: Round,
    /// Shots fired this tick, so clients can draw everyone's beams.
    // ponytail: sent once, so a dropped snapshot loses that beam. Resend for a few ticks if that shows on Wi-Fi.
    pub shots: Vec<Shot>,
    /// Latest `InputPacket::time` from this client, and how long the server held it before sending this snapshot.
    pub echo: f64,
    pub echo_held: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Shot {
    pub shooter: u32,
    pub from: Vec3,
    pub to: Vec3,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InputPacket {
    /// Must stay the first field: the server reads it alone before decoding the rest, so a mismatch is caught even
    /// when the rest of the format has changed.
    pub build: String,
    /// Client clock when sent, echoed back in snapshots to measure the network round trip.
    pub time: f64,
    /// The most recent unacked inputs, oldest first. Redundant on purpose to survive packet loss.
    pub inputs: Vec<Input>,
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    postcard::to_stdvec(v).expect("serialize")
}

pub fn decode<'a, T: Deserialize<'a>>(b: &'a [u8]) -> Option<T> {
    postcard::from_bytes(b).ok()
}

pub fn look_rot(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
}

pub fn eye(s: &PlayerState) -> Vec3 {
    s.pos + Vec3::Y * EYE
}

pub fn aim_dir(s: &PlayerState) -> Vec3 {
    look_rot(s.yaw, s.pitch) * Vec3::NEG_Z
}

/// Advances one tick of movement. Returns true if a shot was fired this tick.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> bool {
    s.yaw = inp.yaw;
    s.pitch = inp.pitch.clamp(-1.55, 1.55);

    let fwd = Vec3::new(-s.yaw.sin(), 0.0, -s.yaw.cos());
    let right = Vec3::new(s.yaw.cos(), 0.0, -s.yaw.sin());
    let wish = if s.frozen { Vec3::ZERO } else { (fwd * inp.forward + right * inp.right).normalize_or_zero() };

    if s.on_ground && inp.jump && !s.frozen {
        s.vel.y = JUMP_SPEED;
        s.on_ground = false;
    }
    if s.on_ground {
        let speed = s.vel.with_y(0.0).length();
        if speed > 0.0 {
            let drop = speed.max(STOP_SPEED) * FRICTION * DT;
            s.vel *= ((speed - drop).max(0.0) / speed).max(0.0);
        }
        accelerate(&mut s.vel, wish, MAX_SPEED, ACCEL);
    } else {
        accelerate(&mut s.vel, wish, AIR_CAP, AIR_ACCEL * MAX_SPEED / AIR_CAP);
    }
    s.vel.y -= GRAVITY * DT;

    move_and_collide(s);

    s.fire_cd = s.fire_cd.saturating_sub(1);
    if inp.fire && s.fire_cd == 0 && !s.frozen {
        s.fire_cd = FIRE_COOLDOWN;
        return true;
    }
    false
}

fn accelerate(vel: &mut Vec3, wish: Vec3, wish_speed: f32, accel: f32) {
    let add = wish_speed - vel.dot(wish);
    if add <= 0.0 {
        return;
    }
    *vel += wish * (accel * MAX_SPEED * DT).min(add);
}

/// Overlap with a small skin so resting exactly on a face doesn't count, despite float error.
fn overlaps(p: Vec3, c: Vec3, h: Vec3) -> bool {
    const SKIN: f32 = 1e-3;
    (p.x - c.x).abs() < h.x + RADIUS - SKIN
        && (p.z - c.z).abs() < h.z + RADIUS - SKIN
        && p.y < c.y + h.y - SKIN
        && p.y + HEIGHT > c.y - h.y + SKIN
}

// ponytail: player collides as an AABB, axis-by-axis. No step-up or slopes; add a proper sweep when the map needs it.
fn move_and_collide(s: &mut PlayerState) {
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
            let (lo, hi) = if axis == 1 {
                (c.y - h.y - HEIGHT, c.y + h.y)
            } else {
                (c[axis] - h[axis] - RADIUS, c[axis] + h[axis] + RADIUS)
            };
            if s.pos[axis] < before {
                s.pos[axis] = hi;
                if axis == 1 {
                    s.on_ground = true;
                }
            } else {
                s.pos[axis] = lo;
            }
            s.vel[axis] = 0.0;
        }
    }
    if s.pos.y <= 0.0 {
        s.pos.y = 0.0;
        s.vel.y = 0.0;
        s.on_ground = true;
    }
}

/// Distance to the first map surface (floor or box) along the ray, capped at RANGE.
pub fn trace_world(o: Vec3, d: Vec3) -> f32 {
    let mut best = if d.y < 0.0 { (-o.y / d.y).min(RANGE) } else { RANGE };
    for &(c, h) in &MAP.boxes {
        let (min, max) = (c - h, c + h);
        let (t1, t2) = ((min - o) / d, (max - o) / d);
        let (tn, tf) = (t1.min(t2).max_element(), t1.max(t2).min_element());
        if tn <= tf && tf > 0.0 && tn < best {
            best = tn.max(0.0);
        }
    }
    best
}

/// Ray vs vertical capsule standing at `feet`. Returns the hit distance.
pub fn trace_capsule(o: Vec3, d: Vec3, feet: Vec3) -> Option<f32> {
    let a = feet + Vec3::Y * RADIUS;
    let b = feet + Vec3::Y * (HEIGHT - RADIUS);
    let (t, p) = closest_ray_segment(o, d, a, b);
    let dist2 = (o + d * t).distance_squared(p);
    if dist2 > RADIUS * RADIUS {
        return None;
    }
    // ponytail: entry point approximated from the closest approach; exact for rays perpendicular to the capsule.
    Some((t - (RADIUS * RADIUS - dist2).sqrt()).max(0.0))
}

/// Closest points between ray o + t*d (t >= 0, |d| = 1) and segment a..b. Returns (t, point on segment).
fn closest_ray_segment(o: Vec3, d: Vec3, a: Vec3, b: Vec3) -> (f32, Vec3) {
    let e = b - a;
    let r = o - a;
    let ee = e.dot(e);
    let de = d.dot(e);
    let denom = ee - de * de;
    let mut s = if denom > 1e-6 { ((e.dot(r) - de * d.dot(r)) / denom).clamp(0.0, 1.0) } else { (e.dot(r) / ee).clamp(0.0, 1.0) };
    let t = (d.dot(a + e * s - o)).max(0.0);
    s = ((o + d * t - a).dot(e) / ee).clamp(0.0, 1.0);
    (t, a + e * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_hits() {
        let feet = Vec3::new(0.0, 0.0, -10.0);
        let body = trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, feet).unwrap();
        assert!((body - 9.6).abs() < 1e-3);
        assert!(trace_capsule(Vec3::new(0.5, 1.0, 0.0), Vec3::NEG_Z, feet).is_none());
        assert!(trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, feet).is_none());
        let down = trace_capsule(Vec3::new(0.0, 5.0, -10.0), Vec3::NEG_Y, feet).unwrap();
        assert!((down - 3.2).abs() < 1e-3);
        let diag = Vec3::new(0.0, -1.0, -10.0).normalize();
        assert!(trace_capsule(Vec3::new(0.0, 2.0, 0.0), diag, feet).is_some());
    }


    #[test]
    fn never_leaves_map() {
        let (w, d) = MAP.size();
        let spawns = MAP.spawns(Team::T).iter().chain(MAP.spawns(Team::Ct));
        for (i, sp) in spawns.enumerate() {
            let mut s = PlayerState { pos: *sp, ..Default::default() };
            assert!(!MAP.boxes.iter().any(|&(c, h)| overlaps(s.pos, c, h)), "spawn {i} inside a box");
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
                let inside = s.pos.x.abs() < w / 2.0 && s.pos.z.abs() < d / 2.0 && s.pos.y >= 0.0;
                assert!(inside && s.pos.is_finite(), "spawn {i} tick {t}: {:?}", s.pos);
            }
        }
    }

    #[test]
    fn walls_block_and_frozen_stays_put() {
        let start = MAP.spawns(Team::T)[0];
        let mut s = PlayerState { pos: start, ..Default::default() };
        let north = Input { forward: 1.0, ..Default::default() };
        for _ in 0..1280 {
            step_player(&mut s, &north);
        }
        assert!(s.on_ground && s.pos.z < start.z, "moved north");
        let wall = trace_world(eye(&s), Vec3::NEG_Z);
        assert!(wall < RANGE && wall > 0.0, "a wall somewhere north: {wall}");

        let mut f = PlayerState { pos: start, frozen: true, ..Default::default() };
        let fire = Input { fire: true, jump: true, ..north };
        assert!(!step_player(&mut f, &fire));
        assert_eq!(f.pos, start);
    }
}
