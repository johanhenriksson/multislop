//! The courses, 30 m wide and run from the start line (south, +z) north to the finish, over lava. "gauntlet" is drawn
//! by hand: stepping stones, a climb of blocks, a one-cell beam that zigzags, jump pads onto a ledge, islands across
//! wide gaps, and a platform swept by two laser bars. "random" deals a new course of the same kinds of obstacles every
//! race, named `random-<seed>` so clients build the same one.
//!
//! Layout: `#` wall, `S` start spawn, `C` checkpoint, `F` finish, `J` jump pad, `R` laser sweeper pivot, `.` open.
//! Heights: floor per cell in half meters (base 36), so `4` is a 2 m platform; 0 is lava. A rise of one step (0.5 m)
//! is walked up, one of 1 m needs a jump.
// ponytail: one floor per cell (a height field of blocks), so no bridges or roofs. Add layers if a course needs them.

use glam::Vec3;
use std::cell::RefCell;
use std::rc::Rc;

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

/// The maps this game can play: a new random course every race, or the hand-made one.
pub const MAPS: [&str; 2] = ["random", "gauntlet"];

thread_local! {
    static COURSE: RefCell<Rc<Map>> = RefCell::new(Rc::new(Map::parse(LAYOUT, HEIGHTS)));
}

/// The course being raced, the gauntlet until `load` picks another.
pub fn course() -> Rc<Map> {
    COURSE.with(|c| c.borrow().clone())
}

/// Makes `name` (one of `MAPS`, or `random-<seed>`) the course; false for a name that is neither.
pub fn load(name: &str) -> bool {
    let map = match name.strip_prefix("random-").map(str::parse) {
        _ if name == "gauntlet" => Map::parse(LAYOUT, HEIGHTS),
        Some(Ok(seed)) => generate(seed),
        _ => return false,
    };
    COURSE.with(|c| *c.borrow_mut() = Rc::new(map));
    true
}

/// Cells across the course between its side walls.
const INNER: usize = 13;
/// Obstacles per random course, all different; a checkpoint follows every second one.
const SECTIONS: usize = 6;

/// A row of a generated course between the side walls: layout character and height in units for each cell.
type Row = [(u8, u8); INNER];

const LAVA_ROW: Row = [(b'.', 0); INNER];

fn flat(c: u8, h: u8) -> Row {
    [(c, h); INNER]
}

/// Cells `lo..=hi` at height `h`, lava around them.
fn band(lo: usize, hi: usize, h: u8) -> Row {
    let mut r = LAVA_ROW;
    r[lo..=hi].fill((b'.', h));
    r
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// In `lo..=hi`.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u32) as i32
    }

    fn chance(&mut self, p: f32) -> bool {
        (self.next() % 1000) as f32 / 1000.0 < p
    }

    /// A window `widths` cells wide overlapping `lo..=hi` by at least `overlap`.
    fn shift(&mut self, (lo, hi): (usize, usize), widths: (i32, i32), overlap: usize) -> (usize, usize) {
        let w = self.range(widths.0, widths.1) as usize;
        let a = (lo as i32 + overlap as i32 - w as i32).max(0);
        let b = (hi as i32 + 1 - overlap as i32).min((INNER - w) as i32);
        let lo = self.range(a, b) as usize;
        (lo, lo + w - 1)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Stones,
    Climb,
    Beam,
    Pads,
    Islands,
    Lasers,
    Pillars,
}

/// A course from `seed`: the start line, `SECTIONS` obstacles with landings between them, and the finish. Built so a
/// bot can get through; checked too, trying the next seed if not.
fn generate(seed: u32) -> Map {
    for attempt in 0..32u32 {
        let map = deal(seed.wrapping_add(attempt.wrapping_mul(0x9E37_79B9)));
        let w = map.width();
        if map.spawns.iter().all(|&s| map.to_finish[map.cell(s).1 * w + map.cell(s).0] < u16::MAX) {
            return map;
        }
    }
    Map::parse(LAYOUT, HEIGHTS)
}

