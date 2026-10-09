//! Player and weapon models built from primitives, so the game ships as a single binary with no asset files.
//! Everything fits inside the hit capsule (radius 0.4, height 1.8). Models face -Z with the origin at the feet.

use crate::shared::{RADIUS, Team};
use crate::weapons::{ALL_WEAPONS, Weapon};
use bevy::prelude::*;

/// Moving parts of a player model: legs swing with speed, arms and rifle follow the aim pitch.
#[derive(Component)]
pub struct Rig {
    pub legs: [Entity; 2],
    pub arms: Entity,
    /// One model per weapon, indexed by `Weapon as usize`; the held one is shown.
    pub guns: Vec<Entity>,
    /// Walk cycle phase in radians, advanced by distance moved.
    pub phase: f32,
    pub last_pos: Option<Vec3>,
    /// Distance run since the last footstep.
    pub stride: f32,
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

pub fn spawn_soldier(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    team: Team,
    root: impl Bundle,
) -> Entity {
    let pal = palette(team);
    let mut part = |size: Vec3, color: Color, at: Vec3| {
        (Mesh3d(meshes.add(Cuboid::from_size(size))), MeshMaterial3d(mats.add(color)), Transform::from_translation(at))
    };
    let boots = Color::srgb(0.08, 0.08, 0.08);
    let legs = [-0.11, 0.11].map(|x| {
        let pants = part(Vec3::new(0.17, 0.78, 0.2), pal.pants, Vec3::new(0.0, -0.39, 0.0));
        let boot = part(Vec3::new(0.19, 0.14, 0.28), boots, Vec3::new(0.0, -0.83, -0.04));
        commands.spawn((Transform::from_xyz(x, 0.9, 0.0), Visibility::Inherited, children![pants, boot])).id()
    });
    let torso = part(Vec3::new(0.44, 0.62, 0.26), pal.shirt, Vec3::new(0.0, 1.2, 0.0));
    let vest = part(Vec3::new(0.47, 0.4, 0.31), pal.vest, Vec3::new(0.0, 1.27, 0.0));
    let head = part(Vec3::new(0.24, 0.26, 0.26), pal.head, Vec3::new(0.0, 1.66, 0.0));
    // A strip of skin around the eyes for the balaclava, or a helmet for CT.
    let top = match pal.helmet {
        Some(c) => part(Vec3::new(0.29, 0.11, 0.31), c, Vec3::new(0.0, 1.8, 0.0)),
        None => part(Vec3::new(0.2, 0.05, 0.02), pal.skin, Vec3::new(0.0, 1.69, -0.131)),
    };
    let right_arm = part(Vec3::new(0.1, 0.1, 0.42), pal.shirt, Vec3::new(0.21, -0.05, -0.18));
    let left_arm = part(Vec3::new(0.1, 0.1, 0.5), pal.shirt, Vec3::new(-0.11, -0.05, -0.3));
    let hand = part(Vec3::new(0.09, 0.09, 0.09), pal.skin, Vec3::new(0.0, -0.04, -0.52));
    let arms = commands.spawn((Transform::from_xyz(0.0, 1.45, 0.0), Visibility::Inherited, children![right_arm, left_arm, hand])).id();
    let guns: Vec<Entity> = ALL_WEAPONS
        .iter()
        .map(|&w| {
            let at = match w {
                Weapon::Knife => Vec3::new(0.21, -0.05, -0.42),
                Weapon::Glock | Weapon::Usp | Weapon::Deagle => Vec3::new(0.0, 0.0, -0.5),
                _ => Vec3::new(0.06, 0.0, -0.22),
            };
            spawn_weapon(commands, meshes, mats, w, Transform::from_translation(at), false)
        })
        .collect();
    commands.entity(arms).add_children(&guns);

    let rig = Rig { legs, arms, guns, phase: 0.0, last_pos: None, stride: 0.0 };
    let body = commands.spawn((root, rig, Visibility::Inherited, children![torso, vest, head, top])).id();
    commands.entity(body).add_children(&legs).add_child(arms);
    debug_assert!(0.47 / 2.0 < RADIUS);
    body
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
    }
}

/// Gloved hands and forearms for the first-person view: the shooting hand on the grip, plus a support hand under
/// the barrel for two-handed weapons. Forearms run back towards the bottom corners of the screen.
fn hands(weapon: Weapon) -> Vec<(Shape, Finish)> {
    let grip = match weapon {
        Weapon::Knife => [0.0, 0.0, 0.03],
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
        Weapon::Glock | Weapon::Usp | Weapon::Deagle => Some(0.0),
        Weapon::Knife => None,
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
    }
}

/// First-person position of each weapon's origin, relative to the camera.
pub fn viewmodel_offset(weapon: Weapon) -> Vec3 {
    match weapon {
        Weapon::Knife => Vec3::new(0.22, -0.22, -0.34),
        Weapon::Glock | Weapon::Usp | Weapon::Deagle => Vec3::new(0.19, -0.19, -0.4),
        Weapon::Awp => Vec3::new(0.21, -0.21, -0.12),
        _ => Vec3::new(0.23, -0.21, -0.26),
    }
}

/// A weapon model; first person also gets gloved hands holding it.
pub fn spawn_weapon(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    weapon: Weapon,
    at: Transform,
    first_person: bool,
) -> Entity {
    let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let root = commands.spawn((at, Visibility::Hidden)).id();
    let mut shapes = parts(weapon);
    if first_person {
        shapes.extend(hands(weapon));
    }
    for (shape, finish) in shapes {
        let material = mats.add(match finish {
            Black => StandardMaterial { base_color: Color::srgb(0.07, 0.07, 0.08), perceptual_roughness: 0.5, ..default() },
            Polymer => StandardMaterial { base_color: Color::srgb(0.14, 0.14, 0.14), perceptual_roughness: 0.8, ..default() },
            Wood => StandardMaterial { base_color: Color::srgb(0.45, 0.24, 0.1), perceptual_roughness: 0.6, ..default() },
            Green => StandardMaterial { base_color: Color::srgb(0.2, 0.27, 0.17), perceptual_roughness: 0.7, ..default() },
            Silver => StandardMaterial { base_color: Color::srgb(0.72, 0.72, 0.74), metallic: 0.8, perceptual_roughness: 0.35, ..default() },
            Steel => StandardMaterial { base_color: Color::srgb(0.78, 0.8, 0.83), metallic: 0.9, perceptual_roughness: 0.25, ..default() },
            Glove => StandardMaterial { base_color: Color::srgb(0.09, 0.09, 0.08), perceptual_roughness: 0.9, ..default() },
            Sleeve => StandardMaterial { base_color: Color::srgb(0.2, 0.22, 0.16), perceptual_roughness: 0.95, ..default() },
        });
        let (mesh, transform) = match shape {
            Block(size, pos, tilt) => (Mesh::from(Cuboid::from_size(size)), Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(tilt))),
            Tube(r, len, pos) => (Mesh::from(Cylinder::new(r, len)), Transform::from_translation(pos).with_rotation(along_z)),
        };
        let part = commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), transform)).id();
        commands.entity(root).add_child(part);
    }
    root
}
