//! The stadium as the host should draw it: a night sky over floodlit turf, concrete boards under glass, a crowd in
//! banked stands, glowing goals with nets, hazard-striped cover, and pickups on their pads.

use super::{BLUE, NEUTRAL, ORANGE};
use crate::map::{CELL, GOAL_DEPTH, GOAL_HALF_WIDTH, GOAL_HEIGHT, GOAL_LINE, MAP, WALL_TOP};
use crate::shared::{BALL_RADIUS, Item};
use glam::{Quat, Vec2, Vec3};
use sdk::client::{Blend, Color, Command, Environment, Material, MeshData, NodeKind, Transform, look_rot};
use sdk::scene::Scene;

/// The boards: concrete up to here, glass above.
pub const BOARD: f32 = 3.0;
/// Half the pitch inside the walls, across (x).
const HALF_WIDTH: f32 = 24.0;
/// Glass posts stand this far apart.
const POST_EVERY: f32 = 8.0;
/// The key light, a cool white from the floodlights at the -x, -z corner.
const KEY_DIR: Vec3 = Vec3::new(0.42, -0.72, 0.55);
const ZENITH: [f32; 3] = [0.01, 0.015, 0.05];
const HORIZON: [f32; 3] = [0.07, 0.08, 0.17];
/// Stands: rows of seats, each this deep and this much higher than the one in front.
const ROWS: u32 = 18;
const ROW_DEPTH: f32 = 0.85;
const ROW_RISE: f32 = 0.5;
const SEAT_WIDTH: f32 = 0.62;
/// The front row's floor, just over the boards, and its distance out from the pitch's center line.
const STAND_FLOOR: f32 = 3.4;
const STAND_FRONT: f32 = 27.0;
const STAND_LENGTH: f32 = 46.0;

/// A pickup's scene nodes, shown while it's there to take.
pub struct Pickup {
    pub item: Item,
    pub at: Vec3,
    /// The model, hovering and spinning.
    pub model: u32,
    pub glow: u32,
    pub light: u32,
}

/// What the client moves after building: the pickups.
pub struct World {
    pub pickups: Vec<Pickup>,
}

pub fn build(scene: &mut Scene) -> World {
    scene.push(Command::Environment(Environment {
        sky: srgb(HORIZON),
        ambient: Color::srgb(0.55, 0.65, 1.0),
        ambient_brightness: 220.0,
        sun_dir: KEY_DIR.normalize(),
        sun_color: Color::srgb(0.85, 0.92, 1.0),
        sun_illuminance: 9000.0,
        fog_color: Color::srgb(0.04, 0.05, 0.1),
        fog_sun_color: Color::srgba(0.25, 0.3, 0.45, 0.3),
        fog_sun_exponent: 10.0,
        fog_start: 90.0,
        fog_end: 700.0,
    }));
    sky(scene);
    let ground = scene.mesh(MeshData::Plane(Vec2::splat(1400.0)));
    let asphalt = scene.material(Material { color: Color::srgb(0.025, 0.025, 0.03), roughness: 0.95, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh: ground, material: asphalt, shadows: false }, Transform::at(Vec3::Y * -0.02), true);

    turf(scene);
    lines(scene);
    let concrete = texture(scene, concrete());
    let concrete = scene.material(Material { texture: Some(concrete), roughness: 0.85, ..Default::default() });
    walls(scene, concrete);
    blocks(scene, concrete);
    goals(scene);
    for sign in [-1.0, 1.0] {
        stand(scene, Vec3::X * sign, STAND_FRONT, STAND_LENGTH, ROWS);
        stand(scene, Vec3::Z * sign, GOAL_LINE + GOAL_DEPTH + CELL + 1.0, HALF_WIDTH, ROWS * 2 / 3);
    }
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        floodlight(scene, Vec3::new(x * 47.0, 0.0, z * 58.0));
    }

    let pad_mesh = scene.mesh(MeshData::Cylinder { radius: CELL * 0.4, height: 0.06 });
    let pad = scene.material(glow([0.3, 0.9, 1.0], 6.0));
    for &p in &MAP.pads {
        scene.spawn(None, NodeKind::Mesh { mesh: pad_mesh, material: pad, shadows: false }, Transform::at(p + Vec3::Y * 0.03), true);
        let light = NodeKind::PointLight { color: Color::srgb(0.3, 0.85, 1.0), intensity: 300_000.0, range: 12.0 };
        scene.spawn(None, light, Transform::at(p + Vec3::Y * 1.5), true);
    }
    World { pickups: MAP.pickups.iter().map(|&(at, item)| pickup(scene, at, item)).collect() }
}

fn srgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

/// An unlit glowing material: `strength` times its color as emissive.
pub fn glow(c: [f32; 3], strength: f32) -> Material {
    Material { color: srgb(c), emissive: c.map(|v| v * strength), unlit: true, ..Default::default() }
}

/// sRGB to the linear values vertex colors take.
fn linear(c: [f32; 3]) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), 1.0]
}

/// The team color of the half `z` is in, or a neutral white for something spanning both.
fn half_color(z: f32, half_depth: f32) -> [f32; 3] {
    match z {
        _ if z.abs() < half_depth => NEUTRAL,
        z if z > 0.0 => ORANGE,
        _ => BLUE,
    }
}

