//! The dust client: everything the player sees and hears, sent to the host as commands. CS 1.6 style HUD, soldier
//! models animated from interpolated players, first-person weapons, tracers, impacts, blood, the bomb and its beeps.

mod blood;
mod hud;
mod model;
mod scene;
mod sfx;
mod textures;
mod world;

use crate::shared::*;
use crate::weapons::{ALL_WEAPONS, DRAW_SECS, Weapon};
use glam::{Quat, Vec2, Vec3};
use model::{Rig, barrel_length, spawn_soldier, spawn_weapon, viewmodel_offset};
use scene::Scene;
use sdk::client::*;
use sdk::{Entity, decode, encode};
use std::collections::{HashMap, VecDeque};

const MOUSE_SENS: f32 = 0.002;
/// Field of view, and scoped in with the AWP.
const FOV: f32 = 1.6;
const ZOOM_FOV: f32 = 0.42;
const HUD_AMBER: Color = Color::srgba(1.0, 0.68, 0.12, 0.9);
const CROSSHAIR_GREEN: Color = Color::srgba(0.25, 1.0, 0.25, 0.85);
const TEXT_SIZE: f32 = 20.0;

/// Announcer lines, CS style.
#[derive(Clone, Copy, PartialEq)]
enum Announce {
    TWin,
    CtWin,
    Planted,
    Defused,
    Go,
    EnemyDown,
}

/// A short-lived visual effect to spawn this frame.
#[derive(Clone, Copy)]
enum FxKind {
    Muzzle(Vec3),
    /// A puff of rocket smoke.
    Smoke(Vec3),
    /// An explosion this big: a fireball and a cloud of smoke.
    Blast(Vec3, f32),
    /// Bullet striking the map: dust puff and a hole, at a position with the surface normal.
    Impact(Vec3, Vec3),
    Blood(Vec3),
}

