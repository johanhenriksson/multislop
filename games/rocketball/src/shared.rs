//! What client and server both run: movement (the arena's: Quake style, fast and floaty), the rocket launcher and the
//! AWP, the ball's physics and the wire format.

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

pub const MAX_HP: i32 = 100;
/// Mega health goes up to this; anything over `MAX_HP` drains 1 HP a second.
pub const MEGA_HP: i32 = 200;
pub const MAX_ARMOR: i32 = 100;
pub const HEAVY_ARMOR: i32 = 150;
/// Armor takes this share of damage until it runs out.
pub const ARMOR_TAKES: f32 = 2.0 / 3.0;
/// Rockets carried besides the load.
pub const SPAWN_RESERVE: u16 = 8;
pub const MAX_RESERVE: u16 = 16;

pub const ROCKET_SPEED: f32 = 40.0;
/// Seconds a rocket flies before going off on its own.
pub const ROCKET_FUSE: f32 = 5.0;
pub const BLAST_DAMAGE: f32 = 100.0;
pub const BLAST_RADIUS: f32 = 4.5;
/// Blasts throw players up to this fast at the center, nothing at `BLAST_RADIUS`, away and a bit up.
pub const KNOCKBACK: f32 = 11.0;
const KNOCK_LIFT: f32 = 0.4;

/// A straight rocket that goes off on whatever it touches. Hold fire to keep shooting.
pub const ROCKET: Ability = Ability {
    trigger: Trigger::Auto,
    cost: Cost::Ammo { mag: 4, reserve: SPAWN_RESERVE, reload: 2.0 },
    cycle: 0.8,
    accuracy: Accuracy { spread: 0.0, moving: 0.0, airborne: 0.0, kick: 0.0 },
    delivery: Delivery::Projectile { speed: ROCKET_SPEED, gravity: 0.0, bounce: 0.0, fuse: ROCKET_FUSE, contact: true },
    payload: Payload::Explode { damage: BLAST_DAMAGE, radius: BLAST_RADIUS, armor_pen: 1.0 - ARMOR_TAKES },
};

/// Dust's AWP, for picking off attackers from the back: one body shot kills without armor. Unscoped spread; scoped
/// is `AWP_SCOPED_SPREAD`.
pub const AWP: Ability = Ability {
    trigger: Trigger::Semi,
    cost: Cost::Ammo { mag: 5, reserve: SPAWN_ROUNDS, reload: 3.0 },
    cycle: 1.45,
    accuracy: Accuracy { spread: 0.06, moving: 0.1, airborne: 0.12, kick: 0.03 },
    delivery: Delivery::Hitscan { range: RANGE, falloff: 1.0 },
    payload: Payload::Damage { amount: AWP_DAMAGE, armor_pen: 1.0 - ARMOR_TAKES },
};
pub const AWP_DAMAGE: f32 = 115.0;
pub const AWP_SCOPED_SPREAD: f32 = 0.0005;
/// Running speed kept while scoped in.
const SCOPED_SPEED: f32 = 0.6;
/// AWP rounds carried besides the magazine.
pub const SPAWN_ROUNDS: u16 = 10;
pub const MAX_ROUNDS: u16 = 20;
/// An AWP round into the ball sends it on at this speed, along the shot.
pub const SHOT_PUSH: f32 = 10.0;
/// Seconds to take out the other gun.
pub const DRAW_SECS: f32 = 0.6;
/// How far back AWP shots are lag compensated.
pub const MAX_REWIND_TICKS: f32 = (0.4 * TICK_HZ) as f32;

/// What a player holds. Everyone carries both: the launcher to play the ball, the AWP to guard from the back.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Gun {
    #[default]
    Launcher,
    Awp,
}

impl Gun {
    pub fn ability(self) -> &'static Ability {
        match self {
            Gun::Launcher => &ROCKET,
            Gun::Awp => &AWP,
        }
    }
}

