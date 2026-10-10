//! Server-side bots. They produce an `Input` per tick like a client would, so the server simulates them with the
//! same movement, shooting and lag compensation code as everyone else.

use crate::map::MAP;
use crate::shared::*;
use crate::weapons::{Slot, Weapon};
use glam::Vec3;

/// Seconds between first seeing an enemy and being allowed to shoot. The main difficulty knob.
const REACTION_SECS: f32 = 0.6;
/// How fast bots can turn while aiming, in radians per second.
const AIM_TURN_SPEED: f32 = 3.5;
/// How far off a bot's aim can be, in radians (0.05 is about 1 m at 20 m; the hit capsule's radius is 0.4 m).
/// Rerolled for each new target and after every shot, and shrinks by AIM_SETTLE per tick while tracking, so far
/// targets and quick follow-up shots miss more.
const AIM_ERROR: f32 = 0.06;
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
    /// A radio call this bot makes this tick, picked up by the server.
    pub radio: Option<Radio>,
    /// No more radio from this bot before this tick.
    radio_quiet_until: u32,
    /// Said "in position" this round already.
    positioned: bool,
    seq: u32,
    /// The site this bot heads for this round, and the spot on it to plant at or hold.
    site: char,
    spot: Vec3,
    /// A detour on the way, so bots don't all walk the same shortest path every round.
    via: Option<Vec3>,
    /// CT: spots next to cover that watch where Ts enter the site, and when to move to another one.
    cover: Vec<Vec3>,
    reposition_at: u32,
    /// Strafe direction in a fight (-1 or 1) and when to switch it.
    strafe: f32,
    strafe_until: u32,
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
            spot: Vec3::ZERO,
            via: None,
            cover: vec![],
            reposition_at: 0,
            strafe: 1.0,
            strafe_until: 0,
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
            radio: None,
            radio_quiet_until: 0,
            positioned: false,
        }
    }

    /// Radio a call with some probability, at most every few seconds.
    fn say(&mut self, call: Radio, tick: u32, chance: f32) {
        if tick >= self.radio_quiet_until && self.random() < chance {
            self.radio = Some(call);
            self.radio_quiet_until = tick + secs(8.0);
        }
    }

    fn roll_aim_error(&mut self, scale: f32) -> (f32, f32) {
        let e = AIM_ERROR * scale;
        ((self.random() * 2.0 - 1.0) * e, (self.random() * 2.0 - 1.0) * e * 0.5)
    }

    /// Spots near a site, next to a wall or crate, that can see where the Ts come in. Shuffled; best first.
    fn cover_spots(&mut self, site: char) -> Vec<Vec3> {
        let center = MAP.site_center(site);
        // Where the T path enters the site area: the first point on it within 14 m of the site.
        let field = MAP.distance_field(MAP.cell(center));
        let w = MAP.width();
        let mut cell = MAP.cell(MAP.spawns(Team::T)[0]);
        while MAP.center(cell.0, cell.1).distance(center.with_y(0.0)) > 14.0 {
            let here = field[cell.1 * w + cell.0];
            match MAP.neighbors(cell.0, cell.1).find(|&(x, z)| field[z * w + x] < here) {
                Some(next) => cell = next,
                None => break,
            }
        }
        let choke = MAP.ground(cell.0, cell.1);
        let near_cover = |x: usize, z: usize| {
            [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
                let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
                !MAP.walkable(nx, nz)
            })
        };
        let mut spots: Vec<Vec3> = (0..MAP.depth())
            .flat_map(|z| (0..w).map(move |x| (x, z)))
            .filter(|&(x, z)| MAP.walkable(x, z) && near_cover(x, z))
            .map(|(x, z)| MAP.ground(x, z))
            .filter(|p| p.distance(center.with_y(p.y)) < 12.0 && p.distance(choke) > 6.0)
            .filter(|p| can_see(*p + Vec3::Y * EYE, choke))
            .collect();
        for i in (1..spots.len()).rev() {
            let j = (self.random() * (i + 1) as f32) as usize;
            spots.swap(i, j.min(i));
        }
        spots
    }

    /// A random floor cell on a bomb site, near its middle.
    fn random_site_spot(&mut self, site: char) -> Vec3 {
        let center = MAP.site_center(site);
        let spots: Vec<Vec3> = MAP.site_cells().filter(|p| MAP.site_at(*p) == Some(site) && p.distance(center) < 8.0).collect();
        let pick = (self.random() * spots.len() as f32) as usize;
        spots.get(pick).map_or(center, |p| p.with_y(MAP.floor_at(*p)))
    }

    /// A random point that makes a sensible detour between `from` and `to` (at most `percent`% of the shortest
    /// path), so each bot takes its own route.
    fn pick_via(&mut self, from: Vec3, to: Vec3, percent: u32) -> Option<Vec3> {
        let (to_goal, to_start) = (MAP.distance_field(MAP.cell(to)), MAP.distance_field(MAP.cell(from)));
        let w = MAP.width();
        let (sx, sz) = MAP.cell(from);
        let direct = to_goal[sz * w + sx] as u32;
        let candidates: Vec<(usize, usize)> = (0..MAP.depth())
            .flat_map(|z| (0..w).map(move |x| (x, z)))
            .filter(|&(x, z)| {
                let (a, b) = (to_start[z * w + x] as u32, to_goal[z * w + x] as u32);
                MAP.walkable(x, z) && a > 4 && b > 4 && a + b <= direct * percent / 100 + 4
            })
            .collect();
        let pick = (self.random() * candidates.len() as f32) as usize;
        candidates.get(pick).map(|&(x, z)| MAP.ground(x, z))
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
        t_site: char,
        tick: u32,
    ) -> Input {
        self.seq += 1;
        let mut inp = Input { seq: self.seq, yaw: me.yaw, pitch: me.pitch, view_tick: tick as f32, ..Default::default() };
        // Best gun in hand.
        inp.slot = match (me.primary, me.slot) {
            (Some(_), Slot::Primary) | (None, Slot::Secondary) => 0,
            (Some(_), _) => 1,
            (None, _) => 2,
        };
        let spec = me.weapon().spec();
        if round.number != self.round {
            self.round = round.number;
            // Ts go for the site the server picked, except the odd lurker; CTs split up to cover both.
            let other = |s: char| if s == 'A' { 'B' } else { 'A' };
            self.site = match team {
                Team::T if self.random() < 0.15 => other(t_site),
                Team::T => t_site,
                Team::Ct if self.random() < 0.5 => 'A',
                Team::Ct => 'B',
            };
            self.spot = self.random_site_spot(self.site);
            if team == Team::Ct {
                self.cover = self.cover_spots(self.site);
                if let Some(&first) = self.cover.first() {
                    self.spot = first;
                }
                self.reposition_at = tick + secs(6.0 + 6.0 * self.random());
            }
            // Ts detour less so they still arrive roughly together.
            let detour = if team == Team::T { 128 } else { 135 };
            self.via = self.pick_via(me.pos, self.spot, detour);
            self.positioned = false;
            self.target = None;
        }

        let eye = eye(me);
        let visible_enemies = players.iter().filter(|p| p.alive && p.team != team && can_see(eye, p.pos) && in_view(me, p.pos)).count();
        // Our target just went down.
        if self.target.is_some_and(|(t, _)| players.iter().any(|p| p.id == t && !p.alive)) && visible_enemies == 0 {
            self.say(Radio::SectorClear, tick, 0.6);
        }
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
                self.aim_error = self.roll_aim_error(1.0);
                self.say(if visible_enemies >= 2 { Radio::NeedBackup } else { Radio::EnemySpotted }, tick, 0.7);
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
            let reacted = tick - since >= (REACTION_SECS * TICK_HZ as f32) as u32;
            let ready = me.fire_cd == 0 && me.reloading == 0;
            // Automatic weapons in bursts as recoil builds; semi-automatic ones need the trigger released.
            let trigger = if spec.auto { me.recoil < 3.0 } else { !me.trigger };
            // Counter-strafe: only shoot once slowed down, since moving throws shots off.
            let steady = me.vel.with_y(0.0).length() < 0.4 * 6.35 || !me.on_ground;
            inp.fire = lined_up && reacted && ready && trigger && steady;
            inp.zoom = me.weapon() == Weapon::Awp;
            if inp.fire {
                self.aim_error = self.roll_aim_error(if spec.auto { 0.5 } else { 1.0 });
            }
            // Strafe side to side while lining up or waiting to fire; back off while reloading.
            if tick >= self.strafe_until {
                self.strafe = if self.random() < 0.5 { -1.0 } else { 1.0 };
                self.strafe_until = tick + secs(0.35 + 0.5 * self.random());
            }
            let about_to_fire = lined_up && reacted && ready && trigger;
            if me.reloading > 0 {
                (inp.forward, inp.right) = (-1.0, self.strafe);
            } else if !about_to_fire && me.weapon() != Weapon::Awp {
                inp.right = self.strafe;
            }
            return inp;
        }

        // Top up between fights.
        inp.reload = me.ammo().is_some_and(|(mag, reserve)| reserve > 0 && mag < spec.mag / 2);
        if self.via.is_some_and(|v| v.with_y(0.0).distance(me.pos.with_y(0.0)) < 2.5) {
            self.via = None;
        }
        let (goal, use_bomb) = self.goal(id, me, team, round, callout);
        let at_goal = me.pos.with_y(0.0).distance(goal.with_y(0.0)) < 0.8;
        if use_bomb {
            if team == Team::T {
                self.say(Radio::CoverMe, tick, 1.0);
            }
            inp.use_key = true;
            return inp;
        }
        // CTs holding a site move between cover spots now and then so they're not always in the same place.
        if team == Team::Ct && tick >= self.reposition_at && !matches!(round.bomb, Bomb::Planted(_)) && self.cover.len() > 1 {
            let pick = (self.random() * self.cover.len() as f32) as usize;
            self.spot = self.cover[pick];
            self.reposition_at = tick + secs(6.0 + 6.0 * self.random());
        }
        // Look down the path the enemy would take to get here.
        let enemy_spawn = MAP.spawns(if team == Team::T { Team::Ct } else { Team::T })[0];
        let watch = path_dir(&mut self.watch, &mut self.watch_goal, me.pos, enemy_spawn);
        if at_goal {
            if team == Team::Ct && !self.positioned {
                self.positioned = true;
                self.say(Radio::InPosition, tick, 0.5);
            }
            let (yaw, _) = look_at(Vec3::ZERO, watch);
            inp.yaw = me.yaw + wrap(yaw - me.yaw).clamp(-4.0 * DT, 4.0 * DT);
            return inp;
        }
        let dir = path_dir(&mut self.field, &mut self.field_goal, me.pos, goal);
        // A CT moving between spots on its site keeps watching the T entrance and sidesteps instead.
        let near_site = me.pos.with_y(0.0).distance(MAP.site_center(self.site).with_y(0.0)) < 15.0;
        let facing = if team == Team::Ct && near_site { watch } else { dir };
        let (yaw, _) = look_at(Vec3::ZERO, facing);
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
        let route = self.via.unwrap_or(self.spot);
        // Rotate to wherever a teammate is in a fight, unless there's bomb business to do.
        let site = callout.unwrap_or(route);
        match (team, round.bomb) {
            (Team::T, Bomb::Carried(c)) if c == id => {
                let on_site = MAP.site_at(me.pos) == Some(self.site);
                (route, on_site && me.on_ground)
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

fn secs(s: f32) -> u32 {
    (s * TICK_HZ as f32) as u32
}

fn look_at(from: Vec3, to: Vec3) -> (f32, f32) {
    let d = to - from;
    // Forward is (-sin yaw, 0, -cos yaw).
    (f32::atan2(-d.x, -d.z), f32::atan2(d.y, d.with_y(0.0).length()))
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
