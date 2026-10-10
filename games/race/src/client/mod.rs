//! The Raceslop client: the course, the laser bars, runners in their own colors with name tags, and the HUD.

mod hud;
mod runner;
mod sfx;
mod world;

use crate::shared::*;
use glam::Vec3;
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
    zap: u32,
    splash: u32,
    checkpoint: u32,
    finish: u32,
    beep: u32,
    go: u32,
    pad: u32,
    jump: u32,
    land: u32,
    ambience: u32,
}

#[derive(Default)]
pub struct RaceClient {
    scene: Scene,
    sounds: Sounds,
    hud: hud::Hud,
    lasers: Vec<u32>,
    lava: u32,
    me: Option<u32>,
    /// Our entity's teleport count as last seen, to face north again after being sent back.
    teleport: Option<u8>,
    pred: PlayerState,
    /// Camera height over the feet' ground, eased over step-ups and step-downs.
    cam_y: f32,
    /// How hard we last landed and when, for the camera dip.
    landed: (f32, f64),
    yaw: f32,
    pitch: f32,
    time: f64,
    started: bool,
    play: Vec<Play>,
    /// Everyone as of the newest snapshot.
    players: Vec<NetPlayer>,
    race: Option<Race>,
    /// When the newest snapshot came in, to run the lasers smoothly between snapshots.
    race_at: f64,
    /// Countdown seconds already beeped.
    beeped: Option<u32>,
    rigs: HashMap<u32, runner::Rig>,
    feed: VecDeque<(String, f64)>,
    /// Centered message and when it was shown.
    message: Option<(String, f64)>,
    tab: bool,
}

impl RaceClient {
    pub fn new() -> RaceClient {
        RaceClient::default()
    }
}

impl ClientGame for RaceClient {
    fn start(&mut self, map: &str) -> Vec<Command> {
        crate::map::load(map);
        let s = &mut self.scene;
        let mut sound = |wav: Vec<u8>| {
            let id = s.id();
            s.push(Command::Sound { id, wav });
            id
        };
        self.sounds = Sounds {
            zap: sound(sfx::zap()),
            splash: sound(sfx::splash()),
            checkpoint: sound(sfx::checkpoint()),
            finish: sound(sfx::finish()),
            beep: sound(sfx::beep(false)),
            go: sound(sfx::beep(true)),
            pad: sound(sfx::pad()),
            jump: sound(sfx::jump()),
            land: sound(sfx::land()),
            ambience: sound(sfx::ambience()),
        };
        self.lava = world::build(&mut self.scene);
        self.lasers = world::lasers(&mut self.scene);
        self.build_hud();
        std::mem::take(&mut self.scene.out)
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        let you = snap.you;
        self.me = Some(you);
        self.players = snap.entities.iter().filter_map(NetPlayer::from_entity).collect();
        self.race = decode(&snap.state);
        self.race_at = self.time;
        for e in snap.events.iter().filter_map(|e| decode::<Event>(e)) {
            self.on_event(e, you);
        }
        let Some(e) = snap.entities.iter().find(|e| e.id == you) else { return };
        if self.teleport.is_some_and(|t| t != e.teleport) {
            (self.yaw, self.pitch) = (e.yaw, 0.0);
        }
        self.teleport = Some(e.teleport);
    }

    fn input(&mut self, raw: &RawInput) -> Vec<u8> {
        let axis = |pos, neg| raw.held(pos) as i32 as f32 - raw.held(neg) as i32 as f32;
        let inp = Input {
            forward: axis(Key::W, Key::S),
            right: axis(Key::D, Key::A),
            yaw: self.yaw,
            pitch: self.pitch,
            jump: raw.held(Key::Space) || raw.held(Key::Backspace),
            reset: raw.held(Key::R),
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
        if step.launched {
            self.sound(self.sounds.pad, 0.6);
        } else if step.jumped {
            self.sound(self.sounds.jump, 0.3);
        }
        if let Some(speed) = step.landed.filter(|&v| v > 2.0) {
            self.landed = (speed, self.time);
            self.sound(self.sounds.land, (speed / 15.0).min(0.6));
        }
    }

    fn frame(&mut self, f: &FrameIn) -> FrameOut {
        self.time = f.time;
        if let Some(p) = f.me.as_ref().and_then(NetPlayer::from_entity) {
            self.pred = p.state;
        }
        if !self.started {
            self.started = true;
            self.play.push(Play { sound: self.sounds.ambience, volume: 0.2, looped: true });
        }
        if f.input.captured {
            self.yaw -= f.input.mouse.x * MOUSE_SENS;
            self.pitch = (self.pitch - f.input.mouse.y * MOUSE_SENS).clamp(-1.55, 1.55);
        }
        self.tab = f.input.held(Key::Tab);
        self.render_players(&f.others, f.dt);
        self.render_lasers();
        self.scene.out.push(Command::set_transform(self.lava, world::lava_at(self.time)));
        self.countdown();
        self.update_hud();
        let camera = Camera { pos: f.me_pos.with_y(self.camera_y(f.me_pos.y, f.dt)), rot: look_rot(self.yaw, self.pitch), fov: FOV };
        FrameOut { commands: std::mem::take(&mut self.scene.out), sounds: std::mem::take(&mut self.play), camera, lines: vec![], spheres: vec![] }
    }
}

fn glowing(c: Color, strength: f32) -> Material {
    let [r, g, b, _] = c.0;
    Material { color: c, emissive: [r * strength, g * strength, b * strength], ..Default::default() }
}

fn ordinal(n: u8) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

impl RaceClient {
    fn sound(&mut self, sound: u32, volume: f32) {
        self.play.push(Play { sound, volume, looped: false });
    }