pub const BALL_RADIUS: f32 = 1.2;
const BALL_GRAVITY: f32 = 9.0;
pub const BALL_MAX_SPEED: f32 = 45.0;
/// Speed kept off the floor (and block tops), off walls, blocks, crossbar and ceiling, and off an edge or corner,
/// which is rounded off so a ball rolls up over a step instead of popping back.
const FLOOR_BOUNCE: f32 = 0.6;
const WALL_BOUNCE: f32 = 0.75;
const EDGE_BOUNCE: f32 = 0.2;
/// Slower than this into a surface, the ball rests on it instead of bouncing.
const SETTLE: f32 = 1.0;
/// Rolling friction, m/s², and air drag, the share of speed lost per second.
const ROLL_FRICTION: f32 = 1.0;
const AIR_DRAG: f32 = 0.03;
/// Longest move per substep: well under the ball's radius and the thinnest box, so nothing is tunneled through.
const BALL_SUBSTEP: f32 = 0.25;
/// Blasts push the ball up to this fast with its surface at the center, nothing from `BLAST_PUSH_RADIUS` on, away
/// from the blast and lifted by `BLAST_LIFT` before normalizing.
pub const BLAST_PUSH: f32 = 22.0;
pub const BLAST_PUSH_RADIUS: f32 = 6.0;
const BLAST_LIFT: f32 = 0.25;
/// A player running into the ball sends it off at this times their closing speed, and at least `DRIBBLE`.
const TOUCH_GAIN: f32 = 1.5;
const DRIBBLE: f32 = 3.0;

/// Entity ids: players count up from 1, rockets from `ROCKET_IDS`, and the ball has its own.
pub const BALL_ID: u32 = u32::MAX - 1;
pub const ROCKET_IDS: u32 = 1 << 30;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    /// Defends the +z end.
    Orange,
    /// Defends the -z end.
    Blue,
}

impl Team {
    pub fn other(self) -> Team {
        match self {
            Team::Orange => Team::Blue,
            Team::Blue => Team::Orange,
        }
    }

    /// The sign of z at the end this team defends.
    pub fn end(self) -> f32 {
        match self {
            Team::Orange => 1.0,
            Team::Blue => -1.0,
        }
    }
}

/// What a pickup gives.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    /// +25 HP up to 100.
    Health,
    /// +50 armor up to 100.
    Armor,
    /// +8 rockets in reserve up to 16, and +5 AWP rounds up to 20.
    Rockets,
    /// +100 HP up to 200.
    Mega,
    /// +100 armor up to 150.
    HeavyArmor,
}

impl Item {
    pub fn from_letter(c: u8) -> Option<Item> {
        Some(match c {
            b'h' => Item::Health,
            b'a' => Item::Armor,
            b'r' => Item::Rockets,
            b'M' => Item::Mega,
            b'A' => Item::HeavyArmor,
            _ => return None,
        })
    }

    /// Seconds until it's back once taken.
    pub fn respawn_secs(self) -> f32 {
        match self {
            Item::Health => 20.0,
            Item::Armor => 25.0,
            Item::Rockets => 15.0,
            Item::Mega | Item::HeavyArmor => 35.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug)]
pub struct Input {
    /// Stamped by the host, not sent as part of the input.
    #[serde(skip)]
    pub seq: u32,
    /// The server tick the client was seeing, stamped by the host like `seq`; AWP shots are rewound to it.
    #[serde(skip)]
    pub view_tick: f32,
    pub forward: f32,
    pub right: f32,
    pub yaw: f32,
    pub pitch: f32,
    /// Held: jumps again on every landing.
    pub jump: bool,
    /// Held: fires a rocket every cycle, or the AWP once per press.
    pub fire: bool,
    pub reload: bool,
    /// The gun wanted in hand; a change takes `DRAW_SECS`.
    pub gun: Gun,
    /// Scoped in (AWP).
    pub zoom: bool,
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
    pub hands: Hands,
    pub gun: Gun,
    /// Rockets (loaded, reserve).
    pub ammo: (u8, u16),
    /// AWP rounds (loaded, reserve).
    pub rounds: (u8, u16),
    /// Kickoff countdown, goal and end screens: look around, nothing else. Part of the state so prediction agrees
    /// with the server.
    pub frozen: bool,
}

impl PlayerState {
    /// Standing at `pos` facing `yaw` with a fresh load.
    pub fn spawn(pos: Vec3, yaw: f32, frozen: bool) -> PlayerState {
        let (ammo, rounds) = ((ROCKET.full_ammo().unwrap().0, SPAWN_RESERVE), AWP.full_ammo().unwrap());
        PlayerState { pos, yaw, ammo, rounds, frozen, ..Default::default() }
    }

