//! The Rocketball client: a floodlit night stadium, the ball glowing in the color of the last team to touch it, players
//! and rockets in their team colors, a launcher or a scoped AWP in hand, goal celebrations and the HUD.

mod hud;
mod model;
mod sfx;
mod world;

use crate::game::RESPAWN_SECS;
use crate::map::GOAL_LINE;
use crate::shared::*;
use glam::{Quat, Vec2, Vec3};
use model::{Rig, barrel, viewmodel};
use sdk::Interpolated;
use sdk::client::*;
use sdk::scene::Scene;
use std::collections::{HashMap, VecDeque};

const MOUSE_SENS: f32 = 0.002;
const FOV: f32 = 1.6;
/// Scoped in with the AWP.
const ZOOM_FOV: f32 = 0.42;
pub const ORANGE: [f32; 3] = [1.0, 0.45, 0.1];
pub const BLUE: [f32; 3] = [0.15, 0.45, 1.0];
/// Things that belong to neither team: the untouched ball, midfield trim.
pub const NEUTRAL: [f32; 3] = [0.85, 0.92, 1.0];
/// Effects alive at once. Smoke and ball trails are skipped beyond this, so a rocket barrage can't flood the scene;
/// explosions and goals always show.
const MAX_FX: usize = 300;
/// Puffs along a rocket's flight, and ball trail segments, this far apart.
const SMOKE_EVERY: f32 = 0.7;
/// The ball leaves a trail above this speed.
const TRAIL_SPEED: f32 = 16.0;

pub fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

pub fn team_rgb(team: Team) -> [f32; 3] {
    match team {
        Team::Orange => ORANGE,
        Team::Blue => BLUE,
    }
}

/// The team color lightened for text on dark backgrounds.
pub fn team_text(team: Team) -> Color {
    match team {
        Team::Orange => Color::srgb(1.0, 0.62, 0.3),
        Team::Blue => Color::srgb(0.45, 0.72, 1.0),
    }
}

#[derive(Default)]
struct Sounds {
    launch: u32,
    awp: u32,
    explosion: u32,
    reload: u32,
    hit: u32,
    hurt: u32,
    thump: u32,
    clang: u32,
    horn: u32,
    roar: u32,
    swell: u32,
    ambience: u32,
    chime: u32,
    clank: u32,
    clack: u32,
    beep: u32,
    go: u32,
    whistle: u32,
    end: u32,
    pad: u32,
    jump: u32,
    respawn: u32,
}

/// Something short-lived: a puff that grows and fades, a light that dims, or confetti that tumbles down.
struct Fx {
    node: u32,
    pos: Vec3,
    /// Confetti flutters down with this velocity, turning at `spin` (axis times radians per second).
    vel: Option<Vec3>,
    rot: Quat,
    spin: Vec3,
    /// Its own material, faded with it and freed after.
    material: Option<(u32, Material)>,
    light: Option<f32>,
    age: f32,
    life: f32,
    size: f32,
    /// Grows to `1 + grow` times `size` over its life (shrinks when negative).
    grow: f32,
    /// A streak this long along its local Y, only its thickness following `size` and `grow`.
    length: Option<f32>,
}

/// Meshes and materials effects share.
#[derive(Default)]
struct Assets {
    puff: u32,
    /// A unit cylinder, stretched into ball trail segments.
    streak: u32,
    confetti: u32,
    /// Confetti in the scoring team's color, white and gold, by team.
    confetti_colors: [[u32; 3]; 2],
    rocket: u32,
    rocket_tail: u32,
    tracer: u32,
}

/// The ball's scene nodes and how it's drawn.
#[derive(Default)]
struct Ball {
    node: u32,
    material: u32,
    base: Material,
    shell_material: u32,
    /// Lights in the ball: untouched, Orange, Blue; only the one for the last toucher is on.
    lights: [u32; 3],
    /// Last toucher as last drawn; None until the first frame.
    touch: Option<Option<Team>>,
    rot: Quat,
    pos: Vec3,
    shown: bool,
    trail_from: Vec3,
}

