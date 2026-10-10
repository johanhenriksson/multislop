//! Courses: nine holes dealt from a seed, named `random-<seed>` so clients build the same ones from the name. Every
//! hole is built up front and laid side by side, `SPACING` apart along x, floating in the sky.
//!
//! A hole is dealt by a turtle walking a grid of 1 m cells: a tee, a path of pieces (straights, turns, ramps, hills,
//! side slopes, bumpers, a windmill, a gap over the void, a split, water), and a green with the cup. Floor heights are
//! kept per cell corner and interpolated in between, so ramps and hills are smooth slopes. Rails stand wherever the
//! fairway meets the void. A hole that runs into itself, or whose cup can't be reached, is dealt again.

use glam::{IVec2, Vec2, Vec3};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::f32::consts::PI;
use std::rc::Rc;

pub const HOLES: usize = 9;
/// Holes are laid side by side this far apart along x, centered on x = hole * SPACING.
pub const SPACING: f32 = 200.0;
/// Rails rise this far above the floor, and are this thick.
pub const RAIL_HEIGHT: f32 = 0.3;
const RAIL_THICK: f32 = 0.16;
/// Blocks (the split's divider) and posts (bumpers) stand this far above the floor; posts are this wide.
const BLOCK_HEIGHT: f32 = 0.35;
const POST_WIDTH: f32 = 0.45;
/// Water lies this far under the fairway around it, over a basin this deep.
pub const WATER_DROP: f32 = 0.12;
pub const BASIN_DROP: f32 = 0.5;
/// A ball this far under the hole's lowest floor has fallen off.
const FALL: f32 = 2.5;
/// Windmill hubs, and how far their bars reach either side.
pub const HUB_WIDTH: f32 = 0.24;
pub const WINDMILL_REACH: f32 = 1.45;
pub const WINDMILL_HEIGHT: f32 = 0.45;
/// Highest a course goes above or below the first tee.
const MAX_RISE: f32 = 3.0;

/// The maps this game can play: a new course every time.
pub const MAPS: [&str; 1] = ["random"];
/// Looks a course can have, picked from its seed.
pub const LOOKS: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    Void,
    /// Open air the path crosses: no rails along it.
    Gap,
    Fairway,
    Tee,
    Green,
    Water,
    /// A cell-sized block on the fairway.
    Block,
    /// A post in the middle of the cell, which balls roll around.
    Post,
}

impl Cell {
    /// Has a floor a ball can roll on (under a post too).
    pub fn floor(self) -> bool {
        matches!(self, Cell::Fairway | Cell::Tee | Cell::Green | Cell::Block | Cell::Post)
    }

    /// Inside the rails.
    pub fn inside(self) -> bool {
        self.floor() || self == Cell::Water
    }
}

pub struct Hole {
    /// World position of the grid's (0, 0) corner.
    pub origin: Vec3,
    pub w: usize,
    pub d: usize,
    cells: Vec<Cell>,
    /// Floor height at each cell corner, (w + 1) x (d + 1), meters.
    corners: Vec<f32>,
    /// Rails, blocks, posts, windmill hubs and the faces under ledges, as (center, half extents).
    pub boxes: Vec<(Vec3, Vec3)>,
    /// The first this many boxes are seen; the rest are faces hidden under ledges.
    pub drawn: usize,
    /// On the floor.
    pub tee: Vec3,
    pub cup: Vec3,
    /// Windmill pivots, on the floor.
    pub windmills: Vec<Vec3>,
    pub par: u8,
    /// A ball under this has fallen off.
    pub low: f32,
    /// Cells from each cell to the cup, over the floor and across gaps; `u16::MAX` where it can't be reached.
    to_cup: Vec<u16>,
}

pub struct Course {
    pub holes: Vec<Hole>,
    pub look: u32,
}

thread_local! {
    static COURSE: RefCell<Rc<Course>> = RefCell::new(Rc::new(generate(1)));
}

/// The course being played, `random-1` until `load` picks another.
pub fn course() -> Rc<Course> {
    COURSE.with(|c| c.borrow().clone())
}

