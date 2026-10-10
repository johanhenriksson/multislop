//! Server-side bots. Each tick the game gives every bot a role by where the ball is: the nearest of a team attacks
//! (lines up behind the ball toward the enemy goal, then runs through it or rockets it), the one furthest back keeps
//! (between the ball and its own goal) with the AWP, and the rest support (trailing the ball on their own side).
//! Enemies nearer than the ball shot draw rockets at their feet, or AWP rounds at the body from much further. They
//! produce an `Input` per tick like a client would.

use crate::map::{GOAL_LINE, MAP};
use crate::shared::*;
use glam::Vec3;

/// Seconds between spotting an enemy and being allowed to fire. The main difficulty knob.
const REACTION_SECS: f32 = 0.5;
/// Turn speed while aiming, radians per second.
const TURN_SPEED: f32 = 5.0;
/// Aim error in radians, rerolled per target and per shot, settling by AIM_SETTLE per tick while tracking.
const AIM_ERROR: f32 = 0.06;
const AIM_SETTLE: f32 = 0.99;
/// Fires once aim (error included) is this close to on target.
const FIRE_ANGLE: f32 = 0.03;
/// Half the field of view in which bots notice enemies (60° each side).
const VIEW_HALF_ANGLE: f32 = 1.05;
/// Enemies further than this are left alone, with the launcher and with the AWP.
const FIGHT_RANGE: f32 = 25.0;
const SNIPE_RANGE: f32 = 70.0;
/// Where the attacker lines up, behind the ball on the line from the enemy goal through it.
const BEHIND: f32 = 3.5;
/// Lined up when the ball is within this angle of the way to the enemy goal (cos 30°).
const LINED_UP: f32 = 0.866;
/// Range in which a lined-up attacker rockets the ball rather than running through it.
const SHOT_RANGE: (f32, f32) = (4.0, 15.0);
/// The keeper comes out to clear a ball this near its goal.
const CLEAR_RANGE: f32 = 20.0;
/// Supporters trail the ball by this much.
const TRAIL: f32 = 12.0;
/// Go for health below this, and for pickups within this range.
const LOW_HP: i32 = 50;
const PICKUP_RANGE: f32 = 25.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Attack,
    Keep,
    Support,
}

/// What a bot knows about another player.
pub struct Seen {
    pub id: u32,
    pub team: Team,
    pub pos: Vec3,
    pub vel: Vec3,
    pub alive: bool,
}

/// The world as bots see it this tick.
pub struct View<'a> {
    pub ball: Vec3,
    pub ball_vel: Vec3,
    pub players: &'a [Seen],
    /// Whether each of `MAP.pickups` is up.
    pub pickups: &'a [bool],
}

pub struct Bot {
    seq: u32,
    rng: u32,
    yaw: f32,
    pitch: f32,
    /// Distance field towards `cell`, recomputed when the cell headed for changes.
    cell: (usize, usize),
    field: Vec<u16>,
    /// Current enemy and the tick it was first seen.
    target: Option<(u32, u32)>,
    aim_error: (f32, f32),
    last_pos: Vec3,
    stuck_ticks: u32,
}

