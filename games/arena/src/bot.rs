//! Server-side bots: they roam the arena, and rail whoever they spot after a reaction delay, turning at a limited
//! speed with some aim error. They produce an `Input` per tick like a client would.

use crate::map::MAP;
use crate::shared::*;
use glam::Vec3;

/// Seconds between spotting someone and being allowed to fire. The main difficulty knob.
const REACTION_SECS: f32 = 0.45;
/// Turn speed while aiming, radians per second.
const TURN_SPEED: f32 = 5.0;
/// Aim error in radians, rerolled per target and per shot, settling by AIM_SETTLE per tick while tracking.
const AIM_ERROR: f32 = 0.035;
const AIM_SETTLE: f32 = 0.99;
/// Fires once aim (error included) is this close to on target.
const FIRE_ANGLE: f32 = 0.02;
/// Half the field of view in which bots notice others (60° each side).
const VIEW_HALF_ANGLE: f32 = 1.05;

/// What a bot knows about another player.
pub struct Seen {
    pub id: u32,
    pub pos: Vec3,
    pub alive: bool,
}

pub struct Bot {
    seq: u32,
    rng: u32,
    yaw: f32,
    pitch: f32,
    goal: Vec3,
    /// Distance field towards the goal cell.
    field: Vec<u16>,
    /// Current target and the tick it was first seen.
    target: Option<(u32, u32)>,
    aim_error: (f32, f32),
    strafe: f32,
    strafe_until: u32,
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
            goal: Vec3::ZERO,
            field: vec![],
            target: None,
            aim_error: (0.0, 0.0),
            strafe: 1.0,
            strafe_until: 0,
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

    /// A new random spot to head for, one this bot can walk to from where it is.
    fn new_goal(&mut self, from: Vec3) {
        let here = MAP.cell(from);
        for _ in 0..20 {
            let (x, z) = ((self.random() * MAP.width() as f32) as usize, (self.random() * MAP.depth() as f32) as usize);
            if MAP.is_wall(x, z) {
                continue;
            }
            let field = MAP.distance_field((x, z));
            if field[here.1 * MAP.width() + here.0] != u16::MAX {
                (self.goal, self.field) = (MAP.ground(x, z), field);
                return;
            }
        }
    }

    pub fn think(&mut self, id: u32, me: &PlayerState, alive: bool, others: &[Seen], tick: u32) -> Input {
        self.seq += 1;
        if !alive {
            // Hold fire to respawn as soon as allowed, and pick a fresh goal from wherever we come back.
            (self.target, self.field) = (None, vec![]);
            return Input { seq: self.seq, fire: true, yaw: self.yaw, ..Default::default() };
        }
        if self.field.is_empty() || me.pos.with_y(0.0).distance(self.goal.with_y(0.0)) < 1.5 || self.stuck_ticks > 128 {
            self.new_goal(me.pos);
            self.stuck_ticks = 0;
        }
        self.stuck_ticks = if me.pos.distance(self.last_pos) < 0.02 { self.stuck_ticks + 1 } else { 0 };
        self.last_pos = me.pos;

        let eye = eye(me);
        let visible = |p: &&Seen| p.id != id && p.alive && in_view(self.yaw, eye, chest(p.pos)) && can_see(eye, chest(p.pos));
        let keep = self.target.and_then(|(t, since)| others.iter().find(|p| p.id == t).filter(visible).map(|p| (p.pos, since)));
        let target = match keep {
            Some(t) => Some(t),
            None => {
                let nearest = others.iter().filter(visible).min_by(|a, b| a.pos.distance(me.pos).total_cmp(&b.pos.distance(me.pos)));
                self.target = nearest.map(|p| (p.id, tick));
                self.roll_aim_error();
                nearest.map(|p| (p.pos, tick))
            }
        };

        let path = path_dir(&self.field, me.pos);
        let mut inp = Input { seq: self.seq, ..Default::default() };
        let (want_yaw, want_pitch) = match target {
            Some((pos, _)) => {
                let (y, p) = look_at(eye, chest(pos));
                self.aim_error = (self.aim_error.0 * AIM_SETTLE, self.aim_error.1 * AIM_SETTLE);
                (y + self.aim_error.0, p + self.aim_error.1)
            }
            None if path != Vec3::ZERO => (f32::atan2(-path.x, -path.z), 0.0),
            None => (self.yaw, 0.0),
        };
        let turn = TURN_SPEED * DT;
        self.yaw += wrap(want_yaw - self.yaw).clamp(-turn, turn);
        self.pitch += (want_pitch - self.pitch).clamp(-turn, turn);
        (inp.yaw, inp.pitch) = (self.yaw, self.pitch);

        // Move along the path, relative to where we face; in a fight, strafe side to side as well.
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        (inp.forward, inp.right) = (path.dot(fwd).round(), path.dot(right).round());
        if let Some((pos, since)) = target {
            if tick >= self.strafe_until {
                self.strafe = if self.random() < 0.5 { -1.0 } else { 1.0 };
                self.strafe_until = tick + (TICK_HZ as f32 * (0.4 + self.random())) as u32;
            }
            inp.right = self.strafe;
            let (y, p) = look_at(eye, chest(pos));
            let off = wrap(y + self.aim_error.0 - self.yaw).abs().max((p + self.aim_error.1 - self.pitch).abs());
            let reacted = tick - since >= (REACTION_SECS * TICK_HZ as f32) as u32;
            inp.fire = reacted && off < FIRE_ANGLE && me.hands.cooldown == 0;
            if inp.fire {
                self.roll_aim_error();
            }
        }
        inp.jump = self.random() < 0.01 || self.stuck_ticks > 30;
        inp
    }
}

/// Which way to walk to get one cell closer to the goal.
fn path_dir(field: &[u16], pos: Vec3) -> Vec3 {
    let w = MAP.width();
    let (x, z) = MAP.cell(pos);
    let here = field.get(z * w + x).copied().unwrap_or(u16::MAX);
    let next = MAP.neighbors(x, z).filter(|&(nx, nz)| field[nz * w + nx] < here).min_by_key(|&(nx, nz)| field[nz * w + nx]);
    next.map_or(Vec3::ZERO, |(nx, nz)| (MAP.center(nx, nz) - pos).with_y(0.0).normalize_or_zero())
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