/// Triangles collected for one custom mesh.
#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Builder {
    /// A flat quad with corners in order around it, turned to face `facing`. UVs are the corners' world position
    /// on the quad's plane divided by `tile`, so neighboring quads line up; 0 leaves them out.
    fn quad(&mut self, mut corners: [Vec3; 4], facing: Vec3, color: [f32; 4], tile: f32) {
        let mut n = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or_zero();
        if n.dot(facing) < 0.0 {
            corners.reverse();
            n = -n;
        }
        let u = if n.y.abs() > 0.9 { Vec3::X } else { Vec3::Y.cross(n).normalize() };
        let v = if n.y.abs() > 0.9 { Vec3::Z } else { Vec3::NEG_Y };
        let base = self.positions.len() as u32;
        for c in corners {
            self.positions.push(c.to_array());
            self.normals.push(n.to_array());
            if tile > 0.0 {
                self.uvs.push([c.dot(u) / tile, c.dot(v) / tile]);
            }
            self.colors.push(color);
        }
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A box from `lo` to `hi`, with only the faces whose normals `faces` keeps.
    fn cuboid(&mut self, lo: Vec3, hi: Vec3, color: [f32; 4], faces: impl Fn(Vec3) -> bool) {
        let c = |x: f32, y: f32, z: f32| Vec3::new(if x > 0.0 { hi.x } else { lo.x }, if y > 0.0 { hi.y } else { lo.y }, if z > 0.0 { hi.z } else { lo.z });
        for n in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
            if !faces(n) {
                continue;
            }
            // Two axes across the face, and its four corners going around.
            let (a, b) = (n.any_orthonormal_vector(), n.cross(n.any_orthonormal_vector()));
            let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(s, t)| {
                let p = n + a * s + b * t;
                c(p.x, p.y, p.z)
            });
            self.quad(corners, n, color, 0.0);
        }
    }

    fn mesh(self) -> MeshData {
        let Builder { positions, normals, uvs, colors, indices } = self;
        MeshData::Custom { positions, normals, uvs, colors, indices }
    }
}

/// Turf: a stripe across the pitch every 4 m, alternating light and dark as mown, each half faintly in its team's
/// color, and the goal recesses' floors.
fn turf(scene: &mut Scene) {
    let grass = texture(scene, grass());
    let material = scene.material(Material { texture: Some(grass), roughness: 0.9, ..Default::default() });
    let mut b = Builder::default();
    const STRIPE: f32 = 4.0;
    let stripes = (GOAL_LINE * 2.0 / STRIPE) as i32;
    for i in 0..stripes {
        let (z0, z1) = (-GOAL_LINE + i as f32 * STRIPE, -GOAL_LINE + (i + 1) as f32 * STRIPE);
        let base = if i % 2 == 0 { [0.2, 0.46, 0.16] } else { [0.15, 0.36, 0.12] };
        let team = if z0 >= 0.0 { ORANGE } else { BLUE };
        let c = [0, 1, 2].map(|k| base[k] + (team[k] * 0.4 - base[k]) * 0.12);
        let corners = [Vec3::new(-HALF_WIDTH, 0.0, z0), Vec3::new(HALF_WIDTH, 0.0, z0), Vec3::new(HALF_WIDTH, 0.0, z1), Vec3::new(-HALF_WIDTH, 0.0, z1)];
        b.quad(corners, Vec3::Y, linear(c), 2.0);
    }
    for end in [-1.0, 1.0] {
        let (z0, z1) = (end * GOAL_LINE, end * (GOAL_LINE + GOAL_DEPTH));
        let w = GOAL_HALF_WIDTH;
        b.quad([Vec3::new(-w, 0.0, z0), Vec3::new(w, 0.0, z0), Vec3::new(w, 0.0, z1), Vec3::new(-w, 0.0, z1)], Vec3::Y, linear([0.12, 0.28, 0.1]), 2.0);
    }
    let mesh = scene.mesh(b.mesh());
    scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::default(), true);
}

/// Painted lines, all in one mesh just over the turf: the touchlines, goal lines and halfway line, the center circle
/// and spot, and a big and a small box in front of each goal.
fn lines(scene: &mut Scene) {
    const W: f32 = 0.14;
    const Y: f32 = 0.012;
    let white = linear([0.92, 0.94, 0.92]);
    let mut b = Builder::default();
    let segment = |b: &mut Builder, a: Vec2, c: Vec2| {
        let d = (c - a).normalize_or_zero();
        let (a, c) = (a - d * W / 2.0, c + d * W / 2.0);
        let side = d.perp() * W / 2.0;
        let p = |v: Vec2| Vec3::new(v.x, Y, v.y);
        b.quad([p(a - side), p(a + side), p(c + side), p(c - side)], Vec3::Y, white, 0.0);
    };
    let (x, z) = (HALF_WIDTH - 0.5, GOAL_LINE - 0.5);
    let corners = [Vec2::new(-x, -z), Vec2::new(x, -z), Vec2::new(x, z), Vec2::new(-x, z)];
    for i in 0..4 {
        segment(&mut b, corners[i], corners[(i + 1) % 4]);
    }
    segment(&mut b, Vec2::new(-x, 0.0), Vec2::new(x, 0.0));
    for end in [-1.0, 1.0] {
        for (half, depth) in [(15.0, 16.0), (10.5, 6.0)] {
            let (front, back) = (end * z, end * (z - depth));
            segment(&mut b, Vec2::new(-half, front), Vec2::new(-half, back));
            segment(&mut b, Vec2::new(-half, back), Vec2::new(half, back));
            segment(&mut b, Vec2::new(half, back), Vec2::new(half, front));
        }
        // The spot the penalty would be taken from, if there were penalties.
        disc(&mut b, Vec3::new(0.0, Y, end * (z - 11.0)), 0.25, white);
    }
    const SEGMENTS: u32 = 72;
    for i in 0..SEGMENTS {
        let a = |i: u32| Vec2::from_angle(i as f32 / SEGMENTS as f32 * std::f32::consts::TAU);
        let (r0, r1) = (10.0 - W / 2.0, 10.0 + W / 2.0);
        let p = |v: Vec2| Vec3::new(v.x, Y, v.y);
        b.quad([p(a(i) * r0), p(a(i + 1) * r0), p(a(i + 1) * r1), p(a(i) * r1)], Vec3::Y, white, 0.0);
    }
    disc(&mut b, Vec3::Y * Y, 0.35, white);
    let material = scene.material(Material { emissive: [0.12, 0.12, 0.12], roughness: 0.9, ..Default::default() });
    let mesh = scene.mesh(b.mesh());
    scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::default(), true);
}

