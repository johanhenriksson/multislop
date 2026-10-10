//! The map as the host should draw it: terrain, sandstone walls with a frieze along the top, crates, doors and
//! shutters, sky, clouds and mountains on the horizon, plus the C4 model. Aztec swaps in mossy grey stone, vines,
//! overgrown paving, an overcast sky and jungle hills.

use super::scene::Scene;
use super::textures;
use crate::map::{self, MAP};
use crate::shared::trace_world;
use glam::{Vec2, Vec3};
use sdk::client::{Blend, Color, Environment, Material, MeshData, NodeKind, Transform};

/// The bomb's scene nodes: the model, its LED material and the light that flashes with it.
pub struct Bomb {
    pub root: u32,
    pub led: u32,
    pub led_material: Material,
    pub light: u32,
}

pub fn build(scene: &mut Scene) -> Bomb {
    let aztec = map::name() == "aztec";
    // Desert sky, hazy distance, warm hard sunlight; aztec is overcast jungle with a greenish haze and dim cool light.
    let desert = Environment {
        sky: Color::srgb(0.5, 0.68, 0.9),
        ambient: Color::srgb(0.8, 0.85, 1.0),
        ambient_brightness: 350.0,
        sun_dir: Vec3::new(-0.5, -0.9, -0.35),
        sun_color: Color::srgb(1.0, 0.93, 0.8),
        sun_illuminance: 16000.0,
        fog_color: Color::srgb(0.72, 0.76, 0.82),
        fog_sun_color: Color::srgba(1.0, 0.9, 0.7, 0.4),
        fog_sun_exponent: 30.0,
        fog_start: 140.0,
        fog_end: 700.0,
    };
    let environment = if aztec {
        Environment {
            sky: Color::srgb(0.55, 0.6, 0.62),
            ambient: Color::srgb(0.8, 0.9, 0.85),
            ambient_brightness: 400.0,
            sun_color: Color::srgb(0.85, 0.9, 0.95),
            sun_illuminance: 9000.0,
            fog_color: Color::srgb(0.5, 0.56, 0.54),
            fog_sun_color: Color::NONE,
            fog_start: 50.0,
            ..desert
        }
    } else {
        desert
    };
    scene.push(sdk::client::Command::Environment(environment));

    let mut texture = |t: textures::Texture| {
        let id = scene.id();
        scene.push(t.command(id));
        id
    };
    let (stone_tex, frieze_tex) = if aztec { (textures::aztec_stone(), textures::aztec_frieze()) } else { (textures::sandstone(), textures::frieze()) };
    let (stone_tex, frieze_tex, ground_tex) = (texture(stone_tex), texture(frieze_tex), texture(textures::ground(aztec)));
    let (wood_tex, ammo_tex, door_tex) = (texture(textures::wood_crate()), texture(textures::ammo_crate()), texture(textures::door()));
    let clouds_tex = texture(textures::clouds());
    let vines_tex = aztec.then(|| texture(textures::vines()));
    // Damp, dull stone on aztec, without the overcast sky glinting off it.
    let textured = |tex: u32| Material { texture: Some(tex), roughness: if aztec { 1.0 } else { 0.95 }, ..Default::default() };
    let stone_mat = textured(stone_tex);
    let (stone, frieze, ground) = (scene.material(stone_mat), scene.material(textured(frieze_tex)), scene.material(textured(ground_tex)));
    // Weathered, damp wood on aztec.
    let wood_tint = if aztec { Color::srgb(0.62, 0.64, 0.6) } else { Color::WHITE };
    let (wood, ammo) = (scene.material(Material { color: wood_tint, ..textured(wood_tex) }), scene.material(textured(ammo_tex)));
    let door = scene.material(Material { blend: Blend::Mask(0.5), color: wood_tint, ..textured(door_tex) });
    let vines = vines_tex.map(|tex| scene.material(Material { blend: Blend::Mask(0.5), ..textured(tex) }));
    // A few shades of the same stone so neighboring buildings don't look cut from one block.
    let shades = if aztec {
        [Color::WHITE, Color::srgb(0.9, 0.92, 0.88), Color::srgb(1.0, 0.97, 0.9)]
    } else {
        [Color::WHITE, Color::srgb(0.93, 0.88, 0.82), Color::srgb(1.0, 0.94, 0.86)]
    };
    let stones: Vec<u32> = shades
        .into_iter()
        .map(|tint| scene.material(Material { color: tint, ..stone_mat }))
        .collect();
    let unshadowed = |scene: &mut Scene, mesh: u32, material: u32, t: Transform| {
        scene.spawn(None, NodeKind::Mesh { mesh, material, shadows: false }, t, true);
    };

    let (tops, edges) = terrain_mesh(aztec);
    let terrain = scene.mesh(tops);
    scene.mesh_node(None, terrain, ground, Transform::default());
    let ledges = scene.mesh(edges);
    let ledge_mat = scene.material(Material { color: shades[1], ..stone_mat });
    scene.mesh_node(None, ledges, ledge_mat, Transform::default());
    // Desert (or jungle floor) out to the mountains, just below the lowest floor so it never shows inside the map.
    let outside = if aztec { Color::srgb(0.3, 0.42, 0.2) } else { Color::WHITE };
    let desert = scene.material(Material { uv_scale: 300.0, color: outside, ..textured(ground_tex) });
    let plane = scene.mesh(MeshData::Plane(Vec2::splat(1200.0)));
    unshadowed(scene, plane, desert, Transform::at(Vec3::new(0.0, -0.5, 0.0)));

    let boxed = |scene: &mut Scene, center: Vec3, half: Vec3, tile: Vec2, world: bool, material: u32| {
        let mesh = scene.mesh(textured_box(center, half, tile, world));
        scene.mesh_node(None, mesh, material, Transform::default());
    };
    let cell = MAP.cell;
    for (&(c, h), &kind) in MAP.boxes.iter().zip(&MAP.kinds) {
        if kind == b'#' {
            let shade = stones[((c.x * 0.37 + c.z * 0.61).abs() as usize) % stones.len()];
            boxed(scene, c, h, Vec2::splat(4.0), true, shade);
            // A roof over a passage is plain stone; walls stand on the ground.
            if c.y - h.y > -0.5 {
                continue;
            }
            // The carved band just under the top, standing slightly proud of the wall.
            let top = c.y + h.y;
            let band = Vec3::new(c.x, top - 0.75, c.z);
            boxed(scene, band, Vec3::new(h.x + 0.05, 0.25, h.z + 0.05), Vec2::new(4.0, 0.5), true, frieze);
            // Cap stone ledge along the top.
            let cap = Vec3::new(c.x, top - 0.1, c.z);
            boxed(scene, cap, Vec3::new(h.x + 0.12, 0.1, h.z + 0.12), Vec2::splat(4.0), true, shade);
            // Vines hanging from under the band on some aztec walls.
            if let Some(vines) = vines.filter(|_| (c.x * 1.7 + c.z * 2.3).sin() > -0.2) {
                let at = Vec3::new(c.x, top - 1.75, c.z);
                boxed(scene, at, Vec3::new(h.x + 0.03, 0.75, h.z + 0.03), Vec2::new(2.0, 1.5), false, vines);
            }
        } else {
            // Crates, a crate per meter: wooden ones, and green ammo boxes only on and next to the bomb sites so those
            // read as sites. Tall ones are drawn as crates stacked (same footprint, so collision is unchanged),
            // alternating wood and ammo on a site like the original's crate piles.
            let reach = Vec3::new(h.x + cell / 2.0, 0.0, h.z + cell / 2.0);
            let around = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)];
            let on_site = around.iter().any(|&(sx, sz)| MAP.site_at((c + reach * Vec3::new(sx, 0.0, sz)).with_y(c.y - h.y)).is_some());
            let green = on_site && (c.x * 7.3 + c.z * 3.1).sin() > -0.4;
            let layers = (h.y / 0.6).round().max(1.0) as usize;
            let layer = h.y / layers as f32;
            for i in 0..layers {
                let center = Vec3::new(c.x, c.y - h.y + layer * (2 * i + 1) as f32, c.z);
                let mat = if on_site && green ^ (i == 1) { ammo } else { wood };
                boxed(scene, center, Vec3::new(h.x, layer, h.z), Vec2::new(1.0, layer * 2.0), false, mat);
            }
        }
    }

    // A darker base course along walls where they meet open ground, at that spot's floor height. Two shared meshes
    // (one per wall direction) since there are hundreds of them.
    let base_tint = if aztec { Color::srgb(0.62, 0.68, 0.6) } else { Color::srgb(0.78, 0.7, 0.62) };
    let base_mat = scene.material(Material { color: base_tint, ..stone_mat });
    let base_meshes = [
        scene.mesh(textured_box(Vec3::ZERO, Vec3::new(0.05, 0.2, cell / 2.0), Vec2::new(4.0, 4.0), false)),
        scene.mesh(textured_box(Vec3::ZERO, Vec3::new(cell / 2.0, 0.2, 0.05), Vec2::new(4.0, 4.0), false)),
    ];
    for (x, z, (dx, dz), _) in wall_faces() {
        let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
        let normal = Vec3::new(dx as f32, 0.0, dz as f32);
        let at = MAP.center(x, z) + normal * (cell / 2.0 + 0.05) + Vec3::Y * (MAP.ground(nx, nz).y + 0.2);
        scene.mesh_node(None, base_meshes[if dx != 0 { 0 } else { 1 }], base_mat, Transform::at(at));
    }

    // Arched doors at street level and shutters higher up, sprinkled on walls that face open ground, as many per
    // meter of wall whatever the cell size. On small cells the wall has to go on to both sides of a door.
    let faces: std::collections::HashSet<_> = wall_faces().map(|(x, z, d, _)| (x, z, d)).collect();
    for (x, z, (dx, dz), side) in wall_faces() {
        let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
        let roll = ((x * 73 + z * 151 + side * 37) as f32 * 12.9898).sin().fract().abs() * 2.0 / cell;
        let (size, lift) = match roll {
            r if r < 0.04 => (Vec2::new(1.6, 2.6), 0.0),
            r if r < 0.09 => (Vec2::new(1.0, 1.6), 2.4),
            _ => continue,
        };
        let (ax, az) = (dz.unsigned_abs(), dx.unsigned_abs());
        let reach = ((size.x / 2.0 - cell / 2.0) / cell).ceil().max(0.0) as usize;
        let room = (1..=reach).all(|i| faces.contains(&(x + ax * i, z + az * i, (dx, dz))) && x >= ax * i && z >= az * i && faces.contains(&(x - ax * i, z - az * i, (dx, dz))));
        if !room {
            continue;
        }
        let normal = Vec3::new(dx as f32, 0.0, dz as f32);
        let floor = MAP.ground(nx, nz).y;
        let at = MAP.center(x, z) + normal * (cell / 2.0 + 0.03) + Vec3::Y * (floor + lift + size.y / 2.0);
        let half = if dx != 0 { Vec3::new(0.03, size.y / 2.0, size.x / 2.0) } else { Vec3::new(size.x / 2.0, size.y / 2.0, 0.03) };
        boxed(scene, at, half, size, false, door);
        // A stone frame standing out from the wall: jambs, a lintel, and a sill under windows.
        let along = if dx != 0 { Vec3::Z } else { Vec3::X };
        let piece = |center: Vec3, len: f32, height: f32, depth: f32| {
            let half = along * len + Vec3::Y * height + normal.abs() * depth;
            (center + normal * depth, half)
        };
        let mut frame = vec![
            piece(at + along * (size.x / 2.0 + 0.1), 0.1, size.y / 2.0, 0.07),
            piece(at - along * (size.x / 2.0 + 0.1), 0.1, size.y / 2.0, 0.07),
            piece(at + Vec3::Y * (size.y / 2.0 + 0.1), size.x / 2.0 + 0.22, 0.1, 0.09),
        ];
        if lift > 0.0 {
            frame.push(piece(at - Vec3::Y * (size.y / 2.0 + 0.05), size.x / 2.0 + 0.16, 0.05, 0.13));
        }
        for (center, half) in frame {
            boxed(scene, center, half, Vec2::splat(4.0), true, stone);
        }
    }

    // Sky dome: pale and hazy at the horizon, deeper blue overhead, like the desert skybox; grey and rainy on aztec.
    // Linear colors: on dust roughly sRGB (0.78, 0.85, 0.92) at the horizon to (0.33, 0.55, 0.85) at the top.
    let (horizon, zenith) = if aztec { (Vec3::new(0.25, 0.3, 0.3), Vec3::new(0.12, 0.15, 0.2)) } else { (Vec3::new(0.58, 0.69, 0.82), Vec3::new(0.09, 0.26, 0.69)) };
    let sky = scene.mesh(sky_dome(900.0, horizon, zenith));
    let backdrop = Material { unlit: true, fog: false, double_sided: true, ..Default::default() };
    let sky_mat = scene.material(backdrop);
    unshadowed(scene, sky, sky_mat, Transform::default());
    // A cloud layer high overhead, outside the fog.
    let clouds = scene.mesh(MeshData::Plane(Vec2::splat(2400.0)));
    let overcast = if aztec { Color::srgb(0.62, 0.65, 0.68) } else { Color::WHITE };
    let clouds_mat = scene.material(Material { texture: Some(clouds_tex), blend: Blend::Blend, uv_scale: 4.0, color: overcast, ..backdrop });
    unshadowed(scene, clouds, clouds_mat, Transform::at(Vec3::new(0.0, 220.0, 0.0)));
    // Two rings of mountains: brown ones nearer, bluish ones behind, softened by the fog (dark green jungle hills on
    // aztec). Vertex colors are linear: the desert ones are roughly sRGB (0.55, 0.45, 0.36) and (0.6, 0.6, 0.66).
    let (near, far) = if aztec { ([0.05, 0.08, 0.04], [0.12, 0.15, 0.14]) } else { ([0.24, 0.15, 0.09], [0.3, 0.27, 0.27]) };
    for (radius, low, high, color, seed) in [(230.0, 25.0, 70.0, near, 1.0), (340.0, 50.0, 130.0, far, 4.0)] {
        let ring = scene.mesh(mountain_ring(radius, low, high, color, seed));
        let mat = scene.material(backdrop);
        unshadowed(scene, ring, mat, Transform::default());
    }

    // Bomb sites, tinted red.
    let site_mesh = scene.mesh(MeshData::Plane(Vec2::splat(cell)));
    let site_mat = scene.material(Material { color: Color::srgba(0.8, 0.15, 0.1, 0.1), blend: Blend::Blend, unlit: true, ..Default::default() });
    for p in MAP.site_cells() {
        scene.mesh_node(None, site_mesh, site_mat, Transform::at(p + Vec3::Y * 0.03));
    }
    c4(scene)
}

