//! Blood: droplets flung out of every hit that splat where they land, a big splash on the wall behind, pools under
//! the dead, and blood across your own screen when you're hit.

use super::scene::Scene;
use super::textures;
use crate::shared::trace_world_normal;
use glam::{Quat, Vec2, Vec3};
use sdk::client::{Blend, Color, Command, Image, Material, MeshData, NodeKind, Transform, UiNode, percent, px};
use std::collections::VecDeque;

const GRAVITY: f32 = 14.0;
/// Droplets that haven't hit anything by then are gone.
const DROP_SECS: f32 = 1.2;
const MAX_DROPS: usize = 400;
/// Splats and pools on the map at once; the oldest go first. All of them go at the next round.
const MAX_SPLATS: usize = 150;
const SCREEN_SECS: f64 = 2.0;
const POOL_SECS: f32 = 3.0;

/// A droplet in flight along a parabola from `from`, landing after `land` seconds at `hit` (point, normal).
struct Drop {
    node: u32,
    from: Vec3,
    vel: Vec3,
    born: f64,
    land: f32,
    hit: Option<(Vec3, Vec3)>,
    size: f32,
    splat: f32,
}

impl Drop {
    fn at(&self, t: f32) -> Vec3 {
        self.from + self.vel * t - Vec3::Y * 0.5 * GRAVITY * t * t
    }
}

/// A pool spreading under a body.
struct Pool {
    node: u32,
    base: Transform,
    born: f64,
    size: f32,
}

#[derive(Default)]
pub struct Blood {
    texture: u32,
    splat_material: u32,
    drop_material: u32,
    drop_mesh: u32,
    plane: u32,
    drops: Vec<Drop>,
    splats: VecDeque<u32>,
    pools: Vec<Pool>,
    /// Screen splatters: HUD node and when it appeared.
    screen: Vec<(u32, f64)>,
    rng: u32,
    /// Lifts each new splat a little above the last, so overlapping ones don't flicker.
    layer: f32,
}

impl Blood {
    pub fn init(&mut self, scene: &mut Scene) {
        self.texture = scene.id();
        scene.push(textures::blood().command(self.texture));
        let wet = Material { texture: Some(self.texture), blend: Blend::Blend, roughness: 0.25, ..Default::default() };
        self.splat_material = scene.material(wet);
        self.drop_material = scene.material(Material { color: Color::srgb(0.45, 0.0, 0.01), roughness: 0.2, ..Default::default() });
        self.drop_mesh = scene.mesh(MeshData::Sphere { radius: 1.0, ico: Some(1) });
        self.plane = scene.mesh(MeshData::Plane(Vec2::ONE));
        self.rng = 0x2545_F491;
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.random()
    }

    /// A hit on someone: `wound` is where the bullet went in, `feet` where they stand, `dir` the way it travelled.
    #[allow(clippy::too_many_arguments)]
    pub fn hit(&mut self, scene: &mut Scene, time: f64, wound: Vec3, feet: Vec3, dir: Vec3, damage: u8, headshot: bool, killed: bool) {
        let base = (8 + damage as usize / 3) as f32 * if headshot { 1.8 } else { 1.0 };
        let count = (base as usize + if killed { 12 } else { 0 }).min(60);
        let bigger = if headshot { 1.4 } else { 1.0 };
        for i in 0..count {
            // Most of it out the far side, some back towards the shooter.
            let out = if i % 10 < 7 { dir * self.between(2.0, 6.5) } else { -dir * self.between(1.0, 3.0) };
            let jitter = Vec3::new(self.random() - 0.5, self.random() - 0.5, self.random() - 0.5) * self.between(1.0, 5.0);
            let vel = out + jitter + Vec3::Y * self.between(0.5, 3.0);
            let (size, splat) = (self.between(0.025, 0.055), self.between(0.12, 0.4) * bigger);
            self.fling(scene, time, wound, vel, size, splat);
        }
        // A big splash on whatever is close behind, and some on the floor under the wound.
        let (t, normal) = trace_world_normal(wound, dir);
        if t < 3.0 {
            let size = (0.7 + damage as f32 / 60.0) * bigger;
            self.splat(scene, wound + dir * t, normal, size);
        }
        let floor = trace_world_normal(wound, Vec3::NEG_Y).0;
        let size = self.between(0.4, 0.8) * bigger;
        self.splat(scene, wound - Vec3::Y * floor, Vec3::Y, size);
        if killed {
            let size = self.between(1.6, 2.4);
            let floor = trace_world_normal(feet + Vec3::Y * 0.5, Vec3::NEG_Y).0;
            let node = self.splat(scene, feet + Vec3::Y * (0.5 - floor), Vec3::Y, size);
            let base = Transform { scale: Vec3::splat(size * 0.2), ..self.placed(feet + Vec3::Y * (0.5 - floor), Vec3::Y) };
            scene.push(Command::set_transform(node, base));
            self.pools.push(Pool { node, base, born: time, size });
        }
    }