/// A spawned effect: grows and fades over its life, then goes.
struct Fx {
    node: u32,
    pos: Vec3,
    /// Its own material, faded out, and freed with it.
    material: Option<(u32, Material)>,
    /// A point light's starting intensity.
    light: Option<f32>,
    age: f32,
    life: f32,
    /// Size multiplier reached at the end of its life, growing linearly from 1.
    grow: f32,
    size: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Life {
    Alive,
    /// Shown as a corpse until the round restarts.
    Dead,
    /// Alive again but the respawn teleport hasn't been interpolated past yet: hidden.
    Respawning,
}

#[derive(Default)]
struct Sounds {
    guns: Vec<(Weapon, u32)>,
    reload: u32,
    buy: u32,
    headshot: u32,
    hit_body: u32,
    kill: u32,
    hurt: u32,
    death: u32,
    beep: u32,
    explosion: u32,
    steps: Vec<u32>,
    impact: u32,
    wind: u32,
    voices: Vec<(Announce, u32)>,
    radio: Vec<(Radio, u32)>,
}

/// One kill feed entry: killer and victim with their teams, the weapon, headshot.
struct KillLine {
    killer: (String, Option<Team>),
    weapon: Weapon,
    victim: (String, Option<Team>),
    headshot: bool,
    at: f64,
}

/// HUD node ids.
#[derive(Default)]
struct Hud {
    scoreboard: u32,
    death_fade: u32,
    crosshair: [(u32, Vec2); 4],
    health: u32,
    armor: u32,
    money: u32,
    timer: u32,
    status: u32,
    score_panel: u32,
    sections: [u32; 2],
    announce: u32,
    radio_menu: u32,
    target: u32,
    chat: u32,
    scope: u32,
    hurt: u32,
    ammo: u32,
    buy_menu: u32,
    buy_title: u32,
    buy_rows: Vec<u32>,
    kill_feed: u32,
    feed_rows: Vec<u32>,
}

/// First-person weapon animation state.
#[derive(Default)]
struct WeaponAnim {
    held: Option<Weapon>,
    drawn_at: f64,
    bob: f32,
    sway: Vec2,
    last_look: Vec2,
    /// The viewmodel shown, as last sent.
    shown: Option<Weapon>,
}

/// Effect meshes and materials shared by every instance.
#[derive(Default)]
struct FxAssets {
    puff: u32,
    hole_mesh: u32,
    hole_material: u32,
    grenade_mesh: u32,
    grenade_material: u32,
    rocket_mesh: u32,
    rocket_material: u32,
}

pub struct DustClient {
    scene: Scene,
    sounds: Sounds,
    hud: Hud,
    fx_assets: FxAssets,
    bomb: Option<world::Bomb>,
    /// The bomb light was lit, as last sent.
    bomb_lit: Option<bool>,
    viewmodels: Vec<(Weapon, u32)>,
    rigs: HashMap<u32, Rig>,
    /// Grenade entity id to its scene node.
    grenades: HashMap<u32, u32>,
    /// Bullet holes, oldest first, so only the most recent stay.
    decals: VecDeque<u32>,
    blood: blood::Blood,
    live_fx: Vec<Fx>,
    me: Option<u32>,
    /// Our predicted player.
    pred: PlayerState,
    hp: i32,
    armor: u8,
    money: u32,
    yaw: f32,
    pitch: f32,
    /// Seconds since the client started, as of the latest frame.
    time: f64,
    /// Bullet tracers: (from, to, seconds left, lifetime, color).
    tracers: Vec<(Vec3, Vec3, f32, f32, Color)>,
    play: Vec<Play>,
    wind_started: bool,
    died_at: Option<f64>,
    round: Option<Round>,
    teams: HashMap<u32, Team>,
    names: HashMap<u32, String>,
    /// Clock time of the next bomb beep, and of the last one (for the light).
    next_beep: f64,
    last_beep: f64,
    /// Where, when (clock) and how big the latest explosion was, for the flash.
    explosion: Option<(Vec3, f64, f32)>,
    /// The teammate we watch while dead: id, interpolated eye position and view.
    spectate: Option<(u32, Vec3, f32, f32)>,
    /// The player under the crosshair, if any.
    aim_target: Option<u32>,
    /// Interpolated position of a remote bomb carrier.
    carrier_pos: Option<Vec3>,
    /// Scoped in with the AWP.
    zoom: bool,
    /// Buy menu open.
    menu: bool,
    /// Radio menu (Z) open.
    radio_menu: bool,
    /// Tab held: scoreboard.
    tab: bool,
    /// Key presses caught each frame, sent with the next input.
    want_radio: u8,
    want_slot: u8,
    want_buy: u8,
    want_reload: bool,
    kill_feed: VecDeque<KillLine>,
    feed_dirty: bool,
    hurt_at: Option<f64>,
    fx: Vec<FxKind>,
    /// Everyone as of the newest snapshot, for the scoreboard.
    players: Vec<NetPlayer>,
    /// Distance run since the last footstep.
    stride: f32,
    steps_played: u32,
    /// Radio chat lines and when they were said.
    chat: VecDeque<(String, f64)>,
    /// Big centered announcement and when it was made.
    announce: Option<(String, f64)>,
    /// Viewmodel kick from the last shot, 1 right after firing and decaying to 0.
    kick: f32,
    anim: WeaponAnim,
}

impl Default for DustClient {
    fn default() -> DustClient {
        DustClient {
            scene: Scene::default(),
            sounds: Sounds::default(),
            hud: Hud::default(),
            fx_assets: FxAssets::default(),
            bomb: None,
            bomb_lit: None,
            viewmodels: Vec::new(),
            rigs: HashMap::new(),
            grenades: HashMap::new(),
            decals: VecDeque::new(),
            blood: blood::Blood::default(),
            live_fx: Vec::new(),
            me: None,
            pred: PlayerState::default(),
            hp: MAX_HP,
            armor: 0,
            money: 0,
            yaw: 0.0,
            pitch: 0.0,
            time: 0.0,
            tracers: Vec::new(),
            play: Vec::new(),
            wind_started: false,
            died_at: None,
            round: None,
            teams: HashMap::new(),
            names: HashMap::new(),
            next_beep: 0.0,
            last_beep: 0.0,
            explosion: None,
            spectate: None,
            aim_target: None,
            carrier_pos: None,
            zoom: false,
            menu: false,
            radio_menu: false,
            tab: false,
            want_radio: 0,
            want_slot: 0,
            want_buy: 0,
            want_reload: false,
            kill_feed: VecDeque::new(),
            feed_dirty: false,
            hurt_at: None,
            fx: Vec::new(),
            players: Vec::new(),
            stride: 0.0,
            steps_played: 0,
            chat: VecDeque::new(),
            announce: None,
            kick: 0.0,
            anim: WeaponAnim::default(),
        }
    }
}

impl DustClient {
    pub fn new() -> DustClient {
        DustClient::default()
    }
}

impl ClientGame for DustClient {
    // ponytail: dust is the only map, so `map` picks nothing yet.
    fn start(&mut self, _map: &str) -> Vec<Command> {
        self.load_sounds();
        self.bomb = Some(world::build(&mut self.scene));
        // First-person weapons; each barrel tip lines up with `muzzle`.
        self.viewmodels = ALL_WEAPONS
            .into_iter()
            .map(|w| (w, spawn_weapon(&mut self.scene, CAMERA, w, Transform::at(viewmodel_offset(w)), true)))
            .collect();
        let s = &mut self.scene;
        self.fx_assets = FxAssets {
            puff: s.mesh(MeshData::Sphere { radius: 1.0, ico: Some(1) }),
            hole_mesh: s.mesh(MeshData::Plane(Vec2::splat(0.09))),
            hole_material: s.material(Material { color: Color::srgb(0.08, 0.06, 0.05), unlit: true, ..Default::default() }),
            grenade_mesh: s.mesh(MeshData::Sphere { radius: 0.07, ico: None }),
            grenade_material: s.material(Material { color: Color::srgb(0.18, 0.24, 0.12), roughness: 0.7, ..Default::default() }),
            rocket_mesh: s.mesh(MeshData::Cylinder { radius: 0.05, height: 0.6 }),
            rocket_material: s.material(Material { color: Color::srgb(0.25, 0.27, 0.22), emissive: [2.0, 0.6, 0.1], ..Default::default() }),
        };
        self.build_hud();
        self.blood.init(&mut self.scene);
        std::mem::take(&mut self.scene.out)
    }

