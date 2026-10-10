use glam::Vec3;
use crate::map::MAP;
use crate::weapons::{AWP_SCOPED_SPREAD, DRAW_SECS, SCOPED_SPEED, Slot, Weapon};
use sdk::ability::Hands;
use serde::{Deserialize, Serialize};

pub use sdk::client::look_rot;
pub use sdk::{TICK_HZ, decode, encode};
use sdk::Entity;
pub const DT: f32 = 1.0 / TICK_HZ as f32;
/// Rounds per half: then the teams trade sides.
pub const HALF: u32 = 10;

pub const RADIUS: f32 = 0.4;
pub const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.6;
/// Crouched (Ctrl): shorter, lower eye, slower and more accurate, taking `DUCK_SECS` to go down or up.
pub const DUCK_HEIGHT: f32 = 1.0;
pub const DUCK_EYE: f32 = 0.85;
const DUCK_SECS: f32 = 0.2;
/// Crouched speed and spread as fractions of standing (CS 1.6 speed).
const DUCK_SPEED: f32 = 0.34;
const DUCK_SPREAD: f32 = 0.7;

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
pub const STEP: f32 = 0.5;
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
    /// Stamped by the host, not sent as part of the input.
    #[serde(skip)]
    pub seq: u32,
    pub forward: f32,
    pub right: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub jump: bool,
    pub fire: bool,
    /// E: plant or defuse.
    pub use_key: bool,
    /// Weapon switch request: 0 none, 1 primary, 2 secondary, 3 knife, 4 grenade, 5 last used.
    pub slot: u8,
    pub reload: bool,
    /// Shift: walk slowly and silently.
    pub walk: bool,
    /// Scoped in (AWP).
    pub zoom: bool,
    /// Ctrl: crouch.
    pub duck: bool,
    /// G: drop the weapon in hand (not the knife).
    pub drop: bool,
    /// Buy menu item (1-based), 0 for none. Handled by the server; inputs are applied exactly once.
    pub buy: u8,
    /// Radio call from the Z menu (1-based into `Radio::MENU`), 0 for none.
    pub radio: u8,
    /// Server tick the client was rendering remote players at, used for lag compensation. Stamped by the host.
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
    pub primary: Option<Weapon>,
    pub secondary: Weapon,
    pub grenade: Option<Weapon>,
    pub slot: Slot,
    pub last_slot: Slot,
    /// (magazine, reserve) for primary, secondary and grenade (the count, no reserve).
    pub ammo: [(u8, u16); 3],
    /// Cooldown, reload, bloom and trigger state of the weapon in hand.
    pub hands: Hands,
    /// Freeze time: can look around but not move or shoot. Part of the state so prediction agrees with the server.
    pub frozen: bool,
    /// How far crouched, 0 standing to 1 fully down.
    pub duck: f32,
}

/// A player as clients see it, carried in an `sdk::Entity`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    /// The entity id, filled in by `NetPlayer::from_entity`.
    #[serde(skip)]
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
    /// Round trip as the client measures it, for the scoreboard. 0 for bots.
    pub ping: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum EntityData {
    Player(NetPlayer),
    /// A thrown grenade.
    Grenade(Weapon),
    /// A weapon lying on the ground.
    Dropped(Weapon),
}

impl NetPlayer {
    pub fn entity(&self, teleport: u8) -> Entity {
        let s = &self.state;
        let data = encode(&EntityData::Player(self.clone()));
        Entity { id: self.id, pos: s.pos, vel: s.vel, yaw: s.yaw, pitch: s.pitch, teleport, data }
    }

