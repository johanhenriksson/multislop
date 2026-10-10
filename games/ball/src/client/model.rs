//! The rocket launcher (dust's model) and the players carrying it (the arena's armored figures, in team colors).
//! Models face -Z with the origin at the feet; the launcher's grip is at its origin.

use super::{rgb, team_rgb};
use crate::shared::Team;
use glam::{Quat, Vec3};
use sdk::client::{Color, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

#[derive(Clone, Copy)]
enum Finish {
    Black,
    Green,
    Glove,
    Sleeve,
}

/// A block (size, center, tilt about X) or a cylinder along Z (radius, length, center).
enum Shape {
    Block(Vec3, Vec3, f32),
    Tube(f32, f32, Vec3),
}

use Finish::*;
use Shape::*;

fn b(size: [f32; 3], at: [f32; 3], tilt: f32, f: Finish) -> (Shape, Finish) {
    (Block(Vec3::from(size), Vec3::from(at), tilt), f)
}

fn t(radius: f32, len: f32, at: [f32; 3], f: Finish) -> (Shape, Finish) {
    (Tube(radius, len, Vec3::from(at)), f)
}

/// A fat olive tube with rings at both ends, two grips and a sight.
fn launcher() -> Vec<(Shape, Finish)> {
    vec![
        t(0.055, 0.95, [0.0, 0.03, -0.12], Green),
        t(0.066, 0.06, [0.0, 0.03, -0.58], Black),
        t(0.066, 0.06, [0.0, 0.03, 0.33], Black),
        b([0.03, 0.11, 0.05], [0.0, -0.07, 0.0], 0.3, Black),
        b([0.03, 0.09, 0.04], [0.0, -0.06, -0.26], 0.0, Black),
        b([0.02, 0.06, 0.09], [0.055, 0.1, -0.12], 0.0, Black),
    ]
}

/// Gloved hands on both grips, forearms running back towards the bottom corners of the screen.
fn hands() -> Vec<(Shape, Finish)> {
    let grip = [0.0, -0.075, 0.075];
    vec![
        b([0.06, 0.07, 0.09], grip, 0.3, Glove),
        b([0.06, 0.06, 0.3], [grip[0] + 0.03, grip[1] - 0.08, grip[2] + 0.18], 0.45, Sleeve),
        b([0.06, 0.06, 0.1], [-0.02, -0.05, -0.25], -0.2, Glove),
        b([0.06, 0.06, 0.36], [-0.11, -0.14, -0.06], 0.4, Sleeve),
    ]
}

/// Where the barrel tip is, ahead of the grip.
pub const BARREL: f32 = 0.61;
/// First-person position of the grip, relative to the camera.
pub const VIEWMODEL: Vec3 = Vec3::new(0.23, -0.21, -0.26);

/// A launcher under `parent`; first person also gets hands holding it, their sleeves in `sleeve`.
pub fn spawn_launcher(scene: &mut Scene, parent: u32, at: Transform, sleeve: Option<u32>) -> u32 {
    let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let root = scene.empty(Some(parent), at, true);
    let mut shapes = launcher();
    if sleeve.is_some() {
        shapes.extend(hands());
    }
    for (shape, finish) in shapes {
        let material = match (finish, sleeve) {
            (Sleeve, Some(m)) => m,
            _ => scene.shared_material(format!("finish {}", finish as u8), || {
                let m = |r, g, b, roughness| Material { color: Color::srgb(r, g, b), roughness, ..Default::default() };
                match finish {
                    Black => m(0.07, 0.07, 0.08, 0.5),
                    Green => m(0.2, 0.27, 0.17, 0.7),
                    Glove | Sleeve => m(0.09, 0.09, 0.08, 0.9),
                }
            }),
        };
        let (mesh, transform) = match shape {
            Block(size, pos, tilt) => (scene.cuboid(size), Transform::at(pos).rotated(Quat::from_rotation_x(tilt))),
            Tube(r, len, pos) => {
                let mesh = scene.shared_mesh(format!("tube {r} {len}"), || MeshData::Cylinder { radius: r, height: len });
                (mesh, Transform::at(pos).rotated(along_z))
            }
        };
        let kind = NodeKind::Mesh { mesh, material, shadows: sleeve.is_none() };
        scene.spawn(Some(root), kind, transform, true);
    }
    root
}

/// Plates in the team's color: the torso, shoulders and arms, so a team reads from across the pitch. The
/// viewmodel's sleeves use it too.
pub fn plate_material(team: Team) -> Material {
    let c = team_rgb(team);
    Material { color: Color::srgb(c[0] * 0.8, c[1] * 0.8, c[2] * 0.8), roughness: 0.45, metallic: 0.3, ..Default::default() }
}

/// The team color, glowing: stripes and visors.
fn team_glow(scene: &mut Scene, team: Team) -> u32 {
    let c = team_rgb(team);
    scene.shared_material(format!("glow {team:?}"), || Material { color: rgb(c), emissive: c.map(|v| v * 5.0), unlit: true, ..Default::default() })
}

/// Moving parts of a remote player.
pub struct Rig {
    pub root: u32,
    pub arms: u32,
    pub legs: [u32; 2],
    pub team: Team,
    pub stride: f32,
    pub last_pos: Vec3,
}

/// An armored figure with plates in the team color, a glowing visor and stripe, a rocket launcher on the right shoulder.
pub fn spawn_rig(scene: &mut Scene, team: Team) -> Rig {
    let armor = scene.shared_material("armor".into(), || Material { color: Color::srgb(0.13, 0.13, 0.15), roughness: 0.4, metallic: 0.6, ..Default::default() });
    let plate = scene.shared_material(format!("plate {team:?}"), || plate_material(team));
    let glow = team_glow(scene, team);
    let root = scene.empty(None, Transform::default(), false);
    let torso = scene.cuboid(Vec3::new(0.56, 0.62, 0.32));
    scene.mesh_node(Some(root), torso, plate, Transform::at(Vec3::Y * 1.12));
    let stripe = scene.cuboid(Vec3::new(0.58, 0.08, 0.34));
    scene.mesh_node(Some(root), stripe, glow, Transform::at(Vec3::Y * 1.2));
    let shoulders = scene.cuboid(Vec3::new(0.66, 0.14, 0.3));
    scene.mesh_node(Some(root), shoulders, plate, Transform::at(Vec3::Y * 1.4));
    let head = scene.shared_mesh("head".into(), || MeshData::Sphere { radius: 0.19, ico: None });
    scene.mesh_node(Some(root), head, armor, Transform::at(Vec3::Y * 1.62));
    let visor = scene.cuboid(Vec3::new(0.3, 0.07, 0.1));
    scene.mesh_node(Some(root), visor, glow, Transform::at(Vec3::new(0.0, 1.64, -0.15)));
    let leg = scene.cuboid(Vec3::new(0.18, 0.8, 0.2));
    let legs = [-0.13, 0.13].map(|x| {
        let hip = scene.empty(Some(root), Transform::at(Vec3::new(x, 0.8, 0.0)), true);
        scene.mesh_node(Some(hip), leg, armor, Transform::at(Vec3::Y * -0.4));
        hip
    });
    let arms = scene.empty(Some(root), Transform::at(Vec3::Y * 1.35), true);
    let arm = scene.cuboid(Vec3::new(0.13, 0.13, 0.4));
    scene.mesh_node(Some(arms), arm, plate, Transform::at(Vec3::new(0.3, -0.12, -0.15)));
    scene.mesh_node(Some(arms), arm, plate, Transform::at(Vec3::new(-0.15, -0.14, -0.3)).rotated(Quat::from_rotation_y(-0.6)));
    // On the shoulder, sticking out ahead.
    spawn_launcher(scene, arms, Transform::at(Vec3::new(0.24, 0.0, -0.3)).scaled(1.3), None);
    Rig { root, arms, legs, team, stride: 0.0, last_pos: Vec3::ZERO }
}