/// A flat disc on the turf, as a fan of thin quads.
fn disc(b: &mut Builder, at: Vec3, r: f32, color: [f32; 4]) {
    for i in 0..16 {
        let p = |i: u32| at + Vec3::new((i as f32 / 16.0 * std::f32::consts::TAU).cos(), 0.0, (i as f32 / 16.0 * std::f32::consts::TAU).sin()) * r;
        b.quad([at, p(i), p(i + 1), at], Vec3::Y, color, 0.0);
    }
}

/// The faces of the walls that look onto the pitch: (inward normal, point where the run starts, run direction,
/// length), merged into runs along each wall and split at the halfway line.
fn wall_faces() -> Vec<(Vec3, Vec3, Vec3, f32)> {
    let mut faces: Vec<(Vec3, Vec3, Vec3, f32)> = vec![];
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            if MAP.is_wall(x, z) {
                continue;
            }
            let c = MAP.center(x, z);
            for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if !MAP.is_wall(nx as usize, nz as usize) {
                    continue;
                }
                let out = Vec3::new(dx as f32, 0.0, dz as f32);
                let along = Vec3::new(dz.abs() as f32, 0.0, dx.abs() as f32);
                let start = c + out * (CELL / 2.0) - along * (CELL / 2.0);
                // Joins the run this face continues, unless that run is in the other half.
                let joins = |f: &&mut (Vec3, Vec3, Vec3, f32)| {
                    f.0 == -out && (f.1 + f.2 * f.3).distance(start) < 0.01 && (f.1.z + f.2.z * f.3 / 2.0).signum() == c.z.signum()
                };
                match faces.iter_mut().find(joins) {
                    Some(f) => f.3 += CELL,
                    None => faces.push((-out, start, along, CELL)),
                }
            }
        }
    }
    faces
}

/// Concrete boards around the pitch with a glowing strip along their top in the color of their half, and glass
/// above them up to the ceiling, on posts.
fn walls(scene: &mut Scene, concrete: u32) {
    for &(c, h) in MAP.boxes.iter().filter(|(c, h)| c.y - h.y < 0.1 && (c.y + h.y) >= WALL_TOP - 0.01) {
        let mesh = scene.mesh(textured_box(c.with_y(BOARD / 2.0), h.with_y(BOARD / 2.0), 4.0));
        scene.mesh_node(None, mesh, concrete, Transform::default());
    }
    let glass = scene.material(Material {
        color: Color::srgba(0.65, 0.8, 1.0, 0.07),
        roughness: 0.05,
        metallic: 0.2,
        blend: Blend::Blend,
        double_sided: true,
        ..Default::default()
    });
    let metal = scene.material(Material { color: Color::srgb(0.12, 0.13, 0.16), metallic: 0.8, roughness: 0.35, ..Default::default() });
    let pane_height = WALL_TOP - BOARD;
    let post = scene.cuboid(Vec3::new(0.18, pane_height, 0.18));
    for (n, start, along, len) in wall_faces() {
        let mid = start + along * (len / 2.0);
        let strip = scene.shared_material(format!("strip {:?}", half_color(mid.z, 0.0)), || glow(half_color(mid.z, 0.0), 5.0));
        let size = along * len + n.abs() * 0.08 + Vec3::Y * 0.14;
        let mesh = scene.cuboid(size);
        scene.spawn(None, NodeKind::Mesh { mesh, material: strip, shadows: false }, Transform::at(mid + n * 0.04 + Vec3::Y * (BOARD - 0.2)), true);
        // A dimmer strip low down, so the boards' foot reads too.
        let dim = scene.shared_material(format!("dim strip {:?}", half_color(mid.z, 0.0)), || glow(half_color(mid.z, 0.0), 1.2));
        let mesh = scene.cuboid(along * len + n.abs() * 0.06 + Vec3::Y * 0.06);
        scene.spawn(None, NodeKind::Mesh { mesh, material: dim, shadows: false }, Transform::at(mid + n * 0.03 + Vec3::Y * 0.35), true);

        let pane = scene.cuboid(along * len + n.abs() * 0.04 + Vec3::Y * pane_height);
        let behind = mid - n * 0.15 + Vec3::Y * (BOARD + pane_height / 2.0);
        scene.spawn(None, NodeKind::Mesh { mesh: pane, material: glass, shadows: false }, Transform::at(behind), true);
        let rail = scene.cuboid(along * len + Vec3::splat(0.12));
        scene.spawn(None, NodeKind::Mesh { mesh: rail, material: metal, shadows: false }, Transform::at(behind.with_y(WALL_TOP)), true);
        let posts = (len / POST_EVERY).round().max(1.0) as i32;
        for i in 0..=posts {
            let at = start + along * (len * i as f32 / posts as f32) - n * 0.15 + Vec3::Y * (BOARD + pane_height / 2.0);
            scene.spawn(None, NodeKind::Mesh { mesh: post, material: metal, shadows: false }, Transform::at(at), true);
        }
    }
}

