//! Player and weapon models built from primitives, so the game ships as a single binary with no asset files.
//! Everything fits inside the hit capsule (radius 0.4, height 1.8). Models face -Z with the origin at the feet.

use super::scene::Scene;
use crate::shared::{RADIUS, Team};
use crate::weapons::{ALL_WEAPONS, Weapon};
use glam::{Quat, Vec3};
use sdk::client::{Color, Material, MeshData, Transform};

/// Moving parts of a player model (scene node ids): legs swing with speed, arms and rifle follow the aim pitch.
pub struct Rig {
    pub root: u32,
    pub legs: [u32; 2],
    pub arms: u32,
    /// One model per weapon, indexed by `Weapon as usize`; the held one is shown.
    pub guns: Vec<u32>,
    /// Walk cycle phase in radians, advanced by distance moved.
    pub phase: f32,
    pub last_pos: Option<Vec3>,
    /// Distance run since the last footstep.
    pub stride: f32,
    /// When (client clock) we saw this player die, for the fall.
    pub died_at: Option<f64>,
    /// The gun shown, as last sent.
    pub gun: Option<usize>,
}

impl Rig {
    /// Advances the walk cycle and returns the leg swing angle and the distance moved this frame.
    pub fn animate(&mut self, pos: Vec3, dt: f32) -> (f32, f32) {
        let moved = self.last_pos.map_or(0.0, |p| (pos - p).with_y(0.0).length());
        self.last_pos = Some(pos);
        self.phase += moved * 2.6;
        let speed = if dt > 0.0 { moved / dt } else { 0.0 };
        let amount = (speed / 6.35).min(1.0) * 0.6;
        (self.phase.sin() * amount, moved)
    }
}

struct Palette {
    shirt: Color,
    pants: Color,
    vest: Color,
    head: Color,
    skin: Color,
    helmet: Option<Color>,
}

fn palette(team: Team) -> Palette {
    match team {
        // Desert gear with a balaclava.
        Team::T => Palette {
            shirt: Color::srgb(0.72, 0.58, 0.36),
            pants: Color::srgb(0.42, 0.33, 0.22),
            vest: Color::srgb(0.33, 0.3, 0.22),
            head: Color::srgb(0.12, 0.12, 0.12),
            skin: Color::srgb(0.78, 0.58, 0.42),
            helmet: None,
        },
        // Navy uniform, vest and helmet.
        Team::Ct => Palette {
            shirt: Color::srgb(0.2, 0.27, 0.45),
            pants: Color::srgb(0.17, 0.2, 0.3),
            vest: Color::srgb(0.12, 0.14, 0.2),
            head: Color::srgb(0.82, 0.62, 0.48),
            skin: Color::srgb(0.82, 0.62, 0.48),
            helmet: Some(Color::srgb(0.1, 0.12, 0.16)),
        },
    }
}

