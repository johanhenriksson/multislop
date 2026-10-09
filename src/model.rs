//! Player and weapon models built from primitives, so the game ships as a single binary with no asset files.
//! Everything fits inside the hit capsule (radius 0.4, height 1.8). Models face -Z with the origin at the feet.

use crate::shared::{RADIUS, Team};
use bevy::prelude::*;

/// Moving parts of a player model: legs swing with speed, arms and rifle follow the aim pitch.
#[derive(Component)]
pub struct Rig {
    pub legs: [Entity; 2],
    pub arms: Entity,
    /// Walk cycle phase in radians, advanced by distance moved.
    pub phase: f32,
    pub last_pos: Option<Vec3>,
}

impl Rig {
    /// Advances the walk cycle and returns the leg swing angle.
    pub fn animate(&mut self, pos: Vec3, dt: f32) -> f32 {
        let moved = self.last_pos.map_or(0.0, |p| (pos - p).with_y(0.0).length());
        self.last_pos = Some(pos);
        self.phase += moved * 2.6;
        let speed = if dt > 0.0 { moved / dt } else { 0.0 };
        let amount = (speed / 6.35).min(1.0) * 0.6;
        self.phase.sin() * amount
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
    let rifle = spawn_rifle(commands, meshes, mats, Transform::from_xyz(0.06, 0.0, -0.3));
    commands.entity(arms).add_child(rifle);

    let rig = Rig { legs, arms, phase: 0.0, last_pos: None };
    let body = commands.spawn((root, rig, Visibility::Inherited, children![torso, vest, head, top])).id();
    commands.entity(body).add_children(&legs).add_child(arms);
    debug_assert!(0.47 / 2.0 < RADIUS);
    body
}

/// An AK-ish rifle with its grip at the origin, pointing -Z. The barrel tip is 0.55 m ahead of the origin.
pub fn spawn_rifle(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    at: Transform,
) -> Entity {
    let metal = mats.add(Color::srgb(0.1, 0.1, 0.11));
    let wood = mats.add(Color::srgb(0.42, 0.25, 0.12));
    let mut part = |mesh: Mesh, mat: &Handle<StandardMaterial>, t: Transform| {
        (Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat.clone()), t)
    };
    let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    commands
        .spawn((
            at,
            Visibility::Inherited,
            children![
                part(Cuboid::new(0.06, 0.1, 0.4).into(), &metal, Transform::default()),
                part(Cylinder::new(0.016, 0.34).into(), &metal, Transform::from_xyz(0.0, 0.02, -0.38).with_rotation(along_z)),
                part(Cuboid::new(0.07, 0.075, 0.2).into(), &wood, Transform::from_xyz(0.0, 0.0, -0.27)),
                part(Cuboid::new(0.045, 0.17, 0.08).into(), &metal, Transform::from_xyz(0.0, -0.12, -0.08).with_rotation(Quat::from_rotation_x(-0.25))),
                part(Cuboid::new(0.05, 0.09, 0.24).into(), &wood, Transform::from_xyz(0.0, -0.03, 0.31)),
                part(Cuboid::new(0.04, 0.1, 0.05).into(), &wood, Transform::from_xyz(0.0, -0.09, 0.12)),
            ],
        ))
        .id()
}
