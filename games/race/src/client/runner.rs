//! Runners as others see them: a rounded figure in a white suit with a jersey, sleeves, helmet fin, belt and shoes in
//! the player's color, animated from how it moves: arms and legs swing with its stride and it leans into a run, tucks
//! going up, flails coming down and squashes on landing.

use super::{glowing, readable, rgb};
use crate::shared::MAX_SPEED;
use glam::{Quat, Vec3};
use sdk::client::*;
use sdk::scene::Scene;
use std::f32::consts::{PI, TAU};

/// Meters run per stride (a step with each leg).
const STRIDE: f32 = 2.6;

pub struct Rig {
    pub root: u32,
    /// Everything above the feet, leaned and squashed as one.
    body: u32,
    hips: [u32; 2],
    knees: [u32; 2],
    shoulders: [u32; 2],
    elbows: [u32; 2],
    pub tag: u32,
    /// Where in the stride, radians.
    phase: f32,
    /// Smoothed blends: how much it runs (0 standing, 1 at full speed), and how much it's in the air.
    run: f32,
    air: f32,
    /// Landing squash, dying away.
    squash: f32,
    on_ground: bool,
    /// Speed downwards when last in the air, for how hard it lands.
    fall: f32,
    time: f32,
}

/// The parts are all one sphere, scaled.
pub fn spawn(s: &mut Scene, color: [u8; 3]) -> Rig {
    let ball = s.shared_mesh("ball".into(), || MeshData::Sphere { radius: 1.0, ico: None });
    let suit = s.shared_material("suit".into(), || Material { color: Color::srgb(0.92, 0.92, 0.94), roughness: 0.45, ..Default::default() });
    let dark = s.shared_material("visor".into(), || Material { color: Color::srgb(0.03, 0.03, 0.05), roughness: 0.1, metallic: 0.6, ..Default::default() });
    let jersey = s.shared_material(format!("jersey {color:?}"), || Material { roughness: 0.55, ..glowing(rgb(color), 0.3) });
    let trim = s.shared_material(format!("trim {color:?}"), || glowing(rgb(color), 2.0));
    let shorts = s.shared_material("shorts".into(), || Material { color: Color::srgb(0.1, 0.1, 0.12), roughness: 0.8, ..Default::default() });
    let part = |s: &mut Scene, parent: u32, material: u32, at: [f32; 3], size: [f32; 3]| {
        s.mesh_node(Some(parent), ball, material, Transform { scale: size.into(), ..Transform::at(at.into()) });
    };
    let joint = |s: &mut Scene, parent: u32, at: [f32; 3]| s.empty(Some(parent), Transform::at(at.into()), true);

    let root = s.empty(None, Transform::default(), false);
    let body = s.empty(Some(root), Transform::default(), true);
    part(s, body, jersey, [0.0, 1.34, 0.0], [0.25, 0.2, 0.16]);
    part(s, body, jersey, [0.0, 1.13, 0.0], [0.19, 0.2, 0.14]);
    part(s, body, shorts, [0.0, 0.94, 0.0], [0.19, 0.13, 0.14]);
    part(s, body, trim, [0.0, 1.0, 0.0], [0.2, 0.035, 0.15]);
    part(s, body, suit, [0.0, 1.53, 0.0], [0.07, 0.07, 0.07]);
    part(s, body, suit, [0.0, 1.68, 0.0], [0.16, 0.17, 0.17]);
    part(s, body, dark, [0.0, 1.68, -0.1], [0.13, 0.075, 0.08]);
    part(s, body, trim, [0.0, 1.78, 0.03], [0.04, 0.14, 0.19]);
    let mut knees = [0; 2];
    let hips = [-0.11, 0.11].map(|x| {
        let hip = joint(s, body, [x, 0.92, 0.0]);
        part(s, hip, suit, [0.0, -0.21, 0.0], [0.095, 0.23, 0.1]);
        let knee = joint(s, hip, [0.0, -0.44, 0.0]);
        part(s, knee, suit, [0.0, -0.2, 0.0], [0.07, 0.22, 0.075]);
        part(s, knee, trim, [0.0, -0.44, -0.04], [0.075, 0.045, 0.13]);
        knees[(x > 0.0) as usize] = knee;
        hip
    });
    let mut elbows = [0; 2];
    let shoulders = [-0.27, 0.27].map(|x| {
        let shoulder = joint(s, body, [x, 1.44, 0.0]);
        part(s, shoulder, jersey, [0.0, 0.0, 0.0], [0.075, 0.075, 0.075]);
        part(s, shoulder, jersey, [0.0, -0.13, 0.0], [0.07, 0.16, 0.07]);
        let elbow = joint(s, shoulder, [0.0, -0.27, 0.0]);
        part(s, elbow, suit, [0.0, -0.12, 0.0], [0.055, 0.14, 0.055]);
        part(s, elbow, dark, [0.0, -0.27, 0.0], [0.05, 0.05, 0.05]);
        elbows[(x > 0.0) as usize] = elbow;
        shoulder
    });
    let tag = s.ui(None, UiNode { text: Some(Text { shadow: true, center: true, ..Text::new("", 14.0, readable(color)) }), hidden: true, ..Default::default() });
    Rig { root, body, hips, knees, shoulders, elbows, tag, phase: 0.0, run: 0.0, air: 0.0, squash: 0.0, on_ground: true, fall: 0.0, time: 0.0 }
}