#[derive(Default)]
pub struct BallClient {
    scene: Scene,
    sounds: Sounds,
    hud: hud::Hud,
    assets: Assets,
    pickups: Vec<world::Pickup>,
    /// Whether each pickup was shown as there to take, as last sent.
    pickups_up: Vec<bool>,
    me: Option<u32>,
    team: Option<Team>,
    pred: PlayerState,
    alive: bool,
    hp: i32,
    armor: i32,
    yaw: f32,
    pitch: f32,
    time: f64,
    started: bool,
    play: Vec<Play>,
    /// Everyone as of the newest snapshot.
    players: Vec<NetPlayer>,
    state: Option<Match>,
    rigs: HashMap<u32, Rig>,
    /// Rockets in flight: their node, and where the last puff of smoke was left.
    rockets: HashMap<u32, (u32, Vec3)>,
    ball: Ball,
    fx: Vec<Fx>,
    /// Kill feed lines, in the killer's team color.
    feed: VecDeque<(String, Option<Team>, f64)>,
    /// A short centered line (a kill, "GO!") and when it was shown.
    message: Option<(String, f64)>,
    died_at: f64,
    hurt_at: f64,
    tab: bool,
    /// The gun wanted in hand (1, 2, Q), and scoped in (right mouse).
    gun: Gun,
    last_gun: Gun,
    zoom: bool,
    /// First-person launcher and AWP, by `Gun as usize`.
    viewmodels: [u32; 2],
    /// Viewmodel kick, 1 right after firing and decaying to 0, the run bob's phase, and the sway behind turning
    /// with the view we last turned to.
    kick: f32,
    bob: f32,
    sway: Vec2,
    last_look: Vec2,
    /// Camera shake from nearby blasts, decaying.
    shake: f32,
    /// The kickoff count last beeped.
    count: u32,
    swell_at: f64,
    rng: u32,
}

impl BallClient {
    pub fn new() -> BallClient {
        BallClient { rng: 0x2545_F491, ..Default::default() }
    }
}

impl ClientGame for BallClient {
    // ponytail: one map, so `map` picks nothing.
    fn start(&mut self, _map: &str) -> Vec<Command> {
        let s = &mut self.scene;
        let mut sound = |wav: Vec<u8>| {
            let id = s.id();
            s.push(Command::Sound { id, wav });
            id
        };
        self.sounds = Sounds {
            launch: sound(sfx::launch()),
            awp: sound(sfx::awp()),
            explosion: sound(sfx::explosion()),
            reload: sound(sfx::reload()),
            hit: sound(sfx::hit()),
            hurt: sound(sfx::hurt()),
            thump: sound(sfx::thump()),
            clang: sound(sfx::clang()),
            horn: sound(sfx::horn()),
            roar: sound(sfx::roar()),
            swell: sound(sfx::swell()),
            ambience: sound(sfx::ambience()),
            chime: sound(sfx::chime()),
            clank: sound(sfx::clank()),
            clack: sound(sfx::clack()),
            beep: sound(sfx::beep(false)),
            go: sound(sfx::beep(true)),
            whistle: sound(sfx::whistle(1)),
            end: sound(sfx::whistle(3)),
            pad: sound(sfx::pad()),
            jump: sound(sfx::jump()),
            respawn: sound(sfx::respawn()),
        };
        self.pickups = world::build(&mut self.scene).pickups;
        let s = &mut self.scene;
        let flat = |c: [f32; 3]| Material { color: rgb(c), emissive: c.map(|v| v * 0.6), roughness: 0.6, double_sided: true, ..Default::default() };
        let confetti_colors = [ORANGE, BLUE].map(|team| [team, [1.0, 1.0, 1.0], [1.0, 0.8, 0.2]].map(|c| s.material(flat(c))));
        self.assets = Assets {
            puff: s.mesh(MeshData::Sphere { radius: 1.0, ico: Some(2) }),
            streak: s.mesh(MeshData::Cylinder { radius: 1.0, height: 1.0 }),
            confetti: s.mesh(MeshData::Cuboid(Vec3::new(0.22, 0.01, 0.15))),
            confetti_colors,
            rocket: s.mesh(MeshData::Cylinder { radius: 0.06, height: 0.6 }),
            rocket_tail: s.mesh(MeshData::Sphere { radius: 0.11, ico: Some(1) }),
            tracer: s.material(Material { color: Color::srgba(1.0, 0.9, 0.5, 0.5), unlit: true, blend: Blend::Blend, ..Default::default() }),
        };
        self.viewmodels = [Gun::Launcher, Gun::Awp].map(|g| model::spawn_gun(&mut self.scene, CAMERA, g, Transform::at(viewmodel(g)), true));
        self.spawn_ball();
        self.build_hud();
        std::mem::take(&mut self.scene.out)
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        let you = snap.you;
        self.me = Some(you);
        self.players = snap.entities.iter().filter_map(NetPlayer::from_entity).collect();
        let before = self.state.as_ref().map(|m| m.phase);
        self.state = decode(&snap.state);
        for e in snap.events.iter().filter_map(|e| decode::<Event>(e)) {
            self.on_event(e, you);
        }
        let now = self.state.as_ref().map(|m| m.phase);
        if let Some(phase) = now {
            self.on_phase(before, phase);
        }
        let Some(me) = self.players.iter().find(|p| p.id == you) else { return };
        let (alive, hp, armor, team, pos, yaw) = (me.alive, me.hp, me.armor, me.team, me.state.pos, me.state.yaw);
        if alive && !self.alive {
            // Respawned: face the way the server spawned us.
            (self.yaw, self.pitch) = (yaw, 0.0);
            self.sound(self.sounds.respawn, 0.5);
        } else if alive && hp < self.hp {
            self.hurt_at = self.time;
            self.sound(self.sounds.hurt, 0.7);
        }
        if matches!(now, Some(Phase::Kickoff { .. })) && !matches!(before, Some(Phase::Kickoff { .. })) {
            // Lined up for kickoff: face the center spot, as the server turned us.
            (self.yaw, self.pitch) = (f32::atan2(pos.x, pos.z), 0.0);
        }
        (self.alive, self.hp, self.armor, self.team) = (alive, hp, armor, Some(team));
    }

