//! The Minigolf client: the course, your ball and an orbit camera around it, everyone else's balls as ghosts with
//! name tags, the aim arrow and power, and the HUD.

mod hud;
mod sfx;
mod world;

use crate::map::course;
use crate::shared::*;
use glam::Vec3;
use sdk::Interpolated;
use sdk::client::*;
use sdk::scene::Scene;
use std::collections::{HashMap, VecDeque};

const MOUSE_SENS: f32 = 0.002;
const FOV: f32 = 1.2;
/// Mouse travel (counts) pulled back for a full-power shot.
const FULL_PULL: f32 = 500.0;
/// Camera pitch above the ball and distance from it, and how fast W/S zoom (m/s).
const PITCH: (f32, f32) = (0.17, 1.22);
const ZOOM: (f32, f32) = (2.0, 10.0);
const ZOOM_SPEED: f32 = 6.0;
/// Ghosts closer to the camera than this are hidden, so they never block a shot.
const GHOST_NEAR: f32 = 1.0;
/// Trail points kept per ghost, and how far apart.
const TRAIL: usize = 12;
const TRAIL_STEP: f32 = 0.15;

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
    tock: u32,
    clink: u32,
    rattle: u32,
    splash: u32,
    whoosh: u32,
    cheer: u32,
    go: u32,
    ambience: u32,
}

struct Ghost {
    /// The ball's node, and whether it's drawn solid.
    node: Option<(u32, bool)>,
    tag: u32,
    trail: VecDeque<Vec3>,
}

#[derive(Default)]
pub struct GolfClient {
    scene: Scene,
    sounds: Sounds,
    hud: hud::Hud,
    windmills: Vec<u32>,
    /// Our ball's node, made once we know our color.
    ball: Option<u32>,
    me: Option<u32>,
    pred: Ball,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    /// Pulling back for a shot: mouse travel so far.
    pull: Option<f32>,
    /// A shot to send with the next input: yaw and power.
    shot: Option<(f32, f32)>,
    /// Whose ball we watch once done with the hole.
    watch: Option<u32>,
    /// The hole the camera last faced the start of.
    faced: Option<u8>,
    time: f64,
    started: bool,
    play: Vec<Play>,
    /// Everyone as of the newest snapshot.
    players: Vec<NetPlayer>,
    round: Option<Round>,
    ghosts: HashMap<u32, Ghost>,
    feed: VecDeque<(String, f64)>,
    /// Centered message and when it was shown.
    message: Option<(String, f64)>,
    tab: bool,
}

impl GolfClient {
    pub fn new() -> GolfClient {
        GolfClient { pitch: 0.45, zoom: 4.5, ..Default::default() }
    }
}

impl ClientGame for GolfClient {
    fn start(&mut self, map: &str) -> Vec<Command> {
        crate::map::load(map);
        let s = &mut self.scene;
        let mut sound = |wav: Vec<u8>| {
            let id = s.id();
            s.push(Command::Sound { id, wav });
            id
        };
        self.sounds = Sounds {
            tock: sound(sfx::tock()),
            clink: sound(sfx::clink()),
            rattle: sound(sfx::rattle()),
            splash: sound(sfx::splash()),
            whoosh: sound(sfx::whoosh()),
            cheer: sound(sfx::cheer()),
            go: sound(sfx::go()),
            ambience: sound(sfx::ambience()),
        };
        self.windmills = world::build(&mut self.scene);
        self.build_hud();
        std::mem::take(&mut self.scene.out)
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        let you = snap.you;
        self.me = Some(you);
        self.players = snap.entities.iter().filter_map(NetPlayer::from_entity).collect();
        self.round = decode(&snap.state);
        for e in snap.events.iter().filter_map(|e| decode::<Event>(e)) {
            self.on_event(e, you);
        }
        // Each new hole starts looking down the tee, which always faces north.
        if let Some(r) = self.round.filter(|r| Some(r.hole) != self.faced) {
            (self.faced, self.yaw, self.watch) = (Some(r.hole), 0.0, None);
        }
        if self.ball.is_none()
            && let Some(p) = self.players.iter().find(|p| p.id == you)
        {
            let mesh = self.scene.shared_mesh("ball".into(), || MeshData::Sphere { radius: BALL_RADIUS, ico: Some(3) });
            let material = self.scene.material(Material { color: rgb(p.color), roughness: 0.3, ..Default::default() });
            self.ball = Some(self.scene.mesh_node(None, mesh, material, Transform::default()));
        }
    }