/// Makes `random-<seed>` the course; false for any other name.
pub fn load(name: &str) -> bool {
    let Some(Ok(seed)) = name.strip_prefix("random-").map(str::parse) else { return false };
    COURSE.with(|c| *c.borrow_mut() = Rc::new(generate(seed)));
    true
}

impl Course {
    /// The hole a point is over: the one whose slot along x it's in.
    pub fn hole_at(&self, p: Vec3) -> &Hole {
        &self.holes[((p.x / SPACING).round().max(0.0) as usize).min(self.holes.len() - 1)]
    }
}

impl Hole {
    fn grid(&self, p: Vec3) -> Option<(usize, usize, f32, f32)> {
        let (gx, gz) = (p.x - self.origin.x, p.z - self.origin.z);
        if !(gx >= 0.0 && gz >= 0.0) {
            return None;
        }
        let (x, z) = (gx as usize, gz as usize);
        (x < self.w && z < self.d).then_some((x, z, gx - x as f32, gz - z as f32))
    }

    pub fn cell(&self, x: usize, z: usize) -> Cell {
        self.cells[z * self.w + x]
    }

    pub fn kind_at(&self, p: Vec3) -> Cell {
        self.grid(p).map_or(Cell::Void, |(x, z, ..)| self.cell(x, z))
    }

    /// The floor height at a corner of the grid.
    pub fn corner(&self, x: usize, z: usize) -> f32 {
        self.corners[z * (self.w + 1) + x]
    }

    /// The floor under `p` and which way it faces, over cells with a floor.
    pub fn floor(&self, p: Vec3) -> Option<(f32, Vec3)> {
        let (x, z, fx, fz) = self.grid(p)?;
        if !self.cell(x, z).floor() {
            return None;
        }
        let (h00, h10, h01, h11) = (self.corner(x, z), self.corner(x + 1, z), self.corner(x, z + 1), self.corner(x + 1, z + 1));
        let top = h00 + (h10 - h00) * fx;
        let bottom = h01 + (h11 - h01) * fx;
        let dx = (h10 - h00) * (1.0 - fz) + (h11 - h01) * fz;
        let dz = bottom - top;
        Some((top + dz * fz, Vec3::new(-dx, 1.0, -dz).normalize()))
    }

    /// The water surface under `p`, over water.
    pub fn water(&self, p: Vec3) -> Option<f32> {
        let (x, z, ..) = self.grid(p).filter(|&(x, z, ..)| self.cell(x, z) == Cell::Water)?;
        Some(self.water_level(x, z))
    }

    pub fn water_level(&self, x: usize, z: usize) -> f32 {
        self.corner(x, z).min(self.corner(x + 1, z)).min(self.corner(x, z + 1)).min(self.corner(x + 1, z + 1)) - WATER_DROP
    }

    /// Cells to the cup from `p`, `u16::MAX` off the course.
    pub fn to_cup(&self, p: Vec3) -> u16 {
        self.grid(p).map_or(u16::MAX, |(x, z, ..)| self.to_cup[z * self.w + x])
    }

    /// The world point of a grid point at height `y`.
    pub fn world(&self, gx: f32, gz: f32, y: f32) -> Vec3 {
        Vec3::new(self.origin.x + gx, y, self.origin.z + gz)
    }
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

    /// In `lo..hi`.
    fn float(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 10_000) as f32 / 10_000.0 * (hi - lo)
    }

    fn chance(&mut self, p: f32) -> bool {
        (self.next() % 1000) as f32 / 1000.0 < p
    }

    fn sign(&mut self) -> f32 {
        if self.chance(0.5) { 1.0 } else { -1.0 }
    }
}

fn generate(seed: u32) -> Course {
    let holes = (0..HOLES as u32).map(|i| hole(seed.wrapping_mul(0x9E37_79B9).wrapping_add(i.wrapping_mul(0x85EB_CA6B)), i as usize)).collect();
    Course { holes, look: seed % LOOKS }
}

