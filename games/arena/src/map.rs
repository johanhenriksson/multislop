//! The maps, all four-way symmetric. Arena: 80 x 80 m. Raised corner balconies reached by stairs, wall-side perches and a central
//! tower reached by jump pads, pillars and low crates for cover. Foundry (`maps/foundry*.txt`, 72 x 72 m): the floor
//! at 1 m around a sunken crucible with a pillar in it, furnace towers and catwalks in the corners. Spire
//! (`maps/spire*.txt`, 80 x 80 m): a ziggurat climbable up all four faces, under corner towers only pads reach.
//!
//! Layout: `#` wall, `J` jump pad, `S` spawn, `.` open. Heights: floor per cell in half meters (base 36), so `4` is a
//! 2 m platform and `8` a 4 m one; raised cells are solid blocks, and a rise of one step (0.5 m) is walked up.
// ponytail: one floor per cell (a height field of blocks), so no bridges or roofs. Add layers if a map needs them.

use glam::Vec3;
use std::cell::Cell;
use std::sync::LazyLock;

/// Meters per grid cell.
pub const CELL: f32 = 2.0;
/// Meters per height unit.
const UNIT: f32 = 0.5;
/// Walls are tall enough that a jump pad never clears them.
pub const WALL_TOP: f32 = 10.0;

const LAYOUT: &str = "\
########################################
#.......#......................#.......#
#.......#......................#.......#
#..S....#.....S..........S.....#....S..#
#.......#......................#.......#
#......................................#
#......................................#
#####..............................#####
#...........#..............#...........#
#....##.....#..............#.....##....#
#....##.....#...S......S...#.....##....#
#......................................#
#.S..................................S.#
#......................................#
#.......#.....J..........J.....#.......#
#.......#......................#.......#
#...........S..............S...........#
#......................................#
#....J............................J....#
#......................................#
#......................................#
#....J............................J....#
#......................................#
#...........S..............S...........#
#.......#......................#.......#
#.......#.....J..........J.....#.......#
#......................................#
#.S..................................S.#
#......................................#
#....##.....#...S......S...#.....##....#
#....##.....#..............#.....##....#
#...........#..............#...........#
#####..............................#####
#......................................#
#......................................#
#.......#......................#.......#
#..S....#.....S..........S.....#....S..#
#.......#......................#.......#
#.......#......................#.......#
########################################";

const HEIGHTS: &str = "\
0000000000000000000000000000000000000000
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0444444400000022000000002200000044444440
0444444400000022000000002200000044444440
0000032100000000000000000000000012300000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000000000002000000000000002000000000000
0000000000002000000000000002000000000000
0020000000000000000000000000000000000200
0020000000000000000000000000000000000200
0888000000000000222222220000000000008880
0888000000000000222222220000000000008880
0888000000000000228888220000000000008880
0888000000000000228888220000000000008880
0888000000000000228888220000000000008880
0888000000000000228888220000000000008880
0888000000000000222222220000000000008880
0888000000000000222222220000000000008880
0020000000000000000000000000000000000200
0020000000000000000000000000000000000200
0000000000002000000000000002000000000000
0000000000002000000000000002000000000000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000000000000000000000000000000000000000
0000032100000000000000000000000012300000
0444444400000022000000002200000044444440
0444444400000022000000002200000044444440
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0444444400000000000000000000000044444440
0000000000000000000000000000000000000000";

/// The maps this game can play.
pub const MAPS: [&str; 3] = ["arena", "foundry", "spire"];

static PARSED: [LazyLock<Map>; 3] = [
    LazyLock::new(|| Map::parse(LAYOUT, HEIGHTS)),
    LazyLock::new(|| Map::parse(include_str!("maps/foundry.txt"), include_str!("maps/foundry_heights.txt"))),
    LazyLock::new(|| Map::parse(include_str!("maps/spire.txt"), include_str!("maps/spire_heights.txt"))),
];

thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

/// The map being played: the arena until `load` picks another. A game module plays one map at a time, so it's a
/// global rather than threaded through everything that looks at the map.
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