    fn snapshot(&mut self, snap: &ClientSnapshot) {
        let you = snap.you;
        self.me = Some(you);
        let players: Vec<NetPlayer> = snap.entities.iter().filter_map(NetPlayer::from_entity).collect();
        let events: Vec<Event> = snap.events.iter().filter_map(|e| decode(e)).collect();
        let Some(round) = decode::<Round>(&snap.state) else { return };
        self.players = players.clone();
        for p in &players {
            self.teams.insert(p.id, p.team);
            if self.names.get(&p.id) != Some(&p.name) {
                self.names.insert(p.id, p.name.clone());
            }
        }
        for e in &events {
            match *e {
                Event::Explosion { pos, radius } => {
                    self.explosion = Some((pos, self.time, radius));
                    self.fx.push(FxKind::Blast(pos, radius));
                    let volume = 1.0 / (1.0 + pos.distance(self.pred.pos) / 30.0);
                    self.sound(self.sounds.explosion, volume);
                }
                Event::Radio(r) => {
                    self.sound(self.radio_sound(r.call), 0.75);
                    let line = format!("{} (RADIO): {}", self.name(r.from), r.call.text());
                    self.chat.push_back((line, self.time));
                    if self.chat.len() > 4 {
                        self.chat.pop_front();
                    }
                }
                Event::Hit(h) => self.on_hit(&h, &players),
                Event::Shot(s) if s.shooter != you => self.on_remote_shot(&s),
                Event::Shot(_) => {}
            }
        }
        let prev = self.round.replace(round);
        if let Some(prev) = prev {
            // Keyed on entering a phase, not on the exact previous one: a round can end before a live snapshot arrives.
            match (prev.phase, round.phase) {
                (Phase::Freeze, Phase::Live) => self.say(Announce::Go, ""),
                (Phase::End { .. }, _) => {}
                (_, Phase::End { winner: Team::T }) => self.say(Announce::TWin, "Terrorists Win!"),
                (_, Phase::End { winner: Team::Ct }) => self.say(Announce::CtWin, "Counter-Terrorists Win!"),
                _ => {}
            }
            match (prev.bomb, round.bomb) {
                (Bomb::Carried(_), Bomb::Planted(_)) => self.say(Announce::Planted, "The bomb has been planted."),
                (Bomb::Planted(_), Bomb::Defused(_)) => self.say(Announce::Defused, "Bomb has been defused."),
                _ => {}
            }
        }
        if prev.map(|r| r.number) != Some(round.number) {
            // New round: everyone is back in spawn, so face the way the server spawned us.
            if let Some(me) = players.iter().find(|p| p.id == you) {
                (self.yaw, self.pitch) = (me.state.yaw, 0.0);
            }
            self.died_at = None;
            self.explosion = None;
            self.blood.clear(&mut self.scene);
        }
        if let Bomb::Exploded(pos) = round.bomb {
            if !matches!(prev.map(|r| r.bomb), Some(Bomb::Exploded(_))) {
                self.explosion = Some((pos, self.time, BOMB_RADIUS));
                let volume = 1.0 / (1.0 + pos.distance(self.pred.pos) / 30.0);
                self.sound(self.sounds.explosion, volume);
            }
        }
        if let Some(me) = players.iter().find(|p| p.id == you) {
            if me.money < self.money && round.buy_secs > 0.0 {
                self.sound(self.sounds.buy, 0.6);
            }
            (self.armor, self.money) = (me.armor, me.money);
            if self.hp > 0 && me.hp <= 0 {
                self.sound(self.sounds.death, 0.8);
                self.died_at = Some(self.time);
            }
            if me.hp > 0 {
                self.died_at = None;
            }
            self.hp = me.hp;
        }
    }

    fn input(&mut self, raw: &RawInput) -> Vec<u8> {
        let axis = |pos, neg| raw.held(pos) as i32 as f32 - raw.held(neg) as i32 as f32;
        let inp = Input {
            forward: axis(Key::W, Key::S),
            right: axis(Key::D, Key::A),
            yaw: self.yaw,
            pitch: self.pitch,
            jump: raw.held(Key::Space) || raw.held(Key::Backspace),
            walk: raw.held(Key::ShiftLeft) || raw.held(Key::ShiftRight),
            use_key: raw.held(Key::E),
            fire: raw.captured && !self.menu && raw.held(Key::MouseLeft),
            slot: std::mem::take(&mut self.want_slot),
            reload: std::mem::take(&mut self.want_reload),
            zoom: self.zoom,
            duck: raw.held(Key::ControlLeft) || raw.held(Key::ControlRight),
            buy: std::mem::take(&mut self.want_buy),
            radio: std::mem::take(&mut self.want_radio),
            ..Default::default()
        };
        encode(&inp)
    }

    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, fresh: bool) {
        let was_reloading = NetPlayer::from_entity(me).is_some_and(|p| p.state.hands.reloading > 0);
        let fired = predict(me, input, seq);
        if !fresh {
            return;
        }
        let Some(p) = NetPlayer::from_entity(me) else { return };
        self.pred = p.state;
        if self.me.is_none() || self.hp <= 0 {
            return;
        }
        let pred = self.pred;
        if let Some(Fired { weapon, .. }) = fired {
            self.sound(self.gun_sound(weapon), 0.6);
            self.kick = 1.0;
        }
        // Thrown things show up as entities from the server; shots are drawn right away.
        if let Some(Fired { weapon, dir: d, .. }) = fired.filter(|f| f.weapon.ability().range() > 0.0) {
            let o = eye(&pred);
            let (t, normal) = trace_world_normal(o, d);
            self.add_tracer(weapon, muzzle(o, d, weapon), o + d * t);
            if weapon != Weapon::Knife {
                self.fx.push(FxKind::Muzzle(muzzle(o, d, weapon)));
            }
            if t < weapon.ability().range().min(RANGE - 1.0) {
                self.fx.push(FxKind::Impact(o + d * t, normal));
                self.sound(self.sounds.impact, 0.25);
            }
            if weapon == Weapon::Awp {
                self.zoom = false;
            }
        }
        if pred.hands.reloading > 0 && !was_reloading {
            self.sound(self.sounds.reload, 0.5);
        }
        // Footsteps when running; walking (Shift) is silent.
        let speed = pred.vel.with_y(0.0).length();
        if pred.on_ground && speed > 6.35 * WALK_SPEED + 0.3 {
            self.stride += speed * DT;
            if self.stride > 1.8 {
                self.stride = 0.0;
                self.steps_played += 1;
                let step = self.sounds.steps[self.steps_played as usize % 4];
                self.sound(step, 0.3);
            }
        }
    }

