//! The arena as the host should draw it: dusk sky, paneled metal floor, walls and platforms with glowing edges, and
//! jump pads that light up their surroundings.

use crate::map::{CELL, MAP};
use glam::{Vec2, Vec3};
use sdk::client::{Color, Command, Environment, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

pub fn build(scene: &mut Scene) {
    scene.push(Command::Environment(Environment {
        sky: Color::srgb(0.16, 0.12, 0.28),
        ambient: Color::srgb(0.6, 0.65, 1.0),
        ambient_brightness: 260.0,
        sun_dir: Vec3::new(0.45, -0.6, -0.65),
        sun_color: Color::srgb(1.0, 0.62, 0.42),
        sun_illuminance: 7000.0,
        fog_color: Color::srgb(0.2, 0.15, 0.3),
        fog_sun_color: Color::srgba(1.0, 0.5, 0.35, 0.5),
        fog_sun_exponent: 20.0,
        fog_start: 60.0,
        fog_end: 400.0,
    }));

    let floor_tex = texture(scene, panels(Color::srgb(0.32, 0.33, 0.36), 4));
    let wall_tex = texture(scene, panels(Color::srgb(0.22, 0.23, 0.27), 2));
    let block_tex = texture(scene, panels(Color::srgb(0.42, 0.4, 0.38), 4));
    let textured = |tex: u32| Material { texture: Some(tex), roughness: 0.6, metallic: 0.3, ..Default::default() };
    let (w, d) = MAP.size();
    let floor = scene.material(Material { uv_scale: w / 4.0, ..textured(floor_tex) });
    let plane = scene.mesh(MeshData::Plane(Vec2::new(w, d)));
    scene.mesh_node(None, plane, floor, Transform::default());

    let (wall, block) = (scene.material(textured(wall_tex)), scene.material(textured(block_tex)));
    let trim = scene.material(Material { color: Color::srgb(1.0, 0.45, 0.1), emissive: [6.0, 1.8, 0.3], unlit: true, ..Default::default() });
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
    let pad = scene.material(Material { color: Color::srgb(0.3, 0.9, 1.0), emissive: [1.5, 6.0, 8.0], unlit: true, ..Default::default() });
    for &p in &MAP.pads {
        scene.spawn(None, NodeKind::Mesh { mesh: pad_mesh, material: pad, shadows: false }, Transform::at(p + Vec3::Y * 0.03), true);
        let light = NodeKind::PointLight { color: Color::srgb(0.3, 0.85, 1.0), intensity: 300_000.0, range: 12.0 };
        scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
    }
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
    let mut level: Vec<[f32; 3]> = (0..SIZE * SIZE)
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
    let (mut size, mut mips, mut rgba) = (SIZE, 0, vec![]);
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
    Texture { size: SIZE, mips, rgba }
}

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}