impl Rig {
    /// Poses the runner standing at `pos` facing `yaw`, moving at `vel`.
    pub fn animate(&mut self, out: &mut Vec<Command>, pos: Vec3, yaw: f32, vel: Vec3, on_ground: bool, shown: bool, dt: f32) {
        let ease = |rate: f32| 1.0 - (-rate * dt).exp();
        let speed = vel.with_y(0.0).length();
        if on_ground && !self.on_ground {
            self.squash = (self.fall / 40.0).min(0.25);
        }
        if !on_ground {
            self.fall = -vel.y;
        }
        self.on_ground = on_ground;
        self.time += dt;
        self.squash *= 1.0 - ease(10.0);
        self.run += ((speed / MAX_SPEED).min(1.0) - self.run) * ease(10.0);
        self.air += (!on_ground as u8 as f32 - self.air) * ease(14.0);
        if on_ground {
            self.phase = (self.phase + speed * dt * TAU / STRIDE).rem_euclid(TAU);
        }
        // Going up tucks, coming down flails; `air` blends from the running pose to those.
        let up = ((vel.y + 1.0) / 4.0).clamp(0.0, 1.0);
        let (r, air) = (self.run, self.air);
        let mix = |ground: f32, rise: f32, fall: f32| ground + (fall + (rise - fall) * up - ground) * air;
        let flail = (self.time * 14.0).sin() * 0.3;

        let (sy, sxz) = (1.0 - self.squash + 0.06 * air * up, 1.0 + self.squash * 0.6 - 0.03 * air * up);
        let lean = mix(-0.22 * r, -0.12, 0.06);
        let bob = r * (1.0 - air) * 0.06 * (0.5 + 0.5 * (2.0 * self.phase).cos());
        out.push(Command::Set { id: self.root, transform: Some(Transform::at(pos).rotated(Quat::from_rotation_y(yaw))), visible: Some(shown), light: None });
        let body = Transform { scale: Vec3::new(sxz, sy, sxz), ..Transform::at(Vec3::Y * bob).rotated(Quat::from_rotation_x(lean)) };
        out.push(Command::set_transform(self.body, body));
        for i in 0..2 {
            let (phi, side) = (self.phase + PI * i as f32, [-1.0, 1.0][i]);
            let hip = mix(r * 0.85 * phi.sin(), [1.2, 0.45][i], [0.35, -0.15][i] + flail * side * 0.5);
            let knee = mix(-0.1 - r * (0.15 + 1.25 * phi.cos().max(0.0)), [-1.8, -1.0][i], [-0.5, -0.25][i]);
            let arm = mix(-r * 0.9 * phi.sin(), 2.6, 0.3 + flail * side);
            let spread = mix(0.1, 0.35, 1.4) * side;
            let elbow = mix(0.25 + r * 1.2, 0.4, 0.5);
            out.push(Command::set_transform(self.hips[i], Transform::at(Vec3::new(0.11 * side, 0.92, 0.0)).rotated(Quat::from_rotation_x(hip))));
            out.push(Command::set_transform(self.knees[i], Transform::at(Vec3::Y * -0.44).rotated(Quat::from_rotation_x(knee))));
            let shoulder = Quat::from_rotation_z(spread) * Quat::from_rotation_x(arm);
            out.push(Command::set_transform(self.shoulders[i], Transform::at(Vec3::new(0.27 * side, 1.44, 0.0)).rotated(shoulder)));
            out.push(Command::set_transform(self.elbows[i], Transform::at(Vec3::Y * -0.27).rotated(Quat::from_rotation_x(elbow))));
        }
    }
}
