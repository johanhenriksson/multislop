//! The pitch: 48 x 88 m inside the walls, the same when turned 180°, with a goal recessed into the middle of each end
//! wall. Orange defends the +z end, Blue the -z end.
//!
//! Layout: `#` wall, `.` open, `S` respawn spot, `K` kickoff spot (each belongs to the team whose half it's in), `J`
//! jump pad, and pickups by letter (`h` health, `a` armor, `r` rockets, `M` mega health, `A` heavy armor). Heights:
//! floor per cell in half meters, so `3` is a chest-high block, `4` a 2 m platform and `8` a 4 m pillar; raised cells
//! are solid blocks, and a rise of one step (0.5 m) is walked up. The goals' crossbars hang in the air, so they're
//! added as boxes of their own.
// ponytail: one floor per cell (a height field of blocks), so no bridges or roofs. Add layers if a map needs them.

use crate::shared::{Item, Team};
use glam::Vec3;
use std::sync::LazyLock;

/// Meters per grid cell.
pub const CELL: f32 = 2.0;
/// Meters per height unit.
const UNIT: f32 = 0.5;
/// Walls go up to the ceiling, which keeps the ball in.
pub const WALL_TOP: f32 = 18.0;
/// The goal lines: the end walls' faces, at z = ±GOAL_LINE.
pub const GOAL_LINE: f32 = 44.0;
/// Goal mouths: 16 m wide and 8 m tall; the recess behind is 4 m deep.
pub const GOAL_HALF_WIDTH: f32 = 8.0;
pub const GOAL_HEIGHT: f32 = 8.0;
pub const GOAL_DEPTH: f32 = 4.0;
/// Depth of the crossbar, the box over each mouth from `GOAL_HEIGHT` up to the ceiling.
const CROSSBAR: f32 = 1.0;

const LAYOUT: &str = "\
##########################
#########........#########
#########........#########
#.r....................h.#
#...S.S............S.S...#
#............K...........#
#........................#
#........................#
#........................#
#........................#
#........................#
#........................#
#........................#
#...........K............#
#a......................a#
#........................#
#r......................h#
#........................#
#.......K........K.......#
#........................#
#........................#
#........................#
#........................#
#M......................A#
#..J.....................#
#.....................J..#
#A......................M#
#........................#
#........................#
#........................#
#........................#
#.......K........K.......#
#........................#
#h......................r#
#........................#
#a......................a#
#............K...........#
#........................#
#........................#
#........................#
#........................#
#........................#
#........................#
#........................#
#...........K............#
#...S.S............S.S...#
#.h....................r.#
#########........#########
#########........#########
##########################";

const HEIGHTS: &str = "\
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000800000000800000000
00000000000000000000000000
00000000000000000000000000
00000330000000000003300000
00000330000000000003300000
00000000000000000000000000
00000000000000000000000000
00000000033000033000000000
00000000000000000000000000
00000000000000000000000000
01100000000000000000000110
02200800000000000000800220
03300000000000000000000330
04400000000000000000000440
04400000000000000000000440
04400000000000000000000440
04400000000000000000000440
04400000000000000000000440
04400000000000000000000440
03300000000000000000000330
02200800000000000000800220
01100000000000000000000110
00000000000000000000000000
00000000000000000000000000
00000000033000033000000000
00000000000000000000000000
00000000000000000000000000
00000330000000000003300000
00000330000000000003300000
00000000000000000000000000
00000000000000000000000000
00000000800000000800000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000
00000000000000000000000000";

pub static MAP: LazyLock<Map> = LazyLock::new(|| Map::parse(LAYOUT, HEIGHTS));

/// The maps this game can play.
pub const MAPS: [&str; 1] = ["pitch"];

pub struct Map {
    rows: Vec<Vec<u8>>,
    /// Floor height per cell, meters.
    heights: Vec<Vec<f32>>,
    /// Solid boxes as (center, half extents): runs of equal cells merged, then the crossbars.
    pub boxes: Vec<(Vec3, Vec3)>,
    /// Respawn spots near each team's goal, indexed by `Team as usize`.
    pub spawns: [Vec<Vec3>; 2],
    /// Kickoff spots per team, nearest the center spot first: the two wide ones, the back, the keeper.
    pub kickoffs: [Vec<Vec3>; 2],
    /// Jump pads, standing on their floor.
    pub pads: Vec<Vec3>,
    /// Pickups on their floor, in layout order: the order `Match::pickups` reports them in.
    pub pickups: Vec<(Vec3, Item)>,
}