    /// The held gun's (loaded, reserve).
    pub fn loaded(&self) -> (u8, u16) {
        match self.gun {
            Gun::Launcher => self.ammo,
            Gun::Awp => self.rounds,
        }
    }
}

/// A player as clients see it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetPlayer {
    /// The entity id, filled in by `NetPlayer::from_entity`.
    #[serde(skip)]
    pub id: u32,
    pub team: Team,
    pub state: PlayerState,
    pub hp: i32,
    pub armor: i32,
    pub alive: bool,
    pub goals: u32,
    pub kills: i32,
    pub deaths: u32,
    pub bot: bool,
    pub name: String,
    /// Round trip as the client measures it, for the scoreboard. 0 for bots.
    pub ping: u16,
}

/// A rocket in flight, drawn from snapshots.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct NetRocket {
    pub owner: u32,
    pub team: Team,
}

/// What an `sdk::Entity` is, in its `data`. The ball's last toucher is in `Match::last_touch`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Thing {
    Player(NetPlayer),
    /// The ball, `BALL_ID`: pos is its center. Its `teleport` is bumped when it's put back on the center spot.
    Ball,
    Rocket(NetRocket),
}

impl NetPlayer {
    pub fn entity(&self, teleport: u8) -> Entity {
        let s = &self.state;
        Entity { id: self.id, pos: s.pos, vel: s.vel, yaw: s.yaw, pitch: s.pitch, teleport, data: encode(&Thing::Player(self.clone())) }
    }

    pub fn from_entity(e: &Entity) -> Option<NetPlayer> {
        let Thing::Player(mut p) = decode(&e.data)? else { return None };
        p.id = e.id;
        (p.state.pos, p.state.vel, p.state.yaw, p.state.pitch) = (e.pos, e.vel, e.yaw, e.pitch);
        Some(p)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    /// Everyone frozen on their kickoff spot, the ball on the center spot; play starts when `countdown` (seconds)
    /// runs out.
    Kickoff { countdown: f32 },
    Play,
    /// `team` just scored, credited to `scorer` (the scoring team's last toucher, or with `own_goal` the defending
    /// team's when only they touched it; None if nobody did). Kickoff follows, or the end in golden goal.
    Goal { team: Team, scorer: Option<u32>, own_goal: bool },
    /// The match is over (`None` a draw); the scoreboard shows until the next one.
    End { winner: Option<Team> },
}

/// The match as clients see it, in `Frame::state`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Match {
    /// Goals, indexed by `Team as usize`.
    pub score: [u32; 2],
    pub phase: Phase,
    /// Seconds left on the clock, which only runs in play: of the match, or of overtime.
    pub time_left: f32,
    /// Golden goal: the next goal wins.
    pub overtime: bool,
    /// Whether each of `MAP.pickups` is there to take.
    pub pickups: Vec<bool>,
    /// The team that last touched the ball (pushed or blasted it); None since kickoff.
    pub last_touch: Option<Team>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum HitKind {
    /// Run into, or flown into someone.
    Push,
    /// Thrown by a blast.
    Blast,
    /// Hit by an AWP round.
    Shot,
}

// ponytail: events are sent once, so a dropped snapshot loses that effect. Resend for a few ticks if that shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A rocket left `from` going `dir`: the launch flash and sound. The rocket itself is an entity.
    Launch { shooter: u32, from: Vec3, dir: Vec3 },
    /// An AWP round from `from` that stopped at `to`.
    Shot { shooter: u32, from: Vec3, to: Vec3 },
    /// A rocket went off; `team` is its shooter's.
    Explosion { pos: Vec3, radius: f32, team: Team },
    /// The ball was sent off at `strength` m/s, from `pos` (its center).
    BallHit { pos: Vec3, strength: f32, kind: HitKind },
    /// `killer` is the victim themself for a suicide.
    Kill { killer: u32, victim: u32 },
    Goal { team: Team, scorer: Option<u32>, own_goal: bool },
    Pickup { player: u32, kind: Item },
}