    fn input(&mut self, _raw: &RawInput) -> Vec<u8> {
        let (yaw, shot) = self.shot.take().unwrap_or((self.yaw, 0.0));
        encode(&Input { yaw, pitch: self.pitch, shot, ..Default::default() })
    }

    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, fresh: bool) {
        let step = predict(me, input, seq);
        if !fresh {
            return;
        }
        if let Some(p) = NetPlayer::from_entity(me) {
            self.pred = p.ball;
        }
        if let Some(power) = step.shot {
            self.sound(self.sounds.tock, 0.25 + power * 0.6);
        }
        if step.knock > 0.6 {
            self.sound(self.sounds.clink, (step.knock / 6.0).min(0.7));
        }
        if step.holed {
            self.sound(self.sounds.rattle, 0.9);
        }
    }

    fn frame(&mut self, f: &FrameIn) -> FrameOut {
        self.time = f.time;
        if let Some(p) = f.me.as_ref().and_then(NetPlayer::from_entity) {
            self.pred = p.ball;
        }
        if !self.started {
            self.started = true;
            self.play.push(Play { sound: self.sounds.ambience, volume: 0.15, looped: true });
        }
        self.tab = f.input.held(Key::Tab);
        let done = self.done();
        self.aim(&f.input, f.dt, done);
        if done && f.input.captured && f.input.pressed(Key::MouseLeft) {
            self.watch_next();
        }

        let target = self.watched(&f.others).unwrap_or(f.me_pos);
        let camera = self.camera(target);
        if let Some(ball) = self.ball {
            self.scene.push(Command::set_transform(ball, Transform::at(f.me_pos)));
        }
        let lines = self.render_ghosts(&f.others, camera.pos);
        let lines = lines.into_iter().chain(self.arrow(f.me_pos, done)).collect();
        for (&bar, t) in self.windmills.iter().zip(world::windmill_bars(self.pred.windmill_time())) {
            self.scene.push(Command::set_transform(bar, t));
        }
        self.update_hud();
        FrameOut { commands: std::mem::take(&mut self.scene.out), sounds: std::mem::take(&mut self.play), camera, lines, spheres: vec![] }
    }
}

/// What a score on a hole of par `par` is called.
fn term(strokes: u8, par: u8) -> String {
    if strokes == 1 {
        return "Hole in one!".into();
    }
    match strokes as i32 - par as i32 {
        ..=-3 => "Albatross!".into(),
        -2 => "Eagle!".into(),
        -1 => "Birdie!".into(),
        0 => "Par".into(),
        1 => "Bogey".into(),
        2 => "Double bogey".into(),
        over => format!("+{over}"),
    }
}

impl GolfClient {
    fn sound(&mut self, sound: u32, volume: f32) {
        self.play.push(Play { sound, volume, looped: false });
    }

    /// Louder the closer it happened.
    fn sound_at(&mut self, sound: u32, volume: f32, at: Vec3) {
        self.sound(sound, volume / (1.0 + at.distance(self.pred.pos) / 8.0));
    }

    fn name(&self, id: u32) -> String {
        self.players.iter().find(|p| p.id == id).map_or_else(|| format!("player {id}"), |p| p.name.clone())
    }

    fn par(&self) -> u8 {
        self.round.map_or(3, |r| course().holes[r.hole as usize].par)
    }

    /// Done with this hole: holed out, picked up, or it's over.
    fn done(&self) -> bool {
        self.pred.holed || self.players.iter().any(|p| Some(p.id) == self.me && p.done)
    }