/// Cover: chest-high blocks with hazard-striped caps, tall pillars with a band of light, and the sideline platforms
/// and their stairs; every top edge glowing in the color of its half.
fn blocks(scene: &mut Scene, concrete: u32) {
    let hazard = texture(scene, hazard());
    let hazard = scene.material(Material { texture: Some(hazard), roughness: 0.6, ..Default::default() });
    for &(c, h) in MAP.boxes.iter().filter(|(c, h)| c.y - h.y < 0.1 && (c.y + h.y) < WALL_TOP - 0.01) {
        let top = c.y + h.y;
        let color = half_color(c.z, h.z);
        let trim = scene.shared_material(format!("trim {color:?}"), || glow(color, 4.0));
        let mesh = scene.mesh(textured_box(c, h, 4.0));
        scene.mesh_node(None, mesh, concrete, Transform::default());
        let against_wall = c.x.abs() + h.x > HALF_WIDTH - 0.1;
        if top > 3.0 {
            // A pillar: bands of light around it.
            for y in [1.2, top - 0.9] {
                let band = scene.cuboid(Vec3::new(h.x + 0.03, 0.12, h.z + 0.03) * 2.0);
                scene.spawn(None, NodeKind::Mesh { mesh: band, material: trim, shadows: false }, Transform::at(c.with_y(y)), true);
            }
        } else if !against_wall {
            let cap = Vec3::new(h.x + 0.01, 0.13, h.z + 0.01);
            let mesh = scene.mesh(textured_box(c.with_y(top - 0.12), cap, 1.0));
            scene.mesh_node(None, mesh, hazard, Transform::default());
        }
        let edge = c + Vec3::Y * (h.y + 0.005);
        for (offset, size) in [
            (Vec3::Z * h.z, Vec3::new(h.x + 0.04, 0.04, 0.05)),
            (Vec3::NEG_Z * h.z, Vec3::new(h.x + 0.04, 0.04, 0.05)),
            (Vec3::X * h.x, Vec3::new(0.05, 0.04, h.z + 0.04)),
            (Vec3::NEG_X * h.x, Vec3::new(0.05, 0.04, h.z + 0.04)),
        ] {
            let mesh = scene.cuboid(size * 2.0);
            scene.spawn(None, NodeKind::Mesh { mesh, material: trim, shadows: false }, Transform::at(edge + offset), true);
        }
    }
}

/// The goals: posts and crossbar glowing in the defending team's color, a net over the recess's back, sides and top,
/// a light inside, and the glass box over the mouth that the crossbar hangs from.
fn goals(scene: &mut Scene) {
    let net_tex = texture(scene, net());
    let glass = scene.material(Material {
        color: Color::srgba(0.65, 0.8, 1.0, 0.07),
        roughness: 0.05,
        metallic: 0.2,
        blend: Blend::Blend,
        double_sided: true,
        ..Default::default()
    });
    let post = scene.mesh(MeshData::Cylinder { radius: 0.16, height: GOAL_HEIGHT });
    let bar = scene.mesh(MeshData::Cylinder { radius: 0.16, height: GOAL_HALF_WIDTH * 2.0 + 0.32 });
    let thin_side = scene.mesh(MeshData::Cylinder { radius: 0.06, height: GOAL_DEPTH });
    let thin_back = scene.mesh(MeshData::Cylinder { radius: 0.06, height: GOAL_HALF_WIDTH * 2.0 });
    let across = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let deep = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    for &(c, h) in MAP.boxes.iter().filter(|(c, h)| c.y - h.y > 0.1) {
        let mesh = scene.cuboid(h * 2.0);
        scene.spawn(None, NodeKind::Mesh { mesh, material: glass, shadows: false }, Transform::at(c), true);
    }
    for (end, color) in [(1.0, ORANGE), (-1.0, BLUE)] {
        let frame = scene.material(glow(color, 6.0));
        let dim = scene.material(glow(color, 2.0));
        let line = end * GOAL_LINE;
        for x in [-GOAL_HALF_WIDTH, GOAL_HALF_WIDTH] {
            scene.spawn(None, NodeKind::Mesh { mesh: post, material: frame, shadows: true }, Transform::at(Vec3::new(x, GOAL_HEIGHT / 2.0, line)), true);
            let side = Vec3::new(x, GOAL_HEIGHT, line + end * GOAL_DEPTH / 2.0);
            scene.spawn(None, NodeKind::Mesh { mesh: thin_side, material: dim, shadows: false }, Transform::at(side).rotated(deep), true);
        }
        scene.spawn(None, NodeKind::Mesh { mesh: bar, material: frame, shadows: true }, Transform::at(Vec3::new(0.0, GOAL_HEIGHT, line)).rotated(across), true);
        let back = Vec3::new(0.0, GOAL_HEIGHT, line + end * (GOAL_DEPTH - 0.1));
        scene.spawn(None, NodeKind::Mesh { mesh: thin_back, material: dim, shadows: false }, Transform::at(back).rotated(across), true);

        let net = scene.material(Material {
            color: Color::srgb(0.95, 0.95, 0.95),
            texture: Some(net_tex),
            emissive: color.map(|v| v * 0.25),
            roughness: 1.0,
            blend: Blend::Mask(0.5),
            double_sided: true,
            ..Default::default()
        });
        let mut b = Builder::default();
        let (w, h, inset) = (GOAL_HALF_WIDTH - 0.05, GOAL_HEIGHT, 0.1);
        let (front, rear) = (line, line + end * (GOAL_DEPTH - inset));
        let white = [1.0; 4];
        b.quad([Vec3::new(-w, 0.0, rear), Vec3::new(w, 0.0, rear), Vec3::new(w, h, rear), Vec3::new(-w, h, rear)], Vec3::Z * -end, white, 2.0);
        for x in [-w, w] {
            b.quad([Vec3::new(x, 0.0, front), Vec3::new(x, 0.0, rear), Vec3::new(x, h, rear), Vec3::new(x, h, front)], Vec3::X * -x.signum(), white, 2.0);
        }
        b.quad([Vec3::new(-w, h, front), Vec3::new(w, h, front), Vec3::new(w, h, rear), Vec3::new(-w, h, rear)], Vec3::NEG_Y, white, 2.0);
        let mesh = scene.mesh(b.mesh());
        scene.spawn(None, NodeKind::Mesh { mesh, material: net, shadows: false }, Transform::default(), true);

        let light = NodeKind::PointLight { color: srgb(color), intensity: 3_000_000.0, range: 18.0 };
        scene.spawn(None, light, Transform::at(Vec3::new(0.0, GOAL_HEIGHT - 1.0, line + end * GOAL_DEPTH * 0.6)), true);
    }
}