pub struct Map {
    rows: Vec<Vec<u8>>,
    /// Floor height per cell, meters.
    heights: Vec<Vec<f32>>,
    /// Solid boxes as (center, half extents), merged from runs of equal cells.
    pub boxes: Vec<(Vec3, Vec3)>,
    pub spawns: Vec<Vec3>,
    /// Jump pads, standing on their floor.
    pub pads: Vec<Vec3>,
}

impl Map {
    fn parse(layout: &str, heights: &str) -> Map {
        let rows: Vec<Vec<u8>> = layout.lines().map(|l| l.bytes().collect()).collect();
        let heights = heights.lines().map(|l| l.chars().map(|c| c.to_digit(36).unwrap_or(0) as f32 * UNIT).collect()).collect();
        let mut map = Map { rows, heights, boxes: vec![], spawns: vec![], pads: vec![] };
        let (w, d) = (map.width(), map.depth());
        let mut used = vec![vec![false; w]; d];
        for z in 0..d {
            for x in 0..w {
                match map.rows[z][x] {
                    b'S' => map.spawns.push(map.ground(x, z)),
                    b'J' => map.pads.push(map.ground(x, z)),
                    _ => {}
                }
                let top = map.top(x, z);
                if used[z][x] || top <= 0.0 {
                    continue;
                }
                // Greedy rectangle: extend right, then down while the whole run is as tall.
                let same = |x: usize, z: usize| x < w && z < d && !used[z][x] && map.top(x, z) == top;
                let mut x1 = x;
                while same(x1 + 1, z) {
                    x1 += 1;
                }
                let mut z1 = z;
                while (x..=x1).all(|xx| same(xx, z1 + 1)) {
                    z1 += 1;
                }
                for row in &mut used[z..=z1] {
                    row[x..=x1].fill(true);
                }
                let (a, b) = (map.center(x, z), map.center(x1, z1));
                let half = Vec3::new((b.x - a.x) / 2.0 + CELL / 2.0, top / 2.0, (b.z - a.z) / 2.0 + CELL / 2.0);
                map.boxes.push(((a + b) / 2.0 + Vec3::Y * top / 2.0, half));
            }
        }
        map
    }

    /// Top of whatever fills a cell: a wall, a platform, or 0 for open floor.
    pub fn top(&self, x: usize, z: usize) -> f32 {
        if self.is_wall(x, z) { WALL_TOP } else { self.heights[z][x] }
    }

    pub fn depth(&self) -> usize {
        self.rows.len()
    }

    pub fn width(&self) -> usize {
        self.rows[0].len()
    }

    /// Floor size in meters (x, z).
    pub fn size(&self) -> (f32, f32) {
        (self.width() as f32 * CELL, self.depth() as f32 * CELL)
    }

    pub fn center(&self, x: usize, z: usize) -> Vec3 {
        let (w, d) = self.size();
        Vec3::new(x as f32 * CELL + CELL / 2.0 - w / 2.0, 0.0, z as f32 * CELL + CELL / 2.0 - d / 2.0)
    }

    /// Cell center standing on its floor.
    pub fn ground(&self, x: usize, z: usize) -> Vec3 {
        self.center(x, z).with_y(self.heights[z][x])
    }

    /// Grid cell containing this position.
    pub fn cell(&self, pos: Vec3) -> (usize, usize) {
        let (w, d) = self.size();
        let x = ((pos.x + w / 2.0) / CELL).clamp(0.0, self.width() as f32 - 1.0);
        let z = ((pos.z + d / 2.0) / CELL).clamp(0.0, self.depth() as f32 - 1.0);
        (x as usize, z as usize)
    }

    pub fn is_wall(&self, x: usize, z: usize) -> bool {
        self.rows.get(z).and_then(|r| r.get(x)).is_none_or(|&c| c == b'#')
    }

    pub fn height(&self, x: usize, z: usize) -> f32 {
        self.heights[z][x]
    }

    /// Standing on a jump pad.
    pub fn on_pad(&self, pos: Vec3) -> bool {
        self.pads.iter().any(|p| (p.x - pos.x).abs() < CELL / 2.0 && (p.z - pos.z).abs() < CELL / 2.0 && (pos.y - p.y).abs() < 0.05)
    }