    fn on_event(&mut self, e: Event, you: u32) {
        let line = match e {
            Event::Holed { id, strokes } => {
                let what = term(strokes, self.par());
                if id == you {
                    if strokes < self.par() {
                        self.sound(self.sounds.cheer, 0.6);
                    }
                    self.message = Some((format!("{what}\n{strokes} stroke{}", if strokes == 1 { "" } else { "s" }), self.time));
                    return;
                }
                let at = course().holes[self.round.map_or(0, |r| r.hole as usize)].cup;
                self.sound_at(self.sounds.rattle, 0.6, at);
                format!("{}: {what} ({strokes})", self.name(id))
            }
            Event::PickedUp { id } => {
                if id == you {
                    self.message = Some((format!("Picked up\n{MAX_STROKES} strokes"), self.time));
                    return;
                }
                format!("{} picked up", self.name(id))
            }
            Event::Out { id, at, water } => {
                self.sound_at(if water { self.sounds.splash } else { self.sounds.whoosh }, 0.7, at);
                if id == you {
                    self.message = Some((format!("{}\n+1 stroke", if water { "Water!" } else { "Out of bounds!" }), self.time));
                }
                return;
            }
            Event::Go => {
                self.sound(self.sounds.go, 0.6);
                return;
            }
        };
        self.feed.push_back((line, self.time));
        if self.feed.len() > 5 {
            self.feed.pop_front();
        }
    }

    /// The mouse turns the camera; holding the left button and pulling back sets the power instead, and letting go
    /// shoots. Right click cancels.
    fn aim(&mut self, input: &RawInput, dt: f32, done: bool) {
        let playing = self.round.is_some_and(|r| r.phase == Phase::Play);
        if !(playing && !done && self.pred.can_shoot() && input.captured) {
            self.pull = None;
        } else if self.pull.is_none() && input.pressed(Key::MouseLeft) {
            self.pull = Some(0.0);
        }
        if input.captured {
            match &mut self.pull {
                Some(pull) => *pull = (*pull + input.mouse.y).max(0.0),
                None => {
                    self.yaw -= input.mouse.x * MOUSE_SENS;
                    self.pitch = (self.pitch + input.mouse.y * MOUSE_SENS).clamp(PITCH.0, PITCH.1);
                }
            }
        }
        if let Some(pull) = self.pull {
            if input.pressed(Key::MouseRight) {
                self.pull = None;
            } else if !input.held(Key::MouseLeft) {
                let power = (pull / FULL_PULL).min(1.0);
                if power > 0.02 {
                    self.shot = Some((self.yaw, power));
                }
                self.pull = None;
            }
        }
        let zoom = input.held(Key::S) as i32 as f32 - input.held(Key::W) as i32 as f32;
        self.zoom = (self.zoom + zoom * ZOOM_SPEED * dt).clamp(ZOOM.0, ZOOM.1);
    }

    /// Players still on the hole, other than us.
    fn playing(&self) -> Vec<u32> {
        self.players.iter().filter(|p| Some(p.id) != self.me && !p.done).map(|p| p.id).collect()
    }

    fn watch_next(&mut self) {
        let ids = self.playing();
        let at = self.watch.and_then(|w| ids.iter().position(|&id| id == w));
        self.watch = ids.get(at.map_or(0, |i| (i + 1) % ids.len())).copied();
    }

    /// Where the ball we're watching is, once we're done with the hole.
    fn watched(&mut self, others: &[Interpolated]) -> Option<Vec3> {
        if !self.done() {
            return None;
        }
        if self.watch.is_none_or(|w| !self.playing().contains(&w)) {
            self.watch = None;
            self.watch_next();
        }
        others.iter().find(|o| Some(o.entity.id) == self.watch).map(|o| o.entity.pos)
    }

