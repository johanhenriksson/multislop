//! Server-side bots. They produce an `Input` per tick like a client would, so the server simulates them with the
//! same movement, shooting and lag compensation code as everyone else.

use crate::map::MAP;
use crate::shared::*;
use bevy::math::Vec3;

/// Seconds between first seeing an enemy and being allowed to shoot. The main difficulty knob.
const REACTION_SECS: f32 = 0.6;
/// How fast bots can turn while aiming, in radians per second.
const AIM_TURN_SPEED: f32 = 3.5;
/// How far off a bot's aim can be, in radians (0.05 is about 1 m at 20 m; the hit capsule's radius is 0.4 m).
/// Rerolled for each new target and after every shot, and shrinks by AIM_SETTLE per tick while tracking, so far
/// targets and quick follow-up shots miss more.
const AIM_ERROR: f32 = 0.05;
const AIM_SETTLE: f32 = 0.995;
const SIGHT_RANGE: f32 = 60.0;
/// Half of the field of view in which bots notice enemies, in radians (60° each side).
const VIEW_HALF_ANGLE: f32 = 1.05;

/// What a bot knows about another player.
pub struct Seen {
    pub id: u32,
    pub team: Team,
    pub pos: Vec3,
    pub alive: bool,
}

pub struct Bot {
    seq: u32,
    /// The site this bot heads for this round.
    site: char,
    round: u32,
    /// Current enemy and the tick we first saw them.
    target: Option<(u32, u32)>,
    /// Current aim error as (yaw, pitch) in radians.
    aim_error: (f32, f32),
    /// Distance field towards the current goal cell, recomputed when the goal changes.
    field: Vec<u16>,
    field_goal: (usize, usize),
    /// Distance field towards the enemy spawn, to face the way enemies will come from while holding a spot.
    watch: Vec<u16>,
    watch_goal: (usize, usize),
    last_pos: Vec3,
    stuck_ticks: u32,
    rng: u32,
}

impl Bot {
    pub fn new(seed: u32) -> Bot {
        Bot {
            seq: 0,
            site: 'A',
            round: 0,
            target: None,
            aim_error: (0.0, 0.0),
            field: vec![],
            field_goal: (usize::MAX, 0),
            watch: vec![],
            watch_goal: (usize::MAX, 0),
            last_pos: Vec3::ZERO,
            stuck_ticks: 0,
            rng: seed.wrapping_mul(0x9E37_79B9) | 1,
        }
    }

