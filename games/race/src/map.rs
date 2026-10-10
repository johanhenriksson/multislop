//! The course: 30 m wide and 172 m long, over lava. From the start line (south, +z) north to the finish: stepping
//! stones, a climb of blocks, a one-cell beam that zigzags, jump pads onto a ledge, islands across wide gaps, and a
//! platform swept by two laser bars.
//!
//! Layout: `#` wall, `S` start spawn, `C` checkpoint, `F` finish, `J` jump pad, `R` laser sweeper pivot, `.` open.
//! Heights: floor per cell in half meters (base 36), so `4` is a 2 m platform; 0 is lava. A rise of one step (0.5 m)
//! is walked up, one of 1 m needs a jump.
// ponytail: one floor per cell (a height field of blocks), so no bridges or roofs. Add layers if a course needs them.

use glam::Vec3;
use std::sync::LazyLock;

/// Meters per grid cell.
pub const CELL: f32 = 2.0;
/// Meters per height unit.
const UNIT: f32 = 0.5;
/// Walls are tall enough that a jump pad never clears them.
pub const WALL_TOP: f32 = 10.0;
/// Highest rise a bot will jump onto, and the furthest it will jump, in cells.
const JUMP_RISE: f32 = 1.0;
const JUMP_CELLS: usize = 3;
/// A jump pad reaches anything within this many cells and this high above it.
const PAD_CELLS: usize = 3;
const PAD_RISE: f32 = 6.0;

const LAYOUT: &str = "\
###############
#FFFFFFFFFFFFF#
#FFFFFFFFFFFFF#
#FFFFFFFFFFFFF#
#.............#
#.............#
#.............#
#.............#
#......R......#
#.............#
#.............#
#.............#
#.............#
#.............#
#......R......#
#.............#
#.............#
#CCCCCCCCCCCCC#
#CCCCCCCCCCCCC#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#CCCCCCCCCCCCC#
#CCCCCCCCCCCCC#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#..J...J...J..#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#CCCCCCCCCCCCC#
#CCCCCCCCCCCCC#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.............#
#.S.S.S.S.S.S.#
#.S.S.S.S.S.S.#
#.............#
###############";

const HEIGHTS: &str = "\
000000000000000
066666666666660
066666666666660
066666666666660
066666666666660
066666666666660
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
000066666660000
066666666666660
066666666666660
000000000000000
000000000000000
000000006660000
000000006660000
000000000000000
000000000000000
000005555000000
000005555000000
000000000000000
000000000000000
004444000000000
004444000000000
000000000000000
000000000000000
044444444444440
044444444444440
0eeeeeeeeeeeee0
0eeeeeeeeeeeee0
0eeeeeeeeeeeee0
0eeeeeeeeeeeee0
044444444444440
044444444444440
044444444444440
044444444444440
044444444444440
044444444444440
000040000000000
000040000000000
000040000000000
000040000000000
000044444440000
000000000040000
000000000040000
000000000040000
000444444440000
000400000000000
000400000000000
000400000000000
044444444444440
044444444444440
000000000000000
000000000000000
000aaaaaaaaa000
000aaaaaaaaa000
000888888888000
000888888888000
000666666666000
000666666666000
000444444444000
000000000000000
000000000000000
000400040004000
000000000000000
000004000400000
000000000000000
000400040004000
000000000000000
000004000400000
000000000000000
000400040004000
000000000000000
000000000000000
044444444444440
044444444444440
044444444444440
044444444444440
000000000000000";

pub static MAP: LazyLock<Map> = LazyLock::new(|| Map::parse(LAYOUT, HEIGHTS));

/// The maps this game can play.
pub const MAPS: [&str; 1] = ["gauntlet"];

pub struct Map {
    rows: Vec<Vec<u8>>,
    /// Floor height per cell, meters.
    heights: Vec<Vec<f32>>,
    /// Solid boxes as (center, half extents), merged from runs of equal cells.
    pub boxes: Vec<(Vec3, Vec3)>,
    pub spawns: Vec<Vec3>,
    /// Jump pads, standing on their floor.
    pub pads: Vec<Vec3>,
    /// Laser sweeper pivots, on their floor.
    pub sweepers: Vec<Vec3>,
    /// Rows holding a checkpoint, from the start onwards; each checkpoint is two rows deep.
    checkpoint_rows: Vec<usize>,
    /// Moves from each cell to the finish for a bot (walking, jumping gaps, riding pads); `u16::MAX` is a dead end.
    pub to_finish: Vec<u16>,
}

