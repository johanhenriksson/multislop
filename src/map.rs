//! The dust map, traced from a top-down overview image (12 px per cell). North is up: bomb site B top-left,
//! A bottom-left, CT spawn between them on the left, T spawn in the big yard bottom-right, the loop on the right,
//! tunnels through the middle.
//!
//! `#` wall, `X` tall crate, `x` low crate (jumpable), `A`/`B` bomb site floor, `T`/`C` team spawns, `.` floor.
// ponytail: one floor per spot (a height field), so no bridges over tunnels. Add layers if a map needs them.

use crate::shared::Team;
use bevy::math::Vec3;
use std::sync::LazyLock;

/// Meters per grid cell.
pub const CELL: f32 = 2.0;

const DUST2: &str = "\
########################################################
##############......#########.........##################
#############.......#########.........##################
#############........#######..........##################
####......................................##############
####....BBBBBBBBB.........................##############
####....BBBBxBBBB.........................##############
####....BBBBBBBBB#........#..............###############
####....BBBBBXXB###########...........##################
####....BBBBBXXBB##########.............################
####....BBBBBBBBB##########...........x.################
########BBBBBBBBB##########x............################
########BBBBBBBBB##########.....########################
########BBBBBBBBB##########.....########################
########....x....##########...................##########
########.........##########...................##########
########.........###########..................##########
########.................#############.........#########
########.................###########...............#####
########.................###########.................###
###########......#####..############.................###
###########......#####..############......###........###
############.....#####...##########....#########.....###
#############....####....##########...##########.....###
##############...####....##########...##########.....###
##############...#####................##########......##
########..............................##########......##
########...............................#########......##
#########.............................................##
#########............#....................####........##
#########........######...#########....##########.....##
#########.CCCCCC.######...#########....##########.....##
#########........######...##########..###########.....##
#########........######...#######################.....##
########.........######...########################...###
########.........######...###################...x.....##
########.........######......################.........##
########.........######......################.........##
#######AAAAAA....#########...################.........##
#######AAAAAA.....########...################.........##
######AAAAAAA.......######...####.x.....####..........##
######AxAAAAA.......######...####.....................##
#####AAAXXAAA.........####....###.....................##
####AAAAXXAAA.........####............................##
###.AAAAAAAAA........####.....###.....T.T.T.T.T.T.....##
##..AAAAAAAAA.......#####...#####.....................##
##..AAAAAAAAA.......####....######....................##
##..........................######....................##
##..................####....######.####...###.........##
##################.######...####################......##
################################################......##
########################################################";

