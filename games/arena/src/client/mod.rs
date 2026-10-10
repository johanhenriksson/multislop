//! The Instaslop client: the arena, players in their own colors, rail beams, gibs, a railgun in hand and the HUD.

mod hud;
mod sfx;
mod world;

use crate::shared::*;
use glam::{Quat, Vec3};
use sdk::Interpolated;
use sdk::client::*;
use sdk::scene::Scene;
use std::collections::{HashMap, VecDeque};

const MOUSE_SENS: f32 = 0.002;
const FOV: f32 = 1.6;

pub fn rgb([r, g, b]: [u8; 3]) -> Color {
    Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

/// A player color for text: dark ones lifted towards white so they read on dark backgrounds.
pub fn readable(c: [u8; 3]) -> Color {
    let [r, g, b, _] = rgb(c).0;
    let lift = (0.5 - (0.3 * r + 0.6 * g + 0.1 * b)).max(0.0) * 1.4;
    Color::srgb(r + (1.0 - r) * lift, g + (1.0 - g) * lift, b + (1.0 - b) * lift)
}

#[derive(Default)]
struct Sounds {
    rail: u32,
    frag: u32,
    gib: u32,
    pad: u32,
    jump: u32,
    respawn: u32,
    ambience: u32,
}

/// A remote player's scene nodes.
struct Rig {
    root: u32,
    arms: u32,
    legs: [u32; 2],
    stride: f32,
    last_pos: Vec3,
}

/// Something that fades out over its life: a beam, a gib, a puff. Moves if it has velocity.
struct Fx {
    node: u32,
    /// Its own material, faded with it and freed after.
    material: Option<(u32, Material)>,
    transform: Transform,
    vel: Option<Vec3>,
    spin: Quat,
    age: f32,
    life: f32,
    /// Scale reached at the end of its life, times the starting one.
    grow: Vec3,
}

/// Meshes and materials effects share.
#[derive(Default)]
struct Assets {
    beam: u32,
    gib: u32,
    puff: u32,
    armor: u32,
    /// Player color materials, glowing, by color.
    glow: HashMap<[u8; 3], u32>,
    /// The viewmodel's coils, and their material as last sent.
    coils: u32,
    coils_lit: Option<u8>,
}

#[derive(Default)]
pub struct ArenaClient {
    scene: Scene,
    sounds: Sounds,
    hud: hud::Hud,
    assets: Assets,
    me: Option<u32>,
    pred: PlayerState,
    alive: bool,
    yaw: f32,
    pitch: f32,
    time: f64,
    started: bool,
    play: Vec<Play>,
    /// Everyone as of the newest snapshot.
    players: Vec<NetPlayer>,
    state: Option<Match>,
    rigs: HashMap<u32, Rig>,
    /// Where each remote player is drawn, for gibs.
    drawn: HashMap<u32, Vec3>,
    fx: Vec<Fx>,
    /// Killer and victim, named in their colors, and when.
    feed: VecDeque<([(String, Color); 2], f64)>,
    /// Centered message and when it was shown.
    message: Option<(String, f64)>,
    killer: Option<u32>,
    died_at: f64,
    tab: bool,
    viewmodel: u32,
    /// Viewmodel kick, 1 right after firing and decaying to 0.
    kick: f32,
    rng: u32,
}

impl ArenaClient {
    pub fn new() -> ArenaClient {
        ArenaClient { rng: 0x2545_F491, ..Default::default() }
    }
}

impl ClientGame for ArenaClient {
    // ponytail: one map, so `map` picks nothing yet.
    fn start(&mut self, _map: &str) -> Vec<Command> {
        let s = &mut self.scene;
        let mut sound = |wav: Vec<u8>| {
            let id = s.id();
            s.push(Command::Sound { id, wav });
            id
        };
        self.sounds = Sounds {
            rail: sound(sfx::rail()),
            frag: sound(sfx::frag()),
            gib: sound(sfx::gib()),
            pad: sound(sfx::pad()),
            jump: sound(sfx::jump()),
            respawn: sound(sfx::respawn()),
            ambience: sound(sfx::ambience()),
        };
        world::build(&mut self.scene);
        let s = &mut self.scene;
        self.assets = Assets {
            beam: s.mesh(MeshData::Cylinder { radius: 1.0, height: 1.0 }),
            gib: s.mesh(MeshData::Cuboid(Vec3::splat(0.16))),
            puff: s.mesh(MeshData::Sphere { radius: 1.0, ico: Some(1) }),
            armor: s.material(Material { color: Color::srgb(0.16, 0.17, 0.2), roughness: 0.4, metallic: 0.6, ..Default::default() }),
            glow: HashMap::new(),
            coils: s.material(glowing(Color::srgb(0.3, 0.9, 1.0), 4.0)),
            coils_lit: None,
        };
        self.viewmodel = self.railgun(CAMERA, self.assets.coils, Transform::at(Vec3::new(0.2, -0.2, -0.5)).scaled(0.6));
        self.build_hud();
        std::mem::take(&mut self.scene.out)
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        let you = snap.you;
        self.me = Some(you);
        self.players = snap.entities.iter().filter_map(NetPlayer::from_entity).collect();
        self.state = decode(&snap.state);
        for e in snap.events.iter().filter_map(|e| decode::<Event>(e)) {
            match e {
                Event::Rail { shooter, from, to } if shooter != you => {
                    let d = (to - from).normalize_or_zero();
                    self.beam(shooter, from + d * 0.8 - Vec3::Y * 0.15, to);
                    self.sound(self.sounds.rail, 0.7 / (1.0 + from.distance(self.pred.pos) / 15.0));
                }
                Event::Rail { .. } => {}
                Event::Frag { killer, victim } => self.on_frag(killer, victim),
            }
        }
        let Some((alive, yaw)) = self.players.iter().find(|p| p.id == you).map(|p| (p.alive, p.state.yaw)) else { return };
        if alive && !self.alive {
            // Respawned: face the way the server spawned us.
            (self.yaw, self.pitch) = (yaw, 0.0);
            self.sound(self.sounds.respawn, 0.5);
        }
        self.alive = alive;
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
            let o = eye(&self.pred);
            let muzzle = o + look_rot(self.yaw, self.pitch) * Vec3::new(0.2, -0.18, -0.92);
            self.beam(p.id, muzzle, o + d * trace_world(o, d).0);
            self.sound(self.sounds.rail, 0.7);
            self.kick = 1.0;
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
            self.play.push(Play { sound: self.sounds.ambience, volume: 0.25, looped: true });
        }
        if f.input.captured {
            self.yaw -= f.input.mouse.x * MOUSE_SENS;
            self.pitch = (self.pitch - f.input.mouse.y * MOUSE_SENS).clamp(-1.55, 1.55);
        }
        self.tab = f.input.held(Key::Tab);
        self.render_players(&f.others, f.dt);
        self.render_viewmodel(f.dt);
        self.update_fx(f.dt);
        self.update_hud();
        // Dead: the view drifts up over where you were.
        let rise = if self.alive { 0.0 } else { ((self.time - self.died_at) as f32).min(1.0) * 2.0 };
        let camera = Camera { pos: f.me_pos + Vec3::Y * (EYE + rise), rot: look_rot(self.yaw, self.pitch), fov: FOV };
        FrameOut { commands: std::mem::take(&mut self.scene.out), sounds: std::mem::take(&mut self.play), camera, lines: vec![], spheres: vec![] }
    }
}

