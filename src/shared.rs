use bevy::math::{EulerRot, Quat, Vec3};
use crate::map::MAP;
use crate::weapons::{AWP_SCOPED_SPREAD, DRAW_SECS, SCOPED_SPEED, Slot, Weapon};
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
/// Highest rise you walk up without jumping (stairs, ramps, curbs), and how far down you stay glued to the floor.
const STEP: f32 = 0.5;
/// Walking (Shift) speed as a fraction of running, as in CS. Footsteps are only heard above it.
pub const WALK_SPEED: f32 = 0.52;

pub const MAX_HP: i32 = 100;
pub const MAX_REWIND_TICKS: f32 = (0.4 * TICK_HZ) as f32;
pub const RANGE: f32 = 200.0;
/// Hits above this height (from the feet) are headshots, below `LEGS_HEIGHT` leg shots.
pub const HEAD_HEIGHT: f32 = 1.5;
pub const LEGS_HEIGHT: f32 = 0.8;

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
    /// Seconds of buy time left.
    pub buy_secs: f32,
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
    /// Weapon switch request: 0 none, 1 primary, 2 secondary, 3 knife, 4 last used.
    pub slot: u8,
    pub reload: bool,
    /// Shift: walk slowly and silently.
    pub walk: bool,
    /// Scoped in (AWP).
    pub zoom: bool,
    /// Buy menu item (1-based), 0 for none. Handled by the server; inputs are applied exactly once.
    pub buy: u8,
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
    pub primary: Option<Weapon>,
    pub secondary: Weapon,
    pub slot: Slot,
    pub last_slot: Slot,
    /// (magazine, reserve) for primary and secondary.
    pub ammo: [(u8, u16); 2],
    /// Ticks until the current weapon can fire again (also covers drawing it).
    pub fire_cd: u16,
    /// Ticks left reloading, 0 when not.
    pub reloading: u16,
    /// Grows per shot and recovers over time; drives view kick and spread.
    pub recoil: f32,
    /// Fire was held last tick: semi-automatic weapons need a fresh press per shot.
    pub trigger: bool,
    /// Freeze time: can look around but not move or shoot. Part of the state so prediction agrees with the server.
    pub frozen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    pub id: u32,
    pub state: PlayerState,
    pub hp: i32,
    pub armor: u8,
    pub money: u32,
    pub kills: u32,
    pub deaths: u32,
    pub team: Team,
    pub bot: bool,
    pub name: String,
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
    /// Damage dealt since the last snapshot.
    pub hits: Vec<Hit>,
    /// Latest `InputPacket::time` from this client, and how long the server held it before sending this snapshot.
    pub echo: f64,
    pub echo_held: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Shot {
    pub shooter: u32,
    pub weapon: Weapon,
    pub from: Vec3,
    pub to: Vec3,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Hit {
    pub attacker: u32,
    pub victim: u32,
    pub weapon: Weapon,
    pub damage: u8,
    pub headshot: bool,
    pub killed: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InputPacket {
    /// Must stay the first field: the server reads it alone before decoding the rest, so a mismatch is caught even
    /// when the rest of the format has changed.
    pub build: String,
    /// Player name: the logged-in user on the client's machine.
    pub name: String,
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

impl PlayerState {
    /// The weapon in hand.
    pub fn weapon(&self) -> Weapon {
        match self.slot {
            Slot::Primary => self.primary.unwrap_or(self.secondary),
            Slot::Secondary => self.secondary,
            Slot::Knife => Weapon::Knife,
        }
    }

    /// Magazine and reserve of the weapon in hand; None for the knife.
    pub fn ammo_mut(&mut self) -> Option<&mut (u8, u16)> {
        match self.slot {
            Slot::Primary => Some(&mut self.ammo[0]),
            Slot::Secondary => Some(&mut self.ammo[1]),
            Slot::Knife => None,
        }
    }

    pub fn ammo(&self) -> Option<(u8, u16)> {
        self.clone().ammo_mut().copied()
    }

    /// A fresh loadout: just the team pistol, full ammo.
    pub fn loadout(&mut self, primary: Option<Weapon>, secondary: Weapon) {
        (self.primary, self.secondary) = (primary, secondary);
        self.ammo = [primary.map_or((0, 0), |w| (w.spec().mag, w.spec().reserve)), (secondary.spec().mag, secondary.spec().reserve)];
        self.slot = if primary.is_some() { Slot::Primary } else { Slot::Secondary };
        (self.fire_cd, self.reloading, self.recoil) = (0, 0, 0.0);
    }
}

fn ticks(secs: f32) -> u16 {
    (secs * TICK_HZ as f32).round() as u16
}

/// View kick from recoil as (yaw, pitch) offsets. Bullets follow it, and the camera shows it, so the crosshair stays
/// where the next bullet goes and you have to pull down to hold a spray on target.
pub fn punch(s: &PlayerState) -> (f32, f32) {
    let kick = s.weapon().spec().recoil;
    let up = s.recoil.min(10.0) * kick;
    let side = (s.recoil * 0.9).sin() * kick * 0.6 * (s.recoil / 3.0).min(1.0);
    (side, up)
}

/// Deterministic per-shot random in the unit disk, so the client's predicted bullets match the server's.
fn spread_offset(seq: u32) -> (f32, f32) {
    let mut x = seq.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as f32 / u32::MAX as f32
    };
    let (r, a) = (next().sqrt(), next() * std::f32::consts::TAU);
    (r * a.cos(), r * a.sin())
}

/// Switching, reloading and firing. Returns the weapon and bullet direction when a shot goes off.
fn weapon_tick(s: &mut PlayerState, inp: &Input) -> Option<(Weapon, Vec3)> {
    let want = match inp.slot {
        1 => Some(Slot::Primary),
        2 => Some(Slot::Secondary),
        3 => Some(Slot::Knife),
        4 => Some(s.last_slot),
        _ => None,
    };
    if let Some(slot) = want.filter(|&w| w != s.slot && (w != Slot::Primary || s.primary.is_some())) {
        (s.last_slot, s.slot) = (s.slot, slot);
        (s.fire_cd, s.reloading) = (ticks(DRAW_SECS), 0);
    }
    if s.slot == Slot::Primary && s.primary.is_none() {
        s.slot = Slot::Secondary;
    }
    let weapon = s.weapon();
    let spec = weapon.spec();
    s.fire_cd = s.fire_cd.saturating_sub(1);
    s.recoil = (s.recoil - DT * 5.0).max(0.0);

    let reloading = s.reloading > 0;
    if reloading {
        s.reloading -= 1;
    }
    let (finished, frozen) = (reloading && s.reloading == 0, s.frozen);
    let wants_reload = |(mag, reserve): (u8, u16)| reserve > 0 && mag < spec.mag && (inp.reload || (mag == 0 && inp.fire));
    let mut start_reload = false;
    if let Some(ammo) = s.ammo_mut() {
        if finished {
            let moved = (spec.mag - ammo.0).min(ammo.1.min(255) as u8);
            *ammo = (ammo.0 + moved, ammo.1 - moved as u16);
        } else if !reloading && !frozen && wants_reload(*ammo) {
            start_reload = true;
        }
    }
    if start_reload {
        s.reloading = ticks(spec.reload);
    }

    let loaded = s.ammo().is_none_or(|(mag, _)| mag > 0);
    let fires = inp.fire && !s.frozen && s.fire_cd == 0 && s.reloading == 0 && loaded && (spec.auto || !s.trigger);
    s.trigger = inp.fire;
    if !fires {
        return None;
    }
    if let Some(ammo) = s.ammo_mut() {
        ammo.0 -= 1;
    }
    s.fire_cd = ticks(spec.cycle).max(1);

    let speed = s.vel.with_y(0.0).length() / MAX_SPEED;
    let base = if weapon == Weapon::Awp && inp.zoom { AWP_SCOPED_SPREAD } else { spec.spread };
    let air = if s.on_ground { 0.0 } else { 0.12 };
    let cone = base + speed * spec.move_spread + air + s.recoil * spec.recoil * 0.15;
    let (sx, sy) = spread_offset(inp.seq);
    let (side, up) = punch(s);
    let dir = look_rot(s.yaw + side, s.pitch + up) * Vec3::new(sx * cone, sy * cone, -1.0).normalize();
    s.recoil += 1.0;
    Some((weapon, dir))
}

/// Advances one tick of movement and weapon handling. Returns the weapon and bullet direction if a shot was fired.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> Option<(Weapon, Vec3)> {
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
        // Lighter weapons in hand run faster; the knife fastest.
        let weapon = s.weapon();
        let scoped = if weapon == Weapon::Awp && inp.zoom { SCOPED_SPEED } else { 1.0 };
        let scoped = scoped * if inp.walk { WALK_SPEED } else { 1.0 };
        accelerate(&mut s.vel, wish, MAX_SPEED * weapon.spec().speed * scoped, ACCEL);
    } else {
        accelerate(&mut s.vel, wish, AIR_CAP, AIR_ACCEL * MAX_SPEED / AIR_CAP);
    }
    s.vel.y -= GRAVITY * DT;

    move_and_collide(s);

    weapon_tick(s, inp)
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

// ponytail: player collides with boxes as an AABB, axis-by-axis, and stands on the height field at its center point.
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
        // Floor rising more than a step in the way is a wall; anything less is walked up.
        if axis != 1 && MAP.floor_at(s.pos) > s.pos.y + STEP {
            s.pos[axis] = before;
            s.vel[axis] = 0.0;
        }
    }
    let floor = MAP.floor_at(s.pos);
    let gap = s.pos.y - floor;
    // Land, step up, or stay glued to the floor walking down a slope (unless jumping).
    if gap <= 0.0 || (was_on_ground && s.vel.y <= 0.0 && gap < STEP) {
        s.pos.y = floor;
        s.vel.y = s.vel.y.max(0.0);
        s.on_ground = true;
    }
}