/// Floor height of every cell in quarter meters, as base-36 digits (`8` = 2 m street level, `0` = the bottom of
/// the tunnels, `c` = 3 m platforms). Wall cells are ignored; crates sit at their cell's height. The floor between
/// cell centers is interpolated, so height changes become ramps.
const DUST2_HEIGHTS: &str = "\
88888888888888888888888888888888888888888888888888888888
89999999999999999999888888888888888888888888888888888888
89aaaaaaaaaaaaaaaaa9888888888888888888888888888888888888
89abbbbbbbbbbbbbbba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888866556888888888888888
89abcccccccccccccba9888888888888888865556888888888888888
89abcccccccccccccba9888888888888888866666888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abcccccccccccccba9888888888888888888888888888888888888
89abbbbbbbbbbbbbbba9888888888886532356888888888888888888
89aaaaaaaaaaaaaaaaa9888888888665322356888888888888888888
89999999999999999999888888888653202356888888888888888888
88888888888888888888888888888888888888888888888888888888
88888888888888888888888888888888888888888888888888888888
88888888888888888888888888888888888888888888888888888888
88888888888666666888888888888888888888888888888888888888
88888888888555555888888888888888888888888888888888888888
88888888888833333888888888888888888888888888888888888888
88888888888882333888888888888888888888888888888888888888
88888888888888555888888888888888888888888888888888888888
88888888888888666888888888888888888888888888888888888888
88888888888888888888888888888888888888888888888888888888
88888888888888886666666888888888888888888888888888888888
88888888888888866666655666666666888886653202356688888888
88888888888888888888883555688888886665532088888888888888
88888888888888888888888333888888888555388888888888888888
88888888888888888888888222888888888566588888888888888888
88888888888888888888888000888888888888888888888888888888
88888888888888888888888000888888888888888888888888888888
88888888888888888888888000888888888888888888888888888888
88888888888888888888888000888888888888888888888888888888
88888888888888888888888000000888888888888888888888888888
88888888888888888888888000000889999999999999999999999999
88888888888888888888888888000889aaaaaaaaaaaaaaaaaaaaaaaa
88888888888888888888888888000889abbbbbbbbbbbbbbbbbbbbbba
88888888888888888888888888000889abccccccccccccccccccccba
88888888888888888888888888000889abccccccccccccccccccccba
88888888888888888888888888000289abccccccccccccccccccccba
88888888888888888888888888022357abccccccccccccccccccccba
88888888888888888888888880232289abccccccccccccccccccccba
88888888888888888888888882358889abccccccccccccccccccccba
88888888888888888888888823568889abcccaaaaaaaccccccccccba
88888888888888888888653235688889abaaa99aaa99acccccccccba
88888888888888888888888856888889abccccccccccccccccccccba
88888888888888888888888886888889abbbbbbbbbbbbbbbbbbbbbba
88888888888888888888888888888889aaaaaaaaaaaaaaaaaaaaaaaa
88888888888888888888888888888889999999999999999999999999";

pub static MAP: LazyLock<Map> = LazyLock::new(|| Map::parse(DUST2, DUST2_HEIGHTS));

/// Walls go from below the lowest floor to well above the highest one.
const WALL_BOTTOM: f32 = -1.0;
const WALL_TOP: f32 = 9.0;

pub struct Map {
    rows: Vec<Vec<u8>>,
    /// Floor height per cell, meters.
    heights: Vec<Vec<f32>>,
    /// Floor height at cell corners ((width + 1) x (rows + 1)), averaged from the walkable cells around each.
    corners: Vec<f32>,
    /// Solid boxes as (center, half extents), merged from runs of equal cells.
    pub boxes: Vec<(Vec3, Vec3)>,
    pub t_spawns: Vec<Vec3>,
    pub ct_spawns: Vec<Vec3>,
}

impl Map {
    fn parse(text: &str, heights: &str) -> Map {
        let rows: Vec<Vec<u8>> = text.lines().map(|l| l.bytes().collect()).collect();
        let quarter = |c: char| c.to_digit(36).unwrap_or(8) as f32 * 0.25;
        let heights = heights.lines().map(|l| l.chars().map(quarter).collect()).collect();
        let mut map = Map { rows, heights, corners: vec![], boxes: vec![], t_spawns: vec![], ct_spawns: vec![] };
        map.corners = map.corner_heights();
        let mut used = vec![vec![false; map.width()]; map.rows.len()];
        for z in 0..map.rows.len() {
            for x in 0..map.width() {
                let c = map.rows[z][x];
                match c {
                    b'T' => map.t_spawns.push(map.ground(x, z)),
                    b'C' => map.ct_spawns.push(map.ground(x, z)),
                    _ => {}
                }
                let Some(height) = solid_height(c) else { continue };
                if used[z][x] {
                    continue;
                }
                // Greedy rectangle: extend right, then down while the whole run matches.
                let base = map.heights[z][x];
                let same = |x: usize, z: usize| {
                    map.rows[z].get(x) == Some(&c) && !used[z][x] && (c == b'#' || map.heights[z][x] == base)
                };
                let mut x1 = x;
                while same(x1 + 1, z) {
                    x1 += 1;
                }
                let mut z1 = z;
                while z1 + 1 < map.rows.len() && (x..=x1).all(|xx| same(xx, z1 + 1)) {
                    z1 += 1;
                }
                for row in &mut used[z..=z1] {
                    row[x..=x1].fill(true);
                }
                let (a, b) = (map.center(x, z), map.center(x1, z1));
                let (bottom, top) = if c == b'#' { (WALL_BOTTOM, WALL_TOP) } else { (base, base + height) };
                let half = Vec3::new((b.x - a.x) / 2.0 + CELL / 2.0, (top - bottom) / 2.0, (b.z - a.z) / 2.0 + CELL / 2.0);
                map.boxes.push(((a + b) / 2.0 + Vec3::Y * (bottom + top) / 2.0, half));
            }
        }
        map
    }