    /// Steps from every cell to `goal` walking (up at most one step, down any drop), 8-connected without cutting
    /// corners. `u16::MAX` is unreachable on foot.
    pub fn distance_field(&self, goal: (usize, usize)) -> Vec<u16> {
        let w = self.width();
        let mut dist = vec![u16::MAX; w * self.depth()];
        let mut queue = std::collections::VecDeque::from([goal]);
        dist[goal.1 * w + goal.0] = 0;
        while let Some((x, z)) = queue.pop_front() {
            // Searching backwards from the goal: a neighbor counts if you can walk from it to here.
            for (nx, nz) in self.neighbors(x, z).filter(|&(nx, nz)| self.height(x, z) - self.height(nx, nz) <= UNIT + 1e-3) {
                if dist[nz * w + nx] == u16::MAX {
                    dist[nz * w + nx] = dist[z * w + x] + 1;
                    queue.push_back((nx, nz));
                }
            }
        }
        dist
    }

    /// Open cells around, diagonals only when both sides are open too so paths never clip a corner.
    pub fn neighbors(&self, x: usize, z: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        let h = self.height(x, z);
        // A diagonal past a cell of another height would clip its edge.
        let flat = move |x: usize, z: usize| !self.is_wall(x, z) && self.height(x, z) == h;
        DIRS.into_iter().filter_map(move |(dx, dz)| {
            let (nx, nz) = (x.checked_add_signed(dx)?, z.checked_add_signed(dz)?);
            let open = !self.is_wall(nx, nz) && (dx == 0 || dz == 0 || (flat(nx, z) && flat(x, nz)));
            open.then_some((nx, nz))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_parses() {
        let m = &*MAP;
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert!(m.heights.len() == m.depth() && m.heights.iter().all(|r| r.len() == m.width()), "height grid size");
        assert_eq!((m.spawns.len(), m.pads.len()), (20, 8));
        assert!(m.boxes.len() < 120, "merge should keep box count low: {}", m.boxes.len());
        assert!(m.spawns.iter().any(|s| s.y > 1.0), "some spawns are up on the balconies");
    }

    #[test]
    fn every_map_parses_and_never_traps_anyone() {
        for name in MAPS {
            assert!(load(name));
            let m = &*MAP;
            assert!(m.rows.iter().all(|r| r.len() == m.width()), "{name}: ragged rows");
            assert!(m.heights.len() == m.depth() && m.heights.iter().all(|r| r.len() == m.width()), "{name}: height grid size");
            assert!(m.spawns.len() >= 16 && !m.pads.is_empty(), "{name}: {} spawns, {} pads", m.spawns.len(), m.pads.len());
            assert!(m.boxes.len() < 160, "{name}: merge should keep box count low: {}", m.boxes.len());
            // Everywhere you can stand, you can walk back to the floor everyone spawns on.
            let floor = m.spawns.iter().map(|&s| m.cell(s)).min_by(|a, b| m.height(a.0, a.1).total_cmp(&m.height(b.0, b.1))).unwrap();
            let field = m.distance_field(floor);
            for z in 0..m.depth() {
                for x in (0..m.width()).filter(|&x| !m.is_wall(x, z)) {
                    assert!(field[z * m.width() + x] < u16::MAX, "{name}: stuck at ({x}, {z})");
                }
            }
        }
    }

    #[test]
    fn stairs_lead_up_to_the_balconies() {
        let balcony = MAP.cell(MAP.spawns.iter().find(|s| s.y > 1.0).copied().unwrap());
        let field = MAP.distance_field(balcony);
        let floor = MAP.cell(MAP.spawns.iter().find(|s| s.y == 0.0).copied().unwrap());
        assert!(field[floor.1 * MAP.width() + floor.0] < u16::MAX, "walkable from the floor");
        let tower = MAP.cell(Vec3::ZERO);
        assert_eq!(MAP.distance_field(tower)[floor.1 * MAP.width() + floor.0], u16::MAX, "the tower needs a jump pad");
    }
}