/// What happened to a player in one tick, for effects and sounds.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Step {
    /// The held gun went off, this way.
    pub fired: Option<Vec3>,
    pub jumped: bool,
    /// Thrown by a jump pad.
    pub launched: bool,
    /// Started reloading.
    pub reloaded: bool,
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

/// The middle of the body, where blasts are measured to.
pub fn body(pos: Vec3) -> Vec3 {
    pos + Vec3::Y * 0.9
}

pub fn aim_dir(s: &PlayerState) -> Vec3 {
    look_rot(s.yaw, s.pitch) * Vec3::NEG_Z
}

/// Advances one tick of movement and the gun in hand.
pub fn step_player(s: &mut PlayerState, inp: &Input) -> Step {
    s.yaw = inp.yaw;
    s.pitch = inp.pitch.clamp(-1.55, 1.55);
    let mut step = Step::default();
    if inp.gun != s.gun {
        s.gun = inp.gun;
        s.hands.draw(DRAW_SECS);
    }
    let scoped = s.gun == Gun::Awp && inp.zoom;

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
        accelerate(&mut s.vel, wish, MAX_SPEED * if scoped { SCOPED_SPEED } else { 1.0 }, ACCEL);
    } else {
        accelerate(&mut s.vel, wish, AIR_CAP, AIR_ACCEL);
    }
    s.vel.y -= GRAVITY * DT;
    move_and_collide(s);

    let (mut hands, mut ammo) = (s.hands, s.loaded());
    if hands.tick(s.gun.ability(), Some(&mut ammo), inp.fire, inp.reload, s.frozen).is_some() {
        step.fired = Some(match s.gun {
            Gun::Launcher => aim_dir(s),
            Gun::Awp => {
                let speed = (s.vel.with_y(0.0).length() / MAX_SPEED).min(1.0);
                hands.aim(&AWP, scoped.then_some(AWP_SCOPED_SPREAD), speed, s.on_ground, s.yaw, s.pitch, inp.seq)
            }
        });
    }
    step.reloaded = s.hands.reloading == 0 && hands.reloading > 0;
    s.hands = hands;
    match s.gun {
        Gun::Launcher => s.ammo = ammo,
        Gun::Awp => s.rounds = ammo,
    }
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
    // Rocket jumps off a pad could otherwise clear the ceiling that keeps the ball in.
    if s.pos.y + HEIGHT > WALL_TOP {
        s.pos.y = WALL_TOP - HEIGHT;
        s.vel.y = s.vel.y.min(0.0);
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
    } else if d.y > 0.0 && (WALL_TOP - o.y) / d.y < best {
        (best, normal) = ((WALL_TOP - o.y) / d.y, Vec3::NEG_Y);
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

/// Where a ray (unit `d`) enters the ball centered at `c`.
pub fn trace_ball(o: Vec3, d: Vec3, c: Vec3) -> Option<f32> {
    let t = d.dot(c - o);
    let dist2 = (o + d * t).distance_squared(c);
    let r2 = BALL_RADIUS * BALL_RADIUS;
    (dist2 <= r2 && t + r2.sqrt() > 0.0).then(|| (t - (r2 - dist2).sqrt()).max(0.0))
}

/// Inside the map: between the outer walls, above the ground, below the ceiling.
pub fn in_bounds(p: Vec3) -> bool {
    let (w, d) = MAP.size();
    p.is_finite() && p.x.abs() < w / 2.0 && p.z.abs() < d / 2.0 && p.y >= 0.0 && p.y <= WALL_TOP
}

/// One tick of the ball: gravity and drag, then moved in substeps short enough that it can't pass through
/// anything, bouncing off the floor, the ceiling and every box it touches (sphere against box by closest point, so an
/// edge below its center sends it up over a step and one above sends it back), then rolling friction.
pub fn step_ball(pos: &mut Vec3, vel: &mut Vec3) {
    vel.y -= BALL_GRAVITY * DT;
    *vel = (*vel * (1.0 - AIR_DRAG * DT)).clamp_length_max(BALL_MAX_SPEED);
    let steps = (vel.length() * DT / BALL_SUBSTEP).ceil().max(1.0);
    let mut rolling = false;
    for _ in 0..steps as usize {
        *pos += *vel * DT / steps;
        rolling |= bounce(pos, vel, pos.with_y(0.0));
        rolling |= bounce(pos, vel, pos.with_y(WALL_TOP));
        for &(c, h) in &MAP.boxes {
            rolling |= bounce(pos, vel, pos.clamp(c - h, c + h));
        }
    }
    let speed = vel.with_y(0.0).length();
    if rolling && speed > 0.0 {
        let keep = (speed - ROLL_FRICTION * DT).max(0.0) / speed;
        (vel.x, vel.z) = (vel.x * keep, vel.z * keep);
    }
}

/// Pushes the ball out of a surface whose nearest point to its center is `closest`, bouncing it if it was moving into
/// it. Returns whether that surface is a floor it can roll on.
// ponytail: a center that got inside a box (never at 45 m/s with these substeps) is left there; the game's
// out-of-map check puts it back on the center spot if it ends up outside the walls.
fn bounce(pos: &mut Vec3, vel: &mut Vec3, closest: Vec3) -> bool {
    let d = *pos - closest;
    let dist = d.length();
    if dist >= BALL_RADIUS || dist < 1e-5 {
        return false;
    }
    let n = d / dist;
    *pos += n * (BALL_RADIUS - dist);
    let into = vel.dot(n);
    if into < 0.0 {
        let keep = if -into < SETTLE {
            0.0
        } else if n.abs().max_element() < 0.99 {
            EDGE_BOUNCE
        } else if n.y > 0.0 {
            FLOOR_BOUNCE
        } else {
            WALL_BOUNCE
        };
        *vel -= n * into * (1.0 + keep);
    }
    n.y > 0.7
}

/// A player standing at `feet`, moving at `pvel`, against the ball: pushes it out to touching and, if they were
/// closing in, sends it off along the contact normal at `TOUCH_GAIN` times the closing speed, at least `DRIBBLE`. A
/// ball flying into a still player bounces off the same way. Returns the speed added.
pub fn touch_ball(pos: &mut Vec3, vel: &mut Vec3, feet: Vec3, pvel: Vec3) -> Option<f32> {
    let low = feet.y + RADIUS;
    let closest = feet.with_y(pos.y.clamp(low, low + HEIGHT - 2.0 * RADIUS));
    let d = *pos - closest;
    let reach = BALL_RADIUS + RADIUS;
    if d.length_squared() >= reach * reach {
        return None;
    }
    let n = d.try_normalize().unwrap_or_else(|| pvel.with_y(0.0).try_normalize().unwrap_or(Vec3::X));
    *pos = closest + n * reach;
    let closing = (pvel - *vel).dot(n);
    if closing <= 0.0 {
        return None;
    }
    let kick = (TOUCH_GAIN * closing).max(DRIBBLE);
    *vel = (*vel + n * kick).clamp_length_max(BALL_MAX_SPEED);
    Some(kick)
}

/// A blast at `at` throws the ball: full `BLAST_PUSH` with its surface in the blast, falling off to nothing at
/// `BLAST_PUSH_RADIUS`, away from the blast and lifted, so a rocket under the ball lofts it. Returns the speed added.
pub fn blast_ball(pos: Vec3, vel: &mut Vec3, at: Vec3) -> Option<f32> {
    let d = pos - at;
    let dist = (d.length() - BALL_RADIUS).max(0.0);
    if dist >= BLAST_PUSH_RADIUS {
        return None;
    }
    let push = BLAST_PUSH * (1.0 - dist / BLAST_PUSH_RADIUS);
    *vel = (*vel + (d.normalize_or(Vec3::Y) + Vec3::Y * BLAST_LIFT).normalize() * push).clamp_length_max(BALL_MAX_SPEED);
    Some(push)
}

/// The velocity a blast at `at` adds to a player standing at `feet`: away from it and up, fading out at the blast
/// radius. Teammates and the shooter are thrown too, which is what makes rocket jumps.
pub fn knockback(feet: Vec3, at: Vec3) -> Vec3 {
    let d = body(feet) - at;
    let dist = d.length();
    if dist >= BLAST_RADIUS {
        return Vec3::ZERO;
    }
    (d.normalize_or(Vec3::Y) + Vec3::Y * KNOCK_LIFT).normalize() * KNOCKBACK * (1.0 - dist / BLAST_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::GOAL_LINE;

    fn secs(s: f32) -> usize {
        (s * TICK_HZ as f32) as usize
    }

    #[test]
    fn never_leaves_the_map() {
        for (i, sp) in MAP.spawns.iter().flatten().chain(MAP.kickoffs.iter().flatten()).enumerate() {
            let mut s = PlayerState::spawn(*sp, 0.0, false);
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
    fn launcher_fires_its_load_then_reloads() {
        let mut s = PlayerState::spawn(MAP.spawns[0][0], 0.0, false);
        let fire = Input { fire: true, ..Default::default() };
        let steps: Vec<Step> = (0..secs(5.5)).map(|_| step_player(&mut s, &fire)).collect();
        let shots = steps.iter().filter(|s| s.fired.is_some()).count();
        // 4 at 0.8 s apart (2.4 s), a 2 s reload, then the next load.
        assert_eq!((shots, steps.iter().filter(|s| s.reloaded).count()), (6, 1));
        assert_eq!(s.ammo.1, SPAWN_RESERVE - 4);
    }

    #[test]
    fn switching_to_the_awp_takes_a_draw_then_fires_once_per_press() {
        let mut s = PlayerState::spawn(MAP.spawns[0][0], 0.0, false);
        let awp = Input { gun: Gun::Awp, zoom: true, ..Default::default() };
        let fire = Input { fire: true, ..awp };
        for _ in 0..secs(DRAW_SECS) - 2 {
            step_player(&mut s, &awp);
        }
        assert!(step_player(&mut s, &fire).fired.is_none(), "still drawing");
        for _ in 0..2 {
            step_player(&mut s, &awp);
        }
        let shots = (0..secs(3.0)).filter(|_| step_player(&mut s, &fire).fired.is_some()).count();
        assert_eq!(shots, 1, "held: one shot");
        assert_eq!((s.rounds.0, s.ammo.0), (AWP.full_ammo().unwrap().0 - 1, 4));
    }

    #[test]
    fn ball_bounces_and_settles() {
        let (mut pos, mut vel) = (Vec3::new(0.0, 10.0, 0.0), Vec3::new(3.0, 0.0, 2.0));
        let mut bounces = 0;
        for _ in 0..secs(15.0) {
            let falling = vel.y < 0.0;
            step_ball(&mut pos, &mut vel);
            bounces += (falling && vel.y > 0.5) as u32;
            assert!(pos.y >= BALL_RADIUS - 1e-3, "{pos}");
        }
        assert!(bounces >= 3, "{bounces} bounces");
        assert!((pos.y - BALL_RADIUS).abs() < 1e-3 && vel.length() < 1e-3, "at rest on the floor: {pos} {vel}");
    }

    #[test]
    fn ball_never_tunnels_or_leaves() {
        let mut rng = 99u32;
        for _ in 0..60 {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let (a, b) = (rng as f32 * 1e-9, (rng >> 7) as f32 * 1e-8);
            let dir = Vec3::new(a.cos() * b.cos(), b.sin().abs() * 0.5 - 0.1, a.sin() * b.cos()).normalize();
            let (mut pos, mut vel) = (Vec3::new(0.0, 3.0, 0.0), dir * BALL_MAX_SPEED);
            for t in 0..secs(6.0) {
                step_ball(&mut pos, &mut vel);
                let inside = pos.x.abs() <= 24.0 - BALL_RADIUS + 0.01 && pos.z.abs() <= GOAL_LINE + 4.0 - BALL_RADIUS + 0.01;
                assert!(inside && in_bounds(pos), "shot {dir} tick {t}: {pos}");
            }
        }
        // Straight at the side wall and into a goal's back wall at full speed.
        for v in [Vec3::X, Vec3::Z] {
            let (mut pos, mut vel) = (Vec3::new(0.0, BALL_RADIUS, 0.0), v * BALL_MAX_SPEED);
            let mut most = 0.0f32;
            for _ in 0..secs(3.0) {
                step_ball(&mut pos, &mut vel);
                most = most.max(pos.dot(v));
            }
            assert!(most <= MAP.size().0.max(MAP.size().1), "{most}");
            assert!(most < if v == Vec3::X { 24.0 } else { GOAL_LINE + 4.0 } - BALL_RADIUS + 0.01, "{v}: {most}");
        }
    }

    /// Rolls the ball from `from` along `dir` at `speed` for `secs`, returning where it ends up.
    fn roll(from: Vec3, dir: Vec3, speed: f32, secs_: f32) -> Vec3 {
        let (mut pos, mut vel) = (from.with_y(BALL_RADIUS), dir * speed);
        for _ in 0..secs(secs_) {
            step_ball(&mut pos, &mut vel);
        }
        pos
    }

    #[test]
    fn blocks_stop_a_rolling_ball_but_it_climbs_stairs() {
        // The chest-high blocks at x -16..-12, z -26..-22: rolled at from the open side, it comes back.
        let back = roll(Vec3::new(-14.0, 0.0, -14.0), Vec3::NEG_Z, 12.0, 3.0);
        assert!(back.z > -22.0 + BALL_RADIUS - 0.01 && back.y < BALL_RADIUS + 0.1, "stopped by the block: {back}");
        // A 4 m pillar at x -16..-14, z -10..-8 does the same at full speed.
        let back = roll(Vec3::new(-15.0, 0.0, 0.0), Vec3::NEG_Z, 30.0, 2.0);
        assert!(back.z > -8.0, "stopped by the pillar: {back}");
        // The 0.5 m stairs at z -12..-6 up to the 2 m sideline platform at x -24..-20: a step is below the center,
        // so a ball rolls up onto the first, and a fast one hops on up to the platform.
        let up = roll(Vec3::new(-22.0, 0.0, -20.0), Vec3::Z, 8.0, 4.0);
        assert!((up.y - 0.5 - BALL_RADIUS).abs() < 0.05, "up on the first step: {up}");
        let up = roll(Vec3::new(-22.0, 0.0, -20.0), Vec3::Z, 15.0, 4.0);
        assert!((up.y - 2.0 - BALL_RADIUS).abs() < 0.05, "up on the platform: {up}");
    }

    #[test]
    fn a_blast_under_the_ball_lofts_it() {
        let (mut pos, mut vel) = (Vec3::new(0.0, BALL_RADIUS, 10.0), Vec3::ZERO);
        let push = blast_ball(pos, &mut vel, Vec3::new(0.0, 0.05, 9.5)).unwrap();
        assert!(push > 0.9 * BLAST_PUSH && vel.y > 15.0 && vel.z > 0.0, "{push} {vel}");
        let mut top = 0.0f32;
        for _ in 0..secs(3.0) {
            step_ball(&mut pos, &mut vel);
            top = top.max(pos.y);
        }
        assert!(top > 1.5 + BALL_RADIUS + 10.0, "over the blocks: {top}");
        assert_eq!(blast_ball(pos.with_y(BALL_RADIUS), &mut vel, pos + Vec3::X * (BLAST_PUSH_RADIUS + BALL_RADIUS)), None);
    }

    #[test]
    fn running_into_the_ball_pushes_it() {
        let mut s = PlayerState::spawn(Vec3::new(0.0, 0.0, 10.0), 0.0, false);
        let (mut pos, mut vel) = (Vec3::new(0.0, BALL_RADIUS, 0.0), Vec3::ZERO);
        let mut kicks = vec![];
        for _ in 0..secs(2.0) {
            step_player(&mut s, &Input { forward: 1.0, ..Default::default() });
            kicks.extend(touch_ball(&mut pos, &mut vel, s.pos, s.vel));
            step_ball(&mut pos, &mut vel);
        }
        assert!(!kicks.is_empty() && kicks[0] > MAX_SPEED, "{kicks:?}");
        assert!(vel.z < -MAX_SPEED && pos.z < s.pos.z - 1.0, "the ball runs on ahead: {pos} {vel}");
    }
}