    fn corner_heights(&self) -> Vec<f32> {
        let (w, d) = (self.width(), self.rows.len());
        let mut corners = Vec::with_capacity((w + 1) * (d + 1));
        for z in 0..=d {
            for x in 0..=w {
                let around = [(x.wrapping_sub(1), z.wrapping_sub(1)), (x, z.wrapping_sub(1)), (x.wrapping_sub(1), z), (x, z)];
                let cells: Vec<(usize, usize)> = around.into_iter().filter(|&(cx, cz)| cx < w && cz < d).collect();
                let walkable: Vec<f32> = cells.iter().filter(|&&(cx, cz)| self.walkable(cx, cz)).map(|&(cx, cz)| self.heights[cz][cx]).collect();
                let pick = if walkable.is_empty() { cells.iter().map(|&(cx, cz)| self.heights[cz][cx]).collect() } else { walkable };
                corners.push(pick.iter().sum::<f32>() / pick.len().max(1) as f32);
            }
        }
        corners
    }

    /// Floor height under a position, interpolated between cell corners so slopes are smooth.
    pub fn floor_at(&self, pos: Vec3) -> f32 {
        let (w, d) = self.size();
        let gx = ((pos.x + w / 2.0) / CELL).clamp(0.0, self.width() as f32 - 1e-3);
        let gz = ((pos.z + d / 2.0) / CELL).clamp(0.0, self.rows.len() as f32 - 1e-3);
        let (x, z) = (gx as usize, gz as usize);
        let (fx, fz) = (gx.fract(), gz.fract());
        let stride = self.width() + 1;
        let h = |dx: usize, dz: usize| self.corners[(z + dz) * stride + x + dx];
        let top = h(0, 0) + (h(1, 0) - h(0, 0)) * fx;
        let bottom = h(0, 1) + (h(1, 1) - h(0, 1)) * fx;
        top + (bottom - top) * fz
    }

    /// Height of a grid corner, for building the floor mesh.
    pub fn corner(&self, x: usize, z: usize) -> f32 {
        self.corners[z * (self.width() + 1) + x]
    }

    /// Cell center standing on the floor.
    pub fn ground(&self, x: usize, z: usize) -> Vec3 {
        let c = self.center(x, z);
        c.with_y(self.floor_at(c))
    }

    pub fn depth(&self) -> usize {
        self.rows.len()
    }

    pub fn width(&self) -> usize {
        self.rows[0].len()
    }

    /// Floor size in meters (x, z).
    pub fn size(&self) -> (f32, f32) {
        (self.width() as f32 * CELL, self.rows.len() as f32 * CELL)
    }

    pub fn center(&self, x: usize, z: usize) -> Vec3 {
        let (w, d) = self.size();
        Vec3::new(x as f32 * CELL + CELL / 2.0 - w / 2.0, 0.0, z as f32 * CELL + CELL / 2.0 - d / 2.0)
    }

    fn cell_at(&self, pos: Vec3) -> u8 {
        let (w, d) = self.size();
        let (x, z) = ((pos.x + w / 2.0) / CELL, (pos.z + d / 2.0) / CELL);
        if x < 0.0 || z < 0.0 {
            return b'#';
        }
        self.rows.get(z as usize).and_then(|r| r.get(x as usize)).copied().unwrap_or(b'#')
    }

    /// Grid cell containing this position.
    pub fn cell(&self, pos: Vec3) -> (usize, usize) {
        let (w, d) = self.size();
        let x = ((pos.x + w / 2.0) / CELL).clamp(0.0, self.width() as f32 - 1.0);
        let z = ((pos.z + d / 2.0) / CELL).clamp(0.0, self.rows.len() as f32 - 1.0);
        (x as usize, z as usize)
    }

