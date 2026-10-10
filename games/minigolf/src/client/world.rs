//! The course as the host should draw it: every hole floating in the sky over a sea of clouds, felt on top, rails
//! around, earth (or wafer, or dark metal) underneath, water in its basins, a flag over each cup and the windmills.
//! Three looks, picked from the course's seed: meadow, candy and neon.

use crate::map::{BASIN_DROP, Cell, HUB_WIDTH, Hole, WINDMILL_HEIGHT, WINDMILL_REACH, course};
use crate::shared::{CUP_RADIUS, windmill};
use glam::{Quat, Vec2, Vec3};
use sdk::client::{Blend, Color, Command, Environment, Material, MeshData, NodeKind, Transform};
use sdk::scene::Scene;

/// Colors are sRGB.
struct Look {
    felt: [f32; 3],
    green: [f32; 3],
    tee: [f32; 3],
    rail: [f32; 3],
    /// Rails glow this color (linear), or not at all.
    glow: Option<[f32; 3]>,
    block: [f32; 3],
    earth: [f32; 3],
    water: Color,
    flag: [f32; 3],
    zenith: [f32; 3],
    horizon: [f32; 3],
    clouds: [f32; 3],
    sun: [f32; 3],
    sun_illuminance: f32,
    ambient: f32,
}

const LOOKS: [Look; 3] = [
    // Meadow: green felt, wooden rails, a blue summer sky.
    Look {
        felt: [0.24, 0.62, 0.22],
        green: [0.38, 0.78, 0.3],
        tee: [0.16, 0.44, 0.15],
        rail: [0.6, 0.4, 0.22],
        glow: None,
        block: [0.62, 0.6, 0.56],
        earth: [0.45, 0.32, 0.2],
        water: Color::srgba(0.2, 0.5, 0.85, 0.7),
        flag: [0.95, 0.15, 0.12],
        zenith: [0.25, 0.5, 0.95],
        horizon: [0.75, 0.88, 1.0],
        clouds: [1.0, 1.0, 1.0],
        sun: [1.0, 0.96, 0.88],
        sun_illuminance: 12000.0,
        ambient: 700.0,
    },
    // Candy: pink felt and a mint green, white rails, wafer underneath, a pink sky.
    Look {
        felt: [0.96, 0.56, 0.74],
        green: [0.56, 0.92, 0.82],
        tee: [0.82, 0.4, 0.62],
        rail: [1.0, 0.98, 0.95],
        glow: None,
        block: [0.6, 0.4, 0.9],
        earth: [0.92, 0.78, 0.52],
        water: Color::srgba(0.45, 0.75, 1.0, 0.7),
        flag: [1.0, 0.85, 0.2],
        zenith: [0.62, 0.5, 0.95],
        horizon: [1.0, 0.82, 0.9],
        clouds: [1.0, 0.92, 0.96],
        sun: [1.0, 0.92, 0.95],
        sun_illuminance: 11000.0,
        ambient: 900.0,
    },
    // Neon: dark felt, glowing rails, a night sky.
    Look {
        felt: [0.06, 0.1, 0.22],
        green: [0.1, 0.18, 0.36],
        tee: [0.14, 0.06, 0.26],
        rail: [0.08, 0.08, 0.12],
        glow: Some([0.05, 0.7, 1.0]),
        block: [0.3, 0.1, 0.4],
        earth: [0.06, 0.06, 0.09],
        water: Color::srgba(0.1, 0.9, 0.8, 0.6),
        flag: [1.0, 0.2, 0.8],
        zenith: [0.01, 0.01, 0.06],
        horizon: [0.16, 0.05, 0.28],
        clouds: [0.25, 0.15, 0.4],
        sun: [0.6, 0.6, 1.0],
        sun_illuminance: 2500.0,
        ambient: 450.0,
    },
];

/// How far the sides of the holes go down.
const SKIRT: f32 = 1.2;
/// The flag pole's height.
const POLE: f32 = 1.6;

fn look() -> &'static Look {
    &LOOKS[course().look as usize % LOOKS.len()]
}

fn srgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