    pub fn from_entity(e: &Entity) -> Option<NetPlayer> {
        let Some(EntityData::Player(mut p)) = decode(&e.data) else { return None };
        p.id = e.id;
        (p.state.pos, p.state.vel, p.state.yaw, p.state.pitch) = (e.pos, e.vel, e.yaw, e.pitch);
        Some(p)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Event {
    /// A shot fired, so clients can draw everyone's beams.
    // ponytail: sent once, so a dropped snapshot loses that beam. Resend for a few ticks if that shows on Wi-Fi.
    Shot(Shot),
    Hit(Hit),
    /// Sent to the caller's team only.
    Radio(RadioCall),
    Explosion { pos: Vec3, radius: f32 },
    /// A flashbang going off.
    Flashbang { pos: Vec3 },
    /// Sent to whoever it blinded, for how long.
    Flashed { secs: f32 },
}

/// Client-side prediction of our own player: one input on top of `me`. Returns the use that went off, if any.
pub fn predict(me: &mut Entity, input: &[u8], seq: u32) -> Option<Fired> {
    let (mut p, inp) = (NetPlayer::from_entity(me)?, decode::<Input>(input)?);
    if p.hp <= 0 {
        return None;
    }
    let fired = step_player(&mut p.state, &Input { seq, ..inp });
    *me = p.entity(me.teleport);
    fired
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Shot {
    pub shooter: u32,
    pub weapon: Weapon,
    pub from: Vec3,
    pub to: Vec3,
}

/// Team radio, CS style.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Radio {
    EnemySpotted,
    NeedBackup,
    InPosition,
    SectorClear,
    CoverMe,
}

impl Radio {
    /// The Z radio menu, numbered from 1.
    pub const MENU: [Radio; 5] = [Radio::CoverMe, Radio::EnemySpotted, Radio::NeedBackup, Radio::InPosition, Radio::SectorClear];

    pub fn text(self) -> &'static str {
        match self {
            Radio::EnemySpotted => "Enemy spotted!",
            Radio::NeedBackup => "Need backup!",
            Radio::InPosition => "I'm in position.",
            Radio::SectorClear => "Sector clear.",
            Radio::CoverMe => "Cover me!",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct RadioCall {
    pub from: u32,
    pub call: Radio,
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


/// Height of the player's hull, crouching included.
pub fn height(duck: f32) -> f32 {
    HEIGHT - (HEIGHT - DUCK_HEIGHT) * duck
}

pub fn eye_height(s: &PlayerState) -> f32 {
    EYE - (EYE - DUCK_EYE) * s.duck
}

pub fn eye(s: &PlayerState) -> Vec3 {
    s.pos + Vec3::Y * eye_height(s)
}

/// The middle of the chest, where bots look and aim.
pub fn chest(s: &PlayerState) -> Vec3 {
    s.pos + Vec3::Y * height(s.duck) * (2.0 / 3.0)
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
            Slot::Grenade => self.grenade.unwrap_or(self.secondary),
        }
    }

    /// Something is in this slot.
    /// The pistol slot holds the knife when empty (after dropping the pistol).
    pub fn has(&self, slot: Slot) -> bool {
        match slot {
            Slot::Primary => self.primary.is_some(),
            Slot::Secondary => self.secondary != Weapon::Knife,
            Slot::Grenade => self.grenade.is_some(),
            Slot::Knife => true,
        }
    }

    /// The best thing to have in hand: a primary, else the pistol, else the knife.
    fn best_slot(&self) -> Slot {
        [Slot::Primary, Slot::Secondary].into_iter().find(|&s| self.has(s)).unwrap_or(Slot::Knife)
    }

    /// The weapon in hand and its ammo, unless it's the knife, which can't be dropped.
    pub fn droppable(&self) -> Option<(Weapon, (u8, u16))> {
        (self.slot != Slot::Knife && self.has(self.slot)).then(|| (self.weapon(), self.ammo().unwrap_or((0, 0))))
    }

    /// Lets go of the weapon in hand and takes out the best one left.
    pub fn drop_held(&mut self) {
        match self.slot {
            Slot::Primary => (self.primary, self.ammo[0]) = (None, (0, 0)),
            Slot::Secondary => (self.secondary, self.ammo[1]) = (Weapon::Knife, (0, 0)),
            Slot::Grenade => (self.grenade, self.ammo[2]) = (None, (0, 0)),
            Slot::Knife => return,
        }
        self.slot = self.best_slot();
        self.hands.draw(DRAW_SECS);
    }

    /// Takes a weapon off the ground if its slot is empty. Returns whether it did.
    pub fn pick_up(&mut self, weapon: Weapon, ammo: (u8, u16)) -> bool {
        let slot = weapon.spec().slot;
        if self.has(slot) || slot == Slot::Knife {
            return false;
        }
        match slot {
            Slot::Primary => (self.primary, self.ammo[0]) = (Some(weapon), ammo),
            Slot::Secondary => (self.secondary, self.ammo[1]) = (weapon, ammo),
            Slot::Grenade => (self.grenade, self.ammo[2]) = (Some(weapon), ammo),
            Slot::Knife => {}
        }
        // Holding a worse weapon (or the knife), switch to the better one picked up.
        if self.slot == Slot::Knife || (slot == Slot::Primary && self.slot != Slot::Primary) {
            (self.last_slot, self.slot) = (self.slot, slot);
            self.hands.draw(DRAW_SECS);
        }
        true
    }

    /// Magazine and reserve of the weapon in hand; None for the knife.
    pub fn ammo_mut(&mut self) -> Option<&mut (u8, u16)> {
        match self.slot {
            Slot::Primary => Some(&mut self.ammo[0]),
            Slot::Secondary => Some(&mut self.ammo[1]),
            Slot::Grenade => Some(&mut self.ammo[2]),
            Slot::Knife => None,
        }
    }

    pub fn ammo(&self) -> Option<(u8, u16)> {
        self.clone().ammo_mut().copied()
    }

    /// Fresh guns with full ammo. Grenades are kept.
    pub fn loadout(&mut self, primary: Option<Weapon>, secondary: Weapon) {
        (self.primary, self.secondary) = (primary, secondary);
        let full = |w: Weapon| w.ability().full_ammo().unwrap_or((0, 0));
        (self.ammo[0], self.ammo[1]) = (primary.map_or((0, 0), full), full(secondary));
        self.slot = if primary.is_some() { Slot::Primary } else { Slot::Secondary };
        self.hands = Hands::default();
    }
}

/// View kick from recoil as (yaw, pitch) offsets. Bullets follow it, and the camera shows it, so the crosshair stays
/// where the next bullet goes and you have to pull down to hold a spray on target.
pub fn punch(s: &PlayerState) -> (f32, f32) {
    s.hands.punch(s.weapon().ability())
}

/// A use that went off this tick: which weapon, which way, and how hard (thrown ones).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Fired {
    pub weapon: Weapon,
    pub dir: Vec3,
    pub strength: f32,
}

/// Switching, reloading and firing. Returns the use that went off, if any.
fn weapon_tick(s: &mut PlayerState, inp: &Input) -> Option<Fired> {
    if inp.drop {
        s.drop_held();
    }
    let want = match inp.slot {
        1 => Some(Slot::Primary),
        2 => Some(Slot::Secondary),
        3 => Some(Slot::Knife),
        4 => Some(Slot::Grenade),
        5 => Some(s.last_slot),
        _ => None,
    };
    if let Some(slot) = want.filter(|&w| w != s.slot && s.has(w)) {
        (s.last_slot, s.slot) = (s.slot, slot);
        s.hands.draw(DRAW_SECS);
    }
    if !s.has(s.slot) {
        s.slot = s.best_slot();
    }
    let weapon = s.weapon();
    let ability = weapon.ability();
    let (mut hands, frozen) = (s.hands, s.frozen);
    let strength = hands.tick(ability, s.ammo_mut(), inp.fire, inp.reload, frozen);
    s.hands = hands;
    let strength = strength?;
    let speed = s.vel.with_y(0.0).length() / MAX_SPEED;
    let base = if weapon == Weapon::Awp && inp.zoom { AWP_SCOPED_SPREAD } else { ability.accuracy.spread };
    let spread = base * (1.0 - (1.0 - DUCK_SPREAD) * s.duck);
    let dir = s.hands.aim(ability, Some(spread), speed, s.on_ground, s.yaw, s.pitch, inp.seq);
    // Thrown the last grenade: back to what was in hand before.
    if s.slot == Slot::Grenade && s.ammo[2].0 == 0 {
        s.grenade = None;
        s.slot = if s.has(s.last_slot) && s.last_slot != Slot::Grenade { s.last_slot } else { s.best_slot() };
        s.hands.draw(DRAW_SECS);
    }
    Some(Fired { weapon, dir, strength })
}

/// Advances one tick of movement and weapon handling. Returns the use that went off, if any.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> Option<Fired> {
    s.yaw = inp.yaw;
    s.pitch = inp.pitch.clamp(-1.55, 1.55);

    let fwd = Vec3::new(-s.yaw.sin(), 0.0, -s.yaw.cos());
    let right = Vec3::new(s.yaw.cos(), 0.0, -s.yaw.sin());
    let wish = if s.frozen { Vec3::ZERO } else { (fwd * inp.forward + right * inp.right).normalize_or_zero() };

    // Going down is never blocked; standing up needs headroom.
    let step = DT / DUCK_SECS;
    if inp.duck {
        s.duck = (s.duck + step).min(1.0);
    } else if s.duck > 0.0 {
        let up = (s.duck - step).max(0.0);
        let headroom = !MAP.blocked(s.pos, s.pos.y + STEP, s.pos.y + height(up));
        if headroom && !MAP.boxes.iter().any(|&(c, h)| overlaps(s.pos, c, h, height(up))) {
            s.duck = up;
        }
    }

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
        let scoped = scoped * if inp.walk { WALK_SPEED } else { 1.0 } * (1.0 - (1.0 - DUCK_SPEED) * s.duck);
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

/// Overlap of a player hull `height` tall with a box, with a small skin so resting exactly on a face doesn't count.
fn overlaps(p: Vec3, c: Vec3, h: Vec3, height: f32) -> bool {
    const SKIN: f32 = 1e-3;
    (p.x - c.x).abs() < h.x + RADIUS - SKIN
        && (p.z - c.z).abs() < h.z + RADIUS - SKIN
        && p.y < c.y + h.y - SKIN
        && p.y + height > c.y - h.y + SKIN
}

// ponytail: player collides with boxes as an AABB, axis-by-axis, but with floors (ledges, slabs) only at its center
// point, so the edge of the hull can poke into a slab overhead or hang over a drop.
fn move_and_collide(s: &mut PlayerState) {
    let tall = height(s.duck);
    let was_on_ground = s.on_ground;
    s.on_ground = false;
    for axis in [0, 2, 1] {
        let before = s.pos[axis];
        s.pos[axis] += s.vel[axis] * DT;
        for &(c, h) in &MAP.boxes {
            let mut prev = s.pos;
            prev[axis] = before;
            // Only resolve overlaps this axis' move caused; a touching contact on another axis must not teleport us.
            if !overlaps(s.pos, c, h, tall) || overlaps(prev, c, h, tall) {
                continue;
            }
            let (lo, hi) = if axis == 1 {
                (c.y - h.y - tall, c.y + h.y)
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
        // Floor rising more than a step in the way, or a slab at head height, is a wall; anything less is walked up.
        if axis != 1 && MAP.blocked(s.pos, s.pos.y + STEP, s.pos.y + tall) {
            s.pos[axis] = before;
            s.vel[axis] = 0.0;
        }
        // Head bumps on the underside of a slab.
        if axis == 1 && s.pos.y > before {
            let ceiling = MAP.ceiling(s.pos, before + tall - 1e-3);
            if s.pos.y + tall > ceiling {
                s.pos.y = ceiling - tall;
                s.vel.y = 0.0;
            }
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

/// Distance to where a ray first goes into a floor (the ground or a slab), by marching then bisecting. RANGE if it
/// never does. Steps are shorter than a slab is thick, so a ray can't skip through one.
fn trace_floor(o: Vec3, d: Vec3, max: f32) -> f32 {
    const STEP: f32 = 0.25;
    let below = |t: f32| MAP.solid_at(o + d * t);
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
    // ponytail: floors are hit on their top, or a slab from below; a slab's side edge also reports up.
    let floor_normal = if d.y > 0.0 { Vec3::NEG_Y } else { Vec3::Y };
    if floor < best { (floor, floor_normal) } else { (best, normal) }
}

/// Where a ray enters a player's capsule `height` tall standing at `feet`.
pub fn trace_capsule(o: Vec3, d: Vec3, feet: Vec3, height: f32) -> Option<f32> {
    let a = feet + Vec3::Y * RADIUS;
    let b = feet + Vec3::Y * (height - RADIUS);
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

/// A spot on flat open ground with a clear 30 m line east, for tests.
#[cfg(test)]
pub fn yard() -> Vec3 {
    let flat = |p: Vec3| (0..30).all(|t| (MAP.floor_at(p + Vec3::X * t as f32) - p.y).abs() < 0.2);
    let clear = |p: Vec3| [0.3, 1.0, 1.7].iter().all(|&h| trace_world(p + Vec3::Y * h, Vec3::X) >= 30.0);
    MAP.nodes().map(|n| MAP.node_pos(n)).find(|&p| flat(p) && clear(p)).expect("an open yard")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_hits() {
        let feet = Vec3::new(0.0, 0.0, -10.0);
        let body = trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, feet, HEIGHT).unwrap();
        assert!((body - 9.6).abs() < 1e-3);
        assert!(trace_capsule(Vec3::new(0.5, 1.0, 0.0), Vec3::NEG_Z, feet, HEIGHT).is_none());
        assert!(trace_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::Z, feet, HEIGHT).is_none());
        let down = trace_capsule(Vec3::new(0.0, 5.0, -10.0), Vec3::NEG_Y, feet, HEIGHT).unwrap();
        assert!((down - 3.2).abs() < 1e-3);
        let diag = Vec3::new(0.0, -1.0, -10.0).normalize();
        assert!(trace_capsule(Vec3::new(0.0, 2.0, 0.0), diag, feet, HEIGHT).is_some());
    }


    #[test]
    fn never_leaves_map() {
        let (w, d) = MAP.size();
        let spawns = MAP.spawns(Team::T).iter().chain(MAP.spawns(Team::Ct));
        for (i, sp) in spawns.enumerate() {
            let mut s = PlayerState { pos: *sp, ..Default::default() };
            assert!(!MAP.boxes.iter().any(|&(c, h)| overlaps(s.pos, c, h, HEIGHT)), "spawn {i} inside a box");
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

        let first = tick(&mut s, fire).expect("fires immediately").dir;
        assert!(first.angle_between(Vec3::NEG_Z) < 0.01, "first shot standing still is accurate");
        let mut shots = vec![first];
        for _ in 0..TICK_HZ as usize {
            shots.extend(tick(&mut s, fire).map(|f| f.dir));
        }
        assert!((9..=11).contains(&shots.len()), "AK fires ~10/s: {}", shots.len());
        assert!(shots.last().unwrap().y > shots[0].y + 0.05, "spray climbs");

        // Empty the mag; holding fire on empty starts the reload, which refills from reserve.
        while s.ammo[0].0 > 0 {
            tick(&mut s, fire);
        }
        tick(&mut s, fire);
        assert!(s.hands.reloading > 0);
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
            let mut s = PlayerState { pos: yard(), on_ground: true, ..Default::default() };
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
    fn crouching_is_lower_slower_and_stands_back_up() {
        let mut s = PlayerState { pos: yard(), on_ground: true, ..Default::default() };
        s.loadout(None, Weapon::Glock);
        let crouch = Input { duck: true, right: 1.0, ..Default::default() };
        for _ in 0..(DUCK_SECS * TICK_HZ as f32) as usize + 1 {
            step_player(&mut s, &crouch);
        }
        assert_eq!(s.duck, 1.0, "fully down after DUCK_SECS");
        assert!((eye_height(&s) - DUCK_EYE).abs() < 1e-6 && height(s.duck) == DUCK_HEIGHT);
        for _ in 0..TICK_HZ as usize {
            step_player(&mut s, &crouch);
        }
        let crouched = s.vel.with_y(0.0).length();
        for _ in 0..TICK_HZ as usize {
            step_player(&mut s, &Input { right: 1.0, ..Default::default() });
        }
        let standing = s.vel.with_y(0.0).length();
        assert_eq!(s.duck, 0.0, "stands up in the open");
        assert!((crouched / standing - DUCK_SPEED).abs() < 0.02, "{crouched} vs {standing}");
        assert!(crouched < 6.35 * WALK_SPEED, "too slow for footsteps");
    }


    /// Runs towards `goal` along the bots' distance field, checking it never sinks into a floor, until it gets there
    /// or `secs` are up. Returns where it ended.
    fn walk_to(from: Vec3, goal: usize, secs: f32) -> Vec3 {
        let mut s = PlayerState { pos: from, on_ground: true, ..Default::default() };
        s.loadout(None, Weapon::Glock);
        let (mut field, mut field_goal) = (vec![], usize::MAX);
        let target = MAP.node_pos(goal);
        for _ in 0..(secs * TICK_HZ as f32) as usize {
            if MAP.node(s.pos) == goal && s.pos.with_y(0.0).distance(target.with_y(0.0)) < 0.3 {
                break;
            }
            let dir = crate::bot::path_dir(&mut field, &mut field_goal, s.pos, target);
            let yaw = f32::atan2(-dir.x, -dir.z);
            step_player(&mut s, &Input { yaw, forward: 1.0, ..Default::default() });
            assert!(s.pos.y >= MAP.floor_at(s.pos) - 1e-3, "never sinks into the floor");
        }
        s.pos
    }

    fn lowest_node() -> usize {
        MAP.nodes().min_by(|&a, &b| MAP.node_pos(a).y.total_cmp(&MAP.node_pos(b).y)).unwrap()
    }

    #[test]
    fn walks_down_to_the_lowest_floor() {
        let low = lowest_node();
        let end = walk_to(yard(), low, 60.0);
        assert!((end.y - MAP.node_pos(low).y).abs() < 0.3, "got down to {}: {end}", MAP.node_pos(low));
        assert!(yard().y > MAP.node_pos(low).y + 3.0, "the yard is high up");
    }

    /// A cell with a slab over a lower floor: walk to each level from the other, and stay on the level walked to.
    #[test]
    fn walks_under_and_over_a_stacked_floor() {
        let cells = (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z)));
        let (x, z) = cells.filter(|&(x, z)| MAP.floors(x, z).iter().filter(|f| f.open).count() > 1).last().expect("a slab");
        let levels = [0, 1].map(|i| MAP.node(MAP.center(x, z).with_y(MAP.floors(x, z).iter().filter(|f| f.open).nth(i).unwrap().y)));
        let mut at = yard();
        for _ in 0..2 {
            for goal in levels {
                at = walk_to(at, goal, 60.0);
                assert_eq!(MAP.node(at), goal, "on the right level: {at} for {}", MAP.node_pos(goal));
            }
        }
        // Jumping under the slab bumps the head on it.
        let mut s = PlayerState { pos: MAP.node_pos(levels[0]), on_ground: true, ..Default::default() };
        let ceiling = MAP.node_pos(levels[1]).y - crate::map::SLAB;
        let mut top = s.pos.y;
        for _ in 0..TICK_HZ as usize {
            step_player(&mut s, &Input { jump: true, ..Default::default() });
            top = top.max(s.pos.y);
        }
        assert!(top + HEIGHT <= ceiling + 1e-3, "head stays under the slab: {top}");
    }

    #[test]
    fn terrain_blocks_shots() {
        let p = MAP.node_pos(lowest_node());
        let t = trace_world(p + Vec3::Y * 0.5, Vec3::NEG_Y);
        assert!((t - 0.5).abs() < 0.01, "floor below: {t}");
    }

    /// A slab stops shots from below and above, and you can shoot along under it.
    #[test]
    fn slabs_block_shots() {
        let cells = (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z)));
        let (x, z) = cells.filter(|&(x, z)| MAP.floors(x, z).iter().filter(|f| f.open).count() > 1).last().expect("a slab");
        let open: Vec<f32> = MAP.floors(x, z).iter().filter(|f| f.open).map(|f| f.y).collect();
        let under = MAP.center(x, z).with_y(open[0] + 1.0);
        let up = trace_world_normal(under, Vec3::Y);
        assert!((up.0 - (open[1] - crate::map::SLAB - under.y)).abs() < 0.2 && up.1 == Vec3::NEG_Y, "slab overhead: {up:?}");
        let above = MAP.center(x, z).with_y(open[1] + 1.0);
        assert!((trace_world(above, Vec3::NEG_Y) - 1.0).abs() < 0.2, "slab underfoot");
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