fn deal(seed: u32) -> Map {
    use Section::*;
    let mut rng = Rng(seed.wrapping_mul(0x2C1B_3C6D) | 1);
    let mut h = 4;
    let mut spawns = flat(b'.', h);
    (1..INNER).step_by(2).take(6).for_each(|x| spawns[x].0 = b'S');
    // Built from the start north, then flipped so the finish is the first row like a hand-drawn layout.
    let mut rows = vec![flat(b'.', h), spawns, spawns, flat(b'.', h)];
    let mut deck = vec![Stones, Climb, Beam, Pads, Islands, Lasers, Pillars];
    for i in 0..SECTIONS {
        let fits = |k: &Section| match k {
            Pads => h <= 8,
            Climb => h <= 10,
            _ => true,
        };
        let open: Vec<usize> = (0..deck.len()).filter(|&i| fits(&deck[i])).collect();
        let kind = deck.remove(open[rng.range(0, open.len() as i32 - 1) as usize]);
        h = section(kind, h, &mut rng, &mut rows);
        if i + 1 < SECTIONS {
            let c = if i % 2 == 0 { b'C' } else { b'.' };
            rows.extend([flat(c, h); 2]);
        }
    }
    rows.extend([flat(b'F', h); 3]);
    rows.reverse();
    let wall = "#".repeat(INNER + 2);
    let mut layout = vec![wall.clone()];
    let mut heights = vec!["0".repeat(INNER + 2)];
    for r in &rows {
        layout.push(format!("#{}#", r.iter().map(|&(c, _)| c as char).collect::<String>()));
        heights.push(format!("0{}0", r.iter().map(|&(_, h)| char::from_digit(h as u32, 36).unwrap()).collect::<String>()));
    }
    layout.push(wall);
    heights.push("0".repeat(INNER + 2));
    Map::parse(&layout.join("\n"), &heights.join("\n"))
}

