//! The maps, as layers of ASCII grids with a height grid each (`maps/<name>.txt` and `maps/<name>_heights.txt`, the
//! layers separated by a blank line). Dust is de_dust2, converted from the original geometry by
//! `tools/dust2_to_grid.py` (1 m cells): north is up, B site top-left, A top-right, CT spawn between them, T spawn
//! bottom-left. Aztec (2 m cells, one layer): CT spawn west, A north-west, B south-middle, T spawn east, the rope
//! bridge a raised walkway along the north wall of a sunken water room.
//!
//! The base layer: `#` wall (its height is the top), `X` tall crate, `x` low crate (jumpable, both standing on their
//! cell's height), `k` crate whose top is its height, `A`/`B` bomb site floor, `T`/`C` team spawns, `.` floor.
//! Upper layers, blank where there's nothing: `.` (or a site or spawn letter) a floor slab whose top is its height,
//! `#` a roof block whose underside is its height (a box like walls, and a floor nobody stands on). So a cell can have
//! several floors, with open space between them.

use crate::shared::{HEIGHT, STEP, Team};
use glam::Vec3;
use std::cell::Cell;
use std::sync::LazyLock;

/// The maps this game can play.
pub const MAPS: [&str; 2] = ["dust", "aztec"];

static PARSED: [LazyLock<Map>; 2] = [
    LazyLock::new(|| Map::parse(include_str!("maps/dust.txt"), include_str!("maps/dust_heights.txt"), 1.0, 1.05)),
    LazyLock::new(|| Map::parse(include_str!("maps/aztec.txt"), include_str!("maps/aztec_heights.txt"), 2.0, f32::INFINITY)),
];

thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

/// The map being played: dust until `load` picks another. A game module plays one map at a time, so it's a global
/// rather than threaded through everything that looks at the map.
pub static MAP: Current = Current;

pub struct Current;

impl std::ops::Deref for Current {
    type Target = Map;

    fn deref(&self) -> &Map {
        &PARSED[CURRENT.with(Cell::get)]
    }
}

/// Makes `name`, one of `MAPS`, the map; false for any other name.
pub fn load(name: &str) -> bool {
    let Some(i) = MAPS.iter().position(|m| *m == name) else { return false };
    CURRENT.with(|c| c.set(i));
    true
}

/// The name of the map being played.
pub fn name() -> &'static str {
    MAPS[CURRENT.with(Cell::get)]
}

/// Walls go from below the lowest floor; ones at least `WALL_TOP` high get a random extra so the skyline reads as
/// buildings.
const WALL_BOTTOM: f32 = -1.0;
const WALL_TOP: f32 = 6.5;
/// Thickness of a floor slab in an upper layer (more than a step, so you can't step up onto one from under it), and
/// of a roof block.
pub const SLAB: f32 = 0.6;
const ROOF: f32 = 2.0;

/// A floor in a cell: solid from `bottom` up to its surface, which is interpolated between its corner heights.
#[derive(Clone, Copy, Debug)]
pub struct Floor {
    /// Height at the cell center as the map gives it.
    pub y: f32,
    /// Underside: minus infinity for the ground, which is solid all the way down.
    pub bottom: f32,
    /// Surface height at the cell's corners, in the order (x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1).
    pub corners: [f32; 4],
    /// Players can stand here: not the ground under a crate or a wall.
    pub open: bool,
    /// The map character: `A`/`B` for bomb sites.
    pub tag: u8,
}

pub struct Map {
    /// Meters per grid cell.
    pub cell: f32,
    /// The base layer's characters.
    rows: Vec<Vec<u8>>,
    /// Every floor, grouped by cell (row by row), lowest first in each. A floor's index is its node for pathfinding.
    floors: Vec<Floor>,
    /// Start of each cell's floors in `floors`, plus the end.
    first: Vec<u32>,
    /// Floors you can walk to from each floor (`links[link_first[n]..link_first[n + 1]]`), and the reverse.
    links: Vec<u32>,
    link_first: Vec<u32>,
    back: Vec<Vec<u32>>,
    /// Open floors that can be walked to from the T spawn (not, say, a ledge only a jump gets onto).
    reachable: Vec<bool>,
    /// Solid boxes as (center, half extents), merged from runs of equal cells.
    pub boxes: Vec<(Vec3, Vec3)>,
    /// The map character each box is made of: `#` for walls and roofs, otherwise a crate.
    pub kinds: Vec<u8>,
    pub t_spawns: Vec<Vec3>,
    pub ct_spawns: Vec<Vec3>,
}