impl Bot {
    pub fn new(seed: u32) -> Bot {
        Bot {
            seq: 0,
            rng: seed.wrapping_mul(0x9E37_79B9) | 1,
            yaw: 0.0,
            pitch: 0.0,
            cell: (usize::MAX, 0),
            field: vec![],
            target: None,
            aim_error: (0.0, 0.0),
            last_pos: Vec3::ZERO,
            stuck_ticks: 0,
        }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    fn roll_aim_error(&mut self) {
        self.aim_error = ((self.random() * 2.0 - 1.0) * AIM_ERROR, (self.random() * 2.0 - 1.0) * AIM_ERROR * 0.5);
    }

    pub fn think(&mut self, me: &NetPlayer, role: Role, view: &View, tick: u32) -> Input {
        self.seq += 1;
        let s = &me.state;
        // The keeper snipes, and takes out the launcher to clear a ball that comes near.
        // ponytail: the keeper changes as players move, and each change costs a draw. Hold on to the role if that shows.
        let keeping = role == Role::Keep && view.ball.distance(MAP.goal(me.team)) >= CLEAR_RANGE;
        let gun = if keeping { Gun::Awp } else { Gun::Launcher };
        let mut inp = Input { seq: self.seq, yaw: self.yaw, pitch: self.pitch, gun, zoom: gun == Gun::Awp, ..Default::default() };
        if !me.alive {
            self.target = None;
            return inp;
        }
        self.stuck_ticks = if s.pos.distance(self.last_pos) < 0.02 && !s.frozen { self.stuck_ticks + 1 } else { 0 };
        self.last_pos = s.pos;

        let (dest, ball_shot) = plan(me, role, view);
        let sniping = s.gun == Gun::Awp;
        let ball_shot = ball_shot.filter(|_| !sniping);
        let eye = eye(s);
        let enemy = self.enemy(me, view, ball_shot.map(|b| b.distance(eye)), tick);
        let aim = match (enemy, ball_shot) {
            (Some(e), _) if sniping => Some((body(e.pos), true)),
            // At their feet, where a near miss still hurts, leading their run.
            (Some(e), _) => Some((e.pos + e.vel * (e.pos.distance(eye) / ROCKET_SPEED) + Vec3::Y * 0.1, true)),
            (None, Some(at)) => Some((at, false)),
            (None, None) => None,
        };
        let look = aim.map_or(view.ball, |(at, _)| at);
        let (want_yaw, want_pitch) = look_at(eye, look);
        self.aim_error = (self.aim_error.0 * AIM_SETTLE, self.aim_error.1 * AIM_SETTLE);
        let (want_yaw, want_pitch) = (want_yaw + self.aim_error.0, want_pitch + self.aim_error.1);
        let turn = TURN_SPEED * DT;
        self.yaw += wrap(want_yaw - self.yaw).clamp(-turn, turn);
        self.pitch += (want_pitch - self.pitch).clamp(-turn, turn);
        (inp.yaw, inp.pitch) = (self.yaw, self.pitch);

        if let Some((at, at_enemy)) = aim {
            let off = wrap(want_yaw - self.yaw).abs().max((want_pitch - self.pitch).abs());
            let reacted = !at_enemy || self.target.is_some_and(|(_, since)| tick - since >= (REACTION_SECS * TICK_HZ as f32) as u32);
            // Never close enough to catch our own blast, and only with a clear line.
            inp.fire = reacted && off < FIRE_ANGLE && s.hands.cooldown == 0 && at.distance(eye) > 3.5 && can_see(eye, at);
            if inp.fire {
                self.roll_aim_error();
            }
        }
        inp.reload = s.loaded().0 == 0;

        let wish = self.steer(s.pos, dest);
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        (inp.forward, inp.right) = (wish.dot(fwd), wish.dot(right));
        inp.jump = self.stuck_ticks > 30 || (wish != Vec3::ZERO && self.random() < 0.002);
        inp
    }

    /// The enemy to fight: the current one while still in sight, else the nearest in view within range. Only if
    /// nearer than the ball shot, when there is one.
    fn enemy<'a>(&mut self, me: &NetPlayer, view: &View<'a>, shot: Option<f32>, tick: u32) -> Option<&'a Seen> {
        let eye = eye(&me.state);
        let range = if me.state.gun == Gun::Awp { SNIPE_RANGE } else { FIGHT_RANGE };
        let near = |p: &&Seen| {
            let d = p.pos.distance(eye);
            p.alive && p.team != me.team && d < range && shot.is_none_or(|s| d < s) && can_see(eye, body(p.pos))
        };
        let keep = self.target.and_then(|(t, _)| view.players.iter().filter(near).find(|p| p.id == t));
        if keep.is_some() {
            return keep;
        }
        let seen = view.players.iter().filter(near).filter(|p| in_view(self.yaw, eye, body(p.pos)));
        let nearest = seen.min_by(|a, b| a.pos.distance(eye).total_cmp(&b.pos.distance(eye)));
        self.target = nearest.map(|p| (p.id, tick));
        if nearest.is_some() {
            self.roll_aim_error();
        }
        nearest
    }

    /// Which way to run for `dest`: straight there when the way is clear, else along the walking distance field.
    fn steer(&mut self, pos: Vec3, dest: Vec3) -> Vec3 {
        let flat = (dest - pos).with_y(0.0);
        if flat.length() < 0.4 {
            return Vec3::ZERO;
        }
        let (x, z) = MAP.cell(dest);
        let ground = MAP.ground(x, z).y;
        if (ground - pos.y).abs() < 0.3 && can_see(pos + Vec3::Y * 0.4, dest.with_y(ground + 0.4)) {
            return flat.normalize();
        }
        if self.cell != (x, z) {
            (self.cell, self.field) = ((x, z), MAP.distance_field((x, z)));
        }
        let w = MAP.width();
        let (x, z) = MAP.cell(pos);
        let here = self.field.get(z * w + x).copied().unwrap_or(u16::MAX);
        let next = MAP.neighbors(x, z).filter(|&(nx, nz)| self.field[nz * w + nx] < here).min_by_key(|&(nx, nz)| self.field[nz * w + nx]);
        next.map_or(flat.normalize(), |(nx, nz)| (MAP.center(nx, nz) - pos).with_y(0.0).normalize_or_zero())
    }
}