    /// Orbiting the ball we follow; on the tee, flying over the hole from the tee to the cup.
    fn camera(&self, target: Vec3) -> Camera {
        if let Some(r) = self.round.filter(|r| r.phase == Phase::Tee) {
            let hole = &course().holes[r.hole as usize];
            let t = (1.0 - r.time / 3.0).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t);
            let to = hole.cup - hole.tee;
            let yaw = f32::atan2(-to.x, -to.z);
            let over = hole.tee.lerp(hole.cup, s);
            return Camera { pos: over - aim(yaw) * 9.0 + Vec3::Y * 11.0, rot: look_rot(yaw, -0.85), fov: FOV };
        }
        let look_at = target + Vec3::Y * 0.2;
        let pos = look_at - aim(self.yaw) * self.pitch.cos() * self.zoom + Vec3::Y * self.pitch.sin() * self.zoom;
        Camera { pos, rot: look_rot(self.yaw, -self.pitch), fov: FOV }
    }

    /// The aim arrow on the ground from our ball, longer and redder with power, while we can shoot.
    fn arrow(&self, ball: Vec3, done: bool) -> Vec<(Vec3, Vec3, Color)> {
        let playing = self.round.is_some_and(|r| r.phase == Phase::Play);
        if !playing || done || !self.pred.can_shoot() {
            return vec![];
        }
        let power = self.pull.map_or(0.0, |p| (p / FULL_PULL).min(1.0));
        let color = Color::srgb(1.0, 1.0 - power * 0.8, 1.0 - power);
        let (dir, base) = (aim(self.yaw), ball - Vec3::Y * (BALL_RADIUS - 0.03));
        let tip = base + dir * (0.4 + power * 3.0);
        let side = Vec3::new(-dir.z, 0.0, dir.x) * 0.15;
        let start = base + dir * (BALL_RADIUS + 0.05);
        vec![(start, tip, color), (tip, tip - dir * 0.2 + side, color), (tip, tip - dir * 0.2 - side, color)]
    }

    /// Everyone else's ball: see-through in their color with a name tag and a short trail, hidden in the cup, while
    /// teleporting, and right in front of the camera. Returns the trails' lines.
    fn render_ghosts(&mut self, others: &[Interpolated], camera: Vec3) -> Vec<(Vec3, Vec3, Color)> {
        let mut seen = HashMap::new();
        for v in others {
            let Some(p) = NetPlayer::from_entity(&v.entity) else { continue };
            let solid = Some(p.id) == self.watch && self.done();
            let shown = !v.teleporting && !p.ball.holed && (solid || p.ball.pos.distance(camera) > GHOST_NEAR);
            seen.insert(p.id, (p.ball.pos, shown, p.color, solid));
        }
        let gone: Vec<u32> = self.ghosts.keys().filter(|id| !seen.contains_key(id)).copied().collect();
        for id in gone {
            let g = self.ghosts.remove(&id).unwrap();
            if let Some((node, _)) = g.node {
                self.scene.push(Command::Despawn { id: node });
            }
            self.scene.ui_despawn(g.tag);
        }
        let mut lines = vec![];
        for (id, (pos, shown, color, solid)) in seen {
            let name = self.name(id);
            let s = &mut self.scene;
            let g = self.ghosts.entry(id).or_insert_with(|| {
                let tag = s.ui(None, UiNode { text: Some(Text { shadow: true, center: true, ..Text::new("", 14.0, readable(color)) }), hidden: true, ..Default::default() });
                Ghost { node: None, tag, trail: VecDeque::new() }
            });
            // Swapping between solid and see-through means a new node with the other material.
            match g.node {
                Some((node, was)) if was == solid => s.push(Command::Set { id: node, transform: Some(Transform::at(pos)), visible: Some(shown), light: None }),
                _ => {
                    if let Some((node, _)) = g.node {
                        s.push(Command::Despawn { id: node });
                    }
                    let mesh = s.shared_mesh("ball".into(), || MeshData::Sphere { radius: BALL_RADIUS, ico: Some(3) });
                    let material = if solid {
                        s.shared_material(format!("solid {color:?}"), || Material { color: rgb(color), roughness: 0.3, ..Default::default() })
                    } else {
                        s.shared_material(format!("ghost {color:?}"), || Material { color: rgb(color).with_alpha(0.45), blend: Blend::Blend, ..Default::default() })
                    };
                    g.node = Some((s.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::at(pos), shown), solid));
                }
            }
            s.ui_set(g.tag, |n| {
                n.world = Some(pos + Vec3::Y * 0.45);
                n.hidden = !shown;
            });
            s.ui_text(g.tag, name);
            if !shown {
                g.trail.clear();
            } else if g.trail.back().is_none_or(|&last| last.distance(pos) > TRAIL_STEP) {
                g.trail.push_back(pos);
                if g.trail.len() > TRAIL {
                    g.trail.pop_front();
                }
            }
            let n = g.trail.len();
            for (i, (a, b)) in g.trail.iter().zip(g.trail.iter().skip(1)).enumerate() {
                lines.push((*a, *b, rgb(color).with_alpha((i + 1) as f32 / n as f32 * 0.6)));
            }
        }
        lines
    }
}
