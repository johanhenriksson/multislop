//! The course as the host should draw it: a bright sky over a sea of lava, platforms with glowing edges, green
//! checkpoint strips, a checkered finish under a banner, and jump pads that light up their surroundings.

use crate::map::{CELL, MAP};
use crate::shared::{LASER_HEIGHT, LASER_RADIUS, LASER_REACH};
use glam::{Vec2, Vec3};
use sdk::client::{Color, Command, Environment, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

pub fn build(scene: &mut Scene) {
    scene.push(Command::Environment(Environment {
        sky: Color::srgb(0.45, 0.65, 0.95),
        ambient: Color::srgb(0.8, 0.85, 1.0),
        ambient_brightness: 400.0,
        sun_dir: Vec3::new(0.35, -0.75, 0.55),
        sun_color: Color::srgb(1.0, 0.95, 0.85),
        sun_illuminance: 12000.0,
        fog_color: Color::srgb(0.6, 0.55, 0.6),
        fog_sun_color: Color::srgba(1.0, 0.6, 0.3, 0.4),
        fog_sun_exponent: 20.0,
        fog_start: 80.0,
        fog_end: 500.0,
    }));

    let wall_tex = texture(scene, panels(Color::srgb(0.3, 0.32, 0.38), 2));
    let block_tex = texture(scene, panels(Color::srgb(0.75, 0.72, 0.68), 4));
    let textured = |tex: u32| Material { texture: Some(tex), roughness: 0.6, metallic: 0.3, ..Default::default() };
    // The lava reaches well past the walls so the course seems to float on it.
    let (w, d) = MAP.size();
    let lava_tex = texture(scene, lava());
    let lava = scene.material(Material { texture: Some(lava_tex), uv_scale: 40.0, unlit: true, ..Default::default() });
    let plane = scene.mesh(MeshData::Plane(Vec2::new(w + 400.0, d + 400.0)));
    scene.spawn(None, NodeKind::Mesh { mesh: plane, material: lava, shadows: false }, Transform::default(), true);

    let (wall, block) = (scene.material(textured(wall_tex)), scene.material(textured(block_tex)));
    let trim = scene.material(Material { color: Color::srgb(0.2, 0.9, 1.0), emissive: [0.5, 4.0, 6.0], unlit: true, ..Default::default() });
    for &(c, h) in &MAP.boxes {
        let is_wall = h.y * 2.0 >= crate::map::WALL_TOP;
        let mesh = scene.mesh(textured_box(c, h, 4.0));
        scene.mesh_node(None, mesh, if is_wall { wall } else { block }, Transform::default());
        // Glowing strips along the top edges, so ledges read from across the arena.
        let top = c + Vec3::Y * (h.y - if is_wall { 0.6 } else { 0.04 });
        for (offset, size) in [
            (Vec3::Z * h.z, Vec3::new(h.x + 0.03, 0.04, 0.05)),
            (Vec3::NEG_Z * h.z, Vec3::new(h.x + 0.03, 0.04, 0.05)),
            (Vec3::X * h.x, Vec3::new(0.05, 0.04, h.z + 0.03)),
            (Vec3::NEG_X * h.x, Vec3::new(0.05, 0.04, h.z + 0.03)),
        ] {
            let mesh = scene.cuboid(size * 2.0);
            scene.spawn(None, NodeKind::Mesh { mesh, material: trim, shadows: false }, Transform::at(top + offset), true);
        }
    }

    let pad_mesh = scene.mesh(MeshData::Cylinder { radius: CELL * 0.4, height: 0.06 });
    let pad = scene.material(Material { color: Color::srgb(1.0, 0.3, 0.9), emissive: [6.0, 1.0, 5.0], unlit: true, ..Default::default() });
    for &p in &MAP.pads {
        scene.spawn(None, NodeKind::Mesh { mesh: pad_mesh, material: pad, shadows: false }, Transform::at(p + Vec3::Y * 0.03), true);
        let light = NodeKind::PointLight { color: Color::srgb(1.0, 0.4, 0.9), intensity: 300_000.0, range: 12.0 };
        scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
    }

    // Checkpoint strips and the checkered finish, a tile on top of each cell.
    let tile = scene.cuboid(Vec3::new(CELL, 0.02, CELL));
    let checkpoint = scene.material(Material { color: Color::srgb(0.2, 1.0, 0.4), emissive: [0.3, 2.0, 0.6], ..Default::default() });
    let squares = [Color::srgb(0.95, 0.95, 0.95), Color::srgb(0.05, 0.05, 0.05)].map(|c| scene.color(c));
    for (x, z) in (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z))) {
        let material = match (MAP.checkpoint(x, z), MAP.is_finish(x, z)) {
            (Some(_), _) => checkpoint,
            (_, true) => squares[(x + z) % 2],
            _ => continue,
        };
        scene.spawn(None, NodeKind::Mesh { mesh: tile, material, shadows: false }, Transform::at(MAP.ground(x, z) + Vec3::Y * 0.01), true);
    }
    // A banner over the finish line, from wall to wall.
    let line = (0..MAP.depth()).rev().find(|&z| MAP.is_finish(1, z)).unwrap();
    let at = MAP.ground(MAP.width() / 2, line) + Vec3::new(0.0, 5.0, CELL / 2.0);
    let banner = scene.cuboid(Vec3::new(w, 1.2, 0.3));
    let gold = scene.material(Material { color: Color::srgb(1.0, 0.8, 0.2), emissive: [3.0, 2.0, 0.3], ..Default::default() });
    scene.mesh_node(None, banner, gold, Transform::at(at));
}

