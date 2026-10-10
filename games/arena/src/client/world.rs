//! The map as the host should draw it: sky, paneled metal floor, walls and platforms with glowing edges, and jump
//! pads that light up their surroundings, each map in its own colors.

use crate::map::{self, CELL, MAP};
use glam::{Vec2, Vec3};
use sdk::client::{Color, Command, Environment, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

/// How a map is dressed: its light and the colors of its surfaces.
struct Look {
    environment: Environment,
    floor: Color,
    wall: Color,
    block: Color,
    /// Glowing strips along the top edges of walls and of blocks.
    wall_trim: (Color, [f32; 3]),
    block_trim: (Color, [f32; 3]),
    pad: (Color, [f32; 3]),
    /// Molten glow over every floor cell lower than the spawns around it, with lights above.
    pit: Option<(Color, [f32; 3])>,
}

fn look(map: &str) -> Look {
    match map {
        // A smoky steelworks: rusted plate, soot-black walls, hazard-yellow edges and a crucible of molten metal.
        "foundry" => Look {
            environment: Environment {
                sky: Color::srgb(0.13, 0.09, 0.07),
                ambient: Color::srgb(1.0, 0.75, 0.55),
                ambient_brightness: 240.0,
                sun_dir: Vec3::new(-0.3, -0.85, 0.4),
                sun_color: Color::srgb(1.0, 0.8, 0.6),
                sun_illuminance: 5000.0,
                fog_color: Color::srgb(0.22, 0.15, 0.1),
                fog_sun_color: Color::srgba(1.0, 0.55, 0.2, 0.4),
                fog_sun_exponent: 12.0,
                fog_start: 30.0,
                fog_end: 220.0,
            },
            floor: Color::srgb(0.34, 0.29, 0.25),
            wall: Color::srgb(0.17, 0.16, 0.15),
            block: Color::srgb(0.44, 0.32, 0.24),
            wall_trim: (Color::srgb(1.0, 0.75, 0.1), [6.0, 4.0, 0.3]),
            block_trim: (Color::srgb(1.0, 0.75, 0.1), [6.0, 4.0, 0.3]),
            pad: (Color::srgb(0.5, 1.0, 0.3), [2.5, 8.0, 1.5]),
            pit: Some((Color::srgb(1.0, 0.4, 0.05), [5.0, 1.4, 0.1])),
        },
        // Night on a rooftop: blue-black plate under moonlight, magenta walls, cyan ledges, gold pads.
        "spire" => Look {
            environment: Environment {
                sky: Color::srgb(0.03, 0.04, 0.11),
                ambient: Color::srgb(0.45, 0.55, 1.0),
                ambient_brightness: 420.0,
                sun_dir: Vec3::new(0.3, -0.8, 0.5),
                sun_color: Color::srgb(0.65, 0.75, 1.0),
                sun_illuminance: 4500.0,
                fog_color: Color::srgb(0.05, 0.06, 0.15),
                fog_sun_color: Color::srgba(0.5, 0.6, 1.0, 0.3),
                fog_sun_exponent: 30.0,
                fog_start: 50.0,
                fog_end: 320.0,
            },
            floor: Color::srgb(0.27, 0.29, 0.36),
            wall: Color::srgb(0.15, 0.16, 0.22),
            block: Color::srgb(0.34, 0.36, 0.44),
            wall_trim: (Color::srgb(1.0, 0.2, 0.8), [6.0, 0.8, 5.0]),
            block_trim: (Color::srgb(0.2, 0.9, 1.0), [0.8, 5.0, 7.0]),
            pad: (Color::srgb(1.0, 0.8, 0.2), [8.0, 5.0, 1.0]),
            pit: None,
        },
        // Dusk: purple sky, warm low sun, grey metal with orange edges and cyan pads.
        _ => Look {
            environment: Environment {
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
            },
            floor: Color::srgb(0.32, 0.33, 0.36),
            wall: Color::srgb(0.22, 0.23, 0.27),
            block: Color::srgb(0.42, 0.4, 0.38),
            wall_trim: (Color::srgb(1.0, 0.45, 0.1), [6.0, 1.8, 0.3]),
            block_trim: (Color::srgb(1.0, 0.45, 0.1), [6.0, 1.8, 0.3]),
            pad: (Color::srgb(0.3, 0.9, 1.0), [1.5, 6.0, 8.0]),
            pit: None,
        },
    }
}

pub fn build(scene: &mut Scene) {
    let look = look(map::name());
    scene.push(Command::Environment(look.environment));

    let floor_tex = texture(scene, panels(look.floor, 4));
    let wall_tex = texture(scene, panels(look.wall, 2));
    let block_tex = texture(scene, panels(look.block, 4));
    let textured = |tex: u32| Material { texture: Some(tex), roughness: 0.6, metallic: 0.3, ..Default::default() };
    let (w, d) = MAP.size();
    let floor = scene.material(Material { uv_scale: w / 4.0, ..textured(floor_tex) });
    let plane = scene.mesh(MeshData::Plane(Vec2::new(w, d)));
    scene.mesh_node(None, plane, floor, Transform::default());

    let glowing = |(color, emissive): (Color, [f32; 3])| Material { color, emissive, unlit: true, ..Default::default() };
    let (wall, block) = (scene.material(textured(wall_tex)), scene.material(textured(block_tex)));
    let (wall_trim, block_trim) = (scene.material(glowing(look.wall_trim)), scene.material(glowing(look.block_trim)));
    for &(c, h) in &MAP.boxes {
        let is_wall = h.y * 2.0 >= map::WALL_TOP;
        let mesh = scene.mesh(textured_box(c, h, 4.0));
        scene.mesh_node(None, mesh, if is_wall { wall } else { block }, Transform::default());
    }
    // Glowing strips along the top edges, so ledges read from across the arena.
    for (center, size, is_wall) in ledges() {
        let mesh = scene.cuboid(size);
        let material = if is_wall { wall_trim } else { block_trim };
        scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::at(center), true);
    }

    if let Some(glow) = look.pit {
        pit(scene, glow);
    }

    let pad_mesh = scene.mesh(MeshData::Cylinder { radius: CELL * 0.4, height: 0.06 });
    let pad = scene.material(glowing(look.pad));
    for &p in &MAP.pads {
        scene.spawn(None, NodeKind::Mesh { mesh: pad_mesh, material: pad, shadows: false }, Transform::at(p + Vec3::Y * 0.03), true);
        let light = NodeKind::PointLight { color: look.pad.0, intensity: 300_000.0, range: 12.0 };
        scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
    }
}