/// Hole number `index` from `seed`, dealt again from the next seed until it works out.
fn hole(seed: u32, index: usize) -> Hole {
    for attempt in 0..64u32 {
        let mut rng = Rng(seed.wrapping_add(attempt.wrapping_mul(0x2C1B_3C6D)) | 1);
        if let Some(h) = deal(&mut rng).and_then(|b| b.finish(index)) {
            return h;
        }
    }
    let mut b = Builder::new();
    b.straight(8);
    b.green(0, 2, 0.0);
    b.finish(index).expect("a straight hole works")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Piece {
    Straight,
    Ramp,
    Hill,
    Slope,
    Bumpers,
    Windmill,
    Gap,
    Split,
    Water,
}

/// One hole's path: the tee, 3-6 pieces with turns between some of them, and the green.
fn deal(rng: &mut Rng) -> Option<Builder> {
    use Piece::*;
    let mut b = Builder::new();
    let mut deck = vec![Straight, Ramp, Hill, Slope, Bumpers, Windmill, Gap, Split, Water, Ramp, Straight];
    let mut turned = true;
    for _ in 0..rng.range(3, 6) {
        if !turned && rng.chance(0.55) {
            b.turn(rng.chance(0.5))?;
        }
        let piece = deck.remove(rng.range(0, deck.len() as i32 - 1) as usize);
        turned = false;
        if !b.piece(piece, rng) {
            return None;
        }
    }
    if rng.chance(0.4) {
        b.turn(rng.chance(0.5))?;
    }
    b.green(rng.range(1, 3), rng.range(-1, 1), rng.float(-0.15, 0.15)).then_some(b)
}

/// Lays a hole out piece by piece from the tee, heading north to start. Cells are kept in turtle space (unbounded,
/// integer) until `finish` lays them on a grid.
struct Builder {
    cells: HashMap<IVec2, (Cell, usize)>,
    corners: HashMap<IVec2, f32>,
    /// The piece being laid; pieces may only touch the one before them.
    piece: usize,
    /// The cell on the center line where the next piece starts, the way it heads, and the floor height there.
    pos: IVec2,
    dir: IVec2,
    h: f32,
    tee: Vec2,
    cup: Vec2,
    /// Windmill pivots: grid point and floor height.
    windmills: Vec<(Vec2, f32)>,
    /// Cells along the path, and the hazards on it, for par.
    length: i32,
    hazards: i32,
}

/// Eases 0 to 1 with flat ends, so slopes start and end gently.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Builder {
    fn new() -> Builder {
        let mut b = Builder {
            cells: HashMap::new(),
            corners: HashMap::new(),
            piece: 0,
            pos: IVec2::ZERO,
            dir: IVec2::new(0, -1),
            h: 0.0,
            tee: Vec2::ZERO,
            cup: Vec2::ZERO,
            windmills: vec![],
            length: 0,
            hazards: 0,
        };
        b.tee = b.point(1.5, 0.0);
        b.stamp(3, 1, |_, _| Cell::Tee, |_, _| 0.0);
        b.advance(3);
        b
    }

    fn right(&self) -> IVec2 {
        IVec2::new(-self.dir.y, self.dir.x)
    }

    /// The grid point `f` meters ahead of where the next piece starts and `l` to the right of the center line.
    fn point(&self, f: f32, l: f32) -> Vec2 {
        self.pos.as_vec2() + Vec2::splat(0.5) + self.dir.as_vec2() * (f - 0.5) + self.right().as_vec2() * l
    }

    /// Lays cells `f` in 0..len ahead and `l` in -half..=half across, as `kind` says, with floor heights from `height`
    /// of meters ahead of the start and to the right of the center line. False if that runs into the hole so far.
    fn stamp(&mut self, len: i32, half: i32, kind: impl Fn(i32, i32) -> Cell, height: impl Fn(f32, f32) -> f32) -> bool {
        let (r, start) = (self.right(), self.point(0.0, 0.0));
        for f in 0..len {
            for l in -half..=half {
                let c = self.pos + self.dir * f + r * l;
                let crowded = (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| c + IVec2::new(dx, dz))).any(|n| self.cells.get(&n).is_some_and(|&(_, p)| p + 2 < self.piece));
                if self.cells.contains_key(&c) || crowded {
                    return false;
                }
                self.cells.insert(c, (kind(f, l), self.piece));
                for corner in [c, c + IVec2::X, c + IVec2::Y, c + IVec2::ONE] {
                    let rel = corner.as_vec2() - start;
                    self.corners.insert(corner, height(rel.dot(self.dir.as_vec2()), rel.dot(r.as_vec2())));
                }
            }
        }
        true
    }

    fn advance(&mut self, len: i32) {
        self.pos += self.dir * len;
        self.length += len;
    }

    fn next_piece(&mut self) {
        self.piece += 1;
    }

    fn straight(&mut self, len: i32) -> bool {
        let h = self.h;
        let ok = self.stamp(len, 1, |_, _| Cell::Fairway, |_, _| h);
        self.advance(len);
        ok
    }

    /// A square corner, turning right or left.
    fn turn(&mut self, right: bool) -> Option<()> {
        self.next_piece();
        let h = self.h;
        self.stamp(3, 1, |_, _| Cell::Fairway, |_, _| h).then_some(())?;
        let center = self.pos + self.dir;
        self.dir = if right { self.right() } else { -self.right() };
        self.pos = center + self.dir * 2;
        self.length += 2;
        Some(())
    }

    /// A piece five cells wide, `len` long, between two-cell lead-ins three wide, so the wide part never touches a
    /// turn. `kind` and `height` are as in `stamp`, measured from the wide part's start.
    fn wide(&mut self, len: i32, kind: impl Fn(i32, i32) -> Cell, height: impl Fn(f32, f32) -> f32) -> bool {
        let ok = self.straight(2) && self.stamp(len, 2, kind, height);
        self.advance(len);
        ok && self.straight(2)
    }

    fn piece(&mut self, piece: Piece, rng: &mut Rng) -> bool {
        self.next_piece();
        let h = self.h;
        match piece {
            Piece::Straight => self.straight(rng.range(3, 8)),
            Piece::Ramp => {
                // At most a 1 m rise over 5 m, so the crest doesn't launch every hard shot over the rails.
                let len = rng.range(5, 7);
                let mut dh = rng.range(2, 4) as f32 * 0.25 * rng.sign();
                if (h + dh).abs() > MAX_RISE {
                    dh = -dh;
                }
                self.h += dh;
                let ok = self.stamp(len, 1, |_, _| Cell::Fairway, |f, _| h + dh * smooth(f / len as f32));
                self.advance(len);
                ok
            }
            Piece::Hill => {
                let (len, amp) = (rng.range(5, 8), rng.float(0.25, 0.5) * rng.sign());
                let ok = self.stamp(len, 1, |_, _| Cell::Fairway, |f, _| h + amp * (PI * f / len as f32).sin().powi(2));
                self.advance(len);
                ok
            }
            Piece::Slope => {
                let (len, tilt) = (rng.range(6, 9), rng.float(0.3, 0.5) * rng.sign());
                let ok = self.stamp(len, 1, |_, _| Cell::Fairway, |f, l| h + tilt * l / 1.5 * (PI * f / len as f32).sin().max(0.0));
                self.advance(len);
                ok
            }
            Piece::Bumpers => {
                let len = rng.range(6, 9);
                let posts: Vec<(i32, i32)> = (1..len - 1).flat_map(|f| (-2..=2).map(move |l| (f, l))).filter(|_| rng.chance(0.2)).collect();
                self.wide(len, |f, l| if posts.contains(&(f, l)) { Cell::Post } else { Cell::Fairway }, |_, _| h)
            }
            Piece::Windmill => {
                self.windmills.push((self.point(2.5, 0.0), h));
                self.hazards += 1;
                self.straight(5)
            }
            Piece::Gap => {
                const KICK: f32 = 0.35;
                self.hazards += 1;
                // A run-up first, so a gap never comes straight after a turn.
                let gap = rng.range(2, 3);
                let ok = self.straight(3) && self.stamp(6 + gap, 1, |f, _| if (3..3 + gap).contains(&f) { Cell::Gap } else { Cell::Fairway }, |f, _| {
                    if f <= 3.0 + 1e-3 { h + KICK * (f / 3.0).powi(2) } else { h }
                });
                self.advance(6 + gap);
                ok
            }
            Piece::Split => {
                let len = rng.range(6, 8);
                self.wide(len, |f, l| if l == 0 && (1..len - 1).contains(&f) { Cell::Block } else { Cell::Fairway }, |_, _| h)
            }
            Piece::Water => {
                let len = rng.range(6, 8);
                if rng.chance(0.5) {
                    let side = if rng.chance(0.5) { 1 } else { -1 };
                    self.wide(len, |f, l| if l * side >= 1 && (1..len - 1).contains(&f) { Cell::Water } else { Cell::Fairway }, |_, _| h)
                } else {
                    self.hazards += 1;
                    let (row, bridge) = (len / 2 - 1, rng.range(-1, 1));
                    self.wide(len, |f, l| if (row..row + 2).contains(&f) && l != bridge { Cell::Water } else { Cell::Fairway }, |_, _| h)
                }
            }
        }
    }

    /// The green: five cells square after a lead-in, gently tilted, with the cup `f` rows in and `l` across.
    fn green(&mut self, f: i32, l: i32, tilt: f32) -> bool {
        self.next_piece();
        let h = self.h;
        if !self.straight(2) {
            return false;
        }
        self.cup = self.point(f as f32 + 0.5, l as f32);
        let ok = self.stamp(5, 2, |_, _| Cell::Green, |f, l| h + tilt * l / 2.5 * (PI * f / 5.0).sin().max(0.0));
        self.advance(5);
        ok
    }

    /// Lays the hole out on a grid with a cell of void around it, at its slot along x, and puts up rails, blocks
    /// and faces. None if the cup can't be reached from the tee.
    fn finish(self, index: usize) -> Option<Hole> {
        let min = self.cells.keys().fold(IVec2::MAX, |a, &c| a.min(c)) - IVec2::ONE;
        let max = self.cells.keys().fold(IVec2::MIN, |a, &c| a.max(c)) + IVec2::ONE;
        let (w, d) = ((max.x - min.x + 1) as usize, (max.y - min.y + 1) as usize);
        let mut cells = vec![Cell::Void; w * d];
        for (&c, &(kind, _)) in &self.cells {
            let c = c - min;
            cells[c.y as usize * w + c.x as usize] = kind;
        }
        let corners: Vec<f32> = (0..(w + 1) * (d + 1)).map(|i| *self.corners.get(&(IVec2::new((i % (w + 1)) as i32, (i / (w + 1)) as i32) + min)).unwrap_or(&0.0)).collect();
        let origin = Vec3::new(index as f32 * SPACING - w as f32 / 2.0, 0.0, -(d as f32) / 2.0);
        let local = |p: Vec2| p - min.as_vec2();
        let mut hole = Hole {
            origin,
            w,
            d,
            cells,
            corners,
            boxes: vec![],
            drawn: 0,
            tee: Vec3::ZERO,
            cup: Vec3::ZERO,
            windmills: vec![],
            par: (1 + self.length / 14 + self.hazards).clamp(2, 5) as u8,
            low: 0.0,
            to_cup: vec![],
        };
        let on_floor = |hole: &Hole, p: Vec2| {
            let at = hole.world(p.x, p.y, 0.0);
            at.with_y(hole.floor(at).map_or(0.0, |(h, _)| h))
        };
        hole.tee = on_floor(&hole, local(self.tee));
        hole.cup = on_floor(&hole, local(self.cup));
        hole.windmills = self.windmills.iter().map(|&(p, h)| hole.world(local(p).x, local(p).y, h)).collect();
        let floors = (0..w * d).filter(|&i| hole.cells[i].floor());
        hole.low = floors.flat_map(|i| [hole.corner(i % w, i / w), hole.corner(i % w + 1, i / w + 1)]).fold(f32::MAX, f32::min) - FALL;
        (hole.boxes, hole.drawn) = hole.solids();
        hole.to_cup = hole.cup_field();
        (hole.to_cup(hole.tee) < u16::MAX).then_some(hole)
    }
}