/// Every wall cell side facing open ground: (x, z, outward direction, side index).
fn wall_faces() -> impl Iterator<Item = (usize, usize, (isize, isize), usize)> {
    (0..MAP.depth()).flat_map(|z| (0..MAP.width()).map(move |x| (x, z))).filter(|&(x, z)| MAP.is_wall(x, z)).flat_map(|(x, z)| {
        [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .enumerate()
            .filter(move |&(_, (dx, dz))| MAP.walkable(x.wrapping_add_signed(dx), z.wrapping_add_signed(dz)))
            .map(move |(side, d)| (x, z, d, side))
    })
}

/// C4: an olive brick of charges taped together, keypad and display on top, a couple of wires and a red light.
fn c4(scene: &mut Scene) -> Bomb {
    let root = scene.empty(None, Transform::default(), false);
    let parts = [
        ([0.34, 0.08, 0.2], [0.0, 0.0, 0.0], Color::srgb(0.36, 0.37, 0.25)),
        ([0.345, 0.082, 0.03], [0.0, 0.0, -0.06], Color::srgb(0.45, 0.38, 0.22)),
        ([0.345, 0.082, 0.03], [0.0, 0.0, 0.06], Color::srgb(0.45, 0.38, 0.22)),
        ([0.13, 0.015, 0.1], [0.05, 0.046, 0.0], Color::srgb(0.12, 0.12, 0.12)),
        ([0.09, 0.008, 0.035], [0.05, 0.056, -0.025], Color::srgb(0.25, 0.45, 0.2)),
        ([0.02, 0.012, 0.16], [-0.1, 0.046, 0.0], Color::srgb(0.7, 0.1, 0.1)),
        ([0.02, 0.012, 0.16], [-0.07, 0.046, 0.0], Color::srgb(0.15, 0.2, 0.6)),
    ];
    for (size, at, color) in parts {
        let (mesh, material) = (scene.cuboid(Vec3::from(size)), scene.color(color));
        scene.mesh_node(Some(root), mesh, material, Transform::at(Vec3::from(at)));
    }
    let led_material = Material { color: Color::srgb(0.6, 0.0, 0.0), emissive: [0.3, 0.0, 0.0], ..Default::default() };
    let led = scene.material(led_material);
    let led_mesh = scene.mesh(MeshData::Sphere { radius: 0.012, ico: None });
    let at = Transform::at(Vec3::new(0.1, 0.056, 0.03));
    scene.mesh_node(Some(root), led_mesh, led, at);
    let light = scene.spawn(Some(root), NodeKind::PointLight { color: Color::srgb(1.0, 0.1, 0.05), intensity: 0.0, range: 3.0 }, at, true);
    Bomb { root, led, led_material, light }
}

fn custom(positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, colors: Vec<[f32; 4]>, indices: Vec<u32>) -> MeshData {
    MeshData::Custom { positions, normals, uvs, colors, indices }
}

/// The floors as two meshes: their tops, sand colored, and the stone sides of ledges and slabs with the undersides of
/// slabs. Tops are darker where something overhead shades them (lower down on aztec, which is overgrown there and
/// has murky water at the bottom of its pits).
fn terrain_mesh(aztec: bool) -> (MeshData, MeshData) {
    let (w, d) = MAP.size();
    let cell = MAP.cell;
    let corner_at = |cx: usize, cz: usize, y: f32| Vec3::new(cx as f32 * cell - w / 2.0, y, cz as f32 * cell - d / 2.0);
    let (mut pos, mut normals, mut colors, mut uvs, mut idx) = (vec![], vec![], vec![], vec![], vec![]);
    let mut sides = Quads::default();
    // Corner indices of each edge, the neighbor across it and that neighbor's matching corners.
    const EDGES: [((usize, usize), (isize, isize), (usize, usize)); 4] = [((0, 1), (0, -1), (3, 2)), ((1, 2), (1, 0), (0, 3)), ((2, 3), (0, 1), (1, 0)), ((3, 0), (-1, 0), (2, 1))];
    for z in 0..MAP.depth() {
        for x in 0..MAP.width() {
            if MAP.is_wall(x, z) {
                continue;
            }
            let cells = [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)];
            // Roofs are drawn as boxes.
            for f in MAP.floors(x, z).iter().filter(|f| f.open || f.bottom == f32::NEG_INFINITY) {
                let quad: [Vec3; 4] = std::array::from_fn(|k| corner_at(cells[k].0, cells[k].1, f.corners[k]));
                let normal = (quad[2] - quad[0]).cross(quad[1] - quad[3]).normalize_or(Vec3::Y);
                let normal = if normal.y < 0.0 { -normal } else { normal };
                let base = pos.len() as u32;
                for (i, p) in quad.into_iter().enumerate() {
                    // Baked contact shadow: floor corners next to walls and crates are darker, like a lightmap.
                    let (cx, cz) = cells[i];
                    let around = [(cx.wrapping_sub(1), cz.wrapping_sub(1)), (cx, cz.wrapping_sub(1)), (cx.wrapping_sub(1), cz), (cx, cz)];
                    let solid = around.iter().filter(|&&(ax, az)| !MAP.walkable(ax, az)).count() as f32;
                    let low = if aztec { 0.55 + 0.45 * (p.y / 2.0).clamp(0.0, 1.0) } else if trace_world(p + Vec3::Y * 0.1, Vec3::Y) < 8.0 { 0.6 } else { 1.0 };
                    let shade = low * (1.0 - 0.14 * solid);
                    pos.push(p.to_array());
                    normals.push(normal.to_array());
                    // The texture carries the floor color; vertex color only darkens, or turns aztec's low ground green.
                    let green = if aztec { 1.0 - (p.y / 1.5).clamp(0.0, 1.0) } else { 0.0 };
                    let tint = if aztec && p.y < 0.3 { [0.04, 0.07, 0.06] } else { [1.0 - 0.8 * green, 0.97 - 0.4 * green, 0.94 - 0.85 * green] };
                    colors.push([shade * tint[0], shade * tint[1], shade * tint[2], 1.0]);
                    uvs.push([p.x / 4.0, p.z / 4.0]);
                }
                idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);

                // Each edge drops to the highest floor below it across the edge (a slab's only down to its underside).
                for ((a, b), (dx, dz), (na, nb)) in EDGES {
                    let (nx, nz) = (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz));
                    if nx >= MAP.width() || nz >= MAP.depth() || (MAP.is_wall(nx, nz) && f.bottom == f32::NEG_INFINITY) {
                        continue;
                    }
                    let below = MAP.floors(nx, nz).iter().filter(|g| g.corners[na] <= f.corners[a] + 0.01 && g.corners[nb] <= f.corners[b] + 0.01 && !MAP.is_wall(nx, nz));
                    let next = below.max_by(|g, h| (g.corners[na] + g.corners[nb]).total_cmp(&(h.corners[na] + h.corners[nb])));
                    let (mut lo_a, mut lo_b) = next.map_or((f.bottom, f.bottom), |g| (g.corners[na], g.corners[nb]));
                    lo_a = lo_a.max(f.bottom).max(-1.0);
                    lo_b = lo_b.max(f.bottom).max(-1.0);
                    if f.corners[a] - lo_a < 0.02 && f.corners[b] - lo_b < 0.02 {
                        continue;
                    }
                    let out = Vec3::new(dx as f32, 0.0, dz as f32);
                    let (ca, cb) = (cells[a], cells[b]);
                    sides.push([corner_at(ca.0, ca.1, lo_a), corner_at(cb.0, cb.1, lo_b), corner_at(cb.0, cb.1, f.corners[b]), corner_at(ca.0, ca.1, f.corners[a])], out);
                }
                if f.bottom > f32::NEG_INFINITY {
                    let under: [Vec3; 4] = std::array::from_fn(|k| corner_at(cells[k].0, cells[k].1, f.bottom));
                    sides.push([under[0], under[1], under[2], under[3]], Vec3::NEG_Y);
                }
            }
        }
    }
    (custom(pos, normals, uvs, colors, idx), sides.mesh())
}