/// The laser sweepers: a glowing hub on each pivot and the bar spinning about it. Returns the bars' nodes, which
/// `Transform`s from `laser` place.
pub fn lasers(scene: &mut Scene) -> Vec<u32> {
    let hub = scene.mesh(MeshData::Sphere { radius: 0.3, ico: Some(2) });
    let bar = scene.mesh(MeshData::Cylinder { radius: LASER_RADIUS, height: LASER_REACH * 2.0 });
    let red = scene.material(Material { color: Color::srgb(1.0, 0.1, 0.1), emissive: [12.0, 0.5, 0.4], unlit: true, ..Default::default() });
    MAP.sweepers
        .iter()
        .map(|&p| {
            scene.spawn(None, NodeKind::Mesh { mesh: hub, material: red, shadows: false }, Transform::at(p + Vec3::Y * LASER_HEIGHT), true);
            let light = NodeKind::PointLight { color: Color::srgb(1.0, 0.15, 0.1), intensity: 200_000.0, range: 10.0 };
            scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
            scene.spawn(None, NodeKind::Mesh { mesh: bar, material: red, shadows: false }, Transform::default(), true)
        })
        .collect()
}

/// Where sweeper `i`'s bar is drawn `time` seconds into the race.
pub fn laser(i: usize, time: f32) -> Transform {
    let dir = crate::shared::laser_dir(i, time);
    Transform::at(MAP.sweepers[i] + Vec3::Y * LASER_HEIGHT).rotated(glam::Quat::from_rotation_arc(Vec3::Y, dir))
}