impl Hole {
    /// Every box a ball bounces off: rails between the inside and the void (runs along a line at the same heights
    /// merged into one), faces under the edges of the floor over gaps and water so a ball dropping past one can't
    /// pop up onto it, blocks, posts and windmill hubs. The faces come last, after how many boxes there are to draw.
    fn solids(&self) -> (Vec<(Vec3, Vec3)>, usize) {
        let (w, d) = (self.w, self.d);
        let mut boxes = vec![];
        // Rails along x and along z, keyed by line, side and heights, as runs of cells.
        let mut runs: BTreeMap<(bool, usize, i32, i32, i32), Vec<usize>> = BTreeMap::new();
        let mut faces = vec![];
        let q = |v: f32| (v * 100.0).round() as i32;
        for z in 0..d {
            for x in 0..w {
                let c = self.cell(x, z);
                if !c.inside() {
                    continue;
                }
                // (neighbor offset, along x, the line's grid coordinate, the edge's two corners)
                let edges = [(0i32, -1i32, true, z, (x, z), (x + 1, z)), (0, 1, true, z + 1, (x, z + 1), (x + 1, z + 1)), (-1, 0, false, x, (x, z), (x, z + 1)), (1, 0, false, x + 1, (x + 1, z), (x + 1, z + 1))];
                for (dx, dz, along_x, line, a, b) in edges {
                    let n = self.cell((x as i32 + dx) as usize, (z as i32 + dz) as usize);
                    let (ha, hb) = (self.corner(a.0, a.1), self.corner(b.0, b.1));
                    let (lo, hi) = (ha.min(hb), ha.max(hb));
                    let side = dx + dz;
                    if n == Cell::Void {
                        let lo = if c == Cell::Water { lo - BASIN_DROP } else { lo };
                        runs.entry((along_x, line, side, q(lo), q(hi))).or_default().push(if along_x { x } else { z });
                    } else if c.floor() && matches!(n, Cell::Gap | Cell::Water) {
                        faces.push((along_x, line, side, if along_x { x } else { z }, lo));
                    }
                }
                let (lo, hi) = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)].iter().map(|&(cx, cz)| self.corner(cx, cz)).fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(h), b.max(h)));
                let center = |half: f32| {
                    let top = hi + BLOCK_HEIGHT;
                    (self.world(x as f32 + 0.5, z as f32 + 0.5, (lo - 0.3 + top) / 2.0), Vec3::new(half, (top - lo + 0.3) / 2.0, half))
                };
                match c {
                    Cell::Block => boxes.push(center(0.5)),
                    Cell::Post => boxes.push(center(POST_WIDTH / 2.0)),
                    _ => {}
                }
            }
        }
        // A box straddling grid line `line` on the outer side `side`, spanning cells `from..=to` along the other axis.
        let edge_box = |along_x: bool, line: usize, side: i32, from: f32, to: f32, lo: f32, hi: f32| {
            let off = side as f32 * RAIL_THICK / 2.0;
            let (mid, half_len) = ((from + to) / 2.0, (to - from) / 2.0);
            let y = (lo + hi) / 2.0;
            let (gx, gz, half) = if along_x {
                (mid, line as f32 + off, Vec3::new(half_len, (hi - lo) / 2.0, RAIL_THICK / 2.0))
            } else {
                (line as f32 + off, mid, Vec3::new(RAIL_THICK / 2.0, (hi - lo) / 2.0, half_len))
            };
            (self.world(gx, gz, y), half)
        };
        for ((along_x, line, side, lo, hi), mut cells) in runs {
            cells.sort_unstable();
            let (lo, hi) = (lo as f32 / 100.0 - 0.02, hi as f32 / 100.0 + RAIL_HEIGHT);
            let mut start = 0;
            for i in 1..=cells.len() {
                if i == cells.len() || cells[i] != cells[i - 1] + 1 {
                    // Rails run past their cells by their thickness, closing the corners.
                    let (from, to) = (cells[start] as f32 - RAIL_THICK, cells[i - 1] as f32 + 1.0 + RAIL_THICK);
                    boxes.push(edge_box(along_x, line, side, from, to, lo, hi));
                    start = i;
                }
            }
        }
        for &p in &self.windmills {
            boxes.push((p + Vec3::Y * (WINDMILL_HEIGHT - 0.15) / 2.0, Vec3::new(HUB_WIDTH / 2.0, (WINDMILL_HEIGHT + 0.15) / 2.0, HUB_WIDTH / 2.0)));
        }
        let drawn = boxes.len();
        for (along_x, line, side, at, lo) in faces {
            boxes.push(edge_box(along_x, line, side, at as f32, at as f32 + 1.0, self.low - 1.0, lo - 0.01));
        }
        (boxes, drawn)
    }

    /// Cells to the cup over the floor (not through blocks and posts) and across gaps.
    fn cup_field(&self) -> Vec<u16> {
        let (w, d) = (self.w, self.d);
        let mut dist = vec![u16::MAX; w * d];
        let Some((cx, cz, ..)) = self.grid(self.cup) else { return dist };
        let open = |x: usize, z: usize| matches!(self.cell(x, z), Cell::Fairway | Cell::Tee | Cell::Green | Cell::Gap);
        dist[cz * w + cx] = 0;
        let mut queue = VecDeque::from([(cx, cz)]);
        while let Some((x, z)) = queue.pop_front() {
            let next = dist[z * w + x] + 1;
            for (nx, nz) in [(x + 1, z), (x.wrapping_sub(1), z), (x, z + 1), (x, z.wrapping_sub(1))] {
                if nx < w && nz < d && open(nx, nz) && dist[nz * w + nx] == u16::MAX {
                    dist[nz * w + nx] = next;
                    queue.push_back((nx, nz));
                }
            }
        }
        dist
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_deal_and_reach_the_cup() {
        for seed in 0..60 {
            let c = generate(seed);
            assert_eq!(c.holes.len(), HOLES);
            for (i, h) in c.holes.iter().enumerate() {
                assert!(h.to_cup(h.tee) < u16::MAX, "seed {seed} hole {i}: cup out of reach");
                assert!((2..=5).contains(&h.par), "seed {seed} hole {i}: par {}", h.par);
                assert!(h.floor(h.tee + Vec3::Y).is_some() && h.floor(h.cup + Vec3::Y).is_some(), "seed {seed} hole {i}: tee and cup on the floor");
                assert!(std::ptr::eq(c.hole_at(h.tee), h), "seed {seed} hole {i}: found by position");
                assert!(h.w as f32 <= SPACING - 20.0, "seed {seed} hole {i}: too wide, {}", h.w);
            }
        }
    }

    #[test]
    fn most_deals_work_out_first_time() {
        let mut rng = Rng(12345);
        let ok = (0..500).filter(|i| deal(&mut rng).and_then(|b| b.finish(i % HOLES)).is_some()).count();
        assert!(ok > 150, "only {ok} of 500 deals worked");
    }

    #[test]
    fn same_seed_same_course() {
        let (a, b, c) = (generate(42), generate(42), generate(43));
        assert_eq!(a.holes[3].boxes, b.holes[3].boxes);
        assert_ne!(a.holes[3].boxes, c.holes[3].boxes);
        assert!(load("random-7") && !load("gauntlet"));
    }

    #[test]
    fn floors_join_up_between_cells() {
        for h in &generate(5).holes {
            for z in 0..h.d {
                for x in 0..h.w {
                    if !h.cell(x, z).floor() {
                        continue;
                    }
                    let p = h.world(x as f32 + 0.999, z as f32 + 0.5, 10.0);
                    let q = p + Vec3::X * 0.002;
                    if let (Some((a, _)), Some((b, _))) = (h.floor(p), h.floor(q)) {
                        assert!((a - b).abs() < 0.01, "step at {x},{z}: {a} vs {b}");
                    }
                }
            }
        }
    }
}