    /// Louder the closer it happened.
    fn sound_at(&mut self, sound: u32, volume: f32, at: Vec3) {
        self.sound(sound, volume / (1.0 + at.distance(self.pred.pos) / 12.0));
    }

    fn name(&self, id: u32) -> String {
        self.players.iter().find(|p| p.id == id).map_or_else(|| format!("player {id}"), |p| p.name.clone())
    }

    fn on_event(&mut self, e: Event, you: u32) {
        let line = match e {
            Event::Fell { id, at, lasered } => {
                self.sound_at(if lasered { self.sounds.zap } else { self.sounds.splash }, 0.8, at);
                if id == you {
                    let why = if lasered { "Lasered!" } else { "Into the lava!" };
                    self.message = Some((format!("{why}\nBack to the checkpoint"), self.time));
                }
                return;
            }
            Event::Checkpoint { id, n } if id == you => {
                self.sound(self.sounds.checkpoint, 0.7);
                let of = crate::map::course().checkpoints();
                self.message = Some((format!("Checkpoint {n} of {of}"), self.time));
                return;
            }
            Event::Checkpoint { .. } => return,
            Event::Finish { id, place, secs } => {
                if id == you {
                    self.sound(self.sounds.finish, 0.8);
                    self.message = Some((format!("You finished {}!\n{secs:.2} s", ordinal(place)), self.time));
                }
                format!("{} finished {} in {secs:.2} s", self.name(id), ordinal(place))
            }
            Event::Go => {
                self.sound(self.sounds.go, 0.7);
                self.message = Some(("GO!".into(), self.time));
                return;
            }
        };
        self.feed.push_back((line, self.time));
        if self.feed.len() > 5 {
            self.feed.pop_front();
        }
    }

    /// Beeps on each of the countdown's last three seconds.
    fn countdown(&mut self) {
        let Some(r) = self.race.filter(|r| r.phase == Phase::Countdown) else { return self.beeped = None };
        let left = r.time.ceil() as u32;
        if left <= 3 && self.beeped != Some(left) {
            self.beeped = Some(left);
            self.sound(self.sounds.beep, 0.6);
        }
    }

    /// Seconds into the race for the lasers: the server's clock, run on since its last snapshot.
    fn laser_time(&self) -> f32 {
        self.race.map_or(0.0, |r| r.laser_time() + ((self.time - self.race_at) as f32).min(0.25))
    }

    fn render_lasers(&mut self) {
        let time = self.laser_time();
        for (i, &bar) in self.lasers.iter().enumerate() {
            self.scene.out.push(Command::set_transform(bar, world::laser(i, time)));
        }
    }

    /// Eye height: eased rather than snapped when the feet step up or down on the ground, and dipping on landing.
    fn camera_y(&mut self, feet: f32, dt: f32) -> f32 {
        let stepped = self.pred.on_ground && (feet - self.cam_y).abs() < 0.6;
        self.cam_y = if stepped { self.cam_y + (feet - self.cam_y) * (1.0 - (-dt * 18.0).exp()) } else { feet };
        // A quick dip and a slower rise back: t e^(1-t) peaks at 1 when t is 1.
        let (speed, at) = self.landed;
        let t = (self.time - at) as f32 / 0.07;
        self.cam_y + EYE - (speed * 0.015).min(0.2) * t * (1.0 - t).exp()
    }

    fn render_players(&mut self, others: &[Interpolated], dt: f32) {
        let mut seen = HashMap::new();
        for v in others {
            let Some(p) = NetPlayer::from_entity(&v.entity) else { continue };
            seen.insert(p.id, (p.state, !v.teleporting, p.color));
        }
        let gone: Vec<u32> = self.rigs.keys().filter(|id| !seen.contains_key(id)).copied().collect();
        for id in gone {
            let rig = self.rigs.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: rig.root });
            self.scene.ui_despawn(rig.tag);
        }
        for (id, (state, shown, color)) in seen {
            if !self.rigs.contains_key(&id) {
                let rig = runner::spawn(&mut self.scene, color);
                self.rigs.insert(id, rig);
            }
            let name = self.name(id);
            let rig = self.rigs.get_mut(&id).unwrap();
            rig.animate(&mut self.scene.out, state.pos, state.yaw, state.vel, state.on_ground, shown, dt);
            let (tag, tag_at) = (rig.tag, state.pos + Vec3::Y * 2.2);
            self.scene.ui_set(tag, |n| {
                n.world = Some(tag_at);
                n.hidden = !shown;
            });
            self.scene.ui_text(tag, name);
        }
    }
}
