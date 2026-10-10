//! The course as the host should draw it: a sunset over a sea of slowly flowing lava with basalt spires standing in
//! it, glass walls, platforms colored by height with glowing edges, green gates over the checkpoints, and a checkered
//! finish under a gold one.

use crate::map::{CELL, WALL_TOP, course};
use crate::shared::{LASER_HEIGHT, LASER_RADIUS, LASER_REACH};
use glam::{Quat, Vec2, Vec3};
use sdk::client::{Blend, Color, Command, Environment, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

/// Sky colors, and the haze the far lava fades into.
const ZENITH: [f32; 3] = [0.2, 0.24, 0.55];
const HORIZON: [f32; 3] = [1.0, 0.58, 0.36];
/// Light travels this way: the sun is low in the north, ahead of the runners.
const SUN_DIR: Vec3 = Vec3::new(-0.3, -0.38, 0.87);
/// A color per meter of platform height, so levels read apart.
const TIERS: [[f32; 3]; 6] = [[1.0, 0.68, 0.22], [0.25, 0.82, 0.72], [0.45, 0.58, 1.0], [1.0, 0.42, 0.48], [0.62, 0.88, 0.28], [0.78, 0.48, 1.0]];
/// The lava plane's size and texture repeats across it, and how fast it flows (m/s).
const LAVA_SIZE: f32 = 1200.0;
const LAVA_REPEATS: f32 = 40.0;
const LAVA_FLOW: Vec2 = Vec2::new(0.25, 0.4);

/// Builds the course; returns the lava plane's node, which `lava_at` moves.
pub fn build(scene: &mut Scene) -> u32 {
    let map = course();
    let srgb = |c: [f32; 3]| Color::srgb(c[0], c[1], c[2]);
    scene.push(Command::Environment(Environment {
        sky: srgb(HORIZON),
        ambient: Color::srgb(0.85, 0.75, 0.9),
        ambient_brightness: 500.0,
        sun_dir: SUN_DIR.normalize(),
        sun_color: Color::srgb(1.0, 0.82, 0.62),
        sun_illuminance: 10000.0,
        fog_color: srgb(HORIZON),
        fog_sun_color: Color::srgba(1.0, 0.75, 0.45, 0.6),
        fog_sun_exponent: 12.0,
        fog_start: 60.0,
        fog_end: 420.0,
    }));
    sky(scene);

    let lava_tex = texture(scene, lava());
    let lava = scene.material(Material { texture: Some(lava_tex), uv_scale: LAVA_REPEATS, unlit: true, ..Default::default() });
    let plane = scene.mesh(MeshData::Plane(Vec2::splat(LAVA_SIZE)));
    let lava = scene.spawn(None, NodeKind::Mesh { mesh: plane, material: lava, shadows: false }, Transform::default(), true);
    spires(scene);

    let block_tex = texture(scene, panels(Color::srgb(0.92, 0.92, 0.92), 4));
    let glass = scene.material(Material {
        color: Color::srgba(0.7, 0.9, 1.0, 0.12),
        roughness: 0.05,
        blend: Blend::Blend,
        double_sided: true,
        ..Default::default()
    });
    let post = scene.material(Material { color: Color::srgb(0.16, 0.17, 0.2), metallic: 0.8, roughness: 0.35, ..Default::default() });
    let rail = scene.material(Material { color: Color::srgb(0.6, 0.95, 1.0), emissive: [1.0, 4.0, 5.0], unlit: true, ..Default::default() });
    for &(c, h) in &map.boxes {
        if h.y * 2.0 >= WALL_TOP {
            let pane = scene.cuboid(h * 2.0);
            scene.spawn(None, NodeKind::Mesh { mesh: pane, material: glass, shadows: false }, Transform::at(c), true);
            // Posts every few cells along the wall, and a glowing rail along its top.
            let along = if h.x > h.z { Vec3::X } else { Vec3::Z };
            let (len, mesh) = (along.dot(h), scene.cuboid(Vec3::new(0.3, h.y * 2.0, 0.3)));
            for i in 0..=(len * 2.0 / (CELL * 4.0)) as i32 {
                scene.mesh_node(None, mesh, post, Transform::at(c - along * len + along * (i as f32 * CELL * 4.0)));
            }
            let mesh = scene.cuboid(Vec3::splat(0.12) + along * (len * 2.0));
            scene.spawn(None, NodeKind::Mesh { mesh, material: rail, shadows: false }, Transform::at(c + Vec3::Y * h.y), true);
            continue;
        }
        let top = h.y * 2.0;
        let tier = TIERS[top as usize % TIERS.len()];
        let block = scene.shared_material(format!("block {tier:?}"), || Material {
            color: srgb(tier),
            texture: Some(block_tex),
            roughness: 0.7,
            metallic: 0.1,
            ..Default::default()
        });
        let trim = scene.shared_material(format!("trim {tier:?}"), || Material {
            color: srgb(tier),
            emissive: tier.map(|v| v * 4.0),
            unlit: true,
            ..Default::default()
        });
        let mesh = scene.mesh(textured_box(c, h, 4.0));
        scene.mesh_node(None, mesh, block, Transform::default());
        // Glowing strips along the top edges, so ledges read from across the course.
        let top = c + Vec3::Y * (h.y - 0.04);
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
    for &p in &map.pads {
        scene.spawn(None, NodeKind::Mesh { mesh: pad_mesh, material: pad, shadows: false }, Transform::at(p + Vec3::Y * 0.03), true);
        let light = NodeKind::PointLight { color: Color::srgb(1.0, 0.4, 0.9), intensity: 300_000.0, range: 12.0 };
        scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
    }

    // Checkpoint strips and the checkered finish, a tile on top of each cell.
    let tile = scene.cuboid(Vec3::new(CELL, 0.02, CELL));
    let checkpoint = scene.material(Material { color: Color::srgb(0.2, 1.0, 0.4), emissive: [0.3, 2.0, 0.6], ..Default::default() });
    let squares = [Color::srgb(0.95, 0.95, 0.95), Color::srgb(0.05, 0.05, 0.05)].map(|c| scene.color(c));
    for (x, z) in (0..map.depth()).flat_map(|z| (0..map.width()).map(move |x| (x, z))) {
        let material = match (map.checkpoint(x, z), map.is_finish(x, z)) {
            (Some(_), _) => checkpoint,
            (_, true) => squares[(x + z) % 2],
            _ => continue,
        };
        scene.spawn(None, NodeKind::Mesh { mesh: tile, material, shadows: false }, Transform::at(map.ground(x, z) + Vec3::Y * 0.01), true);
    }
    // A gate over where each checkpoint and the finish begin.
    let green = scene.material(Material { color: Color::srgb(0.3, 1.0, 0.5), emissive: [0.6, 5.0, 1.2], unlit: true, ..Default::default() });
    let gold = scene.material(Material { color: Color::srgb(1.0, 0.8, 0.2), emissive: [5.0, 3.2, 0.5], unlit: true, ..Default::default() });
    for z in 1..map.depth() - 1 {
        let starts = |f: &dyn Fn(usize) -> bool| f(z) && !f(z + 1);
        let material = if starts(&|z| map.checkpoint(1, z).is_some()) {
            green
        } else if starts(&|z| map.is_finish(1, z)) {
            gold
        } else {
            continue;
        };
        gate(scene, map.ground(map.width() / 2, z) + Vec3::Z * (CELL / 2.0), map.size().0 - CELL * 2.0, material);
    }
    lava
}

/// Where the lava plane sits `time` seconds in: drifting, and jumping back a whole texture repeat so it never strays.
pub fn lava_at(time: f64) -> Transform {
    let repeat = (LAVA_SIZE / LAVA_REPEATS) as f64;
    let at = |speed: f32| ((time * speed as f64) % repeat) as f32;
    Transform::at(Vec3::new(at(LAVA_FLOW.x), 0.0, at(LAVA_FLOW.y)))
}

/// Two glowing posts `width` apart standing on `at`, and a bar across their tops.
fn gate(scene: &mut Scene, at: Vec3, width: f32, material: u32) {
    const HEIGHT: f32 = 4.5;
    let post = scene.cuboid(Vec3::new(0.25, HEIGHT, 0.25));
    for side in [-1.0, 1.0] {
        scene.spawn(None, NodeKind::Mesh { mesh: post, material, shadows: false }, Transform::at(at + Vec3::new(side * width / 2.0, HEIGHT / 2.0, 0.0)), true);
    }
    let bar = scene.cuboid(Vec3::new(width + 0.25, 0.35, 0.25));
    scene.spawn(None, NodeKind::Mesh { mesh: bar, material, shadows: false }, Transform::at(at + Vec3::Y * HEIGHT), true);
}

/// A dome from the horizon glow up to the dusk overhead, and the sun low in it.
fn sky(scene: &mut Scene) {
    const R: f32 = 800.0;
    const RINGS: u32 = 16;
    const SEGMENTS: u32 = 32;
    let (mut positions, mut normals, mut uvs, mut colors, mut indices) = (vec![], vec![], vec![], vec![], vec![]);
    for ring in 0..=RINGS {
        // From a little below the horizon up to straight overhead.
        let up = -0.1 + 1.1 * ring as f32 / RINGS as f32;
        let k = up.max(0.0).powf(0.6);
        // Vertex colors are linear.
        let color = [0, 1, 2].map(|i| (HORIZON[i] + (ZENITH[i] - HORIZON[i]) * k).powf(2.2));
        let (y, r) = ((up * std::f32::consts::FRAC_PI_2).sin(), (up * std::f32::consts::FRAC_PI_2).cos());
        for seg in 0..=SEGMENTS {
            let a = seg as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let n = Vec3::new(a.cos() * r, y, a.sin() * r);
            positions.push((n * R).to_array());
            normals.push((-n).to_array());
            uvs.push([0.0, 0.0]);
            colors.push([color[0], color[1], color[2], 1.0]);
        }
    }
    for ring in 0..RINGS {
        for seg in 0..SEGMENTS {
            let (a, b) = (ring * (SEGMENTS + 1) + seg, (ring + 1) * (SEGMENTS + 1) + seg);
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    let dome = scene.mesh(MeshData::Custom { positions, normals, uvs, colors, indices });
    let sky = scene.material(Material { unlit: true, fog: false, double_sided: true, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh: dome, material: sky, shadows: false }, Transform::default(), true);
    let sun = scene.mesh(MeshData::Sphere { radius: 34.0, ico: Some(3) });
    let glow = scene.material(Material { color: Color::srgb(1.0, 0.78, 0.35), emissive: [6.0, 4.5, 2.5], unlit: true, fog: false, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh: sun, material: glow, shadows: false }, Transform::at(-SUN_DIR.normalize() * (R - 60.0)), true);
}

/// Basalt spires standing in the lava either side of the course, glowing where they meet it.
fn spires(scene: &mut Scene) {
    let (w, d) = course().size();
    let rock_tex = texture(scene, rock());
    let rock = scene.material(Material { color: Color::srgb(0.32, 0.27, 0.26), texture: Some(rock_tex), roughness: 0.95, ..Default::default() });
    for i in 0..90u32 {
        let r = |k: u32| hash(i, k);
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        let x = side * (w / 2.0 + 14.0 + r(1).powi(2) * 160.0);
        let z = (r(2) - 0.5) * (d + 240.0);
        let near = 1.0 - (x.abs() - w / 2.0) / 170.0;
        let half = Vec3::new(1.0 + r(3) * 3.0, 1.5 + r(4) * r(4) * 14.0 * (0.5 + near), 1.0 + r(5) * 3.0);
        let rot = Quat::from_rotation_y(r(6) * 3.0) * Quat::from_rotation_z((r(7) - 0.5) * 0.25);
        let root = scene.empty(None, Transform::at(Vec3::new(x, -0.5, z)).rotated(rot), true);
        // Three blocks, each narrower than the one below, for a tapering spire.
        for k in 0..3 {
            let narrow = 1.0 - k as f32 * 0.28;
            let piece = Vec3::new(half.x * narrow, half.y / 3.0, half.z * narrow);
            let mesh = scene.mesh(textured_box(Vec3::Y * (piece.y * (2 * k + 1) as f32), piece, 6.0));
            scene.mesh_node(Some(root), mesh, rock, Transform::default());
        }
    }
}
/// The laser sweepers: a glowing hub on each pivot and the bar spinning about it. Returns the bars' nodes, which
/// `Transform`s from `laser` place.
pub fn lasers(scene: &mut Scene) -> Vec<u32> {
    let hub = scene.mesh(MeshData::Sphere { radius: 0.3, ico: Some(2) });
    let bar = scene.mesh(MeshData::Cylinder { radius: LASER_RADIUS, height: LASER_REACH * 2.0 });
    let red = scene.material(Material { color: Color::srgb(1.0, 0.1, 0.1), emissive: [12.0, 0.5, 0.4], unlit: true, ..Default::default() });
    course().sweepers
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
    Transform::at(course().sweepers[i] + Vec3::Y * LASER_HEIGHT).rotated(glam::Quat::from_rotation_arc(Vec3::Y, dir))
}

/// A box with its texture mapped in world space, `tile` meters per repeat, so neighboring blocks line up. Its sides
/// darken to a hot red towards the lava at y = 0.
fn textured_box(center: Vec3, half: Vec3, tile: f32) -> MeshData {
    const HEAT: [f32; 4] = [1.0, 0.5, 0.32, 1.0];
    // (normal, u axis, v axis) with u x v = normal, so corners in order are counter-clockwise from outside.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
    ];
    let (mut positions, mut normals, mut uvs, mut colors, mut indices) = (vec![], vec![], vec![], vec![], vec![]);
    for (n, u, v) in faces {
        let base = positions.len() as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = center + n * n.dot(half).abs() + u * su * u.dot(half).abs() + v * sv * v.dot(half).abs();
            // Image rows run downwards, world up is +Y.
            let flip = if v == Vec3::Y { -1.0 } else { 1.0 };
            positions.push(p.to_array());
            normals.push(n.to_array());
            uvs.push([p.dot(u) / tile, flip * p.dot(v) / tile]);
            let k = (p.y / 1.5).clamp(0.0, 1.0);
            colors.push(HEAT.map(|c| c + (1.0 - c) * k));
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData::Custom { positions, normals, uvs, colors, indices }
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

/// Value noise over a `size` texture, `cells` across, that wraps at its edge.
fn noise(size: u32, x: u32, y: u32, cells: u32) -> f32 {
    let s = size / cells;
    let (cx, cy, fx, fy) = (x / s, y / s, (x % s) as f32 / s as f32, (y % s) as f32 / s as f32);
    let h = |i: u32, j: u32| hash(i % cells + cells * 7, j % cells + cells * 3);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = h(cx, cy) + (h(cx + 1, cy) - h(cx, cy)) * sx;
    let bottom = h(cx, cy + 1) + (h(cx + 1, cy + 1) - h(cx, cy + 1)) * sx;
    top + (bottom - top) * sy
}

/// Molten rock: dark crust broken by glowing cracks, tiling.
fn lava() -> Texture {
    const SIZE: u32 = 512;
    let noise = |x, y, cells| noise(SIZE, x, y, cells);
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let n = noise(x, y, 4) * 0.4 + noise(x, y, 8) * 0.25 + noise(x, y, 16) * 0.15 + noise(x, y, 64) * 0.13 + noise(x, y, 128) * 0.07;
            // Hottest where the noise crosses the middle: thin bright veins.
            let heat = (1.0 - (n - 0.5).abs() * 6.0).clamp(0.0, 1.0).powf(1.5);
            [0.35 + 0.65 * heat, 0.05 + 0.6 * heat * heat, 0.02 + 0.2 * heat.powi(4)]
        })
        .collect();
    mipmapped(SIZE, level)
}

/// Weathered basalt: mottled grey with darker cracks.
fn rock() -> Texture {
    const SIZE: u32 = 128;
    let noise = |x, y, cells| noise(SIZE, x, y, cells);
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let n = noise(x, y, 8) * 0.6 + noise(x, y, 32) * 0.4;
            let crack = if (noise(x, y, 16) - 0.5).abs() < 0.03 { 0.45 } else { 1.0 };
            let k = (0.65 + 0.5 * n) * crack;
            [k, k * 0.95, k * 0.92]
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