/// Height digits in quarter meters: 0-9, a-z, then A-Z (base 62).
fn quarters(c: u8) -> Option<f32> {
    let q = match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'z' => c - b'a' + 10,
        b'A'..=b'Z' => c - b'A' + 36,
        _ => return None,
    };
    Some(q as f32 * 0.25)
}

fn walkable_char(c: u8) -> bool {
    matches!(c, b'.' | b'A' | b'B' | b'T' | b'C')
}

type Solid = Option<(u8, f32, f32)>;

impl Map {
    /// `ramp`: neighboring floors at most this far apart in height are joined by a slope, others meet at a ledge.
    fn parse(text: &str, heights: &str, cell: f32, ramp: f32) -> Map {
        let blocks = |s: &str| -> Vec<Vec<Vec<u8>>> {
            s.split("\n\n").filter(|b| !b.trim().is_empty()).map(|b| b.lines().map(|l| l.bytes().collect()).collect()).collect()
        };
        let (layers, heights) = (blocks(text), blocks(heights));
        let rows = layers[0].clone();
        let (w, d) = (rows[0].len(), rows.len());
        let at = |grid: &Vec<Vec<u8>>, x: usize, z: usize| grid.get(z).and_then(|r| r.get(x)).copied().unwrap_or(b' ');
        let mut map = Map { cell, rows, floors: vec![], first: vec![], links: vec![], link_first: vec![], back: vec![], reachable: vec![], boxes: vec![], kinds: vec![], t_spawns: vec![], ct_spawns: vec![] };
        let base_height = |x: usize, z: usize| quarters(at(&heights[0], x, z)).unwrap_or(2.0);

        // Solids per layer as (kind, bottom, top), merged into boxes below, and the floors of each cell.
        let mut solids: Vec<Vec<Solid>> = vec![vec![None; w * d]; layers.len()];
        let mut cells: Vec<Vec<Floor>> = vec![vec![]; w * d];
        let floor = |y: f32, bottom: f32, open: bool, tag: u8| Floor { y, bottom, corners: [y; 4], open, tag };
        for z in 0..d {
            for x in 0..w {
                let (c, h) = (map.rows[z][x], base_height(x, z));
                let ground = match c {
                    b'#' => {
                        solids[0][z * w + x] = Some((c, WALL_BOTTOM, h));
                        floor(WALL_BOTTOM, f32::NEG_INFINITY, false, c)
                    }
                    b'x' | b'X' => {
                        solids[0][z * w + x] = Some((c, h, h + if c == b'X' { 2.2 } else { 1.0 }));
                        floor(h, f32::NEG_INFINITY, false, c)
                    }
                    b'k' => {
                        // The crate stands on the lowest floor next to it.
                        let sides = [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)].map(|(dx, dz)| (x.wrapping_add_signed(dx), z.wrapping_add_signed(dz)));
                        let low = sides.iter().filter(|&&(sx, sz)| walkable_char(at(&map.rows, sx, sz))).map(|&(sx, sz)| base_height(sx, sz)).fold(h, f32::min);
                        let bottom = if low < h { low } else { h - 1.0 };
                        solids[0][z * w + x] = Some((c, bottom, h));
                        floor(bottom, f32::NEG_INFINITY, false, c)
                    }
                    _ => floor(h, f32::NEG_INFINITY, walkable_char(c), c),
                };
                cells[z * w + x].push(ground);
                for (n, layer) in layers.iter().enumerate().skip(1) {
                    let (c, Some(h)) = (at(layer, x, z), quarters(at(&heights[n], x, z))) else { continue };
                    if c == b'#' {
                        solids[n][z * w + x] = Some((c, h, h + ROOF));
                        cells[z * w + x].push(floor(h + ROOF, h, false, c));
                    } else if walkable_char(c) {
                        cells[z * w + x].push(floor(h, h - SLAB, true, c));
                    }
                }
                cells[z * w + x].sort_by(|a, b| a.y.total_cmp(&b.y));
            }
        }
        for layer in &mut solids {
            map.merge_boxes(layer);
        }

        // Corner heights: the average of the floors around each corner close enough in height to share a slope.
        let all = cells.clone();
        for z in 0..d {
            for x in 0..w {
                // Roofs stay flat.
                for f in cells[z * w + x].iter_mut().filter(|f| f.open || f.bottom == f32::NEG_INFINITY) {
                    for (k, (cx, cz)) in [(x, z), (x + 1, z), (x + 1, z + 1), (x, z + 1)].into_iter().enumerate() {
                        let around = [(cx.wrapping_sub(1), cz.wrapping_sub(1)), (cx, cz.wrapping_sub(1)), (cx.wrapping_sub(1), cz), (cx, cz)];
                        let near: Vec<f32> = around
                            .into_iter()
                            .filter(|&(ax, az)| ax < w && az < d)
                            .flat_map(|(ax, az)| &all[az * w + ax])
                            .filter(|g| g.open && (g.y - f.y).abs() <= ramp)
                            .map(|g| g.y)
                            .collect();
                        f.corners[k] = if near.is_empty() { f.y } else { near.iter().sum::<f32>() / near.len() as f32 };
                    }
                }
            }
        }
        for c in cells {
            map.first.push(map.floors.len() as u32);
            map.floors.extend(c);
        }
        map.first.push(map.floors.len() as u32);