/// A stand `out` from the pitch (a unit axis), its front row `front` meters out and `half_length` either side:
/// concrete tiers rising away from the pitch, a crowd on them in one mesh, a back wall and a roof with a strip of
/// lights along its edge.
fn stand(scene: &mut Scene, out: Vec3, front: f32, half_length: f32, rows: u32) {
    let side = out.cross(Vec3::Y);
    let at = |d: f32, y: f32, l: f32| out * (front + d) + Vec3::Y * y + side * l;
    let mut seed = (out.x * 3.0 + out.z * 7.0 + 11.0) as u32;
    let mut rand = || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        hash(seed, seed >> 7)
    };
    let (mut tiers, mut crowd) = (Builder::default(), Builder::default());
    let concrete = linear([0.2, 0.2, 0.22]);
    let l = half_length;
    for row in 0..rows {
        let (d0, d1, y) = (row as f32 * ROW_DEPTH, (row + 1) as f32 * ROW_DEPTH, STAND_FLOOR + row as f32 * ROW_RISE);
        tiers.quad([at(d0, y, -l), at(d1, y, -l), at(d1, y, l), at(d0, y, l)], Vec3::Y, concrete, 0.0);
        let below = if row == 0 { 0.0 } else { y - ROW_RISE };
        tiers.quad([at(d0, below, -l), at(d0, y, -l), at(d0, y, l), at(d0, below, l)], -out, concrete, 0.0);
        for e in [-l, l] {
            tiers.quad([at(d0, 0.0, e), at(d1, 0.0, e), at(d1, y, e), at(d0, y, e)], side * e.signum(), concrete, 0.0);
        }
        // A seat about every 0.6 m, most of them taken.
        for s in 0..(l * 2.0 / SEAT_WIDTH) as i32 {
            if rand() < 0.12 {
                continue;
            }
            let c = at(d0 + 0.4, y, -l + (s as f32 + 0.5) * SEAT_WIDTH + (rand() - 0.5) * 0.12);
            // Fans gather towards the end their team defends; dark clothes and a few whites in between.
            let (home, away) = if c.z > 0.0 { (ORANGE, BLUE) } else { (BLUE, ORANGE) };
            let pick = rand() + (c.z.abs() / GOAL_LINE).min(1.0) * 0.25;
            let shirt = match rand() {
                _ if pick > 0.62 => home.map(|v| v * (0.7 + 0.3 * rand())),
                r if r < 0.12 => away.map(|v| v * 0.8),
                r if r < 0.25 => [0.85, 0.85, 0.82],
                r => [0.06 + r * 0.1, 0.06 + r * 0.08, 0.08 + r * 0.1],
            };
            let skin = [[0.85, 0.65, 0.5], [0.6, 0.42, 0.3], [0.38, 0.26, 0.18], [0.95, 0.78, 0.62]][(rand() * 3.99) as usize];
            let half = (out * 0.15 + side * 0.2).abs();
            let height = 0.5 + rand() * 0.15;
            crowd.cuboid(c - half, c + half + Vec3::Y * height, linear(shirt), |n| n != Vec3::NEG_Y && n != out);
            let head = c + Vec3::Y * (height + 0.12);
            crowd.cuboid(head - Vec3::splat(0.11), head + Vec3::splat(0.11), linear(skin), |n| n == -out || n == Vec3::Y);
        }
    }
    let depth = rows as f32 * ROW_DEPTH;
    let top = STAND_FLOOR + rows as f32 * ROW_RISE;
    let roof_y = top + 5.0;
    let dark = linear([0.1, 0.1, 0.12]);
    tiers.quad([at(depth, 0.0, -l), at(depth, roof_y, -l), at(depth, roof_y, l), at(depth, 0.0, l)], -out, dark, 0.0);
    for e in [-l, l] {
        tiers.quad([at(0.0, top, e), at(depth, top, e), at(depth, roof_y, e), at(depth * 0.45, roof_y, e)], side * e.signum(), dark, 0.0);
    }
    let material = scene.shared_material("stand".into(), || Material { roughness: 0.9, ..Default::default() });
    let mesh = scene.mesh(tiers.mesh());
    scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::default(), true);
    let mesh = scene.mesh(crowd.mesh());
    scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, Transform::default(), true);

    // The roof: a slab out over the back half of the stand, lit along its front edge.
    let roof = scene.cuboid(out.abs() * depth * 0.6 + Vec3::Y * 0.5 + side.abs() * (l * 2.0 + 1.0));
    let metal = scene.color(Color::srgb(0.1, 0.11, 0.13));
    scene.spawn(None, NodeKind::Mesh { mesh: roof, material: metal, shadows: false }, Transform::at(at(depth * 0.7, roof_y, 0.0)), true);
    let strip = scene.cuboid(out.abs() * 0.25 + Vec3::Y * 0.25 + side.abs() * l * 2.0);
    let lamp = scene.shared_material("roof lamp".into(), || glow([1.0, 0.95, 0.85], 8.0));
    scene.spawn(None, NodeKind::Mesh { mesh: strip, material: lamp, shadows: false }, Transform::at(at(depth * 0.4, roof_y - 0.3, 0.0)), true);
    for e in [-0.65, 0.0, 0.65] {
        let light = NodeKind::PointLight { color: Color::srgb(1.0, 0.92, 0.8), intensity: 5_000_000.0, range: 30.0 };
        scene.spawn(None, light, Transform::at(at(depth * 0.4, roof_y - 2.0, e * l)), true);
    }
}