/// Strips (center, size, on a wall) along every top edge with lower ground beside it, one per straight run, and none
/// where a block meets another as tall: a raised floor made of many blocks reads as one.
fn ledges() -> Vec<(Vec3, Vec3, bool)> {
    let (w, d) = (MAP.width(), MAP.depth());
    let mut done = std::collections::HashSet::new();
    let mut strips = vec![];
    for z in 0..d {
        for x in 0..w {
            let top = MAP.top(x, z);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                // A ledge here: lower ground on that side, inside the map.
                let ledge = |x: usize, z: usize| {
                    let n = (x.checked_add_signed(dx), z.checked_add_signed(dz));
                    MAP.top(x, z) == top && matches!(n, (Some(nx), Some(nz)) if nx < w && nz < d && MAP.top(nx, nz) < top)
                };
                if top <= 0.0 || done.contains(&(x, z, dx, dz)) || !ledge(x, z) {
                    continue;
                }
                // Extend along the edge while it stays a ledge at this height.
                let (ax, az) = (dz.unsigned_abs(), dx.unsigned_abs());
                let mut n = 1;
                while x + ax * n < w && z + az * n < d && ledge(x + ax * n, z + az * n) {
                    n += 1;
                }
                for i in 0..n {
                    done.insert((x + ax * i, z + az * i, dx, dz));
                }
                let is_wall = MAP.is_wall(x, z);
                let (a, b) = (MAP.center(x, z), MAP.center(x + ax * (n - 1), z + az * (n - 1)));
                let out = Vec3::new(dx as f32, 0.0, dz as f32);
                let y = top - if is_wall { 0.6 } else { 0.04 };
                let along = Vec3::new(ax as f32, 0.0, az as f32) * ((b - a).length() + CELL + 0.06);
                let size = along + out.abs() * 0.1 + Vec3::Y * 0.08;
                strips.push(((a + b) / 2.0 + out * CELL / 2.0 + Vec3::Y * y, size, is_wall));
            }
        }
    }
    strips
}

/// Molten metal over the open floor at the bottom of the map, lighting the walls of the hole it lies in.
// ponytail: one glowing quad over the bounding box of the lowest cells; fine while each map has a single pit.
fn pit(scene: &mut Scene, (color, emissive): (Color, [f32; 3])) {
    let cells: Vec<(usize, usize)> = (0..MAP.depth())
        .flat_map(|z| (0..MAP.width()).map(move |x| (x, z)))
        .filter(|&(x, z)| !MAP.is_wall(x, z) && MAP.height(x, z) == 0.0)
        .collect();
    let (Some(lo), Some(hi)) = (cells.iter().map(|&(x, z)| MAP.center(x, z)).reduce(Vec3::min), cells.iter().map(|&(x, z)| MAP.center(x, z)).reduce(Vec3::max)) else {
        return;
    };
    let size = hi - lo + Vec3::splat(CELL);
    let mesh = scene.mesh(MeshData::Plane(Vec2::new(size.x, size.z)));
    let material = scene.material(Material { color, emissive, unlit: true, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::at((lo + hi) / 2.0 + Vec3::Y * 0.01), true);
    for corner in [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 1.0)] {
        let light = NodeKind::PointLight { color, intensity: 600_000.0, range: 18.0 };
        scene.spawn(None, light, Transform::at((lo + hi) / 2.0 + corner * size / 3.0 + Vec3::Y * 1.0), true);
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