        for n in 0..map.floors.len() {
            map.link_first.push(map.links.len() as u32);
            let links: Vec<u32> = map.find_links(n, ramp.max(STEP)).collect();
            map.links.extend(links);
        }
        map.link_first.push(map.links.len() as u32);
        let mut back = vec![vec![]; map.floors.len()];
        for n in 0..map.floors.len() {
            for &m in map.links(n) {
                back[m as usize].push(n as u32);
            }
        }
        map.back = back;

        for n in 0..map.floors.len() {
            match map.floors[n].tag {
                b'T' => map.t_spawns.push(map.node_pos(n)),
                b'C' => map.ct_spawns.push(map.node_pos(n)),
                _ => {}
            }
        }
        let from_spawn = map.distance_from(map.node(map.t_spawns.first().copied().unwrap_or_default()));
        map.reachable = (0..map.floors.len()).map(|n| map.floors[n].open && from_spawn[n] < u16::MAX).collect();
        map
    }

    /// Greedy rectangles over equal solids: extend right, then down while the whole run matches.
    fn merge_boxes(&mut self, solids: &mut [Solid]) {
        let (w, d) = (self.width(), self.depth());
        for z in 0..d {
            for x in 0..w {
                let Some(s) = solids[z * w + x] else { continue };
                let same = |solids: &[Solid], x: usize, z: usize| x < w && z < d && solids[z * w + x] == Some(s);
                let mut x1 = x;
                while same(solids, x1 + 1, z) {
                    x1 += 1;
                }
                let mut z1 = z;
                while (x..=x1).all(|xx| same(solids, xx, z1 + 1)) {
                    z1 += 1;
                }
                for row in z..=z1 {
                    solids[row * w + x..=row * w + x1].fill(None);
                }
                let (kind, bottom, mut top) = s;
                if kind == b'#' && bottom == WALL_BOTTOM && top >= WALL_TOP {
                    top += [0.0, 0.0, 1.5, 3.0][(x * 7 + z * 13 + x1 * 3) % 4];
                }
                let (a, b) = (self.center(x, z), self.center(x1, z1));
                let half = Vec3::new((b.x - a.x) / 2.0 + self.cell / 2.0, (top - bottom) / 2.0, (b.z - a.z) / 2.0 + self.cell / 2.0);
                self.boxes.push(((a + b) / 2.0 + Vec3::Y * (bottom + top) / 2.0, half));
                self.kinds.push(kind);
            }
        }
    }

    /// The floors this one leads to: in each of the 8 neighboring cells, the highest open floor at most `climb` up
    /// (any drop is fine), with headroom to get there. Diagonals only when both sides are passable, so paths never
    /// clip a corner.
    fn find_links(&self, n: usize, climb: f32) -> impl Iterator<Item = u32> + '_ {
        let (x, z) = self.cell_of_node(n);
        let from = self.floors[n].y;
        let step = move |dx: isize, dz: isize| -> Option<u32> {
            let (nx, nz) = (x.checked_add_signed(dx)?, z.checked_add_signed(dz)?);
            if nx >= self.width() || nz >= self.depth() {
                return None;
            }
            let range = self.range(nx, nz);
            let open = range.clone().filter(|&m| self.floors[m].open && self.floors[m].y <= from + climb);
            let to = open.max_by(|&a, &b| self.floors[a].y.total_cmp(&self.floors[b].y))?;
            let head = from.max(self.floors[to].y) + HEIGHT;
            let low_ceiling = range.into_iter().any(|m| self.floors[m].y > self.floors[to].y && self.floors[m].bottom < head);
            (!low_ceiling).then_some(to as u32)
        };
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        DIRS.into_iter().filter_map(move |(dx, dz)| {
            let cuts_corner = dx != 0 && dz != 0 && (step(dx, 0).is_none() || step(0, dz).is_none());
            if cuts_corner { None } else { step(dx, dz) }
        })
    }

    fn range(&self, x: usize, z: usize) -> std::ops::Range<usize> {
        let i = z * self.width() + x;
        self.first[i] as usize..self.first[i + 1] as usize
    }

    fn cell_of_node(&self, n: usize) -> (usize, usize) {
        let i = self.first.partition_point(|&f| f as usize <= n) - 1;
        (i % self.width(), i / self.width())
    }

    /// The floors of the cell at `pos`, and where in the cell it is (each 0..1).
    fn floors_at(&self, pos: Vec3) -> (&[Floor], f32, f32) {
        let (w, d) = self.size();
        let gx = ((pos.x + w / 2.0) / self.cell).clamp(0.0, self.width() as f32 - 1e-3);
        let gz = ((pos.z + d / 2.0) / self.cell).clamp(0.0, self.depth() as f32 - 1e-3);
        (&self.floors[self.range(gx as usize, gz as usize)], gx.fract(), gz.fract())
    }

    /// The floors of a cell, lowest first.
    pub fn floors(&self, x: usize, z: usize) -> &[Floor] {
        &self.floors[self.range(x, z)]
    }

    /// The surface of the floor under a position: the highest one at most a step above its feet (or the lowest one).
    pub fn floor_at(&self, pos: Vec3) -> f32 {
        let (floors, fx, fz) = self.floors_at(pos);
        let surfaces = floors.iter().map(|f| surface(f, fx, fz));
        surfaces.clone().filter(|&s| s <= pos.y + STEP).reduce(f32::max).unwrap_or_else(|| surfaces.reduce(f32::min).unwrap_or(0.0))
    }

    /// Whether a floor is in the height range `lo..hi` at this spot.
    pub fn blocked(&self, pos: Vec3, lo: f32, hi: f32) -> bool {
        let (floors, fx, fz) = self.floors_at(pos);
        floors.iter().any(|f| surface(f, fx, fz) > lo && f.bottom < hi)
    }

    /// The lowest slab underside at or above `y` at this spot (infinity if none).
    pub fn ceiling(&self, pos: Vec3, y: f32) -> f32 {
        self.floors_at(pos).0.iter().map(|f| f.bottom).filter(|&b| b >= y).fold(f32::INFINITY, f32::min)
    }

    /// Whether this point is inside a floor (the ground or a slab).
    pub fn solid_at(&self, p: Vec3) -> bool {
        let (floors, fx, fz) = self.floors_at(p);
        floors.iter().any(|f| f.bottom <= p.y && p.y < surface(f, fx, fz))
    }

    /// Cell center standing on its lowest open floor (or the ground if it has none).
    pub fn ground(&self, x: usize, z: usize) -> Vec3 {
        let range = self.range(x, z);
        self.node_pos(range.clone().find(|&n| self.floors[n].open).unwrap_or(range.start))
    }

    /// The pathfinding node at a position: the floor of its cell it stands on.
    pub fn node(&self, pos: Vec3) -> usize {
        let (x, z) = self.cell(pos);
        let (floors, fx, fz) = self.floors_at(pos);
        let below = floors.iter().rposition(|f| surface(f, fx, fz) <= pos.y + STEP);
        self.range(x, z).start + below.unwrap_or(0)
    }

    /// Where a node's floor is at its cell's center.
    pub fn node_pos(&self, n: usize) -> Vec3 {
        let (x, z) = self.cell_of_node(n);
        self.center(x, z).with_y(surface(&self.floors[n], 0.5, 0.5))
    }

    /// Every node players can walk to.
    pub fn nodes(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.floors.len()).filter(|&n| self.reachable[n])
    }

    pub fn links(&self, n: usize) -> &[u32] {
        &self.links[self.link_first[n] as usize..self.link_first[n + 1] as usize]
    }

    /// Steps from every node to `goal`. `u16::MAX` is unreachable.
    pub fn distance_field(&self, goal: usize) -> Vec<u16> {
        self.bfs(goal, |n| &self.back[n][..])
    }

    /// Steps from `start` to every node (drops are one way, so not the same as `distance_field`).
    pub fn distance_from(&self, start: usize) -> Vec<u16> {
        self.bfs(start, |n| self.links(n))
    }

    fn bfs<'a>(&'a self, from: usize, next: impl Fn(usize) -> &'a [u32]) -> Vec<u16> {
        let mut dist = vec![u16::MAX; self.floors.len()];
        let mut queue = std::collections::VecDeque::from([from]);
        dist[from] = 0;
        while let Some(n) = queue.pop_front() {
            for &m in next(n) {
                if dist[m as usize] == u16::MAX {
                    dist[m as usize] = dist[n] + 1;
                    queue.push_back(m as usize);
                }
            }
        }
        dist
    }

    pub fn depth(&self) -> usize {
        self.rows.len()
    }

    pub fn width(&self) -> usize {
        self.rows[0].len()
    }

    /// Floor size in meters (x, z).
    pub fn size(&self) -> (f32, f32) {
        (self.width() as f32 * self.cell, self.depth() as f32 * self.cell)
    }

    pub fn center(&self, x: usize, z: usize) -> Vec3 {
        let (w, d) = self.size();
        Vec3::new(x as f32 * self.cell + self.cell / 2.0 - w / 2.0, 0.0, z as f32 * self.cell + self.cell / 2.0 - d / 2.0)
    }

    /// Grid cell containing this position.
    pub fn cell(&self, pos: Vec3) -> (usize, usize) {
        let (w, d) = self.size();
        let x = ((pos.x + w / 2.0) / self.cell).clamp(0.0, self.width() as f32 - 1.0);
        let z = ((pos.z + d / 2.0) / self.cell).clamp(0.0, self.depth() as f32 - 1.0);
        (x as usize, z as usize)
    }

    pub fn is_wall(&self, x: usize, z: usize) -> bool {
        self.rows.get(z).and_then(|r| r.get(x)) == Some(&b'#')
    }

    /// The base layer here is floor (not a wall or crate).
    pub fn walkable(&self, x: usize, z: usize) -> bool {
        self.rows.get(z).and_then(|r| r.get(x)).is_some_and(|&c| walkable_char(c))
    }

    /// The bomb site floor nearest the middle of the site (the middle itself may be a crate).
    pub fn site_center(&self, site: char) -> Vec3 {
        let cells: Vec<Vec3> = self.site_cells().filter(|p| self.site_at(*p) == Some(site)).collect();
        let mid = cells.iter().sum::<Vec3>() / cells.len().max(1) as f32;
        cells.into_iter().min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid))).unwrap_or(mid)
    }

    /// Which bomb site, if any, this floor position is on.
    pub fn site_at(&self, pos: Vec3) -> Option<char> {
        let f = &self.floors[self.node(pos)];
        (f.open && matches!(f.tag, b'A' | b'B')).then_some(f.tag as char)
    }

    /// Every bomb site floor at its cell's center.
    pub fn site_cells(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.nodes().filter(|&n| matches!(self.floors[n].tag, b'A' | b'B')).map(|n| self.node_pos(n))
    }

    pub fn spawns(&self, team: Team) -> &[Vec3] {
        match team {
            Team::T => &self.t_spawns,
            Team::Ct => &self.ct_spawns,
        }
    }
}