    fn input(&mut self, raw: &RawInput) -> Vec<u8> {
        let axis = |pos, neg| raw.held(pos) as i32 as f32 - raw.held(neg) as i32 as f32;
        let inp = Input {
            forward: axis(Key::W, Key::S),
            right: axis(Key::D, Key::A),
            yaw: self.yaw,
            pitch: self.pitch,
            jump: raw.held(Key::Space) || raw.held(Key::Backspace),
            fire: raw.captured && raw.held(Key::MouseLeft),
            reload: raw.held(Key::R),
            gun: self.gun,
            zoom: self.zoom,
            ..Default::default()
        };
        encode(&inp)
    }

    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, fresh: bool) {
        let step = predict(me, input, seq);
        if !fresh {
            return;
        }
        let Some(p) = NetPlayer::from_entity(me) else { return };
        self.pred = p.state;
        if !p.alive {
            return;
        }
        if let Some(d) = step.fired {
            let at = muzzle(eye(&self.pred), d, self.pred.gun);
            self.muzzle_flash(at);
            let sound = if self.pred.gun == Gun::Awp { self.sounds.awp } else { self.sounds.launch };
            self.sound(sound, 0.8);
            self.kick = 1.0;
        }
        if step.reloaded {
            self.sound(self.sounds.reload, 0.5);
        }
        if step.launched {
            self.sound(self.sounds.pad, 0.6);
        } else if step.jumped {
            self.sound(self.sounds.jump, 0.3);
        }
    }

    fn frame(&mut self, f: &FrameIn) -> FrameOut {
        self.time = f.time;
        if let Some(p) = f.me.as_ref().and_then(NetPlayer::from_entity) {
            self.pred = p.state;
        }
        if !self.started {
            self.started = true;
            self.play.push(Play { sound: self.sounds.ambience, volume: 0.3, looped: true });
        }
        self.controls(&f.input);
        if f.input.captured {
            let sens = if self.zoom { MOUSE_SENS * ZOOM_FOV / FOV } else { MOUSE_SENS };
            self.yaw -= f.input.mouse.x * sens;
            self.pitch = (self.pitch - f.input.mouse.y * sens).clamp(-1.55, 1.55);
        }
        self.render_world(&f.others, f.dt);
        self.render_pickups();
        self.render_viewmodel(f.dt);
        self.update_fx(f.dt);
        self.update_hud();
        self.crowd();

        // Dead: the view drifts up over where you were. Nearby blasts shake it.
        let rise = if self.alive { 0.0 } else { ((self.time - self.died_at) as f32).min(1.0) * 2.0 };
        self.shake *= (-f.dt * 7.0).exp();
        let jolt = self.shake * 0.03;
        let (dy, dp) = ((self.random() - 0.5) * jolt, (self.random() - 0.5) * jolt);
        let fov = if self.zoom { ZOOM_FOV } else { FOV };
        let camera = Camera { pos: f.me_pos + Vec3::Y * (EYE + rise), rot: look_rot(self.yaw + dy, self.pitch + dp), fov };
        FrameOut { commands: std::mem::take(&mut self.scene.out), sounds: std::mem::take(&mut self.play), camera, lines: vec![], spheres: vec![] }
    }
}

/// Barrel tip for a shot fired from eye `o` along `d`, matching the first-person model.
fn muzzle(o: Vec3, d: Vec3, gun: Gun) -> Vec3 {
    let right = d.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(d);
    let at = viewmodel(gun);
    o + right * at.x + up * at.y + d * (barrel(gun) - at.z)
}

impl BallClient {
    /// Gun keys (1 launcher, 2 AWP, Q the other one), the scope (right mouse, AWP only, off while reloading or dead)
    /// and the scoreboard.
    fn controls(&mut self, raw: &RawInput) {
        let want = match () {
            _ if raw.pressed(Key::Digit1) => Some(Gun::Launcher),
            _ if raw.pressed(Key::Digit2) => Some(Gun::Awp),
            _ if raw.pressed(Key::Q) => Some(self.last_gun),
            _ => None,
        };
        if let Some(gun) = want.filter(|&g| g != self.gun) {
            (self.last_gun, self.gun, self.zoom) = (self.gun, gun, false);
        }
        if raw.pressed(Key::MouseRight) && self.gun == Gun::Awp {
            self.zoom = !self.zoom;
        }
        if self.pred.gun != Gun::Awp || self.pred.hands.reloading > 0 || !self.alive {
            self.zoom = false;
        }
        self.tab = raw.held(Key::Tab);
    }

    fn sound(&mut self, sound: u32, volume: f32) {
        self.play.push(Play { sound, volume, looped: false });
    }