impl Map {
    fn parse(layout: &str, heights: &str) -> Map {
        let rows: Vec<Vec<u8>> = layout.lines().map(|l| l.bytes().collect()).collect();
        let heights = heights.lines().map(|l| l.chars().map(|c| c.to_digit(36).unwrap_or(0) as f32 * UNIT).collect()).collect();
        let mut map = Map { rows, heights, boxes: vec![], spawns: [vec![], vec![]], kickoffs: [vec![], vec![]], pads: vec![], pickups: vec![] };
        let (w, d) = (map.width(), map.depth());
        let mut used = vec![vec![false; w]; d];
        for z in 0..d {
            for x in 0..w {
                let (at, team) = (map.ground(x, z), if map.center(x, z).z > 0.0 { Team::Orange } else { Team::Blue });
                match map.rows[z][x] {
                    b'S' => map.spawns[team as usize].push(at),
                    b'K' => map.kickoffs[team as usize].push(at),
                    b'J' => map.pads.push(at),
                    c => map.pickups.extend(Item::from_letter(c).map(|i| (at, i))),
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
        for end in [-1.0, 1.0] {
            let half = Vec3::new(GOAL_HALF_WIDTH, (WALL_TOP - GOAL_HEIGHT) / 2.0, CROSSBAR / 2.0);
            map.boxes.push((Vec3::new(0.0, GOAL_HEIGHT + half.y, end * (GOAL_LINE + half.z)), half));
        }
        for spots in &mut map.kickoffs {
            spots.sort_by(|a, b| a.length().total_cmp(&b.length()));
        }
        map
    }

    /// Top of whatever fills a cell: a wall, a block, or 0 for open floor.
    fn top(&self, x: usize, z: usize) -> f32 {
        if self.is_wall(x, z) { WALL_TOP } else { self.heights[z][x] }
    }

    pub fn depth(&self) -> usize {
        self.rows.len()
    }

    pub fn width(&self) -> usize {
        self.rows[0].len()
    }

    /// Floor size in meters (x, z), walls and goal recesses included.
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

    /// The middle of the goal line `team` defends, on the floor.
    pub fn goal(&self, team: Team) -> Vec3 {
        Vec3::new(0.0, 0.0, team.end() * GOAL_LINE)
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
    fn pitch_parses_and_is_the_same_turned_around() {
        let m = &*MAP;
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert!(m.heights.len() == m.depth() && m.heights.iter().all(|r| r.len() == m.width()), "height grid size");
        assert_eq!((m.size().0 - 2.0 * CELL, 2.0 * GOAL_LINE), (48.0, 88.0), "inside the walls");
        assert_eq!(m.size().1, 2.0 * (GOAL_LINE + GOAL_DEPTH + CELL), "a recess and a wall behind each goal line");
        fn turn<T: Copy>(grid: &[Vec<T>]) -> Vec<Vec<T>> {
            grid.iter().rev().map(|row| row.iter().rev().copied().collect()).collect()
        }
        assert!(turn(&m.rows) == m.rows && turn(&m.heights) == m.heights, "the same turned 180°");
        for t in [Team::Orange, Team::Blue] {
            assert_eq!(m.kickoffs[t as usize].len(), 4);
            assert!(m.spawns[t as usize].len() >= 4);
            assert!(m.kickoffs[t as usize].iter().chain(&m.spawns[t as usize]).all(|p| p.z * t.end() > 0.0), "in its own half");
        }
        assert_eq!((m.pads.len(), m.pickups.len()), (2, 16));
        assert!(m.boxes.len() < 60, "merge should keep box count low: {}", m.boxes.len());
    }

    #[test]
    fn center_and_goal_mouths_are_open() {
        let m = &*MAP;
        let blocks = |p: Vec3| m.boxes.iter().any(|&(c, h)| (p.x - c.x).abs() < h.x && (p.z - c.z).abs() < h.z && c.y - h.y < 0.1);
        for i in 0..400 {
            let a = i as f32 * 0.37;
            let r = (i % 20) as f32 * 0.5;
            assert!(!blocks(Vec3::new(a.cos() * r, 0.0, a.sin() * r)), "clear within 10 m of the center spot");
        }
        // The 16 m in front of each goal, as wide as the mouth, and the recess behind it.
        for t in [Team::Orange, Team::Blue] {
            for x in -7..=7 {
                for d in -4..16 {
                    let p = Vec3::new(x as f32, 0.0, t.end() * (GOAL_LINE - d as f32));
                    assert!(!blocks(p), "{p} in front of {t:?}'s goal");
                }
            }
        }
        // Every kickoff and respawn spot can walk to the center spot.
        let field = m.distance_field(m.cell(Vec3::new(0.5, 0.0, 0.5)));
        for p in m.kickoffs.iter().chain(&m.spawns).flatten() {
            let (x, z) = m.cell(*p);
            assert!(field[z * m.width() + x] < u16::MAX, "{p} walks to the center");
        }
    }
}