/// A floodlight tower standing at `foot`: a tapering pylon topped by a bank of lamps aimed at the pitch, a halo
/// around them, and the light itself.
fn floodlight(scene: &mut Scene, foot: Vec3) {
    const HEIGHT: f32 = 42.0;
    let metal = scene.shared_material("tower".into(), || Material { color: Color::srgb(0.16, 0.17, 0.2), metallic: 0.7, roughness: 0.5, ..Default::default() });
    for (i, w) in [1.6f32, 1.2, 0.9, 0.7].into_iter().enumerate() {
        let seg = HEIGHT / 4.0;
        let mesh = scene.cuboid(Vec3::new(w, seg, w));
        scene.spawn(None, NodeKind::Mesh { mesh, material: metal, shadows: false }, Transform::at(foot + Vec3::Y * (seg * (i as f32 + 0.5))), true);
    }
    let top = foot + Vec3::Y * (HEIGHT + 2.0);
    let aim = (Vec3::new(foot.x * 0.25, 0.0, foot.z * 0.3) - top).normalize();
    let rot = look_rot((-aim.x).atan2(-aim.z), aim.y.asin());
    let bank = scene.empty(None, Transform::at(top).rotated(rot), true);
    let frame = scene.cuboid(Vec3::new(9.0, 5.6, 0.6));
    scene.spawn(Some(bank), NodeKind::Mesh { mesh: frame, material: metal, shadows: false }, Transform::default(), true);
    let lamp = scene.shared_material("lamp".into(), || {
        Material { color: Color::srgb(1.0, 0.97, 0.9), emissive: [30.0, 29.0, 26.0], unlit: true, fog: false, ..Default::default() }
    });
    let face = scene.cuboid(Vec3::new(1.2, 1.0, 0.1));
    for row in 0..4 {
        for col in 0..6 {
            let at = Vec3::new(-3.6 + col as f32 * 1.44, -2.1 + row as f32 * 1.4, -0.32);
            scene.spawn(Some(bank), NodeKind::Mesh { mesh: face, material: lamp, shadows: false }, Transform::at(at), true);
        }
    }
    // No bloom in the host, so layers of soft haze around the bank stand in for the glare.
    let halo = scene.shared_mesh("halo".into(), || MeshData::Sphere { radius: 1.0, ico: Some(3) });
    let haze = scene.shared_material("halo".into(), || Material {
        color: Color::srgba(1.0, 0.95, 0.85, 0.05),
        unlit: true,
        blend: Blend::Blend,
        fog: false,
        ..Default::default()
    });
    for r in [3.0, 4.2, 5.8, 8.0] {
        scene.spawn(Some(bank), NodeKind::Mesh { mesh: halo, material: haze, shadows: false }, Transform::at(Vec3::NEG_Z * 0.8).scaled(r), true);
    }
    let light = NodeKind::PointLight { color: Color::srgb(1.0, 0.95, 0.88), intensity: 1.0e8, range: 160.0 };
    scene.spawn(None, light, Transform::at(top + aim * 3.0), true);
}