fn linear(c: [f32; 3], k: f32) -> [f32; 4] {
    [c[0].powf(2.2) * k, c[1].powf(2.2) * k, c[2].powf(2.2) * k, 1.0]
}

/// Builds every hole; returns the windmill bars' nodes, which `windmill_bar` places.
pub fn build(scene: &mut Scene) -> Vec<u32> {
    let look = look();
    scene.push(Command::Environment(Environment {
        sky: srgb(look.horizon),
        ambient: srgb(look.horizon),
        ambient_brightness: look.ambient,
        sun_dir: Vec3::new(-0.4, -0.75, -0.5).normalize(),
        sun_color: srgb(look.sun),
        sun_illuminance: look.sun_illuminance,
        fog_color: srgb(look.horizon),
        fog_sun_color: Color::srgba(look.sun[0], look.sun[1], look.sun[2], 0.5),
        fog_sun_exponent: 10.0,
        fog_start: 80.0,
        fog_end: 700.0,
    }));
    sky(scene, look);
    clouds(scene, look);

    let felt_tex = texture(scene, felt());
    let ground = scene.material(Material { texture: Some(felt_tex), uv_scale: 1.0, roughness: 0.95, double_sided: true, ..Default::default() });
    let water = scene.material(Material { color: look.water, roughness: 0.1, blend: Blend::Blend, double_sided: true, ..Default::default() });
    let rail = scene.material(Material {
        color: srgb(look.rail),
        emissive: look.glow.unwrap_or([0.0; 3]),
        roughness: 0.6,
        ..Default::default()
    });
    let block = scene.material(Material { color: srgb(look.block), roughness: 0.7, ..Default::default() });
    let mut bars = vec![];
    for hole in &course().holes {
        let (mesh, pool) = terrain(hole, look);
        let mesh = scene.mesh(mesh);
        scene.mesh_node(None, mesh, ground, Transform::default());
        if let Some(pool) = pool {
            let pool = scene.mesh(pool);
            scene.spawn(None, NodeKind::Mesh { mesh: pool, material: water, shadows: false }, Transform::default(), true);
        }
        for &(c, h) in &hole.boxes[..hole.drawn] {
            let mesh = scene.cuboid(h * 2.0);
            let material = if h.x.min(h.z) < 0.1 { rail } else { block };
            scene.mesh_node(None, mesh, material, Transform::at(c));
        }
        cup(scene, hole, look);
        bars.extend(windmills(scene, hole, look));
    }
    bars
}

/// A hole's ground as one mesh: felt over every floor cell, basins under the water, sides going down where the hole
/// meets the void and into the basins, and a bottom. Plus the water surfaces, if there are any.
fn terrain(hole: &Hole, look: &Look) -> (MeshData, Option<MeshData>) {
    let mut m = Mesh::default();
    let mut pool = Mesh::default();
    let at = |x: usize, z: usize, y: f32| hole.world(x as f32, z as f32, y);
    let earth = linear(look.earth, 1.0);
    for z in 0..hole.d {
        for x in 0..hole.w {
            let c = hole.cell(x, z);
            if !c.inside() {
                continue;
            }
            let corners = [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)];
            let h = corners.map(|(cx, cz)| hole.corner(cx, cz));
            let level = hole.water_level(x, z);
            let basin = level - (BASIN_DROP - crate::map::WATER_DROP);
            let top = if c == Cell::Water { [basin; 4] } else { h };
            let color = match c {
                Cell::Tee => look.tee,
                Cell::Green => look.green,
                Cell::Water => look.earth,
                _ => look.felt,
            };
            // Alternate cells a shade apart, like mowing stripes.
            let shade = if (x + z) % 2 == 0 { 1.0 } else { 0.92 };
            m.quad([0, 1, 2, 3].map(|i| at(corners[i].0, corners[i].1, top[i])), linear(color, shade));
            if c == Cell::Water {
                pool.quad(corners.map(|(cx, cz)| at(cx, cz, level)), [1.0; 4]);
            }
            let bottom = h.iter().copied().fold(f32::MAX, f32::min) - SKIRT;
            m.quad(corners.map(|(cx, cz)| at(cx, cz, bottom)), earth);
            // Each side: its two corners and the neighbor across it.
            for (a, b, nx, nz) in [(0, 1, x as i32, z as i32 - 1), (1, 2, x as i32 + 1, z as i32), (2, 3, x as i32, z as i32 + 1), (3, 0, x as i32 - 1, z as i32)] {
                let n = hole.cell(nx as usize, nz as usize);
                let (ca, cb) = (corners[a], corners[b]);
                let down = if !n.inside() {
                    bottom
                } else if c.floor() && n == Cell::Water {
                    basin
                } else {
                    continue;
                };
                m.quad([at(ca.0, ca.1, top[a]), at(cb.0, cb.1, top[b]), at(cb.0, cb.1, down), at(ca.0, ca.1, down)], earth);
            }
        }
    }
    (m.data(), (!pool.positions.is_empty()).then(|| pool.data()))
}