/// Adds one obstacle after a full-width landing at height `h`, ending where the next full-width landing goes; returns
/// that landing's height.
fn section(kind: Section, mut h: u8, rng: &mut Rng, rows: &mut Vec<Row>) -> u8 {
    let mid = INNER as i32 / 2;
    match kind {
        // Single stones a running jump apart on a diagonal lattice, some knocked out, but always a path through.
        Section::Stones => {
            let mut x = rng.range(2, INNER as i32 - 3);
            rows.push(LAVA_ROW);
            for _ in 0..rng.range(4, 6) {
                let mut r = LAVA_ROW;
                for sx in (x.rem_euclid(4)..INNER as i32).step_by(4) {
                    if sx == x || rng.chance(0.65) {
                        r[sx as usize] = (b'.', h + rng.range(0, 1) as u8);
                    }
                }
                rows.extend([r, LAVA_ROW]);
                let d = if rng.chance(0.5) { 2 } else { -2 };
                x += if (0..INNER as i32).contains(&(x + d)) { d } else { -d };
            }
            h
        }
        // Steps a meter high each, every one shifted from the last, then on from the top or back down across a gap.
        Section::Climb => {
            let mut win = rng.shift((0, INNER - 1), (4, 8), 4);
            let base = h;
            for _ in 0..rng.range(2, 3) {
                h += 2;
                rows.extend([band(win.0, win.1, h); 2]);
                win = rng.shift(win, (4, 8), 3);
            }
            if rng.chance(0.5) {
                rows.push(LAVA_ROW);
                h
            } else {
                rows.extend([LAVA_ROW; 2]);
                base
            }
        }
        // A one-cell beam: north a few cells, across, north again.
        Section::Beam => {
            let mut x = rng.range(1, INNER as i32 - 2);
            for _ in 0..rng.range(2, 3) {
                for _ in 0..rng.range(2, 4) {
                    rows.push(band(x as usize, x as usize, h));
                }
                let to = loop {
                    let to = rng.range(0, INNER as i32 - 1);
                    if (3..=8).contains(&(to - x).abs()) {
                        break to;
                    }
                };
                rows.push(band(x.min(to) as usize, x.max(to) as usize, h));
                x = to;
            }
            for _ in 0..rng.range(2, 3) {
                rows.push(band(x as usize, x as usize, h));
            }
            h
        }
        // Jump pads up onto a ledge too high to climb, and down the far side.
        Section::Pads => {
            let mut pads = flat(b'.', h);
            for x in [rng.range(1, 3), rng.range(5, 7), rng.range(9, 11)] {
                pads[x as usize].0 = b'J';
            }
            rows.extend([flat(b'.', h), pads, flat(b'.', h), flat(b'.', h)]);
            rows.extend([flat(b'.', h + 10); 3]);
            h
        }
        // Islands across gaps two cells wide, each a little up or down from the last and overlapping it.
        Section::Islands => {
            let mut win = rng.shift((0, INNER - 1), (2, 4), 1);
            for _ in 0..rng.range(3, 4) {
                rows.extend([LAVA_ROW; 2]);
                h = (h as i32 + if h >= 12 { rng.range(-2, 0) } else { rng.range(-2, 1) }).max(2) as u8;
                rows.extend([band(win.0, win.1, h); 2]);
                win = rng.shift(win, (2, 4), 1);
            }
            rows.extend([LAVA_ROW; 2]);
            h
        }
        // A narrow platform swept by laser bars.
        Section::Lasers => {
            let n = rng.range(2, 3);
            for i in 0..n * 6 + 1 {
                let mut r = band((mid - 3) as usize, (mid + 3) as usize, h);
                if i % 6 == 3 {
                    r[mid as usize].0 = b'R';
                }
                rows.push(r);
            }
            h
        }
        // Pillars too tall to jump, on every other row with gaps to weave through.
        Section::Pillars => {
            for i in 0..rng.range(7, 10) {
                let mut r = flat(b'.', h);
                if i % 2 == 1 {
                    for cell in &mut r {
                        if rng.chance(0.45) {
                            cell.1 = (h + 6).min(19);
                        }
                    }
                    for _ in 0..2 {
                        r[rng.range(0, INNER as i32 - 1) as usize].1 = h;
                    }
                }
                rows.push(r);
            }
            h
        }
    }
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
                // Only jump over lava or what's lower than where we take off, and never past a checkpoint.
                if rise >= 0.0 || self.rows[nz][nx] == b'C' {
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
        let m = &*course();
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert!(m.heights.len() == m.depth() && m.heights.iter().all(|r| r.len() == m.width()), "height grid size");
        assert_eq!((m.spawns.len(), m.pads.len(), m.sweepers.len(), m.checkpoints()), (12, 3, 2, 3));
        assert!(m.spawns.iter().all(|s| s.y > 0.0), "spawns stand on the start platform");
        assert!(m.boxes.len() < 120, "merge should keep box count low: {}", m.boxes.len());
    }

    #[test]
    fn random_courses_can_be_finished() {
        let mut first_try = 0;
        for seed in 0..300 {
            let m = deal(seed);
            assert!(m.rows.iter().all(|r| r.len() == m.width()) && m.heights.len() == m.depth(), "seed {seed}: grid size");
            assert_eq!((m.spawns.len(), m.checkpoints()), (12, 3), "seed {seed}");
            assert!(m.heights.iter().flatten().all(|&h| h < WALL_TOP), "seed {seed}: over the walls");
            let w = m.width();
            first_try += m.spawns.iter().all(|&s| m.to_finish[m.cell(s).1 * w + m.cell(s).0] < u16::MAX) as u32;
        }
        assert!(first_try >= 290, "only {first_try} of 300 courses dealt could be finished");
    }

    #[test]
    fn courses_load_by_name() {
        assert!(load("random-42"));
        let a = course().boxes.clone();
        assert!(load("random-43") && course().boxes != a);
        assert!(load("random-42") && course().boxes == a, "same seed, same course");
        assert!(load("gauntlet") && !load("nowhere"));
    }

    #[test]
    fn checkpoints_count_up_from_the_start() {
        load("gauntlet");
        let order: Vec<u8> = (0..course().depth()).rev().filter_map(|z| course().checkpoint(7, z)).collect();
        assert_eq!(order, [1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn a_bot_can_get_to_the_finish_from_every_spawn() {
        for s in &course().spawns {
            let (x, z) = course().cell(*s);
            assert!(course().to_finish[z * course().width() + x] < u16::MAX, "stuck at {s:?}");
        }
    }
}