    fn frame(&mut self, f: &FrameIn) -> FrameOut {
        self.time = f.time;
        if let Some(p) = f.me.as_ref().and_then(NetPlayer::from_entity) {
            self.pred = p.state;
        }
        if !self.wind_started {
            self.wind_started = true;
            self.play.push(Play { sound: self.sounds.wind, volume: 0.18, looped: true });
        }
        self.controls(&f.input);
        if f.input.captured {
            let sens = if self.zoom { MOUSE_SENS * ZOOM_FOV / FOV } else { MOUSE_SENS };
            self.yaw -= f.input.mouse.x * sens;
            self.pitch = (self.pitch - f.input.mouse.y * sens).clamp(-1.55, 1.55);
        }
        self.render_remotes(&f.others, f.dt);
        let camera = self.camera(f.me_pos);
        self.viewmodel(f.dt);
        let mut spheres = Vec::new();
        self.render_bomb(&mut spheres);
        self.bomb_beep();
        self.spawn_fx();
        self.update_fx(f.dt);
        self.blood.update(&mut self.scene, self.time);
        self.update_hud();
        let lines = self.tracers.iter().map(|&(a, b, ttl, life, color)| (a, b, color.with_alpha(color.0[3] * ttl / life))).collect();
        for t in &mut self.tracers {
            t.2 -= f.dt;
        }
        self.tracers.retain(|t| t.2 > 0.0);
        FrameOut { commands: std::mem::take(&mut self.scene.out), sounds: std::mem::take(&mut self.play), camera, lines, spheres }
    }
}

impl DustClient {
    fn load_sounds(&mut self) {
        let s = &mut self.scene;
        let mut sound = |wav: Vec<u8>| {
            let id = s.id();
            s.push(Command::Sound { id, wav });
            id
        };
        self.sounds = Sounds {
            guns: ALL_WEAPONS.into_iter().map(|w| (w, sound(sfx::gunshot(w)))).collect(),
            reload: sound(sfx::reload()),
            buy: sound(sfx::buy()),
            headshot: sound(sfx::headshot()),
            hit_body: sound(sfx::hit_body()),
            kill: sound(sfx::hit()),
            hurt: sound(sfx::hurt()),
            death: sound(sfx::death()),
            beep: sound(sfx::beep()),
            explosion: sound(sfx::explosion()),
            steps: (0..4).map(|v| sound(sfx::footstep(v))).collect(),
            impact: sound(sfx::impact()),
            wind: sound(sfx::wind()),
            voices: [
                (Announce::TWin, sfx::VOICE_T_WIN),
                (Announce::CtWin, sfx::VOICE_CT_WIN),
                (Announce::Planted, sfx::VOICE_PLANTED),
                (Announce::Defused, sfx::VOICE_DEFUSED),
                (Announce::Go, sfx::VOICE_GO),
                (Announce::EnemyDown, sfx::VOICE_ENEMY_DOWN),
            ]
            .into_iter()
            .map(|(a, wav)| (a, sound(sfx::radio(wav))))
            .collect(),
            radio: [Radio::EnemySpotted, Radio::NeedBackup, Radio::InPosition, Radio::SectorClear, Radio::CoverMe]
                .into_iter()
                .map(|r| (r, sound(sfx::radio(sfx::radio_line(r)))))
                .collect(),
        };
    }

    fn sound(&mut self, sound: u32, volume: f32) {
        self.play.push(Play { sound, volume, looped: false });
    }

    fn gun_sound(&self, weapon: Weapon) -> u32 {
        self.sounds.guns.iter().find(|(w, _)| *w == weapon).map_or(0, |g| g.1)
    }

    fn radio_sound(&self, call: Radio) -> u32 {
        self.sounds.radio.iter().find(|(r, _)| *r == call).map_or(0, |r| r.1)
    }

    fn say(&mut self, a: Announce, text: &str) {
        let voice = self.sounds.voices.iter().find(|(v, _)| *v == a).map_or(0, |v| v.1);
        self.sound(voice, 0.9);
        self.announce = Some((text.into(), self.time));
    }

    fn name(&self, id: u32) -> String {
        self.names.get(&id).cloned().unwrap_or_else(|| format!("player {id}"))
    }

    fn my_team(&self) -> Option<Team> {
        self.me.and_then(|me| self.teams.get(&me)).copied()
    }

    fn on_hit(&mut self, h: &Hit, players: &[NetPlayer]) {
        if Some(h.attacker) == self.me {
            self.sound(if h.headshot { self.sounds.headshot } else { self.sounds.hit_body }, 0.7);
            if h.killed {
                self.sound(self.sounds.kill, 0.6);
                let voice = self.sounds.voices.iter().find(|(v, _)| *v == Announce::EnemyDown).map_or(0, |v| v.1);
                self.sound(voice, 0.6);
            }
        }
        if Some(h.victim) == self.me {
            self.sound(self.sounds.hurt, 0.8);
            self.hurt_at = Some(self.time);
            self.blood.screen(&mut self.scene, self.time, h.damage);
        } else if let Some(v) = players.iter().find(|p| p.id == h.victim) {
            // Where they're drawn, not where the snapshot has them, so the blood comes out of the model.
            let feet = self.rigs.get(&h.victim).and_then(|r| r.last_pos).unwrap_or(v.state.pos);
            let tall = height(v.state.duck);
            let wound = feet + Vec3::Y * if h.headshot { tall - 0.15 } else { tall * 0.6 };
            let shooter = if Some(h.attacker) == self.me { Some(self.pred.pos) } else { players.iter().find(|p| p.id == h.attacker).map(|p| p.state.pos) };
            let dir = (feet - shooter.unwrap_or(feet - Vec3::X)).with_y(0.0).normalize_or(Vec3::X);
            self.fx.push(FxKind::Blood(wound));
            self.blood.hit(&mut self.scene, self.time, wound, feet, dir, h.damage, h.headshot, h.killed);
        }
        if h.killed {
            let side = |id: u32| (self.name(id), self.teams.get(&id).copied());
            let line = KillLine { killer: side(h.attacker), weapon: h.weapon, victim: side(h.victim), headshot: h.headshot, at: self.time };
            self.kill_feed.push_back(line);
            if self.kill_feed.len() > 5 {
                self.kill_feed.pop_front();
            }
            self.feed_dirty = true;
        }
    }