pub fn spawn_soldier(scene: &mut Scene, team: Team, root: Transform) -> Rig {
    let pal = palette(team);
    let dark = Color::srgb(0.08, 0.08, 0.08);
    let gear = Color::srgb(0.22, 0.2, 0.16);
    let body = scene.empty(None, root, true);
    // A box: size, center, and yaw (radians) so forearms can angle in towards the gun.
    let spawn = |scene: &mut Scene, parent: u32, size: [f32; 3], at: [f32; 3], yaw: f32, color: Color| {
        let t = Transform::at(Vec3::from(at)).rotated(Quat::from_rotation_y(yaw));
        let (mesh, material) = (scene.cuboid(Vec3::from(size)), scene.color(color));
        scene.mesh_node(Some(parent), mesh, material, t);
    };

    // Legs swing from the hips: thigh to shin in one, knee pad, boot.
    let legs = [-0.11f32, 0.11].map(|x| {
        let hip = scene.empty(Some(body), Transform::at(Vec3::new(x, 0.92, 0.0)), true);
        spawn(scene, hip, [0.19, 0.84, 0.21], [0.0, -0.42, 0.0], 0.0, pal.pants);
        spawn(scene, hip, [0.2, 0.11, 0.06], [0.0, -0.46, -0.11], 0.0, gear);
        spawn(scene, hip, [0.21, 0.15, 0.3], [0.0, -0.85, -0.04], 0.0, dark);
        hip
    });

    let body_parts = [
        // Hips and belt.
        ([0.42, 0.16, 0.24], [0.0, 0.96, 0.0], pal.pants),
        ([0.44, 0.05, 0.26], [0.0, 1.02, 0.0], dark),
        // Torso, broad shoulders, vest with pouches.
        ([0.44, 0.48, 0.26], [0.0, 1.27, 0.0], pal.shirt),
        ([0.56, 0.13, 0.26], [0.0, 1.47, 0.0], pal.shirt),
        ([0.47, 0.36, 0.31], [0.0, 1.27, 0.0], pal.vest),
        ([0.1, 0.1, 0.05], [-0.13, 1.17, -0.17], gear),
        ([0.1, 0.1, 0.05], [0.0, 1.17, -0.17], gear),
        ([0.1, 0.1, 0.05], [0.13, 1.17, -0.17], gear),
        // Neck and head.
        ([0.12, 0.08, 0.12], [0.0, 1.56, 0.0], pal.skin),
        ([0.24, 0.27, 0.26], [0.0, 1.7, 0.0], pal.head),
    ];
    let extra = match pal.helmet {
        // CT: helmet with a brim, goggles strap.
        Some(c) => vec![([0.29, 0.12, 0.31], [0.0, 1.85, 0.0], c), ([0.25, 0.04, 0.02], [0.0, 1.76, -0.135], dark), ([0.3, 0.12, 0.12], [0.0, 1.32, 0.2], gear)],
        // T: balaclava eye slit, and a backpack.
        None => vec![([0.2, 0.05, 0.02], [0.0, 1.73, -0.131], pal.skin), ([0.32, 0.36, 0.14], [0.0, 1.3, 0.21], gear)],
    };
    for (size, at, color) in body_parts.into_iter().chain(extra) {
        spawn(scene, body, size, at, 0.0, color);
    }

    // Arms aim with the view pitch, pivoting at the chest: upper arms from the shoulders, forearms angled in to the
    // gun, gloved hands.
    let arms = scene.empty(Some(body), Transform::at(Vec3::new(0.0, 1.42, 0.0)), true);
    spawn(scene, arms, [0.12, 0.12, 0.28], [0.27, -0.06, -0.1], 0.15, pal.shirt);
    spawn(scene, arms, [0.11, 0.11, 0.3], [0.18, -0.07, -0.36], 0.45, pal.shirt);
    spawn(scene, arms, [0.1, 0.1, 0.1], [0.1, -0.06, -0.5], 0.0, dark);
    spawn(scene, arms, [0.12, 0.12, 0.28], [-0.27, -0.06, -0.1], -0.15, pal.shirt);
    spawn(scene, arms, [0.11, 0.11, 0.34], [-0.15, -0.05, -0.4], -0.6, pal.shirt);
    spawn(scene, arms, [0.1, 0.1, 0.1], [-0.03, -0.04, -0.58], 0.0, dark);
    let guns: Vec<u32> = ALL_WEAPONS
        .iter()
        .map(|&w| {
            let at = match w {
                Weapon::Knife => Vec3::new(0.1, -0.06, -0.55),
                Weapon::Glock | Weapon::Usp | Weapon::Deagle => Vec3::new(0.04, 0.0, -0.58),
                _ => Vec3::new(0.08, 0.0, -0.45),
            };
            spawn_weapon(scene, arms, w, Transform::at(at), false)
        })
        .collect();
    debug_assert!(0.56 / 2.0 < RADIUS);
    Rig { root: body, legs, arms, guns, phase: 0.0, last_pos: None, stride: 0.0, died_at: None, gun: None }
}