    pub fn walkable(&self, x: usize, z: usize) -> bool {
        self.rows.get(z).and_then(|r| r.get(x)).is_some_and(|&c| solid_height(c).is_none())
    }

    /// Steps from every walkable cell to `goal`, 8-connected without cutting corners. `u16::MAX` is unreachable.
    pub fn distance_field(&self, goal: (usize, usize)) -> Vec<u16> {
        let w = self.width();
        let mut dist = vec![u16::MAX; w * self.rows.len()];
        let mut queue = std::collections::VecDeque::from([goal]);
        dist[goal.1 * w + goal.0] = 0;
        while let Some((x, z)) = queue.pop_front() {
            for (nx, nz) in self.neighbors(x, z) {
                if dist[nz * w + nx] == u16::MAX {
                    dist[nz * w + nx] = dist[z * w + x] + 1;
                    queue.push_back((nx, nz));
                }
            }
        }
        dist
    }

    pub fn neighbors(&self, x: usize, z: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        DIRS.into_iter().filter_map(move |(dx, dz)| {
            let (nx, nz) = (x.checked_add_signed(dx)?, z.checked_add_signed(dz)?);
            // Diagonals only when both sides are open, so paths never clip a wall corner.
            let open = self.walkable(nx, nz) && self.walkable(nx, z) && self.walkable(x, nz);
            open.then_some((nx, nz))
        })
    }

    /// The bomb site cell nearest the middle of the site (the middle itself may be a crate).
    pub fn site_center(&self, site: char) -> Vec3 {
        let cells: Vec<Vec3> = self.site_cells().filter(|p| self.site_at(*p) == Some(site)).collect();
        let mid = cells.iter().sum::<Vec3>() / cells.len().max(1) as f32;
        cells.into_iter().min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid))).unwrap_or(mid)
    }

    /// Which bomb site, if any, this floor position is on.
    pub fn site_at(&self, pos: Vec3) -> Option<char> {
        matches!(self.cell_at(pos), b'A' | b'B').then(|| self.cell_at(pos) as char)
    }

    /// Centers of all bomb site cells, for drawing.
    pub fn site_cells(&self) -> impl Iterator<Item = Vec3> + '_ {
        let all = (0..self.rows.len()).flat_map(move |z| (0..self.width()).map(move |x| (x, z)));
        all.filter(|&(x, z)| matches!(self.rows[z][x], b'A' | b'B')).map(|(x, z)| self.center(x, z))
    }

    pub fn spawns(&self, team: Team) -> &[Vec3] {
        match team {
            Team::T => &self.t_spawns,
            Team::Ct => &self.ct_spawns,
        }
    }
}

fn solid_height(c: u8) -> Option<f32> {
    match c {
        b'#' => Some(5.0),
        b'X' => Some(2.2),
        b'x' => Some(1.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dust2_parses() {
        let m = &*MAP;
        assert!(m.rows.iter().all(|r| r.len() == m.width()), "ragged rows");
        assert!(m.heights.len() == m.rows.len() && m.heights.iter().all(|r| r.len() == m.width()), "height grid size");
        let tunnel = m.site_cells().count() > 0 && (0..m.depth()).any(|z| (0..m.width()).any(|x| m.walkable(x, z) && m.heights[z][x] == 0.0));
        assert!(tunnel, "some floor is sunken");
        assert_eq!((m.t_spawns.len(), m.ct_spawns.len()), (6, 6));
        assert!(m.boxes.len() < 200, "merge should keep box count low: {}", m.boxes.len());
        let a = m.site_cells().find(|p| m.site_at(*p) == Some('A'));
        let b = m.site_cells().find(|p| m.site_at(*p) == Some('B'));
        assert!(a.is_some() && b.is_some());
        assert_eq!(m.site_at(m.t_spawns[0]), None);
    }
}