/// A dome from the city-lit haze on the horizon up to deep night blue, stars, and a pale moon where the key light
/// comes from.
fn sky(scene: &mut Scene) {
    const R: f32 = 900.0;
    const RINGS: u32 = 16;
    const SEGMENTS: u32 = 32;
    let (mut positions, mut normals, mut uvs, mut colors, mut indices) = (vec![], vec![], vec![], vec![], vec![]);
    for ring in 0..=RINGS {
        let up = -0.1 + 1.1 * ring as f32 / RINGS as f32;
        let k = up.max(0.0).powf(0.45);
        let color = linear([0, 1, 2].map(|i| HORIZON[i] + (ZENITH[i] - HORIZON[i]) * k));
        let (y, r) = ((up * std::f32::consts::FRAC_PI_2).sin(), (up * std::f32::consts::FRAC_PI_2).cos());
        for seg in 0..=SEGMENTS {
            let a = seg as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let n = Vec3::new(a.cos() * r, y, a.sin() * r);
            positions.push((n * R).to_array());
            normals.push((-n).to_array());
            uvs.push([0.0, 0.0]);
            colors.push(color);
        }
    }
    for ring in 0..RINGS {
        for seg in 0..SEGMENTS {
            let (a, b) = (ring * (SEGMENTS + 1) + seg, (ring + 1) * (SEGMENTS + 1) + seg);
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    let dome = scene.mesh(MeshData::Custom { positions, normals, uvs, colors, indices });
    let unlit = scene.material(Material { unlit: true, fog: false, double_sided: true, ..Default::default() });
    scene.spawn(None, NodeKind::Mesh { mesh: dome, material: unlit, shadows: false }, Transform::default(), true);

    // Stars: small quads facing the center, brighter and more of them higher up.
    let mut b = Builder::default();
    for i in 0..900u32 {
        let (a, h) = (hash(i, 1) * std::f32::consts::TAU, hash(i, 2));
        let up = 0.08 + h.sqrt() * 0.92;
        let dir = Vec3::new(a.cos() * (1.0 - up * up).sqrt(), up, a.sin() * (1.0 - up * up).sqrt());
        let size = 0.8 + hash(i, 3).powi(4) * 2.2;
        let bright = (0.35 + 0.65 * hash(i, 4)) * (0.4 + 0.6 * up);
        let tint = if hash(i, 5) > 0.8 { [1.0, 0.85, 0.7] } else { [0.85, 0.9, 1.0] };
        let (s, t) = (dir.any_orthonormal_vector() * size, dir.cross(dir.any_orthonormal_vector()) * size);
        let at = dir * (R - 20.0);
        b.quad([at - s - t, at + s - t, at + s + t, at - s + t], -dir, [tint[0] * bright, tint[1] * bright, tint[2] * bright, 1.0], 0.0);
    }
    let stars = scene.mesh(b.mesh());
    scene.spawn(None, NodeKind::Mesh { mesh: stars, material: unlit, shadows: false }, Transform::default(), true);
    let moon = scene.mesh(MeshData::Sphere { radius: 22.0, ico: Some(3) });
    let pale = scene.material(Material { color: Color::srgb(0.85, 0.88, 0.95), emissive: [1.6, 1.7, 1.9], unlit: true, fog: false, ..Default::default() });
    let dir = Vec3::new(-KEY_DIR.x, 0.35, -KEY_DIR.z).normalize();
    scene.spawn(None, NodeKind::Mesh { mesh: moon, material: pale, shadows: false }, Transform::at(dir * (R - 80.0)), true);
}

/// A pickup on its pad: a dark base, a glowing ring in the item's color with a light, and the model over it.
fn pickup(scene: &mut Scene, at: Vec3, item: Item) -> Pickup {
    let color = item_color(item);
    let base = scene.shared_mesh("pickup base".into(), || MeshData::Cylinder { radius: 0.75, height: 0.1 });
    let metal = scene.shared_material("pickup base".into(), || Material { color: Color::srgb(0.1, 0.1, 0.12), metallic: 0.7, roughness: 0.4, ..Default::default() });
    scene.mesh_node(None, base, metal, Transform::at(at + Vec3::Y * 0.05));
    let ring = scene.shared_mesh("pickup ring".into(), || MeshData::Cylinder { radius: 0.62, height: 0.04 });
    let lit = scene.shared_material(format!("pickup {color:?}"), || glow(color, 4.0));
    let ring = scene.spawn(None, NodeKind::Mesh { mesh: ring, material: lit, shadows: false }, Transform::at(at + Vec3::Y * 0.11), true);
    let light = NodeKind::PointLight { color: srgb(color), intensity: PICKUP_LIGHT, range: 7.0 };
    let light = scene.spawn(None, light, Transform::at(at + Vec3::Y * 1.2), true);
    let model = scene.empty(None, Transform::at(at + Vec3::Y * 1.0), true);
    item_model(scene, model, item);
    Pickup { item, at, model, glow: ring, light }
}

pub const PICKUP_LIGHT: f32 = 150_000.0;

pub fn item_color(item: Item) -> [f32; 3] {
    match item {
        Item::Health => [0.35, 1.0, 0.45],
        Item::Mega => [1.0, 0.3, 0.45],
        Item::Armor => [1.0, 0.8, 0.2],
        Item::HeavyArmor => [1.0, 0.3, 0.15],
        Item::Rockets => [0.75, 0.95, 0.3],
    }
}

/// What hovers over the pad: health a white box with red crosses (mega a big one), armor a vest-shaped plate
/// (yellow light, red heavy), rockets an olive crate of warheads.
pub fn item_model(scene: &mut Scene, root: u32, item: Item) {
    let part = |scene: &mut Scene, size: [f32; 3], at: [f32; 3], m: Material| {
        let mesh = scene.cuboid(Vec3::from(size));
        let material = scene.shared_material(format!("item {:?}", m.color.0), || m);
        scene.mesh_node(Some(root), mesh, material, Transform::at(Vec3::from(at)));
    };
    let plain = |c: [f32; 3], glow: f32| Material { color: srgb(c), emissive: c.map(|v| v * glow), roughness: 0.4, ..Default::default() };
    match item {
        Item::Health | Item::Mega => {
            let s = if item == Item::Mega { 1.5 } else { 1.0 };
            let red = plain([0.9, 0.08, 0.06], 0.6);
            part(scene, [0.5 * s; 3], [0.0; 3], plain([0.95, 0.95, 0.93], 0.25));
            for size in [[0.12, 0.34, 0.52], [0.34, 0.12, 0.52], [0.52, 0.34, 0.12], [0.52, 0.12, 0.34]] {
                part(scene, size.map(|v| v * s), [0.0; 3], red);
            }
        }
        Item::Armor | Item::HeavyArmor => {
            let c = if item == Item::Armor { [0.95, 0.75, 0.15] } else { [0.85, 0.15, 0.08] };
            let m = Material { metallic: 0.6, ..plain(c, 0.35) };
            part(scene, [0.6, 0.45, 0.14], [0.0, -0.1, 0.0], m);
            for x in [-0.2, 0.2] {
                part(scene, [0.18, 0.28, 0.14], [x, 0.25, 0.0], m);
            }
            part(scene, [0.64, 0.06, 0.16], [0.0, -0.05, 0.0], plain([0.1, 0.1, 0.1], 0.0));
        }
        Item::Rockets => {
            let olive = plain([0.3, 0.36, 0.2], 0.1);
            part(scene, [0.7, 0.32, 0.44], [0.0, -0.1, 0.0], olive);
            part(scene, [0.74, 0.05, 0.48], [0.0, 0.03, 0.0], plain([0.12, 0.13, 0.1], 0.0));
            let body = scene.shared_mesh("warhead".into(), || MeshData::Cylinder { radius: 0.07, height: 0.3 });
            let tip = scene.shared_mesh("warhead tip".into(), || MeshData::Sphere { radius: 0.07, ico: Some(2) });
            let (olive, red) = (scene.shared_material("warhead".into(), || plain([0.35, 0.4, 0.25], 0.1)), scene.shared_material("warhead tip".into(), || plain([0.8, 0.2, 0.1], 0.5)));
            for x in [-0.2, 0.0, 0.2] {
                scene.mesh_node(Some(root), body, olive, Transform::at(Vec3::new(x, 0.15, 0.0)));
                scene.mesh_node(Some(root), tip, red, Transform::at(Vec3::new(x, 0.3, 0.0)));
            }
        }
    }
}

/// The ball's mesh: a UV sphere whose texture coordinates `ball_texture` is drawn for (u around Y from +X towards
/// +Z, v from the top down).
pub fn ball_mesh() -> MeshData {
    const RINGS: u32 = 24;
    const SEGMENTS: u32 = 48;
    let (mut positions, mut normals, mut uvs, mut indices) = (vec![], vec![], vec![], vec![]);
    for ring in 0..=RINGS {
        let v = ring as f32 / RINGS as f32;
        for seg in 0..=SEGMENTS {
            let u = seg as f32 / SEGMENTS as f32;
            let n = sphere_dir(u, v);
            positions.push((n * BALL_RADIUS).to_array());
            normals.push(n.to_array());
            uvs.push([u, v]);
        }
    }
    for ring in 0..RINGS {
        for seg in 0..SEGMENTS {
            let (a, b) = (ring * (SEGMENTS + 1) + seg, (ring + 1) * (SEGMENTS + 1) + seg);
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    MeshData::Custom { positions, normals, uvs, colors: vec![], indices }
}

fn sphere_dir(u: f32, v: f32) -> Vec3 {
    let (theta, phi) = (v * std::f32::consts::PI, u * std::f32::consts::TAU);
    Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin())
}

/// White panels of a truncated icosahedron with dark seams between them, and the twelve pentagons a shade darker.
fn ball_texture() -> Texture {
    const SIZE: u32 = 512;
    let phi = (1.0 + 5f32.sqrt()) / 2.0;
    let mut pentagons = vec![];
    for (a, b) in [(1.0, phi), (1.0, -phi), (-1.0, phi), (-1.0, -phi)] {
        pentagons.extend([Vec3::new(0.0, a, b), Vec3::new(a, b, 0.0), Vec3::new(b, 0.0, a)]);
    }
    let mut hexagons = vec![];
    for i in 0..8 {
        hexagons.push(Vec3::new(if i & 1 == 0 { 1.0 } else { -1.0 }, if i & 2 == 0 { 1.0 } else { -1.0 }, if i & 4 == 0 { 1.0 } else { -1.0 }));
    }
    for (a, b) in [(1.0 / phi, phi), (1.0 / phi, -phi), (-1.0 / phi, phi), (-1.0 / phi, -phi)] {
        hexagons.extend([Vec3::new(0.0, a, b), Vec3::new(a, b, 0.0), Vec3::new(b, 0.0, a)]);
    }
    let centers: Vec<(Vec3, bool)> = pentagons.iter().map(|p| (p.normalize(), true)).chain(hexagons.iter().map(|h| (h.normalize(), false))).collect();
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let d = sphere_dir((i % SIZE) as f32 / SIZE as f32, (i / SIZE) as f32 / SIZE as f32);
            let (mut best, mut second, mut penta) = (-2.0f32, -2.0f32, false);
            for &(c, p) in &centers {
                let dot = d.dot(c);
                if dot > best {
                    (second, best, penta) = (best, dot, p);
                } else if dot > second {
                    second = dot;
                }
            }
            let seam = ((best - second) / 0.012).clamp(0.0, 1.0);
            let panel = if penta { 0.78 } else { 0.97 };
            let k = 0.12 + (panel - 0.12) * seam;
            [k, k, k * 1.01]
        })
        .collect();
    mipmapped(SIZE, level)
}

pub fn ball_material(scene: &mut Scene) -> Material {
    let tex = texture(scene, ball_texture());
    Material { texture: Some(tex), roughness: 0.35, ..Default::default() }
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

/// A texture from its full-size level of sRGB colors with alpha (0 to 1), every smaller level box-filtered from it.
fn mipmapped_alpha(full: u32, mut level: Vec<[f32; 4]>) -> Texture {
    let (mut size, mut mips, mut rgba) = (full, 0, vec![]);
    loop {
        rgba.extend(level.iter().flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)));
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
                [0, 1, 2, 3].map(|k| (a[k] + b[k] + c[k] + d[k]) / 4.0)
            })
            .collect();
        size = half;
    }
    Texture { size: full, mips, rgba }
}