/// Where to go, and where to rocket the ball if it's lined up for a shot on goal.
fn plan(me: &NetPlayer, role: Role, view: &View) -> (Vec3, Option<Vec3>) {
    let (s, ball) = (&me.state, view.ball);
    let (own, theirs) = (MAP.goal(me.team), MAP.goal(me.team.other()));
    // Low on health or out of rockets: the nearest pickup that helps, if one is up nearby.
    let wants = |item: Item| match item {
        Item::Health | Item::Mega => me.hp < LOW_HP,
        Item::Rockets => s.ammo == (0, 0),
        Item::Armor | Item::HeavyArmor => false,
    };
    let pickup = MAP.pickups.iter().zip(view.pickups).filter(|&(&(at, item), &up)| up && wants(item) && at.distance(s.pos) < PICKUP_RANGE);
    if let Some((&(at, _), _)) = pickup.min_by(|a, b| a.0.0.distance(s.pos).total_cmp(&b.0.0.distance(s.pos))) {
        return (at, None);
    }
    let attack = role == Role::Attack || (role == Role::Keep && ball.distance(own) < CLEAR_RANGE);
    let dest = match role {
        _ if attack => {
            let to_goal = (theirs - ball).with_y(0.0).normalize_or_zero();
            let to_ball = (ball - s.pos).with_y(0.0);
            if to_ball.normalize_or_zero().dot(to_goal) > LINED_UP {
                let dist = to_ball.length();
                let lead = ball + view.ball_vel * (dist / ROCKET_SPEED);
                // Its lower half, so the blast lifts it over whatever is in the way.
                let shot = (s.ammo.0 > 0 && (SHOT_RANGE.0..SHOT_RANGE.1).contains(&dist)).then(|| lead - Vec3::Y * BALL_RADIUS * 0.5);
                return (ball + to_goal * 2.0, shot);
            }
            let behind = ball - to_goal * BEHIND;
            // On the wrong side of it: swing round on the side we're on, rather than through it toward our own goal.
            let side = Vec3::new(to_goal.z, 0.0, -to_goal.x);
            if (s.pos - ball).dot(to_goal) > -1.0 {
                behind + side * (s.pos - ball).dot(side).signum() * BEHIND
            } else {
                behind
            }
        }
        Role::Keep => own + (ball - own).with_y(0.0).normalize_or_zero() * 5.0,
        _ => {
            let back = ball + (own - ball).with_y(0.0).normalize_or_zero() * TRAIL;
            back + Vec3::X * if me.id.is_multiple_of(2) { 5.0 } else { -5.0 }
        }
    };
    (dest.clamp(Vec3::new(-22.5, 0.0, -GOAL_LINE + 1.0), Vec3::new(22.5, 20.0, GOAL_LINE - 1.0)), None)
}

fn in_view(yaw: f32, eye: Vec3, at: Vec3) -> bool {
    let (want, _) = look_at(eye, at);
    wrap(want - yaw).abs() < VIEW_HALF_ANGLE
}

fn look_at(from: Vec3, to: Vec3) -> (f32, f32) {
    let d = to - from;
    (f32::atan2(-d.x, -d.z), f32::atan2(d.y, d.with_y(0.0).length()))
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