fn glowing(c: Color, strength: f32) -> Material {
    let [r, g, b, _] = c.0;
    Material { color: c, emissive: [r * strength, g * strength, b * strength], ..Default::default() }
}

impl ArenaClient {
    fn sound(&mut self, sound: u32, volume: f32) {
        self.play.push(Play { sound, volume, looped: false });
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

    fn color(&self, id: u32) -> [u8; 3] {
        self.players.iter().find(|p| p.id == id).map_or([255; 3], |p| p.color)
    }

    /// The glowing material in a player color, made the first time it's needed.
    fn glow(&mut self, color: [u8; 3]) -> u32 {
        let s = &mut self.scene;
        *self.assets.glow.entry(color).or_insert_with(|| s.material(glowing(rgb(color), 3.0)))
    }

    fn on_frag(&mut self, killer: u32, victim: u32) {
        let me = self.me;
        let at = if Some(victim) == me { Some(self.pred.pos) } else { self.drawn.get(&victim).copied() };
        if let Some(at) = at {
            self.gibs(victim, at);
            self.sound(self.sounds.gib, 0.9 / (1.0 + at.distance(self.pred.pos) / 12.0));
        }
        if Some(killer) == me {
            self.sound(self.sounds.frag, 0.7);
            self.message = Some((format!("You fragged {}", self.name(victim)), self.time));
        }
        if Some(victim) == me {
            (self.killer, self.died_at) = (Some(killer), self.time);
        }
        let named = |id| (self.name(id), readable(self.color(id)));
        self.feed.push_back(([named(killer), named(victim)], self.time));
        if self.feed.len() > 5 {
            self.feed.pop_front();
        }
    }

    /// A rail beam in the shooter's color: a white-hot core and a wider glow that lingers.
    fn beam(&mut self, shooter: u32, from: Vec3, to: Vec3) {
        let (len, dir) = (from.distance(to), (to - from).normalize_or(Vec3::NEG_Z));
        let rot = Quat::from_rotation_arc(Vec3::Y, dir);
        let [r, g, b, _] = rgb(self.color(shooter)).0;
        for (radius, c, life, grow) in [(0.02, Color::srgba(1.0, 1.0, 1.0, 1.0), 0.25, 1.0), (0.05, Color::srgba(r, g, b, 0.5), 1.1, 2.0)] {
            let material = Material { color: c, emissive: [r * 4.0, g * 4.0, b * 4.0], unlit: true, blend: Blend::Blend, ..Default::default() };
            let transform = Transform { pos: (from + to) / 2.0, rot, scale: Vec3::new(radius, len, radius) };
            self.effect(self.assets.beam, material, transform, None, life, Vec3::new(grow, 1.0, grow));
        }
    }

    /// The victim bursts: chunks of armor and glowing bits fly out and bounce, with a puff in their color.
    fn gibs(&mut self, victim: u32, feet: Vec3) {
        let [r, g, b, _] = rgb(self.color(victim)).0;
        let puff = Material { color: Color::srgba(r, g, b, 0.6), unlit: true, blend: Blend::Blend, ..Default::default() };
        self.effect(self.assets.puff, puff, Transform::at(feet + Vec3::Y).scaled(0.4), None, 0.5, Vec3::splat(4.0));
        let glow = self.glow(self.color(victim));
        for i in 0..14 {
            let (a, up, out) = (self.random() * std::f32::consts::TAU, 4.0 + self.random() * 6.0, 2.0 + self.random() * 5.0);
            let vel = Vec3::new(a.cos() * out, up, a.sin() * out);
            let pos = feet + Vec3::Y * (0.4 + self.random() * 1.2);
            let spin = Quat::from_axis_angle(vel.cross(Vec3::Y).normalize_or(Vec3::X), 8.0 * self.random() + 4.0);
            let material = if i % 3 == 0 { glow } else { self.assets.armor };
            let transform = Transform::at(pos).scaled(0.6 + self.random() * 0.8);
            let node = self.scene.spawn(None, NodeKind::Mesh { mesh: self.assets.gib, material, shadows: true }, transform, true);
            self.fx.push(Fx { node, material: None, transform, vel: Some(vel), spin, age: 0.0, life: 2.5, grow: Vec3::splat(0.3) });
        }
    }

    fn effect(&mut self, mesh: u32, material: Material, transform: Transform, vel: Option<Vec3>, life: f32, grow: Vec3) {
        let id = self.scene.material(material);
        let node = self.scene.spawn(None, NodeKind::Mesh { mesh, material: id, shadows: false }, transform, true);
        self.fx.push(Fx { node, material: Some((id, material)), transform, vel, spin: Quat::IDENTITY, age: 0.0, life, grow });
    }

    fn update_fx(&mut self, dt: f32) {
        for fx in &mut self.fx {
            fx.age += dt;
            let k = (fx.age / fx.life).min(1.0);
            let base = fx.transform.scale;
            let mut t = Transform { scale: base * Vec3::ONE.lerp(fx.grow, k), ..fx.transform };
            if let Some(vel) = &mut fx.vel {
                vel.y -= GRAVITY * dt;
                let mut pos = fx.transform.pos + *vel * dt;
                let floor = floor_below(fx.transform.pos + Vec3::Y * 0.05);
                if pos.y < floor + 0.05 {
                    pos.y = floor + 0.05;
                    *vel = Vec3::new(vel.x * 0.5, -vel.y * 0.35, vel.z * 0.5);
                }
                fx.transform.pos = pos;
                fx.transform.rot = (Quat::IDENTITY.slerp(fx.spin, dt * vel.length() * 0.3) * fx.transform.rot).normalize();
                t = Transform { pos, rot: fx.transform.rot, scale: base * Vec3::ONE.lerp(fx.grow, (k - 0.7).max(0.0) / 0.3) };
            }
            self.scene.out.push(Command::set_transform(fx.node, t));
            if let Some((id, m)) = &fx.material {
                let mut faded = *m;
                faded.color = m.color.with_alpha(m.color.0[3] * (1.0 - k));
                faded.emissive = m.emissive.map(|e| e * (1.0 - k));
                self.scene.out.push(Command::Material { id: *id, material: faded });
            }
        }
        let (done, live): (Vec<Fx>, Vec<Fx>) = std::mem::take(&mut self.fx).into_iter().partition(|fx| fx.age >= fx.life);
        self.fx = live;
        for fx in done {
            self.scene.push(Command::Despawn { id: fx.node });
            if let Some((id, _)) = fx.material {
                self.scene.push(Command::Free { id });
            }
        }
    }

    /// A railgun: a dark body with glowing coils along the barrel. Returns its root node.
    fn railgun(&mut self, parent: u32, coils: u32, at: Transform) -> u32 {
        let (s, armor) = (&mut self.scene, self.assets.armor);
        let root = s.empty(Some(parent), at, true);
        let body = s.cuboid(Vec3::new(0.09, 0.12, 0.55));
        let unshadowed = |mesh, material| NodeKind::Mesh { mesh, material, shadows: false };
        s.spawn(Some(root), unshadowed(body, armor), Transform::default(), true);
        let barrel = s.shared_mesh("rail barrel".into(), || MeshData::Cylinder { radius: 0.022, height: 0.5 });
        let along = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        s.spawn(Some(root), unshadowed(barrel, armor), Transform::at(Vec3::new(0.0, 0.03, -0.45)).rotated(along), true);
        let coil = s.shared_mesh("rail coil".into(), || MeshData::Cylinder { radius: 0.05, height: 0.035 });
        for i in 0..4 {
            let z = -0.3 - i as f32 * 0.09;
            s.spawn(Some(root), unshadowed(coil, coils), Transform::at(Vec3::new(0.0, 0.03, z)).rotated(along), true);
        }
        root
    }

    /// Our railgun: kicks back on firing, bobs while running, and its coils glow brighter as it recharges.
    fn render_viewmodel(&mut self, dt: f32) {
        self.kick *= (-dt * 8.0).exp();
        let speed = if self.pred.on_ground { self.pred.vel.with_y(0.0).length() / MAX_SPEED } else { 0.0 };
        let t = self.time as f32 * 11.0;
        let bob = Vec3::new(t.sin() * 0.01, -(t * 2.0).sin().abs() * 0.008, 0.0) * speed;
        let at = Vec3::new(0.2, -0.2, -0.5 + 0.06 * self.kick) + bob;
        let transform = Transform::at(at).rotated(Quat::from_rotation_x(0.15 * self.kick)).scaled(0.6);
        let shown = self.alive && self.me.is_some();
        self.scene.push(Command::Set { id: self.viewmodel, transform: Some(transform), visible: Some(shown), light: None });
        let charge = 1.0 - self.pred.hands.cooldown as f32 / (RAIL.cycle * TICK_HZ as f32);
        let lit = (charge * 8.0) as u8;
        if self.assets.coils_lit != Some(lit) {
            self.assets.coils_lit = Some(lit);
            let strength = if lit >= 8 { 6.0 } else { 0.3 + charge * 1.5 };
            let material = glowing(Color::srgb(0.3, 0.9, 1.0), strength);
            self.scene.push(Command::Material { id: self.assets.coils, material });
        }
    }

    fn render_players(&mut self, others: &[Interpolated], dt: f32) {
        let mut seen = HashMap::new();
        for v in others {
            let Some(p) = NetPlayer::from_entity(&v.entity) else { continue };
            // Alive as of the newest snapshot too, so someone just fragged doesn't linger until interpolation catches up.
            let alive = p.alive && !v.teleporting && self.players.iter().any(|q| q.id == p.id && q.alive);
            seen.insert(p.id, (p.state.pos, p.state.yaw, p.state.pitch, alive, p.color));
        }
        let gone: Vec<u32> = self.rigs.keys().filter(|id| !seen.contains_key(id)).copied().collect();
        for id in gone {
            let rig = self.rigs.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: rig.root });
        }
        self.drawn = seen.iter().map(|(&id, v)| (id, v.0)).collect();
        for (id, (pos, yaw, pitch, alive, color)) in seen {
            if !self.rigs.contains_key(&id) {
                let rig = self.spawn_rig(color);
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
            for (leg, (x, side)) in rig.legs.into_iter().zip([(-0.13, 1.0), (0.13, -1.0)]) {
                out.push(Command::set_transform(leg, Transform::at(Vec3::new(x, 0.8, 0.0)).rotated(Quat::from_rotation_x(swing * side))));
            }
        }
    }