    /// A sound at `at`, quieter with distance from us.
    fn sound_at(&mut self, sound: u32, volume: f32, at: Vec3) {
        let v = volume / (1.0 + at.distance(self.pred.pos) / 15.0);
        self.sound(sound, v);
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    pub(super) fn name(&self, id: u32) -> String {
        self.players.iter().find(|p| p.id == id).map_or_else(|| format!("player {id}"), |p| p.name.clone())
    }

    fn on_event(&mut self, e: Event, you: u32) {
        match e {
            Event::Launch { shooter, from, dir } if shooter != you => {
                self.muzzle_flash(from + dir * 0.9);
                self.sound_at(self.sounds.launch, 0.8, from);
            }
            Event::Launch { .. } => {}
            Event::Shot { shooter, from, to } => {
                let from = if shooter == you { muzzle(eye(&self.pred), (to - from).normalize_or_zero(), Gun::Awp) } else { from };
                self.tracer(from, to);
                if shooter != you {
                    self.muzzle_flash(from + (to - from).normalize_or_zero() * 0.9);
                    self.sound_at(self.sounds.awp, 0.8, from);
                }
            }
            Event::Explosion { pos, radius, team } => {
                self.blast(pos, radius, team);
                self.sound_at(self.sounds.explosion, 1.0, pos);
                self.shake = self.shake.max(1.0 - pos.distance(self.pred.pos) / 14.0);
            }
            Event::BallHit { pos, strength, kind } => {
                let (sound, loud) = match kind {
                    HitKind::Push => (self.sounds.thump, 0.3 + strength / 25.0),
                    HitKind::Blast => (self.sounds.clang, 0.4 + strength / 30.0),
                    HitKind::Shot => (self.sounds.clang, 0.3),
                };
                self.sound_at(sound, loud.min(1.0), pos);
            }
            Event::Kill { killer, victim } => self.on_kill(killer, victim),
            Event::Goal { team, .. } => {
                let mouth = Vec3::new(crate::map::GOAL_HALF_WIDTH, crate::map::GOAL_HEIGHT, GOAL_LINE + crate::map::GOAL_DEPTH);
                self.celebrate(self.ball.pos.clamp(-mouth.with_y(-1.0), mouth), team);
                self.sound(self.sounds.horn, 0.8);
                self.sound(self.sounds.roar, 0.9);
            }
            Event::Pickup { player, kind } => {
                let sound = match kind {
                    Item::Health | Item::Mega => self.sounds.chime,
                    Item::Armor | Item::HeavyArmor => self.sounds.clank,
                    Item::Rockets => self.sounds.clack,
                };
                match self.players.iter().find(|p| p.id == player).map(|p| p.state.pos) {
                    _ if player == you => self.sound(sound, 0.7),
                    Some(at) => self.sound_at(sound, 0.4, at),
                    None => {}
                }
            }
        }
    }

    /// Countdown beeps, and the whistle at kickoff and at the end.
    fn on_phase(&mut self, before: Option<Phase>, now: Phase) {
        match (before, now) {
            (_, Phase::Kickoff { countdown }) => {
                let count = countdown.ceil() as u32;
                if count != self.count && (1..=3).contains(&count) {
                    self.sound(self.sounds.beep, 0.5);
                }
                self.count = count;
            }
            (Some(Phase::Kickoff { .. }), Phase::Play) => {
                self.sound(self.sounds.go, 0.5);
                self.sound(self.sounds.whistle, 0.6);
                self.message = Some(("GO!".into(), self.time));
            }
            (Some(Phase::Play | Phase::Goal { .. }), Phase::End { .. }) => self.sound(self.sounds.end, 0.7),
            _ => {}
        }
    }

    fn on_kill(&mut self, killer: u32, victim: u32) {
        let me = self.me;
        if Some(killer) == me && killer != victim {
            self.sound(self.sounds.hit, 0.7);
            self.message = Some((format!("You killed {}", self.name(victim)), self.time));
        }
        if Some(victim) == me {
            self.died_at = self.time;
        }
        let line = if killer == victim {
            format!("{} blew themself up", self.name(victim))
        } else {
            format!("{}  >  {}", self.name(killer), self.name(victim))
        };
        let team = self.players.iter().find(|p| p.id == killer).map(|p| p.team);
        self.feed.push_back((line, team, self.time));
        if self.feed.len() > 5 {
            self.feed.pop_front();
        }
    }

    /// The ball: a white sphere with dark seams, a glowing shell and lights inside in the last toucher's color.
    fn spawn_ball(&mut self) {
        let s = &mut self.scene;
        let mesh = s.mesh(world::ball_mesh());
        let base = world::ball_material(s);
        let material = s.material(base);
        let node = s.spawn(None, NodeKind::Mesh { mesh, material, shadows: true }, Transform::default(), false);
        let shell_mesh = s.mesh(MeshData::Sphere { radius: BALL_RADIUS * 1.12, ico: Some(4) });
        let shell_material = s.material(Material { color: Color::NONE, unlit: true, blend: Blend::Blend, ..Default::default() });
        s.spawn(Some(node), NodeKind::Mesh { mesh: shell_mesh, material: shell_material, shadows: false }, Transform::default(), true);
        let lights = [NEUTRAL, ORANGE, BLUE].map(|c| {
            let kind = NodeKind::PointLight { color: rgb(c), intensity: 0.0, range: 12.0 };
            s.spawn(Some(node), kind, Transform::default(), true)
        });
        self.ball = Ball { node, material, base, shell_material, lights, rot: Quat::IDENTITY, ..Default::default() };
    }

    /// Players, the ball and rockets, all from the interpolated entities.
    fn render_world(&mut self, others: &[Interpolated], dt: f32) {
        let mut seen = HashMap::new();
        let mut rockets = HashMap::new();
        let mut ball = None;
        for v in others {
            let e = &v.entity;
            match decode::<Thing>(&e.data) {
                Some(Thing::Player(p)) => {
                    // Alive as of the newest snapshot too, so someone just killed doesn't linger.
                    let alive = p.alive && !v.teleporting && self.players.iter().any(|q| q.id == e.id && q.alive);
                    seen.insert(e.id, (e.pos, e.yaw, e.pitch, p.team, p.state.gun, alive));
                }
                Some(Thing::Ball) => ball = Some((e.pos, e.vel)),
                Some(Thing::Rocket(r)) => {
                    rockets.insert(e.id, (e.pos, e.vel, r.team));
                }
                None => {}
            }
        }
        self.render_players(seen, dt);
        if let Some((pos, vel)) = ball {
            self.render_ball(pos, vel);
        }
        self.render_rockets(rockets);
    }

    fn render_players(&mut self, seen: HashMap<u32, (Vec3, f32, f32, Team, Gun, bool)>, dt: f32) {
        let gone: Vec<u32> = self.rigs.iter().filter(|(id, rig)| seen.get(id).is_none_or(|s| s.3 != rig.team)).map(|(&id, _)| id).collect();
        for id in gone {
            let rig = self.rigs.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: rig.root });
        }
        for (id, (pos, yaw, pitch, team, gun, alive)) in seen {
            if !self.rigs.contains_key(&id) {
                let rig = model::spawn_rig(&mut self.scene, team);
                self.rigs.insert(id, rig);
            }
            let rig = self.rigs.get_mut(&id).unwrap();
            let moved = pos.with_y(0.0).distance(rig.last_pos.with_y(0.0));
            rig.last_pos = pos;
            rig.stride += if moved < 2.0 { moved * 2.2 } else { 0.0 };
            let swing = rig.stride.sin() * 0.6 * (moved / dt.max(1e-3) / MAX_SPEED).min(1.0);
            let out = &mut self.scene.out;
            out.push(Command::Set { id: rig.root, transform: Some(Transform::at(pos).rotated(Quat::from_rotation_y(yaw))), visible: Some(alive), light: None });
            out.push(Command::set_transform(rig.arms, Transform::at(Vec3::Y * 1.35).rotated(Quat::from_rotation_x(pitch))));
            for (i, &node) in rig.guns.iter().enumerate() {
                out.push(Command::set_visible(node, i == gun as usize));
            }
            for (leg, (x, side)) in rig.legs.into_iter().zip([(-0.13, 1.0), (0.13, -1.0)]) {
                out.push(Command::set_transform(leg, Transform::at(Vec3::new(x, 0.8, 0.0)).rotated(Quat::from_rotation_x(swing * side))));
            }
        }
    }

    /// Rolls the ball along as it moves, colors it after the last toucher, trails it when fast, and hides it while a
    /// goal is celebrated (it burst).
    fn render_ball(&mut self, pos: Vec3, vel: Vec3) {
        let touch = self.state.as_ref().and_then(|m| m.last_touch);
        let shown = !matches!(self.state.as_ref().map(|m| m.phase), Some(Phase::Goal { .. }));
        let b = &mut self.ball;
        let moved = pos - b.pos;
        if moved.length() < 5.0 {
            let axis = Vec3::Y.cross(moved).normalize_or_zero();
            b.rot = (Quat::from_axis_angle(axis, moved.length() / BALL_RADIUS) * b.rot).normalize();
        } else {
            b.trail_from = pos;
        }
        b.pos = pos;
        let visible = (b.shown != shown).then_some(shown);
        b.shown = shown;
        self.scene.out.push(Command::Set { id: b.node, transform: Some(Transform::at(pos).rotated(b.rot)), visible, light: None });
        let c = touch.map_or(NEUTRAL, team_rgb);
        if b.touch != Some(touch) {
            b.touch = Some(touch);
            let strength = if touch.is_some() { 0.35 } else { 0.08 };
            let material = Material { emissive: c.map(|v| v * strength), ..b.base };
            self.scene.push(Command::Material { id: b.material, material });
            let shell = Material { color: Color::srgba(c[0], c[1], c[2], 0.13), unlit: true, blend: Blend::Blend, ..Default::default() };
            self.scene.push(Command::Material { id: b.shell_material, material: shell });
            let lit = touch.map_or(0, |t| t as usize + 1);
            for (i, &light) in b.lights.iter().enumerate() {
                let intensity = match i {
                    _ if i != lit => 0.0,
                    0 => 150_000.0,
                    _ => 600_000.0,
                };
                self.scene.push(Command::Set { id: light, transform: None, visible: None, light: Some(intensity) });
            }
        }
        let speed = vel.length();
        if !shown || speed <= TRAIL_SPEED {
            b.trail_from = pos;
        } else if pos.distance(b.trail_from) > SMOKE_EVERY * 1.5 && self.fx.len() < MAX_FX {
            let k = ((speed - TRAIL_SPEED) / 15.0).min(1.0);
            let from = std::mem::replace(&mut b.trail_from, pos);
            self.streak(from, pos, Color::srgba(c[0], c[1], c[2], 0.2 + 0.25 * k), BALL_RADIUS * 0.7);
        }
    }

    /// Rockets: an olive body along their flight with a glowing tail and light in the shooter's team color, leaving
    /// puffs of smoke behind.
    fn render_rockets(&mut self, mut now: HashMap<u32, (Vec3, Vec3, Team)>) {
        let gone: Vec<u32> = self.rockets.keys().filter(|id| !now.contains_key(id)).copied().collect();
        for id in gone {
            let (node, _) = self.rockets.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: node });
        }
        let pose = |pos: Vec3, vel: Vec3| Transform::at(pos).rotated(Quat::from_rotation_arc(Vec3::Y, vel.normalize_or(Vec3::Y)));
        let eye = eye(&self.pred);
        let mut smoke = vec![];
        for (&id, (node, last)) in &mut self.rockets {
            let Some((pos, vel, _)) = now.remove(&id) else { continue };
            self.scene.out.push(Command::set_transform(*node, pose(pos, vel)));
            // Not in our face as it leaves.
            if pos.distance(*last) > SMOKE_EVERY && pos.distance(eye) > 2.5 {
                *last = pos;
                smoke.push(pos - vel.normalize_or_zero() * 0.4);
            }
        }
        for at in smoke {
            if self.fx.len() < MAX_FX {
                self.fading(at, Color::srgba(0.75, 0.73, 0.7, 0.5), 0.12, 0.9, 3.0);
            }
        }
        for (id, (pos, vel, team)) in now {
            let s = &mut self.scene;
            let body = s.shared_material("rocket".into(), || Material { color: Color::srgb(0.25, 0.27, 0.22), emissive: [0.4, 0.3, 0.1], ..Default::default() });
            let c = team_rgb(team);
            let tail = s.shared_material(format!("rocket tail {team:?}"), || world::glow(c, 8.0));
            let node = s.spawn(None, NodeKind::Mesh { mesh: self.assets.rocket, material: body, shadows: false }, pose(pos, vel), true);
            s.spawn(Some(node), NodeKind::Mesh { mesh: self.assets.rocket_tail, material: tail, shadows: false }, Transform::at(Vec3::Y * -0.32), true);
            let light = NodeKind::PointLight { color: rgb(c), intensity: 120_000.0, range: 8.0 };
            s.spawn(Some(node), light, Transform::at(Vec3::Y * -0.5), true);
            self.rockets.insert(id, (node, pos));
        }
    }

    /// Pickups spin and bob over their pads; a taken one disappears and leaves its pad dark.
    fn render_pickups(&mut self) {
        let Some(up) = self.state.as_ref().map(|m| m.pickups.clone()) else { return };
        let t = self.time as f32;
        for (i, p) in self.pickups.iter().enumerate() {
            let there = up.get(i).copied().unwrap_or(true);
            if self.pickups_up.get(i) != Some(&there) {
                self.scene.push(Command::set_visible(p.model, there));
                self.scene.push(Command::set_visible(p.glow, there));
                self.scene.push(Command::Set { id: p.light, transform: None, visible: None, light: Some(if there { world::PICKUP_LIGHT } else { 0.0 }) });
            }
            if there {
                let pulse = if p.item == Item::Mega { 1.0 + 0.08 * (t * 5.0).sin() } else { 1.0 };
                let at = p.at + Vec3::Y * (1.0 + 0.12 * (t * 2.0 + i as f32).sin());
                let transform = Transform::at(at).rotated(Quat::from_rotation_y(t * 1.6 + i as f32)).scaled(pulse);
                self.scene.out.push(Command::set_transform(p.model, transform));
            }
        }
        self.pickups_up = up;
    }

    /// Our gun, moving like dust's: bobs while running, sways behind turns, kicks on firing, dips while reloading.
    /// Hidden when scoped.
    fn render_viewmodel(&mut self, dt: f32) {
        let look = Vec2::new(self.yaw, self.pitch);
        let turn = (look - self.last_look) / dt.max(1e-3);
        self.last_look = look;
        let target = (Vec2::new(-turn.x, turn.y) * 0.004).clamp(Vec2::splat(-0.04), Vec2::splat(0.04));
        self.sway = self.sway.lerp(target, (dt * 10.0).min(1.0));
        self.kick *= (-dt * 16.0).exp();
        let speed = if self.pred.on_ground { (self.pred.vel.with_y(0.0).length() / MAX_SPEED).min(1.0) } else { 0.0 };
        self.bob += dt * 10.0 * speed;
        let bob = Vec3::new(self.bob.sin() * 0.012, -(self.bob * 2.0).sin().abs() * 0.01, 0.0) * speed;
        let gun = self.pred.gun;
        let reload = self.pred.hands.reloading as f32 / (gun.ability().reload_secs() * TICK_HZ as f32);
        let dip = (reload * std::f32::consts::PI).sin();
        let at = viewmodel(gun) + bob + Vec3::new(self.sway.x, self.sway.y - 0.15 * dip, 0.05 * self.kick);
        let rot = Quat::from_rotation_x(-0.6 * dip + 0.12 * self.kick);
        let shown = self.alive && self.me.is_some() && !self.zoom;
        for (i, &id) in self.viewmodels.iter().enumerate() {
            let visible = Some(shown && i == gun as usize);
            self.scene.push(Command::Set { id, transform: Some(Transform::at(at).rotated(rot)), visible, light: None });
        }
    }

    /// A puff that grows to `1 + grow` times its size (shrinks when negative) and fades out over `life`.
    // ponytail: each puff has its own material to fade, resent every frame; MAX_FX bounds it. Share a few alpha
    // steps if that ever shows in a profile.
    fn fading(&mut self, at: Vec3, color: Color, size: f32, life: f32, grow: f32) {
        let material = Material { color, unlit: true, blend: Blend::Blend, ..Default::default() };
        let id = self.scene.material(material);
        let node = self.scene.spawn(None, NodeKind::Mesh { mesh: self.assets.puff, material: id, shadows: false }, Transform::at(at).scaled(size), true);
        self.fx.push(Fx { node, pos: at, vel: None, rot: Quat::IDENTITY, spin: Vec3::ZERO, material: Some((id, material)), light: None, age: 0.0, life, size, grow, length: None });
    }

    /// A segment of a glowing tail from `from` to `to`, thinning to nothing. Consecutive ones join into one streak.
    fn streak(&mut self, from: Vec3, to: Vec3, color: Color, radius: f32) {
        let material = Material { color, unlit: true, blend: Blend::Blend, ..Default::default() };
        let id = self.scene.material(material);
        let rot = Quat::from_rotation_arc(Vec3::Y, (to - from).normalize_or(Vec3::Y));
        let (pos, length) = ((from + to) / 2.0, from.distance(to));
        let transform = Transform { pos, rot, scale: Vec3::new(radius, length, radius) };
        let node = self.scene.spawn(None, NodeKind::Mesh { mesh: self.assets.streak, material: id, shadows: false }, transform, true);
        let fx = Fx { node, pos, vel: None, rot, spin: Vec3::ZERO, material: Some((id, material)), light: None, age: 0.0, life: 0.45, size: radius, grow: -1.0, length: Some(length) };
        self.fx.push(fx);
    }

    /// An AWP round's streak, fading fast.
    fn tracer(&mut self, from: Vec3, to: Vec3) {
        let rot = Quat::from_rotation_arc(Vec3::Y, (to - from).normalize_or(Vec3::Y));
        let (pos, length) = ((from + to) / 2.0, from.distance(to));
        let transform = Transform { pos, rot, scale: Vec3::new(0.02, length, 0.02) };
        let node = self.scene.spawn(None, NodeKind::Mesh { mesh: self.assets.streak, material: self.assets.tracer, shadows: false }, transform, true);
        self.fx.push(Fx { node, pos, vel: None, rot, spin: Vec3::ZERO, material: None, light: None, age: 0.0, life: 0.35, size: 0.02, grow: -1.0, length: Some(length) });
    }

    /// A point light that dims to nothing over `life`.
    fn flash(&mut self, at: Vec3, color: Color, intensity: f32, range: f32, life: f32) {
        let node = self.scene.spawn(None, NodeKind::PointLight { color, intensity, range }, Transform::at(at), true);
        self.fx.push(Fx { node, pos: at, vel: None, rot: Quat::IDENTITY, spin: Vec3::ZERO, material: None, light: Some(intensity), age: 0.0, life, size: 1.0, grow: 0.0, length: None });
    }

    fn muzzle_flash(&mut self, at: Vec3) {
        self.fading(at, Color::srgba(1.0, 0.85, 0.4, 0.9), 0.14, 0.07, 1.5);
        self.flash(at, Color::srgb(1.0, 0.8, 0.5), 80_000.0, 8.0, 0.07);
    }

    /// Dust's explosion: a fireball, a cloud of smoke and a flash of light, with a core in the shooter's team color.
    fn blast(&mut self, at: Vec3, radius: f32, team: Team) {
        let c = team_rgb(team);
        self.fading(at, Color::srgba(1.0, 0.55, 0.1, 0.95), radius * 0.15, 0.35, 4.0);
        self.fading(at, Color::srgba(c[0], c[1], c[2], 0.8), radius * 0.1, 0.22, 4.0);
        self.fading(at + Vec3::Y * 0.5, Color::srgba(0.25, 0.23, 0.22, 0.8), radius * 0.2, 1.6, 2.5);
        self.flash(at + Vec3::Y * 0.5, Color::srgb(1.0, 0.6, 0.2), 4_000_000.0, radius * 4.0, 0.4);
    }

    /// A goal: the ball bursts in the scoring team's color around a white-hot core, a flash lights up that end of
    /// the stadium, and confetti rains down.
    fn celebrate(&mut self, at: Vec3, team: Team) {
        let c = team_rgb(team);
        self.fading(at, Color::srgba(c[0], c[1], c[2], 0.9), BALL_RADIUS, 0.7, 7.0);
        self.fading(at, Color::srgba(1.0, 1.0, 1.0, 0.95), BALL_RADIUS * 0.8, 0.25, 4.0);
        self.fading(at, Color::srgba(1.0, 0.55, 0.1, 0.8), BALL_RADIUS, 0.45, 5.0);
        self.flash(at + Vec3::Y, rgb(c), 4.0e7, 60.0, 1.2);
        let colors = self.assets.confetti_colors[team as usize];
        for i in 0..90 {
            let (a, up, out) = (self.random() * std::f32::consts::TAU, 6.0 + self.random() * 9.0, 2.0 + self.random() * 9.0);
            let vel = Vec3::new(a.cos() * out, up, a.sin() * out);
            let spin = Vec3::new(self.random() - 0.5, self.random() - 0.5, self.random() - 0.5) * 24.0;
            // Half in the team's color, the rest white and gold.
            let material = colors[if i % 2 == 0 { 0 } else { 1 + i / 2 % 2 }];
            let node = self.scene.spawn(None, NodeKind::Mesh { mesh: self.assets.confetti, material, shadows: false }, Transform::at(at), true);
            let life = 3.0 + self.random() * 1.5;
            self.fx.push(Fx { node, pos: at, vel: Some(vel), rot: Quat::IDENTITY, spin, material: None, light: None, age: 0.0, life, size: 1.0, grow: 0.0, length: None });
        }
    }

    fn update_fx(&mut self, dt: f32) {
        let out = &mut self.scene.out;
        self.fx.retain_mut(|fx| {
            fx.age += dt;
            let k = fx.age / fx.life;
            if k >= 1.0 {
                out.push(Command::Despawn { id: fx.node });
                if let Some((id, _)) = fx.material {
                    out.push(Command::Free { id });
                }
                return false;
            }
            if let Some(light) = fx.light {
                out.push(Command::Set { id: fx.node, transform: None, visible: None, light: Some(light * (1.0 - k).powi(2)) });
                return true;
            }
            let mut scale = fx.size * (1.0 + fx.grow * k);
            if let Some(vel) = &mut fx.vel {
                // Confetti: falls slowly, held up by the air, and settles on the turf.
                vel.y -= 5.0 * dt;
                *vel *= (-1.6 * dt).exp();
                fx.pos += *vel * dt;
                if fx.pos.y < 0.02 {
                    (fx.pos.y, *vel) = (0.02, Vec3::ZERO);
                } else {
                    fx.rot = (Quat::from_scaled_axis(fx.spin * dt) * fx.rot).normalize();
                }
                scale = fx.size * (1.0 - ((k - 0.8) / 0.2).max(0.0));
            }
            let scale = fx.length.map_or(Vec3::splat(scale), |l| Vec3::new(scale, l, scale));
            out.push(Command::set_transform(fx.node, Transform { pos: fx.pos, rot: fx.rot, scale }));
            if let Some((id, mut material)) = fx.material {
                material.color = material.color.with_alpha(material.color.0[3] * (1.0 - k));
                out.push(Command::Material { id, material });
            }
            true
        });
    }

    /// Whole seconds until we're back, while dead.
    pub(super) fn respawn_in(&self) -> Option<u32> {
        (!self.alive && self.me.is_some()).then(|| (RESPAWN_SECS - (self.time - self.died_at) as f32).ceil().max(1.0) as u32)
    }

    /// The crowd rises when play gets near a goal.
    fn crowd(&mut self) {
        let play = matches!(self.state.as_ref().map(|m| m.phase), Some(Phase::Play));
        if play && self.ball.pos.z.abs() > GOAL_LINE - 16.0 && self.time - self.swell_at > 8.0 {
            self.swell_at = self.time;
            self.sound(self.sounds.swell, 0.5);
        }
    }
}