    fn on_remote_shot(&mut self, s: &Shot) {
        let d = (s.to - s.from).normalize_or_zero();
        // Thrown or launched: just the sound, and a flash for a rocket. What flies is an entity.
        if s.weapon.ability().range() == 0.0 {
            if s.weapon == Weapon::Rocket {
                self.fx.push(FxKind::Muzzle(muzzle(s.from, d, s.weapon)));
            }
            let volume = 0.6 / (1.0 + s.from.distance(self.pred.pos) / 12.0);
            return self.sound(self.gun_sound(s.weapon), volume);
        }
        self.add_tracer(s.weapon, muzzle(s.from, d, s.weapon), s.to);
        if s.weapon != Weapon::Knife {
            self.fx.push(FxKind::Muzzle(muzzle(s.from, d, s.weapon)));
            // A wall or floor where the shot ended (not a player).
            let (t, normal) = trace_world_normal(s.from, d);
            if (t - s.from.distance(s.to)).abs() < 0.05 {
                self.fx.push(FxKind::Impact(s.to, normal));
            }
        }
        let volume = 0.6 / (1.0 + s.from.distance(self.pred.pos) / 12.0);
        self.sound(self.gun_sound(s.weapon), volume);
    }

    fn add_tracer(&mut self, weapon: Weapon, from: Vec3, to: Vec3) {
        let life = match weapon {
            Weapon::Knife | Weapon::He | Weapon::Rocket => return,
            Weapon::Awp => 0.35,
            _ => 0.06,
        };
        self.tracers.push((from, to, life, life, Color::srgba(1.0, 0.9, 0.5, 0.8)));
    }

    /// Dead and the fade to black is done: watch a teammate instead.
    fn spectating(&self) -> bool {
        self.me.is_some() && self.hp <= 0 && self.died_at.is_none_or(|t| self.time - t > 1.6)
    }

    /// Keys that act once per press: buy menu, weapon slots, reload, zoom.
    fn controls(&mut self, raw: &RawInput) {
        let can_buy = self.round.is_some_and(|r| r.buy_secs > 0.0) && self.hp > 0;
        if raw.pressed(Key::B) && can_buy {
            self.menu = !self.menu;
        }
        if !can_buy || raw.pressed(Key::Escape) {
            self.menu = false;
        }
        let digits = [Key::Digit1, Key::Digit2, Key::Digit3, Key::Digit4, Key::Digit5, Key::Digit6, Key::Digit7];
        if raw.pressed(Key::Z) && self.hp > 0 {
            (self.radio_menu, self.menu) = (!self.radio_menu, false);
        }
        if raw.pressed(Key::Escape) || self.hp <= 0 {
            self.radio_menu = false;
        }
        if let Some(n) = digits.iter().position(|&k| raw.pressed(k)) {
            if self.radio_menu {
                (self.want_radio, self.radio_menu) = (n as u8 + 1, false);
            } else if self.menu {
                (self.want_buy, self.menu) = (n as u8 + 1, false);
            } else if n < 4 {
                self.want_slot = n as u8 + 1;
            }
        }
        if raw.pressed(Key::Q) {
            self.want_slot = 5;
        }
        if raw.pressed(Key::R) {
            self.want_reload = true;
        }
        self.tab = raw.held(Key::Tab);
        if raw.pressed(Key::MouseRight) && self.pred.weapon() == Weapon::Awp {
            self.zoom = !self.zoom;
        }
        if self.pred.weapon() != Weapon::Awp || self.pred.hands.reloading > 0 {
            self.zoom = false;
        }
    }

    fn camera(&self, me_pos: Vec3) -> Camera {
        let fov = if self.zoom { ZOOM_FOV } else { FOV };
        if let Some((_, eye, yaw, pitch)) = self.spectate.filter(|_| self.spectating()) {
            return Camera { pos: eye, rot: look_rot(yaw, pitch), fov };
        }
        let (side, up) = punch(&self.pred);
        Camera { pos: me_pos + Vec3::Y * eye_height(&self.pred), rot: look_rot(self.yaw + side, self.pitch + up), fov }
    }