/// Distance to where a ray first goes below the floor, by marching then bisecting. RANGE if it never does.
fn trace_floor(o: Vec3, d: Vec3, max: f32) -> f32 {
    const STEP: f32 = 0.25;
    let below = |t: f32| {
        let p = o + d * t;
        p.y < MAP.floor_at(p)
    };
    let mut t = 0.0;
    while t < max {
        let next = (t + STEP).min(max);
        if below(next) {
            let (mut lo, mut hi) = (t, next);
            for _ in 0..8 {
                let mid = (lo + hi) / 2.0;
                if below(mid) { hi = mid } else { lo = mid }
            }
            return lo;
        }
        t = next;
    }
    RANGE
}

/// Distance to the first map surface (floor or box) along the ray, capped at RANGE.
pub fn trace_world(o: Vec3, d: Vec3) -> f32 {
    trace_world_normal(o, d).0
}

/// Like `trace_world`, plus the surface normal where it hit (for bullet holes).
pub fn trace_world_normal(o: Vec3, d: Vec3) -> (f32, Vec3) {
    let (mut best, mut normal) = (RANGE, Vec3::Y);
    for &(c, h) in &MAP.boxes {
        let (min, max) = (c - h, c + h);
        let (t1, t2) = ((min - o) / d, (max - o) / d);
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
    let floor = trace_floor(o, d, best);
    if floor < best { (floor, Vec3::Y) } else { (best, normal) }
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
                let inside = s.pos.x.abs() < w / 2.0 && s.pos.z.abs() < d / 2.0 && s.pos.y >= MAP.floor_at(s.pos) - 1e-3;
                assert!(inside && s.pos.is_finite(), "spawn {i} tick {t}: {:?}", s.pos);
            }
        }
    }

    #[test]
    fn weapons_fire_reload_and_switch() {
        let mut s = PlayerState { pos: MAP.spawns(Team::T)[0], on_ground: true, ..Default::default() };
        s.loadout(Some(Weapon::Ak47), Weapon::Glock);
        let mut seq = 0;
        let mut tick = |s: &mut PlayerState, inp: Input| {
            seq += 1;
            step_player(s, &Input { seq, ..inp })
        };
        let fire = Input { fire: true, ..Default::default() };

        let first = tick(&mut s, fire).expect("fires immediately").1;
        assert!(first.angle_between(Vec3::NEG_Z) < 0.01, "first shot standing still is accurate");
        let mut shots = vec![first];
        for _ in 0..TICK_HZ as usize {
            shots.extend(tick(&mut s, fire).map(|(_, d)| d));
        }
        assert!((9..=11).contains(&shots.len()), "AK fires ~10/s: {}", shots.len());
        assert!(shots.last().unwrap().y > shots[0].y + 0.05, "spray climbs");

        // Empty the mag; holding fire on empty starts the reload, which refills from reserve.
        while s.ammo[0].0 > 0 {
            tick(&mut s, fire);
        }
        tick(&mut s, fire);
        assert!(s.reloading > 0);
        for _ in 0..(2.5 * TICK_HZ) as usize {
            tick(&mut s, Input::default());
        }
        assert_eq!(s.ammo[0], (30, 60));

        // Switch to the pistol: draw delay, then one shot per click.
        tick(&mut s, Input { slot: 2, ..Default::default() });
        assert_eq!(s.weapon(), Weapon::Glock);
        assert!(tick(&mut s, fire).is_none(), "still drawing");
        for _ in 0..TICK_HZ as usize {
            tick(&mut s, Input::default());
        }
        let held: usize = (0..TICK_HZ as usize).filter(|_| tick(&mut s, fire).is_some()).count();
        assert_eq!(held, 1, "semi-automatic: holding fires once");
        tick(&mut s, Input::default());
        assert!(tick(&mut s, fire).is_some(), "fresh click fires again");
    }

    #[test]
    fn knife_runs_faster_than_rifles() {
        let run = |primary: Option<Weapon>, slot: u8| {
            let mut s = PlayerState { pos: MAP.spawns(Team::T)[0], on_ground: true, ..Default::default() };
            s.loadout(primary, Weapon::Glock);
            step_player(&mut s, &Input { slot, ..Default::default() });
            for _ in 0..TICK_HZ as usize {
                step_player(&mut s, &Input { right: 1.0, ..Default::default() });
            }
            s.vel.with_y(0.0).length()
        };
        let (knife, pistol, ak) = (run(Some(Weapon::Ak47), 3), run(Some(Weapon::Ak47), 2), run(Some(Weapon::Ak47), 1));
        assert!(knife > pistol && pistol > ak, "knife {knife} > pistol {pistol} > AK {ak}");
    }

    #[test]
    fn walks_down_into_the_tunnel() {
        // Walk from T spawn (on its platform) to the lowest floor along the bots' path.
        let cells = (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z)));
        let low = cells.filter(|&(x, z)| MAP.walkable(x, z)).map(|(x, z)| MAP.ground(x, z)).find(|g| g.y < 0.1).expect("a tunnel");
        let mut s = PlayerState { pos: MAP.spawns(Team::T)[0], on_ground: true, ..Default::default() };
        s.loadout(None, Weapon::Glock);
        let field = MAP.distance_field(MAP.cell(low));
        let mut min_y = s.pos.y;
        for _ in 0..(60.0 * TICK_HZ) as usize {
            let (x, z) = MAP.cell(s.pos);
            let w = MAP.width();
            let next = MAP.neighbors(x, z).min_by_key(|&(nx, nz)| field[nz * w + nx]);
            let Some((nx, nz)) = next.filter(|_| field[z * w + x] > 0) else { break };
            let dir = (MAP.center(nx, nz) - s.pos).with_y(0.0).normalize_or_zero();
            let yaw = f32::atan2(-dir.x, -dir.z);
            step_player(&mut s, &Input { yaw, forward: 1.0, ..Default::default() });
            assert!(s.pos.y >= MAP.floor_at(s.pos) - 1e-3, "never sinks into the floor");
            min_y = min_y.min(s.pos.y);
        }
        assert!(min_y < 0.3, "got down into the tunnel: {min_y}");
        assert!(MAP.spawns(Team::T)[0].y > 2.0, "T spawn sits on its platform");
    }

    #[test]
    fn terrain_blocks_shots() {
        let low = (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z))).find(|&(x, z)| MAP.walkable(x, z) && MAP.ground(x, z).y < 0.1);
        let p = MAP.ground(low.unwrap().0, low.unwrap().1);
        let t = trace_world(p + Vec3::Y * 0.5, Vec3::NEG_Y);
        assert!((t - 0.5).abs() < 0.01, "floor below: {t}");
    }

    #[test]
    fn spread_matches_on_client_and_server() {
        let mut a = PlayerState { on_ground: true, vel: Vec3::X * 5.0, ..Default::default() };
        a.loadout(Some(Weapon::Mp5), Weapon::Usp);
        let mut b = a;
        let inp = Input { seq: 77, fire: true, ..Default::default() };
        assert_eq!(step_player(&mut a, &inp), step_player(&mut b, &inp));
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
        assert!(step_player(&mut f, &fire).is_none());
        assert_eq!(f.pos, start);
    }
}