/// A floor's height at (fx, fz) within its cell, each 0..1.
pub fn surface(f: &Floor, fx: f32, fz: f32) -> f32 {
    let [a, b, c, d] = f.corners;
    let top = a + (b - a) * fx;
    let bottom = d + (c - d) * fx;
    top + (bottom - top) * fz
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_parse() {
        for m in &PARSED {
            check(m);
        }
    }

    fn check(m: &Map) {
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert_eq!((m.t_spawns.len(), m.ct_spawns.len()), (16, 16), "a spot for each of a full team");
        let a = m.site_cells().find(|p| m.site_at(*p) == Some('A'));
        let b = m.site_cells().find(|p| m.site_at(*p) == Some('B'));
        assert!(a.is_some() && b.is_some());
        assert_eq!(m.site_at(m.t_spawns[0]), None);
        for site in ['A', 'B'] {
            let field = m.distance_field(m.node(m.site_center(site)));
            for s in m.t_spawns.iter().chain(&m.ct_spawns) {
                assert!(field[m.node(*s)] < u16::MAX, "spawn {s} can't walk to {site}");
            }
        }
    }

    #[test]
    fn dust_has_stacked_floors() {
        let m = &*PARSED[0];
        let cells = (0..m.depth()).flat_map(|z| (0..m.width()).map(move |x| (x, z)));
        let stacked = cells.filter(|&(x, z)| m.floors(x, z).iter().filter(|f| f.open).count() > 1);
        let (x, z) = stacked.last().expect("a floor over another");
        let [low, high] = [0, 1].map(|i| m.floors(x, z).iter().filter(|f| f.open).nth(i).unwrap().y);
        let p = m.center(x, z);
        assert!((m.floor_at(p.with_y(low)) - low).abs() < 0.3, "under the slab");
        assert!((m.floor_at(p.with_y(high)) - high).abs() < 0.3, "on top of it");
        assert!(m.solid_at(p.with_y(high - 0.1)) && !m.solid_at(p.with_y(high - SLAB - 0.1)));
        assert!(m.boxes.len() < 1500, "merge should keep box count low: {}", m.boxes.len());
    }
}