/// Quads with world-mapped UVs (4 m per texture, like the walls), each facing its normal.
#[derive(Default)]
struct Quads {
    pos: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl Quads {
    fn push(&mut self, corners: [Vec3; 4], normal: Vec3) {
        let base = self.pos.len() as u32;
        let along = if normal.y.abs() > 0.5 { Vec3::X } else { Vec3::Y.cross(normal) };
        for p in corners {
            self.pos.push(p.to_array());
            self.normals.push(normal.to_array());
            let v = if normal.y.abs() > 0.5 { p.z } else { -p.y };
            self.uvs.push([p.dot(along) / 4.0, v / 4.0]);
        }
        // Wind counter-clockwise as seen from the side the normal points to.
        let n = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        if n.dot(normal) >= 0.0 {
            self.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        } else {
            self.idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
    }

    fn mesh(self) -> MeshData {
        custom(self.pos, self.normals, self.uvs, vec![], self.idx)
    }
}

/// A box whose UVs are in meters divided by `tile`: anchored to the world (so neighboring walls line up) or to the
/// box itself (so a crate face shows whole crates).
fn textured_box(center: Vec3, half: Vec3, tile: Vec2, world: bool) -> MeshData {
    // (normal, u axis, v axis) with u x v = normal, so corners in order are counter-clockwise from outside.
    let faces = [(Vec3::X, Vec3::NEG_Z, Vec3::Y), (Vec3::NEG_X, Vec3::Z, Vec3::Y), (Vec3::Z, Vec3::X, Vec3::Y), (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::Z, Vec3::X), (Vec3::NEG_Y, Vec3::X, Vec3::Z)];
    let (mut pos, mut normals, mut uvs, mut idx) = (vec![], vec![], vec![], vec![]);
    for (n, u, v) in faces {
        let base = pos.len() as u32;
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let local = n * n.dot(half).abs() + u * su * u.dot(half).abs() + v * sv * v.dot(half).abs();
            let p = center + local;
            let anchor = if world { p } else { local + half };
            // Image rows run downwards, world up is +Y, hence the minus on v.
            let flip = if v == Vec3::Y { -1.0 } else { 1.0 };
            let (cu, cv) = if world { (anchor.dot(u), flip * anchor.dot(v)) } else { ((su + 1.0) * u.dot(half).abs(), (1.0 - sv) * v.dot(half).abs()) };
            pos.push(p.to_array());
            normals.push(n.to_array());
            uvs.push([cu / tile.x, cv / tile.y]);
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    custom(pos, normals, uvs, vec![], idx)
}

/// A jagged ring of peaks around the map, facing inwards.
fn mountain_ring(radius: f32, low: f32, high: f32, color: [f32; 3], seed: f32) -> MeshData {
    const SEGMENTS: usize = 160;
    let (mut pos, mut colors, mut idx) = (vec![], vec![], vec![]);
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let ridge = ((a * 3.0 + seed).sin() * 0.5 + (a * 7.0 + seed * 2.0).sin() * 0.3 + (a * 17.0 + seed).sin() * 0.2) * 0.5 + 0.5;
        let peak = low + (high - low) * ridge.powf(1.5);
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        pos.push((dir * radius + Vec3::Y * -10.0).to_array());
        pos.push((dir * (radius + peak * 0.6) + Vec3::Y * peak).to_array());
        // Rocky: faces alternate lit and shaded, darker at the foot.
        let rock = 0.8 + 0.35 * ((a * 41.0 + seed).sin() * 0.5 + 0.5);
        colors.push([color[0] * 0.6, color[1] * 0.6, color[2] * 0.6, 1.0]);
        colors.push([color[0] * rock, color[1] * rock, color[2] * rock, 1.0]);
    }
    for i in 0..SEGMENTS as u32 {
        let (b, t, nb, nt) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        idx.extend([b, t, nb, t, nt, nb]);
    }
    custom(pos, vec![], vec![], colors, idx)
}

/// A UV sphere colored from `horizon` to `zenith` overhead (linear colors).
fn sky_dome(radius: f32, horizon: Vec3, zenith: Vec3) -> MeshData {
    let (sectors, stacks) = (48u32, 24u32);
    let (mut pos, mut normals, mut colors, mut idx) = (vec![], vec![], vec![], vec![]);
    for i in 0..=stacks {
        let phi = std::f32::consts::PI * (0.5 - i as f32 / stacks as f32);
        for j in 0..=sectors {
            let theta = std::f32::consts::TAU * j as f32 / sectors as f32;
            let n = Vec3::new(phi.cos() * theta.cos(), phi.sin(), phi.cos() * theta.sin());
            pos.push((n * radius).to_array());
            normals.push(n.to_array());
            let up = n.y.clamp(0.0, 1.0).powf(0.5);
            let c = horizon.lerp(zenith, up);
            colors.push([c.x, c.y, c.z, 1.0]);
        }
    }
    for i in 0..stacks {
        for j in 0..sectors {
            let (a, b) = (i * (sectors + 1) + j, (i + 1) * (sectors + 1) + j);
            idx.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    custom(pos, normals, vec![], colors, idx)
}