    /// Works out where a droplet lands up front, so flying it is just moving a node.
    fn fling(&mut self, scene: &mut Scene, time: f64, from: Vec3, vel: Vec3, size: f32, splat: f32) {
        if self.drops.len() >= MAX_DROPS {
            return;
        }
        let mut drop = Drop { node: 0, from, vel, born: time, land: DROP_SECS, hit: None, size, splat };
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while t < DROP_SECS {
            let (a, b) = (drop.at(t), drop.at(t + dt));
            let len = a.distance(b);
            let (along, normal) = trace_world_normal(a, (b - a) / len);
            if along <= len {
                (drop.land, drop.hit) = (t + dt * along / len, Some((a + (b - a) / len * along, normal)));
                break;
            }
            t += dt;
        }
        let kind = NodeKind::Mesh { mesh: self.drop_mesh, material: self.drop_material, shadows: false };
        drop.node = scene.spawn(None, kind, Transform::at(from).scaled(size), true);
        self.drops.push(drop);
    }

    /// Lying on a surface with normal `n`, turned at random so splats don't all look the same.
    fn placed(&mut self, at: Vec3, n: Vec3) -> Transform {
        self.layer = (self.layer + 0.0004) % 0.02;
        let spin = Quat::from_rotation_y(self.random() * std::f32::consts::TAU);
        Transform::at(at + n * (0.01 + self.layer)).rotated(Quat::from_rotation_arc(Vec3::Y, n) * spin)
    }

    fn splat(&mut self, scene: &mut Scene, at: Vec3, n: Vec3, size: f32) -> u32 {
        let t = Transform { scale: Vec3::splat(size), ..self.placed(at, n) };
        let node = scene.spawn(None, NodeKind::Mesh { mesh: self.plane, material: self.splat_material, shadows: false }, t, true);
        self.splats.push_back(node);
        if self.splats.len() > MAX_SPLATS {
            let id = self.splats.pop_front().unwrap();
            self.pools.retain(|p| p.node != id);
            scene.push(Command::Despawn { id });
        }
        node
    }

    /// We got hit: blood across the screen, more the harder.
    pub fn screen(&mut self, scene: &mut Scene, time: f64, damage: u8) {
        for _ in 0..(1 + damage as usize / 20).min(6) {
            let size = self.between(180.0, 420.0);
            let (left, top) = (self.between(-5.0, 85.0), self.between(-5.0, 80.0));
            let node = UiNode {
                absolute: true,
                left: percent(left),
                top: percent(top),
                width: px(size),
                height: px(size),
                image: Some(Image { texture: self.texture, tint: Color::WHITE }),
                ..Default::default()
            };
            let id = scene.ui(None, node);
            self.screen.push((id, time));
        }
    }

    pub fn update(&mut self, scene: &mut Scene, time: f64) {
        let mut landed = Vec::new();
        for d in &self.drops {
            let age = (time - d.born) as f32;
            if age >= d.land {
                scene.push(Command::Despawn { id: d.node });
                landed.extend(d.hit.map(|(at, n)| (at, n, d.splat)));
            } else {
                scene.push(Command::set_transform(d.node, Transform::at(d.at(age)).scaled(d.size)));
            }
        }
        self.drops.retain(|d| ((time - d.born) as f32) < d.land);
        for (at, n, size) in landed {
            self.splat(scene, at, n, size);
        }
        self.pools.retain(|p| {
            let k = ((time - p.born) as f32 / POOL_SECS).min(1.0);
            let grown = p.size * (0.2 + 0.8 * (1.0 - (1.0 - k).powi(3)));
            scene.push(Command::set_transform(p.node, Transform { scale: Vec3::splat(grown), ..p.base }));
            k < 1.0
        });
        self.screen.retain(|&(id, born)| {
            let k = (time - born) / SCREEN_SECS;
            if k >= 1.0 {
                scene.ui_despawn(id);
                return false;
            }
            let alpha = (1.0 - k as f32).powf(0.6) * 0.9;
            scene.ui_set(id, |n| n.image = n.image.map(|i| Image { tint: Color::WHITE.with_alpha(alpha), ..i }));
            true
        });
    }

    /// A new round: the map is clean again.
    pub fn clear(&mut self, scene: &mut Scene) {
        for id in self.drops.drain(..).map(|d| d.node).chain(self.splats.drain(..)) {
            scene.push(Command::Despawn { id });
        }
        self.pools.clear();
    }
}