    fn render_remotes(&mut self, others: &[sdk::Interpolated], dt: f32) {
        let my_team = self.my_team();
        let carrier = match self.round.map(|r| r.bomb) {
            Some(Bomb::Carried(id)) => Some(id),
            _ => None,
        };
        let (mut spectate, mut carrier_pos) = (None, None);
        let mut seen = HashMap::new();
        let mut grenades = HashMap::new();
        for v in others {
            if let Some(EntityData::Grenade(weapon)) = decode(&v.entity.data) {
                grenades.insert(v.entity.id, (v.entity.pos, v.entity.vel, weapon == Weapon::Rocket));
                continue;
            }
            let Some(p) = NetPlayer::from_entity(&v.entity) else { continue };
            let (pos, yaw, pitch) = (p.state.pos, p.state.yaw, p.state.pitch);
            // Dead as soon as we hear of it (the body falls and stays); hidden while a respawn is being interpolated
            // past, so the teleport is never drawn as a slide across the map.
            let alive_now = self.players.iter().any(|q| q.id == p.id && q.hp > 0);
            let alive = alive_now && p.hp > 0 && !v.teleporting;
            let life = if alive { Life::Alive } else if !alive_now { Life::Dead } else { Life::Respawning };
            if alive && spectate.is_none() && Some(p.team) == my_team {
                spectate = Some((p.id, pos + Vec3::Y * eye_height(&p.state), yaw, pitch));
            }
            if Some(p.id) == carrier {
                carrier_pos = Some(pos);
            }
            seen.insert(p.id, (pos, yaw, pitch, life, p.state.weapon(), p.state.duck, p.team));
        }
        self.render_grenades(grenades);
        (self.spectate, self.carrier_pos) = (spectate, carrier_pos);
        // Whoever is under the crosshair, walls permitting, for the "Friend: / Enemy:" line.
        let (eye, dir) = (eye(&self.pred), look_rot(self.yaw, self.pitch) * Vec3::NEG_Z);
        let wall = trace_world(eye, dir);
        self.aim_target = seen
            .iter()
            .filter(|(_, v)| v.3 == Life::Alive)
            .filter_map(|(&id, v)| trace_capsule(eye, dir, v.0, height(v.5)).filter(|&t| t < wall).map(|t| (id, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id);
        // We're looking out of their eyes, so their own model would only block the view.
        let watched = spectate.filter(|_| self.spectating()).map(|s| s.0);

        let gone: Vec<u32> = self.rigs.keys().filter(|id| !seen.contains_key(id)).copied().collect();
        for id in gone {
            let rig = self.rigs.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: rig.root });
        }
        for (&id, &(pos, yaw, pitch, life, weapon, duck, team)) in &seen {
            if !self.rigs.contains_key(&id) {
                let rig = spawn_soldier(&mut self.scene, team, Transform::at(pos).rotated(Quat::from_rotation_y(yaw)));
                self.rigs.insert(id, rig);
            }
            let time = self.time;
            let rig = self.rigs.get_mut(&id).unwrap();
            let alive = life == Life::Alive;
            // A dead player topples over, forwards or backwards, and stays down until the next round.
            let fall = match life {
                Life::Dead => {
                    let since = *rig.died_at.get_or_insert(time);
                    ((time - since) as f32 / 0.45).min(1.0).powi(2)
                }
                _ => {
                    rig.died_at = None;
                    0.0
                }
            };
            let side = if id % 2 == 0 { 1.0 } else { -1.0 };
            // Crouched: down on one knee, legs swung back from the hips.
            let duck = if life == Life::Dead { 0.0 } else { duck };
            let root = Transform::at(pos + Vec3::Y * (0.13 * fall - 0.42 * duck)).rotated(Quat::from_rotation_y(yaw) * Quat::from_rotation_x(side * 1.5 * fall));
            // Players pass through each other; hide anyone overlapping our camera rather than fill the screen with them.
            // ponytail: no player-player collision. Add it to the shared movement if walking through people matters.
            let overlapping = pos.with_y(0.0).distance(self.pred.pos.with_y(0.0)) < RADIUS * 2.0;
            let shown = match life {
                Life::Alive => watched != Some(id) && !overlapping,
                Life::Dead => true,
                Life::Respawning => false,
            };
            let (swing, moved) = rig.animate(pos, dt);
            let (swing, pitch) = if alive { (swing, pitch) } else { (0.0, -0.4) };
            // Their footsteps, louder up close: how you hear someone coming.
            let speed = moved / dt.max(1e-3);
            let mut step = None;
            if alive && speed > 6.35 * WALK_SPEED + 0.3 && speed < 12.0 {
                rig.stride += moved;
                if rig.stride > 1.8 {
                    rig.stride = 0.0;
                    let dist = pos.distance(self.pred.pos);
                    if dist < 35.0 {
                        step = Some((id as usize + (rig.phase * 10.0) as usize, 0.45 / (1.0 + dist / 5.0)));
                    }
                }
            }
            let out = &mut self.scene.out;
            out.push(Command::Set { id: rig.root, transform: Some(root), visible: Some(shown), light: None });
            for (leg, (x, dir)) in rig.legs.into_iter().zip([(-0.11, 1.0), (0.11, -1.0)]) {
                let bend = Quat::from_rotation_x(swing * dir * (1.0 - duck) - 1.0 * duck);
                out.push(Command::set_transform(leg, Transform::at(Vec3::new(x, 0.92, 0.0)).rotated(bend)));
            }
            out.push(Command::set_transform(rig.arms, Transform::at(Vec3::new(0.0, 1.42, 0.0)).rotated(Quat::from_rotation_x(pitch))));
            let gun = weapon as usize;
            if rig.gun != Some(gun) {
                if let Some(old) = rig.gun {
                    out.push(Command::set_visible(rig.guns[old], false));
                }
                out.push(Command::set_visible(rig.guns[gun], true));
                rig.gun = Some(gun);
            }
            if let Some((variant, volume)) = step {
                let sound = self.sounds.steps[variant % 4];
                self.sound(sound, volume);
            }
        }
    }