fn mipmapped(full: u32, level: Vec<[f32; 3]>) -> Texture {
    mipmapped_alpha(full, level.into_iter().map(|[r, g, b]| [r, g, b, 1.0]).collect())
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

/// Grass, near grey so the turf's vertex colors give the hue: mottled patches and fine blade streaks, tiling.
fn grass() -> Texture {
    const SIZE: u32 = 256;
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let patches = noise(SIZE, x, y, 4) * 0.5 + noise(SIZE, x, y, 16) * 0.5;
            // Blades: short streaks, each a little lighter or darker than the next.
            let blade = hash(x, y / 3 + (x % 7) * 13);
            let k = 0.72 + 0.18 * patches + 0.16 * blade;
            [k * 0.97, k, k * 0.95]
        })
        .collect();
    mipmapped(SIZE, level)
}

/// Poured concrete: mottled grey, faint formwork seams, and tie holes.
fn concrete() -> Texture {
    const SIZE: u32 = 256;
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let n = noise(SIZE, x, y, 4) * 0.4 + noise(SIZE, x, y, 16) * 0.35 + noise(SIZE, x, y, 64) * 0.25;
            let grit = hash(x, y) * 0.06;
            let (px, py) = (x % 128, y % 64);
            let seam = if px == 0 || py == 0 { 0.7 } else { 1.0 };
            let hole = [(32, 32), (96, 32)].iter().any(|&(hx, hy): &(u32, u32)| hx.abs_diff(px).pow(2) + hy.abs_diff(py).pow(2) < 9);
            let k = (0.46 + 0.16 * n + grit) * seam * if hole { 0.45 } else { 1.0 };
            [k, k, k * 1.03]
        })
        .collect();
    mipmapped(SIZE, level)
}

/// Yellow and black hazard stripes, worn.
fn hazard() -> Texture {
    const SIZE: u32 = 128;
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let wear = 0.8 + 0.2 * noise(SIZE, x, y, 8) - if hash(x, y) > 0.97 { 0.3 } else { 0.0 };
            let c = if (x + y) / 32 % 2 == 0 { [0.95, 0.72, 0.08] } else { [0.06, 0.06, 0.06] };
            c.map(|v| v * wear)
        })
        .collect();
    mipmapped(SIZE, level)
}

/// A net: white cords in a square grid, cut out by the material's mask. Eight meshes across, so 25 cm ones when
/// mapped 2 m to a repeat; the cords sit on power-of-two boundaries so they survive into the smaller mips.
fn net() -> Texture {
    const SIZE: u32 = 128;
    let level = (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let cord = x % 16 < 3 || y % 16 < 3;
            [1.0, 1.0, 1.0, if cord { 1.0 } else { 0.0 }]
        })
        .collect();
    mipmapped_alpha(SIZE, level)
}

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}