/// The cup on its slope, a white rim around a dark hole, and the flag in it.
fn cup(scene: &mut Scene, hole: &Hole, look: &Look) {
    let normal = hole.floor(hole.cup + Vec3::Y).map_or(Vec3::Y, |(_, n)| n);
    let tilt = Quat::from_rotation_arc(Vec3::Y, normal);
    let rim = scene.shared_mesh("rim".into(), || MeshData::Cylinder { radius: CUP_RADIUS + 0.04, height: 0.02 });
    let hollow = scene.shared_mesh("hollow".into(), || MeshData::Cylinder { radius: CUP_RADIUS, height: 0.022 });
    let white = scene.color(Color::srgb(0.95, 0.95, 0.95));
    let dark = scene.shared_material("hollow".into(), || Material { color: Color::srgb(0.02, 0.02, 0.02), unlit: true, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh: rim, material: white, shadows: false }, Transform::at(hole.cup).rotated(tilt), true);
    scene.spawn(None, NodeKind::Mesh { mesh: hollow, material: dark, shadows: false }, Transform::at(hole.cup + normal * 0.002).rotated(tilt), true);
    let pole = scene.shared_mesh("pole".into(), || MeshData::Cylinder { radius: 0.02, height: POLE });
    scene.mesh_node(None, pole, white, Transform::at(hole.cup + Vec3::Y * POLE / 2.0));
    let cloth = scene.cuboid(Vec3::new(0.5, 0.32, 0.02));
    let flag = scene.shared_material("flag".into(), || Material { color: srgb(look.flag), double_sided: true, ..Default::default() });
    scene.mesh_node(None, cloth, flag, Transform::at(hole.cup + Vec3::new(0.26, POLE - 0.18, 0.0)));
}

/// Each windmill: a post at the pivot with a cap, and its paddle, whose node is returned.
fn windmills(scene: &mut Scene, hole: &Hole, look: &Look) -> Vec<u32> {
    let post = scene.cuboid(Vec3::new(HUB_WIDTH, WINDMILL_HEIGHT + 0.4, HUB_WIDTH));
    let cap = scene.shared_mesh("cap".into(), || MeshData::Sphere { radius: 0.2, ico: Some(2) });
    let paddle = scene.cuboid(Vec3::new(WINDMILL_REACH * 2.0, WINDMILL_HEIGHT - 0.04, 0.12));
    let wood = scene.color(srgb(look.rail));
    let red = scene.shared_material("paddle".into(), || Material { color: srgb(look.flag), emissive: look.glow.map_or([0.0; 3], |_| [3.0, 0.4, 2.0]), ..Default::default() });
    hole.windmills
        .iter()
        .map(|&p| {
            scene.mesh_node(None, post, wood, Transform::at(p + Vec3::Y * (WINDMILL_HEIGHT + 0.4) / 2.0));
            scene.mesh_node(None, cap, red, Transform::at(p + Vec3::Y * (WINDMILL_HEIGHT + 0.45)));
            scene.mesh_node(None, paddle, red, Transform::default())
        })
        .collect()
}