    /// Thrown and launched things: grenades as balls, rockets as a body along their flight trailing smoke.
    fn render_grenades(&mut self, mut now: HashMap<u32, (Vec3, Vec3, bool)>) {
        let gone: Vec<u32> = self.grenades.keys().filter(|id| !now.contains_key(id)).copied().collect();
        for id in gone {
            let node = self.grenades.remove(&id).unwrap();
            self.scene.push(Command::Despawn { id: node });
        }
        // A rocket points along its flight; a grenade just tumbles along.
        let pose = |pos: Vec3, vel: Vec3, rocket: bool| match rocket && vel.length() > 0.1 {
            true => Transform::at(pos).rotated(Quat::from_rotation_arc(Vec3::Y, vel.normalize())),
            false => Transform::at(pos),
        };
        let eye = eye(&self.pred);
        for (&id, &node) in &self.grenades {
            if let Some((pos, vel, rocket)) = now.remove(&id) {
                self.scene.out.push(Command::set_transform(node, pose(pos, vel, rocket)));
                // Smoke behind it, though not in our face as it leaves.
                if rocket && pos.distance(eye) > 2.0 {
                    self.fx.push(FxKind::Smoke(pos - vel.normalize_or_zero() * 0.4));
                }
            }
        }
        for (id, (pos, vel, rocket)) in now {
            let t = pose(pos, vel, rocket);
            let a = &self.fx_assets;
            let (mesh, material) = if rocket { (a.rocket_mesh, a.rocket_material) } else { (a.grenade_mesh, a.grenade_material) };
            let node = self.scene.mesh_node(None, mesh, material, t);
            self.grenades.insert(id, node);
        }
    }

    /// Shows the held weapon with CS-style motion: a figure-eight bob while running, sway behind mouse turns, a kick
    /// per shot, raising on draw, a dip while reloading and a slash for the knife. Hidden when dead or scoped.
    fn viewmodel(&mut self, dt: f32) {
        let held = self.pred.weapon();
        let anim = &mut self.anim;
        if anim.held != Some(held) {
            (anim.held, anim.drawn_at) = (Some(held), self.time);
        }
        let draw = (1.0 - ((self.time - anim.drawn_at) as f32 / DRAW_SECS).min(1.0)).powi(2);
        let reload = self.pred.hands.reloading as f32 / (held.ability().reload_secs() * TICK_HZ as f32).max(1.0);
        let dip = (reload * std::f32::consts::PI).sin();

        let speed = if self.pred.on_ground { (self.pred.vel.with_y(0.0).length() / 6.35).min(1.0) } else { 0.0 };
        anim.bob += dt * 9.0 * speed;
        let bob = Vec3::new(anim.bob.sin() * 0.012, -(anim.bob * 2.0).sin().abs() * 0.01, 0.0) * speed;

        let look = Vec2::new(self.yaw, self.pitch);
        let turn = (look - anim.last_look) / dt.max(1e-3);
        anim.last_look = look;
        let target = (Vec2::new(-turn.x, turn.y) * 0.004).clamp(Vec2::splat(-0.04), Vec2::splat(0.04));
        anim.sway = anim.sway.lerp(target, (dt * 10.0).min(1.0));

        let heavy = matches!(held, Weapon::Awp | Weapon::Deagle);
        let kick = self.kick * if heavy { 1.6 } else { 1.0 };
        self.kick *= (-dt * 16.0).exp();
        let slash = if held == Weapon::Knife { self.kick } else { 0.0 };

        let show = (self.hp > 0 && self.me.is_some() && !self.zoom).then_some(held);
        if anim.shown != show {
            for &(w, node) in &self.viewmodels {
                if Some(w) == anim.shown || Some(w) == show {
                    self.scene.out.push(Command::set_visible(node, Some(w) == show));
                }
            }
            anim.shown = show;
        }
        if let Some(&(w, node)) = self.viewmodels.iter().find(|(w, _)| Some(*w) == show) {
            let offset = Vec3::new(anim.sway.x, anim.sway.y - 0.15 * dip - 0.3 * draw, 0.05 * kick) + bob;
            let rot = Quat::from_rotation_x(-0.6 * dip - 0.9 * draw + 0.12 * kick) * Quat::from_rotation_y(1.2 * slash);
            self.scene.out.push(Command::set_transform(node, Transform::at(viewmodel_offset(w) + offset).rotated(rot)));
        }
    }

    fn render_bomb(&mut self, spheres: &mut Vec<(Vec3, f32, Color)>) {
        let Some(bomb) = &mut self.bomb else { return };
        // The light flashes with each beep while planted.
        let planted = matches!(self.round.map(|r| r.bomb), Some(Bomb::Planted(_)));
        let lit = planted && self.time - self.last_beep < 0.09;
        if self.bomb_lit != Some(lit) {
            self.bomb_lit = Some(lit);
            bomb.led_material.emissive = if lit { [40.0, 0.0, 0.0] } else { [0.3, 0.0, 0.0] };
            self.scene.out.push(Command::Material { id: bomb.led, material: bomb.led_material });
            self.scene.out.push(Command::Set { id: bomb.light, transform: None, visible: None, light: Some(if lit { 4000.0 } else { 0.0 }) });
        }
        let at = match self.round.map(|r| r.bomb) {
            Some(Bomb::Dropped(p) | Bomb::Planted(p)) => Some(p + Vec3::Y * 0.045),
            // On the carrier's back.
            Some(Bomb::Carried(_)) => self.carrier_pos.map(|p| p + Vec3::Y * 1.1),
            _ => None,
        };
        self.scene.out.push(Command::Set { id: bomb.root, transform: Some(Transform::at(at.unwrap_or_default())), visible: Some(at.is_some()), light: None });
        if let Some((pos, start, radius)) = self.explosion {
            let age = (self.time - start) as f32;
            if age < 1.0 {
                spheres.push((pos, radius * age.sqrt(), Color::srgba(1.0, 0.6, 0.1, 1.0 - age)));
            }
        }
    }