/// A box with its texture mapped in world space, `tile` meters per repeat, so neighboring blocks line up.
fn textured_box(center: Vec3, half: Vec3, tile: f32) -> MeshData {
    // (normal, u axis, v axis) with u x v = normal, so corners in order are counter-clockwise from outside.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
    ];
    let (mut positions, mut normals, mut uvs, mut indices) = (vec![], vec![], vec![], vec![]);
    for (n, u, v) in faces {
        let base = positions.len() as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = center + n * n.dot(half).abs() + u * su * u.dot(half).abs() + v * sv * v.dot(half).abs();
            // Image rows run downwards, world up is +Y.
            let flip = if v == Vec3::Y { -1.0 } else { 1.0 };
            positions.push(p.to_array());
            normals.push(n.to_array());
            uvs.push([p.dot(u) / tile, flip * p.dot(v) / tile]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData::Custom { positions, normals, uvs, colors: vec![], indices }
}

/// RGBA8 sRGB pixels, every mip level after the first.
struct Texture {
    size: u32,
    mips: u32,
    rgba: Vec<u8>,
}

fn texture(scene: &mut Scene, t: Texture) -> u32 {
    let id = scene.id();
    scene.push(Command::Texture { id, w: t.size, h: t.size, mips: t.mips, rgba: t.rgba });
    id
}

/// Square metal panels, `per_side` of them across the texture: grime, beveled seams and a rivet in each corner.
fn panels(base: Color, per_side: u32) -> Texture {
    const SIZE: u32 = 256;
    let panel = SIZE / per_side;
    let [r, g, b, _] = base.0;
    let level: Vec<[f32; 3]> = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let (px, py) = (x % panel, y % panel);
            let grime = 0.92 + 0.08 * hash(x / 4, y / 4) + 0.06 * hash(x / 32 + 100, y / 32);
            // Each panel a slightly different shade.
            let shade = 0.9 + 0.2 * hash(x / panel + 7, y / panel + 3);
            let edge = px.min(py).min(panel - 1 - px).min(panel - 1 - py);
            let seam = match edge {
                0 => 0.35,
                1 => 1.25,
                _ => 1.0,
            };
            let rivet = [6, panel - 7].contains(&px) && [6, panel - 7].contains(&py);
            let k = grime * shade * seam * if rivet { 1.4 } else { 1.0 };
            [r * k, g * k, b * k]
        })
        .collect();
    mipmapped(SIZE, level)
}

/// A texture from its full-size level of sRGB colors (0 to 1), with every smaller level box-filtered from it.
fn mipmapped(full: u32, mut level: Vec<[f32; 3]>) -> Texture {
    let (mut size, mut mips, mut rgba) = (full, 0, vec![]);
    loop {
        rgba.extend(level.iter().flat_map(|c| [c[0], c[1], c[2], 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)));
        mips += 1;
        if size == 1 {
            break;
        }
        // Box-filter the next level.
        let half = size / 2;
        level = (0..half * half)
            .map(|i| {
                let (x, y) = (i % half * 2, i / half * 2);
                let p = |dx: u32, dy: u32| level[((y + dy) * size + x + dx) as usize];
                let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
                [0, 1, 2].map(|k| (a[k] + b[k] + c[k] + d[k]) / 4.0)
            })
            .collect();
        size = half;
    }
    Texture { size: full, mips, rgba }
}

/// Molten rock: dark crust broken by glowing cracks, tiling.
fn lava() -> Texture {
    const SIZE: u32 = 128;
    // Value noise that wraps at the texture's edge.
    let noise = |x: u32, y: u32, cells: u32| {
        let s = SIZE / cells;
        let (cx, cy, fx, fy) = (x / s, y / s, (x % s) as f32 / s as f32, (y % s) as f32 / s as f32);
        let h = |i: u32, j: u32| hash(i % cells + cells * 7, j % cells + cells * 3);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let top = h(cx, cy) + (h(cx + 1, cy) - h(cx, cy)) * sx;
        let bottom = h(cx, cy + 1) + (h(cx + 1, cy + 1) - h(cx, cy + 1)) * sx;
        top + (bottom - top) * sy
    };
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let n = noise(x, y, 4) * 0.6 + noise(x, y, 16) * 0.3 + noise(x, y, 32) * 0.1;
            // Hottest where the noise crosses the middle: thin bright veins.
            let heat = (1.0 - (n - 0.5).abs() * 6.0).clamp(0.0, 1.0).powf(1.5);
            [0.35 + 0.65 * heat, 0.05 + 0.6 * heat * heat, 0.02 + 0.2 * heat.powi(4)]
        })
        .collect();
    mipmapped(SIZE, level)
}

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}