#[derive(Clone, Copy)]
enum Finish {
    Black,
    Polymer,
    Wood,
    Green,
    Silver,
    Steel,
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

/// Parts per weapon, in meters, grip at the origin and the barrel along -Z.
fn parts(weapon: Weapon) -> Vec<(Shape, Finish)> {
    match weapon {
        Weapon::Knife => vec![
            b([0.012, 0.035, 0.17], [0.0, 0.0, -0.12], 0.0, Steel),
            b([0.012, 0.022, 0.05], [0.0, 0.006, -0.225], 0.0, Steel),
            b([0.05, 0.012, 0.015], [0.0, 0.0, -0.04], 0.0, Black),
            b([0.025, 0.035, 0.1], [0.0, 0.0, 0.02], 0.0, Black),
        ],
        Weapon::Glock => vec![
            b([0.03, 0.035, 0.19], [0.0, 0.03, -0.05], 0.0, Black),
            b([0.028, 0.02, 0.17], [0.0, 0.005, -0.05], 0.0, Polymer),
            b([0.03, 0.11, 0.05], [0.0, -0.05, 0.02], 0.3, Polymer),
            b([0.008, 0.025, 0.045], [0.0, -0.015, -0.03], 0.0, Polymer),
        ],
        Weapon::Usp => vec![
            b([0.032, 0.038, 0.2], [0.0, 0.03, -0.06], 0.0, Black),
            b([0.03, 0.025, 0.18], [0.0, 0.003, -0.06], 0.0, Polymer),
            b([0.032, 0.11, 0.05], [0.0, -0.05, 0.02], 0.25, Polymer),
            b([0.008, 0.025, 0.045], [0.0, -0.015, -0.04], 0.0, Polymer),
            t(0.017, 0.15, [0.0, 0.03, -0.235], Black),
        ],
        Weapon::Deagle => vec![
            b([0.036, 0.05, 0.27], [0.0, 0.035, -0.09], 0.0, Silver),
            b([0.014, 0.012, 0.26], [0.0, 0.066, -0.09], 0.0, Silver),
            b([0.034, 0.03, 0.2], [0.0, 0.0, -0.05], 0.0, Black),
            b([0.036, 0.125, 0.056], [0.0, -0.065, 0.03], 0.25, Black),
            b([0.01, 0.03, 0.05], [0.0, -0.02, -0.04], 0.0, Black),
        ],
        Weapon::Mp5 => vec![
            b([0.045, 0.065, 0.28], [0.0, 0.0, -0.04], 0.0, Black),
            b([0.056, 0.06, 0.12], [0.0, -0.01, -0.22], 0.0, Polymer),
            t(0.012, 0.07, [0.0, 0.01, -0.31], Black),
            b([0.025, 0.15, 0.04], [0.0, -0.1, -0.11], -0.35, Black),
            b([0.03, 0.09, 0.045], [0.0, -0.08, 0.06], 0.3, Polymer),
            t(0.016, 0.03, [0.0, 0.05, 0.06], Black),
            b([0.02, 0.035, 0.02], [0.0, 0.045, -0.25], 0.0, Black),
            b([0.008, 0.008, 0.2], [0.022, 0.0, 0.2], 0.0, Steel),
            b([0.008, 0.008, 0.2], [-0.022, 0.0, 0.2], 0.0, Steel),
            b([0.06, 0.085, 0.015], [0.0, -0.01, 0.3], 0.0, Black),
        ],
        Weapon::Ak47 => vec![
            b([0.05, 0.08, 0.32], [0.0, 0.0, -0.02], 0.0, Black),
            b([0.045, 0.02, 0.3], [0.0, 0.05, -0.02], 0.0, Black),
            t(0.012, 0.42, [0.0, 0.015, -0.36], Black),
            t(0.01, 0.22, [0.0, 0.048, -0.28], Black),
            b([0.06, 0.055, 0.2], [0.0, 0.0, -0.27], 0.0, Wood),
            b([0.046, 0.026, 0.15], [0.0, 0.048, -0.27], 0.0, Wood),
            b([0.012, 0.05, 0.015], [0.0, 0.045, -0.52], 0.0, Black),
            t(0.017, 0.045, [0.0, 0.015, -0.585], Black),
            b([0.035, 0.11, 0.07], [0.0, -0.09, -0.1], -0.2, Black),
            b([0.035, 0.1, 0.065], [0.0, -0.175, -0.06], -0.6, Black),
            b([0.035, 0.1, 0.045], [0.0, -0.075, 0.07], 0.3, Wood),
            b([0.045, 0.075, 0.26], [0.0, -0.035, 0.26], 0.12, Wood),
            b([0.046, 0.13, 0.03], [0.0, -0.06, 0.39], 0.0, Wood),
        ],
        Weapon::M4a1 => vec![
            b([0.05, 0.07, 0.28], [0.0, 0.0, 0.0], 0.0, Black),
            b([0.045, 0.05, 0.18], [0.0, -0.05, 0.02], 0.0, Black),
            b([0.02, 0.03, 0.17], [0.0, 0.075, 0.0], 0.0, Black),
            b([0.02, 0.03, 0.02], [0.0, 0.05, -0.07], 0.0, Black),
            b([0.02, 0.03, 0.02], [0.0, 0.05, 0.07], 0.0, Black),
            t(0.028, 0.2, [0.0, 0.0, -0.24], Polymer),
            b([0.015, 0.07, 0.02], [0.0, 0.045, -0.35], 0.0, Black),
            t(0.01, 0.08, [0.0, 0.0, -0.38], Black),
            t(0.021, 0.22, [0.0, 0.0, -0.52], Black),
            b([0.03, 0.13, 0.06], [0.0, -0.12, -0.05], -0.12, Black),
            b([0.03, 0.09, 0.04], [0.0, -0.09, 0.09], 0.35, Polymer),
            t(0.016, 0.14, [0.0, 0.0, 0.2], Black),
            b([0.04, 0.085, 0.13], [0.0, -0.02, 0.31], 0.0, Polymer),
        ],
        Weapon::Awp => vec![
            b([0.06, 0.09, 0.5], [0.0, -0.015, 0.06], 0.0, Green),
            b([0.05, 0.035, 0.18], [0.0, 0.04, 0.14], 0.0, Green),
            b([0.062, 0.15, 0.045], [0.0, -0.04, 0.33], 0.0, Green),
            b([0.045, 0.06, 0.24], [0.0, 0.035, -0.08], 0.0, Black),
            t(0.014, 0.6, [0.0, 0.035, -0.5], Black),
            t(0.023, 0.07, [0.0, 0.035, -0.82], Black),
            t(0.022, 0.32, [0.0, 0.115, -0.08], Black),
            t(0.034, 0.08, [0.0, 0.115, -0.27], Black),
            t(0.029, 0.06, [0.0, 0.115, 0.1], Black),
            b([0.016, 0.045, 0.022], [0.0, 0.075, -0.16], 0.0, Black),
            b([0.016, 0.045, 0.022], [0.0, 0.075, 0.0], 0.0, Black),
            b([0.07, 0.013, 0.013], [0.045, 0.035, 0.03], 0.0, Steel),
            b([0.025, 0.025, 0.025], [0.08, 0.035, 0.03], 0.0, Steel),
            b([0.04, 0.07, 0.08], [0.0, -0.08, -0.05], 0.0, Black),
            b([0.012, 0.03, 0.05], [0.0, -0.075, 0.03], 0.0, Black),
        ],
        // Olive body, steel spoon and pin ring on top.
        Weapon::He => vec![
            t(0.03, 0.085, [0.0, 0.0, -0.04], Green),
            b([0.012, 0.03, 0.06], [0.0, 0.03, -0.03], 0.0, Steel),
            t(0.012, 0.005, [0.02, 0.035, -0.06], Steel),
        ],
        // Rocket launcher: a fat olive tube with rings at both ends, two grips and a sight.
        Weapon::Rocket => vec![
            t(0.055, 0.95, [0.0, 0.03, -0.12], Green),
            t(0.066, 0.06, [0.0, 0.03, -0.58], Black),
            t(0.066, 0.06, [0.0, 0.03, 0.33], Black),
            b([0.03, 0.11, 0.05], [0.0, -0.07, 0.0], 0.3, Black),
            b([0.03, 0.09, 0.04], [0.0, -0.06, -0.26], 0.0, Black),
            b([0.02, 0.06, 0.09], [0.055, 0.1, -0.12], 0.0, Black),
        ],
    }
}

/// Gloved hands and forearms for the first-person view: the shooting hand on the grip, plus a support hand under
/// the barrel for two-handed weapons. Forearms run back towards the bottom corners of the screen.
fn hands(weapon: Weapon) -> Vec<(Shape, Finish)> {
    let grip = match weapon {
        Weapon::Knife | Weapon::He => [0.0, 0.0, 0.03],
        Weapon::Glock | Weapon::Usp | Weapon::Deagle => [0.0, -0.05, 0.03],
        _ => [0.0, -0.075, 0.075],
    };
    let mut parts = vec![
        b([0.06, 0.07, 0.09], grip, 0.3, Glove),
        b([0.06, 0.06, 0.3], [grip[0] + 0.03, grip[1] - 0.08, grip[2] + 0.18], 0.45, Sleeve),
    ];
    let support = match weapon {
        Weapon::Ak47 => Some(-0.27),
        Weapon::M4a1 => Some(-0.22),
        Weapon::Mp5 => Some(-0.21),
        Weapon::Awp => Some(-0.3),
        Weapon::Rocket => Some(-0.26),
        Weapon::Glock | Weapon::Usp | Weapon::Deagle => Some(0.0),
        Weapon::Knife | Weapon::He => None,
    };
    if let Some(z) = support {
        let y = if z == 0.0 { -0.06 } else { -0.05 };
        parts.push(b([0.06, 0.06, 0.1], [-0.02, y, z + 0.01], -0.2, Glove));
        parts.push(b([0.06, 0.06, 0.36], [-0.11, y - 0.09, z + 0.2], 0.4, Sleeve));
    }
    parts
}

/// Where the barrel tip is, in meters ahead of the weapon origin.
pub fn barrel_length(weapon: Weapon) -> f32 {
    match weapon {
        Weapon::Knife => 0.25,
        Weapon::Glock => 0.15,
        Weapon::Usp => 0.31,
        Weapon::Deagle => 0.23,
        Weapon::Mp5 => 0.35,
        Weapon::Ak47 => 0.61,
        Weapon::M4a1 => 0.63,
        Weapon::Awp => 0.86,
        Weapon::He => 0.1,
        Weapon::Rocket => 0.61,
    }
}

/// First-person position of each weapon's origin, relative to the camera.
pub fn viewmodel_offset(weapon: Weapon) -> Vec3 {
    match weapon {
        Weapon::Knife | Weapon::He => Vec3::new(0.22, -0.22, -0.34),
        Weapon::Glock | Weapon::Usp | Weapon::Deagle => Vec3::new(0.19, -0.19, -0.4),
        Weapon::Awp => Vec3::new(0.21, -0.21, -0.12),
        _ => Vec3::new(0.23, -0.21, -0.26),
    }
}

/// A weapon model (hidden at first) under `parent`; first person also gets gloved hands holding it.
pub fn spawn_weapon(scene: &mut Scene, parent: u32, weapon: Weapon, at: Transform, first_person: bool) -> u32 {
    let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let root = scene.empty(Some(parent), at, false);
    let mut shapes = parts(weapon);
    if first_person {
        shapes.extend(hands(weapon));
    }
    for (shape, finish) in shapes {
        let material = scene.shared_material(format!("finish {}", finish as u8), || {
            let m = |r, g, b, roughness| Material { color: Color::srgb(r, g, b), roughness, ..Default::default() };
            match finish {
                Black => m(0.07, 0.07, 0.08, 0.5),
                Polymer => m(0.14, 0.14, 0.14, 0.8),
                Wood => m(0.45, 0.24, 0.1, 0.6),
                Green => m(0.2, 0.27, 0.17, 0.7),
                Silver => Material { metallic: 0.8, ..m(0.72, 0.72, 0.74, 0.35) },
                Steel => Material { metallic: 0.9, ..m(0.78, 0.8, 0.83, 0.25) },
                Glove => m(0.09, 0.09, 0.08, 0.9),
                Sleeve => m(0.2, 0.22, 0.16, 0.95),
            }
        });
        let (mesh, transform) = match shape {
            Block(size, pos, tilt) => (scene.cuboid(size), Transform::at(pos).rotated(Quat::from_rotation_x(tilt))),
            Tube(r, len, pos) => {
                let mesh = scene.shared_mesh(format!("tube {r} {len}"), || MeshData::Cylinder { radius: r, height: len });
                (mesh, Transform::at(pos).rotated(along_z))
            }
        };
        scene.mesh_node(Some(root), mesh, material, transform);
    }
    root
}