    /// Beeps speed up as the timer runs down, like the real C4.
    fn bomb_beep(&mut self) {
        let Some(Round { bomb: Bomb::Planted(pos), time_left, .. }) = self.round else { return };
        if self.time < self.next_beep {
            return;
        }
        self.last_beep = self.time;
        self.next_beep = self.time + 0.12 + 0.88 * (time_left / BOMB_SECS) as f64;
        let volume = 0.7 / (1.0 + pos.distance(self.pred.pos) / 15.0);
        self.sound(self.sounds.beep, volume);
    }

    /// Spawns this frame's muzzle flashes, impact puffs, bullet holes and blood.
    fn spawn_fx(&mut self) {
        let (puff, hole_mesh, hole_material) = (self.fx_assets.puff, self.fx_assets.hole_mesh, self.fx_assets.hole_material);
        for fx in std::mem::take(&mut self.fx) {
            let fading = |scene: &mut Scene, at: Vec3, color: Color, size: f32, life: f32, grow: f32| {
                let material = Material { color, unlit: true, blend: Blend::Blend, ..Default::default() };
                let id = scene.material(material);
                let node = scene.spawn(None, NodeKind::Mesh { mesh: puff, material: id, shadows: false }, Transform::at(at).scaled(size), true);
                Fx { node, pos: at, material: Some((id, material)), light: None, age: 0.0, life, grow, size }
            };
            match fx {
                FxKind::Muzzle(at) => {
                    let flash = fading(&mut self.scene, at, Color::srgba(1.0, 0.85, 0.4, 0.9), 0.07, 0.05, 1.5);
                    self.live_fx.push(flash);
                    let kind = NodeKind::PointLight { color: Color::srgb(1.0, 0.8, 0.5), intensity: 60_000.0, range: 8.0 };
                    let node = self.scene.spawn(None, kind, Transform::at(at), true);
                    self.live_fx.push(Fx { node, pos: at, material: None, light: Some(60_000.0), age: 0.0, life: 0.05, grow: 0.0, size: 1.0 });
                }
                FxKind::Impact(at, normal) => {
                    let puff = fading(&mut self.scene, at + normal * 0.08, Color::srgba(0.75, 0.62, 0.45, 0.8), 0.06, 0.45, 5.0);
                    self.live_fx.push(puff);
                    let rot = Quat::from_rotation_arc(Vec3::Y, normal);
                    let hole = self.scene.mesh_node(None, hole_mesh, hole_material, Transform::at(at + normal * 0.012).rotated(rot));
                    self.decals.push_back(hole);
                    if self.decals.len() > 80 {
                        let id = self.decals.pop_front().unwrap();
                        self.scene.push(Command::Despawn { id });
                    }
                }
                FxKind::Smoke(at) => {
                    let smoke = fading(&mut self.scene, at, Color::srgba(0.75, 0.73, 0.7, 0.5), 0.08, 0.8, 3.0);
                    self.live_fx.push(smoke);
                }
                FxKind::Blast(at, radius) => {
                    let fire = fading(&mut self.scene, at, Color::srgba(1.0, 0.55, 0.1, 0.95), radius * 0.15, 0.35, 4.0);
                    let smoke = fading(&mut self.scene, at + Vec3::Y * 0.5, Color::srgba(0.25, 0.23, 0.22, 0.8), radius * 0.2, 1.6, 2.5);
                    self.live_fx.extend([fire, smoke]);
                    let kind = NodeKind::PointLight { color: Color::srgb(1.0, 0.6, 0.2), intensity: 4_000_000.0, range: radius * 4.0 };
                    let node = self.scene.spawn(None, kind, Transform::at(at + Vec3::Y * 0.5), true);
                    self.live_fx.push(Fx { node, pos: at, material: None, light: Some(4_000_000.0), age: 0.0, life: 0.4, grow: 0.0, size: 1.0 });
                }
                FxKind::Blood(at) => {
                    let blood = fading(&mut self.scene, at, Color::srgba(0.6, 0.02, 0.02, 0.9), 0.14, 0.45, 5.0);
                    self.live_fx.push(blood);
                }
            }
        }
    }

    fn update_fx(&mut self, dt: f32) {
        let out = &mut self.scene.out;
        self.live_fx.retain_mut(|fx| {
            fx.age += dt;
            let k = fx.age / fx.life;
            if k >= 1.0 {
                out.push(Command::Despawn { id: fx.node });
                if let Some((id, _)) = fx.material {
                    out.push(Command::Free { id });
                }
                return false;
            }
            let light = fx.light.map(|l| l * (1.0 - k));
            let t = Transform::at(fx.pos).scaled(fx.size * (1.0 + fx.grow * k));
            out.push(Command::Set { id: fx.node, transform: fx.light.is_none().then_some(t), visible: None, light });
            if let Some((id, mut material)) = fx.material {
                material.color = material.color.with_alpha((1.0 - k) * 0.85);
                out.push(Command::Material { id, material });
            }
            true
        });
    }
}

/// Barrel tip for a shot from eye `o` along `d`, matching the first-person model, so tracers leave the gun.
fn muzzle(o: Vec3, d: Vec3, weapon: Weapon) -> Vec3 {
    let right = d.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(d);
    let at = viewmodel_offset(weapon);
    o + right * at.x + up * at.y + d * (barrel_length(weapon) - at.z)
}

fn team_text_color(team: Option<Team>) -> Color {
    match team {
        Some(Team::T) => Color::srgb(1.0, 0.45, 0.3),
        Some(Team::Ct) => Color::srgb(0.55, 0.7, 1.0),
        None => Color::WHITE,
    }
}