impl Map {
    fn parse(layout: &str, heights: &str) -> Map {
        let rows: Vec<Vec<u8>> = layout.lines().map(|l| l.bytes().collect()).collect();
        let heights = heights.lines().map(|l| l.chars().map(|c| c.to_digit(36).unwrap_or(0) as f32 * UNIT).collect()).collect();
        let mut map = Map { rows, heights, boxes: vec![], spawns: vec![], pads: vec![], sweepers: vec![], checkpoint_rows: vec![], to_finish: vec![] };
        let (w, d) = (map.width(), map.depth());
        let mut used = vec![vec![false; w]; d];
        for z in 0..d {
            for x in 0..w {
                match map.rows[z][x] {
                    b'S' => map.spawns.push(map.ground(x, z)),
                    b'J' => map.pads.push(map.ground(x, z)),
                    b'R' => map.sweepers.push(map.ground(x, z)),
                    b'C' if !map.checkpoint_rows.contains(&z) => map.checkpoint_rows.insert(0, z),
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
        map.to_finish = map.finish_field();
        map
    }

    /// Top of whatever fills a cell: a wall, a platform, or 0 for lava.
    fn top(&self, x: usize, z: usize) -> f32 {
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

    /// Something to stand on: not a wall and not lava.
    pub fn is_floor(&self, x: usize, z: usize) -> bool {
        !self.is_wall(x, z) && self.heights[z][x] > 0.0
    }

    pub fn height(&self, x: usize, z: usize) -> f32 {
        self.heights[z][x]
    }

    pub fn is_finish(&self, x: usize, z: usize) -> bool {
        self.rows[z][x] == b'F'
    }

    /// Which checkpoint a cell is, counting from 1 at the start.
    pub fn checkpoint(&self, x: usize, z: usize) -> Option<u8> {
        if self.rows[z][x] != b'C' {
            return None;
        }
        self.checkpoint_rows.iter().position(|&r| r == z).map(|i| (i / 2 + 1) as u8)
    }

    pub fn checkpoints(&self) -> u8 {
        self.checkpoint_rows.len().div_ceil(2) as u8
    }

    /// Standing on a jump pad.
    pub fn on_pad(&self, pos: Vec3) -> bool {
        self.pads.iter().any(|p| (p.x - pos.x).abs() < CELL / 2.0 && (p.z - pos.z).abs() < CELL / 2.0 && (pos.y - p.y).abs() < 0.05)
    }

    fn is_pad(&self, x: usize, z: usize) -> bool {
        self.rows[z][x] == b'J'
    }

    /// Cells a bot can get to from (x, z) in one move: walking (up at most one step), jumping up to `JUMP_RISE` onto
    /// a neighbor or across up to `JUMP_CELLS` over lower ground, or anywhere a jump pad reaches.
    pub fn moves(&self, x: usize, z: usize) -> Vec<(usize, usize)> {
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        let h = self.height(x, z);
        let at = |dx: isize, dz: isize| Some((x.checked_add_signed(dx)?, z.checked_add_signed(dz)?)).filter(|&(x, z)| !self.is_wall(x, z));
        let mut out = vec![];
        if self.is_pad(x, z) {
            let r = PAD_CELLS as isize;
            for (dx, dz) in (-r..=r).flat_map(|dx| (-r..=r).map(move |dz| (dx, dz))) {
                out.extend(at(dx, dz).filter(|&(nx, nz)| self.is_floor(nx, nz) && self.height(nx, nz) <= h + PAD_RISE));
            }
            return out;
        }
        // A diagonal step past a cell of another height would clip its edge.
        let clear = |nx: usize, nz: usize| self.is_floor(nx, nz) && self.height(nx, nz) == h;
        for (dx, dz) in DIRS {
            for k in 1..=JUMP_CELLS as isize {
                let Some((nx, nz)) = at(dx * k, dz * k) else { break };
                if !self.is_floor(nx, nz) {
                    continue;
                }
                let rise = self.height(nx, nz) - h;
                if rise <= JUMP_RISE + 1e-3 && (k > 1 || dx == 0 || dz == 0 || (clear(nx, z) && clear(x, nz))) {
                    out.push((nx, nz));
                }
                // Only jump over lava or what's lower than where we take off.
                if rise >= 0.0 {
                    break;
                }
            }
        }
        out
    }

    /// Whether going from one cell to another (one of `moves`) needs a jump rather than a walk.
    pub fn is_jump(&self, from: (usize, usize), to: (usize, usize)) -> bool {
        let adjacent = from.0.abs_diff(to.0) <= 1 && from.1.abs_diff(to.1) <= 1;
        !adjacent || self.height(to.0, to.1) - self.height(from.0, from.1) > UNIT + 1e-3
    }

    fn finish_field(&self) -> Vec<u16> {
        let (w, d) = (self.width(), self.depth());
        let mut dist: Vec<u16> = (0..w * d).map(|i| if self.is_finish(i % w, i / w) { 0 } else { u16::MAX }).collect();
        let moves: Vec<Vec<(usize, usize)>> = (0..w * d).map(|i| if self.is_floor(i % w, i / w) { self.moves(i % w, i / w) } else { vec![] }).collect();
        // ponytail: relaxes the whole grid until nothing changes; fine for a course of a thousand cells.
        loop {
            let mut changed = false;
            for i in 0..w * d {
                let best = moves[i].iter().map(|&(x, z)| dist[z * w + x].saturating_add(1)).min().unwrap_or(u16::MAX);
                if best < dist[i] {
                    (dist[i], changed) = (best, true);
                }
            }
            if !changed {
                return dist;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_parses() {
        let m = &*MAP;
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert!(m.heights.len() == m.depth() && m.heights.iter().all(|r| r.len() == m.width()), "height grid size");
        assert_eq!((m.spawns.len(), m.pads.len(), m.sweepers.len(), m.checkpoints()), (12, 3, 2, 3));
        assert!(m.spawns.iter().all(|s| s.y > 0.0), "spawns stand on the start platform");
        assert!(m.boxes.len() < 120, "merge should keep box count low: {}", m.boxes.len());
    }

    #[test]
    fn checkpoints_count_up_from_the_start() {
        let order: Vec<u8> = (0..MAP.depth()).rev().filter_map(|z| MAP.checkpoint(7, z)).collect();
        assert_eq!(order, [1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn a_bot_can_get_to_the_finish_from_every_spawn() {
        for s in &MAP.spawns {
            let (x, z) = MAP.cell(*s);
            assert!(MAP.to_finish[z * MAP.width() + x] < u16::MAX, "stuck at {s:?}");
        }
    }
}