    /// An armored figure with a glowing visor and trim in the player's color, holding a railgun.
    fn spawn_rig(&mut self, color: [u8; 3]) -> Rig {
        let (armor, glow) = (self.assets.armor, self.glow(color));
        let s = &mut self.scene;
        let root = s.empty(None, Transform::default(), false);
        let torso = s.cuboid(Vec3::new(0.56, 0.62, 0.32));
        s.mesh_node(Some(root), torso, armor, Transform::at(Vec3::Y * 1.12));
        let stripe = s.cuboid(Vec3::new(0.58, 0.2, 0.34));
        s.mesh_node(Some(root), stripe, glow, Transform::at(Vec3::Y * 1.18));
        let head = s.shared_mesh("head".into(), || MeshData::Sphere { radius: 0.19, ico: None });
        s.mesh_node(Some(root), head, armor, Transform::at(Vec3::Y * 1.62));
        let visor = s.cuboid(Vec3::new(0.3, 0.07, 0.1));
        s.mesh_node(Some(root), visor, glow, Transform::at(Vec3::new(0.0, 1.64, -0.15)));
        let leg = s.cuboid(Vec3::new(0.18, 0.8, 0.2));
        let legs = [-0.13, 0.13].map(|x| {
            let hip = s.empty(Some(root), Transform::at(Vec3::new(x, 0.8, 0.0)), true);
            s.mesh_node(Some(hip), leg, armor, Transform::at(Vec3::Y * -0.4));
            hip
        });
        let arms = s.empty(Some(root), Transform::at(Vec3::Y * 1.35), true);
        self.railgun(arms, glow, Transform::at(Vec3::new(0.26, -0.05, -0.3)));
        Rig { root, arms, legs, stride: 0.0, last_pos: Vec3::ZERO }
    }
}