    fn roll_aim_error(&mut self) -> (f32, f32) {
        ((self.random() * 2.0 - 1.0) * AIM_ERROR, (self.random() * 2.0 - 1.0) * AIM_ERROR * 0.5)
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Decides this tick's input.
    /// `callout` is an enemy position a teammate can currently see.
    pub fn think(
        &mut self,
        id: u32,
        me: &PlayerState,
        team: Team,
        round: &Round,
        players: &[Seen],
        callout: Option<Vec3>,
        tick: u32,
    ) -> Input {
        self.seq += 1;
        let mut inp = Input { seq: self.seq, yaw: me.yaw, pitch: me.pitch, view_tick: tick as f32, ..Default::default() };
        if round.number != self.round {
            self.round = round.number;
            // Ts all go for the same site; CTs split up to cover both.
            let coin = if team == Team::T { round.number % 2 == 0 } else { self.random() < 0.5 };
            self.site = if coin { 'A' } else { 'B' };
            self.target = None;
        }

        let eye = eye(me);
        let enemy = players
            .iter()
            // A new enemy has to be in view; one we're already fighting is tracked while we turn.
            .filter(|p| p.alive && p.team != team && can_see(eye, p.pos))
            .filter(|p| in_view(me, p.pos) || self.target.is_some_and(|(t, _)| t == p.id))
            .min_by(|a, b| a.pos.distance(me.pos).total_cmp(&b.pos.distance(me.pos)));
        match (enemy, self.target) {
            (Some(e), Some((t, _))) if e.id == t => {}
            (Some(e), _) => {
                self.target = Some((e.id, tick));
                self.aim_error = self.roll_aim_error();
            }
            (None, _) => self.target = None,
        }

        if let (Some(e), Some((_, since))) = (enemy, self.target) {
            // Stand and fight: turn towards the target at limited speed, fire once lined up and reacted.
            self.aim_error = (self.aim_error.0 * AIM_SETTLE, self.aim_error.1 * AIM_SETTLE);
            let (yaw, pitch) = look_at(eye, e.pos + Vec3::Y * 1.2);
            let (yaw, pitch) = (yaw + self.aim_error.0, pitch + self.aim_error.1);
            let max = AIM_TURN_SPEED * DT;
            inp.yaw = me.yaw + wrap(yaw - me.yaw).clamp(-max, max);
            inp.pitch = me.pitch + (pitch - me.pitch).clamp(-max, max);
            let lined_up = wrap(yaw - inp.yaw).abs() < 0.02 && (pitch - inp.pitch).abs() < 0.03;
            inp.fire = lined_up && me.fire_cd == 0 && tick - since >= (REACTION_SECS * TICK_HZ as f32) as u32;
            if inp.fire {
                self.aim_error = self.roll_aim_error();
            }
            return inp;
        }

        let (goal, use_bomb) = self.goal(id, me, team, round, callout);
        let at_goal = me.pos.with_y(0.0).distance(goal.with_y(0.0)) < 0.8;
        if use_bomb {
            inp.use_key = true;
            return inp;
        }
        if at_goal {
            // Hold the spot looking down the path the enemy would take to get here.
            let enemy_spawn = MAP.spawns(if team == Team::T { Team::Ct } else { Team::T })[0];
            let look = path_dir(&mut self.watch, &mut self.watch_goal, me.pos, enemy_spawn);
            let (yaw, _) = look_at(Vec3::ZERO, look);
            inp.yaw = me.yaw + wrap(yaw - me.yaw).clamp(-4.0 * DT, 4.0 * DT);
            return inp;
        }
        let dir = path_dir(&mut self.field, &mut self.field_goal, me.pos, goal);
        let (yaw, _) = look_at(Vec3::ZERO, dir);
        let max = 8.0 * DT;
        inp.yaw = me.yaw + wrap(yaw - me.yaw).clamp(-max, max);
        inp.pitch = me.pitch * 0.9;
        // Move along the path in world space, independent of where we're looking.
        let fwd = Vec3::new(-inp.yaw.sin(), 0.0, -inp.yaw.cos());
        let right = Vec3::new(inp.yaw.cos(), 0.0, -inp.yaw.sin());
        (inp.forward, inp.right) = (dir.dot(fwd), dir.dot(right));

        // Stuck on something: hop.
        if me.pos.distance(self.last_pos) < 0.5 * DT {
            self.stuck_ticks += 1;
        } else {
            self.stuck_ticks = 0;
        }
        self.last_pos = me.pos;
        inp.jump = self.stuck_ticks > 20;
        inp
    }

    /// Where to go, and whether we're there and should hold E (plant or defuse).
    fn goal(&mut self, id: u32, me: &PlayerState, team: Team, round: &Round, callout: Option<Vec3>) -> (Vec3, bool) {
        // Rotate to wherever a teammate is in a fight, unless there's bomb business to do.
        let site = callout.unwrap_or(MAP.site_center(self.site));
        match (team, round.bomb) {
            (Team::T, Bomb::Carried(c)) if c == id => {
                let on_site = MAP.site_at(me.pos) == Some(self.site);
                (site, on_site && me.on_ground)
            }
            (Team::T, Bomb::Dropped(p)) => (p, false),
            (_, Bomb::Planted(p)) if team == Team::Ct => (p, me.pos.distance(p) < BOMB_REACH - 0.3),
            (_, Bomb::Planted(p)) => (p + Vec3::X * 3.0, false),
            _ => (site, false),
        }
    }
}

/// Unit direction towards the next cell on the shortest path to `goal`, using a cached distance field.
fn path_dir(field: &mut Vec<u16>, field_goal: &mut (usize, usize), pos: Vec3, goal: Vec3) -> Vec3 {
    let goal_cell = MAP.cell(goal);
    if goal_cell != *field_goal {
        *field = MAP.distance_field(goal_cell);
        *field_goal = goal_cell;
    }
    let w = MAP.width();
    let (x, z) = MAP.cell(pos);
    let here = field[z * w + x];
    let next = MAP.neighbors(x, z).filter(|&(nx, nz)| field[nz * w + nx] < here).min_by_key(|&(nx, nz)| field[nz * w + nx]);
    let target = match next {
        Some((nx, nz)) if here > 1 => MAP.center(nx, nz),
        _ => goal,
    };
    (target - pos).with_y(0.0).normalize_or_zero()
}

/// Whether a position is inside this player's field of view (ignores walls).
pub fn in_view(viewer: &PlayerState, feet: Vec3) -> bool {
    let to = (feet + Vec3::Y * 1.2 - eye(viewer)).normalize_or_zero();
    aim_dir(viewer).dot(to) > VIEW_HALF_ANGLE.cos()
}

/// Line of sight from an eye to a player's chest, within sight range.
pub fn can_see(eye: Vec3, feet: Vec3) -> bool {
    let to = feet + Vec3::Y * 1.2 - eye;
    to.length() < SIGHT_RANGE && trace_world(eye, to.normalize()) > to.length()
}

fn look_at(from: Vec3, to: Vec3) -> (f32, f32) {
    let d = to - from;
    // Forward is (-sin yaw, 0, -cos yaw).
    (f32::atan2(-d.x, -d.z), f32::atan2(d.y, d.with_y(0.0).length()))
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