/// Where windmill `i` of all the course's windmills (hole by hole) is drawn `time` seconds into its hole.
pub fn windmill_bars(time: f32) -> Vec<Transform> {
    let course = course();
    course
        .holes
        .iter()
        .flat_map(|h| h.windmills.iter().enumerate())
        .map(|(i, &p)| {
            let (dir, _) = windmill(i, time);
            Transform::at(p + Vec3::Y * WINDMILL_HEIGHT / 2.0).rotated(Quat::from_rotation_arc(Vec3::X, dir))
        })
        .collect()
}

#[derive(Default)]
struct Mesh {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Mesh {
    /// A quad from its corners in order around it, textured in world space a meter per repeat.
    fn quad(&mut self, corners: [Vec3; 4], color: [f32; 4]) {
        let base = self.positions.len() as u32;
        let n = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or_zero();
        for p in corners {
            self.positions.push(p.to_array());
            // Project onto the plane the quad mostly faces.
            let uv = if n.y.abs() > 0.5 { Vec2::new(p.x, p.z) } else if n.x.abs() > n.z.abs() { Vec2::new(p.z, p.y) } else { Vec2::new(p.x, p.y) };
            self.uvs.push(uv.to_array());
            self.colors.push(color);
        }
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn data(self) -> MeshData {
        MeshData::Custom { positions: self.positions, normals: vec![], uvs: self.uvs, colors: self.colors, indices: self.indices }
    }
}

/// A dome from the horizon up to the zenith, and the sun in it.
fn sky(scene: &mut Scene, look: &Look) {
    const R: f32 = 900.0;
    const RINGS: u32 = 16;
    const SEGMENTS: u32 = 32;
    let (mut positions, mut normals, mut uvs, mut colors, mut indices) = (vec![], vec![], vec![], vec![], vec![]);
    for ring in 0..=RINGS {
        // From well below the horizon (the holes float high) up to straight overhead.
        let up = -0.4 + 1.4 * ring as f32 / RINGS as f32;
        let k = up.max(0.0).powf(0.6);
        let color = [0, 1, 2].map(|i| (look.horizon[i] + (look.zenith[i] - look.horizon[i]) * k).powf(2.2));
        let (y, r) = ((up * std::f32::consts::FRAC_PI_2).sin(), (up * std::f32::consts::FRAC_PI_2).cos());
        for seg in 0..=SEGMENTS {
            let a = seg as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let n = Vec3::new(a.cos() * r, y, a.sin() * r);
            positions.push((n * R + Vec3::X * 800.0).to_array());
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
}

/// Puffy clouds well below the holes, along the whole course.
fn clouds(scene: &mut Scene, look: &Look) {
    let puff = scene.shared_mesh("puff".into(), || MeshData::Sphere { radius: 1.0, ico: Some(2) });
    let white = scene.material(Material { color: srgb(look.clouds), roughness: 1.0, ..Default::default() });
    let span = course().holes.len() as f32 * crate::map::SPACING;
    for i in 0..140u32 {
        let r = |k: u32| hash(i, k);
        let at = Vec3::new(-150.0 + r(1) * (span + 300.0), -30.0 - r(2) * 40.0, (r(3) - 0.5) * 500.0);
        let size = 6.0 + r(4) * 14.0;
        let root = scene.empty(None, Transform::at(at), true);
        for k in 0..4 {
            let off = Vec3::new((r(10 + k) - 0.5) * size * 1.6, (r(20 + k) - 0.3) * size * 0.3, (r(30 + k) - 0.5) * size);
            let scale = Vec3::new(size, size * 0.45, size * 0.8) * (0.6 + r(40 + k) * 0.5);
            let t = Transform { pos: off, rot: Quat::IDENTITY, scale };
            scene.spawn(Some(root), NodeKind::Mesh { mesh: puff, material: white, shadows: false }, t, true);
        }
    }
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

/// Felt: near white with fine fuzz, tinted by the mesh's colors.
fn felt() -> Texture {
    const SIZE: u32 = 128;
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let k = 0.86 + 0.1 * hash(x, y) + 0.04 * hash(x / 8 + 50, y / 8);
            [k, k, k]
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

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}
