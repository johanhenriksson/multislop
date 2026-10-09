//! The dust map, traced from a top-down overview image (12 px per cell). North is up: bomb site B top-left,
//! A bottom-left, CT spawn between them on the left, T spawn in the big yard bottom-right, the loop on the right,
//! tunnels through the middle.
//!
//! `#` wall, `X` tall crate, `x` low crate (jumpable), `A`/`B` bomb site floor, `T`/`C` team spawns, `.` floor.
// ponytail: flat, one floor height. Dust2's ramps and stairs need step-up/slope movement first.

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

pub static MAP: LazyLock<Map> = LazyLock::new(|| Map::parse(DUST2));

pub struct Map {
    rows: Vec<Vec<u8>>,
    /// Solid boxes as (center, half extents), merged from runs of equal cells.
    pub boxes: Vec<(Vec3, Vec3)>,
    pub t_spawns: Vec<Vec3>,
    pub ct_spawns: Vec<Vec3>,
}

impl Map {
    fn parse(text: &str) -> Map {
        let rows: Vec<Vec<u8>> = text.lines().map(|l| l.bytes().collect()).collect();
        let mut map = Map { rows, boxes: vec![], t_spawns: vec![], ct_spawns: vec![] };
        let mut used = vec![vec![false; map.width()]; map.rows.len()];
        for z in 0..map.rows.len() {
            for x in 0..map.width() {
                let c = map.rows[z][x];
                match c {
                    b'T' => map.t_spawns.push(map.center(x, z)),
                    b'C' => map.ct_spawns.push(map.center(x, z)),
                    _ => {}
                }
                let Some(height) = solid_height(c) else { continue };
                if used[z][x] {
                    continue;
                }
                // Greedy rectangle: extend right, then down while the whole run matches.
                let same = |x: usize, z: usize| map.rows[z].get(x) == Some(&c) && !used[z][x];
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
                let half = Vec3::new((b.x - a.x) / 2.0 + CELL / 2.0, height / 2.0, (b.z - a.z) / 2.0 + CELL / 2.0);
                map.boxes.push(((a + b) / 2.0 + Vec3::Y * height / 2.0, half));
            }
        }
        map
    }

    pub fn width(&self) -> usize {
        self.rows[0].len()
    }

    /// Floor size in meters (x, z).
    pub fn size(&self) -> (f32, f32) {
        (self.width() as f32 * CELL, self.rows.len() as f32 * CELL)
    }

    fn center(&self, x: usize, z: usize) -> Vec3 {
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
        assert_eq!((m.t_spawns.len(), m.ct_spawns.len()), (6, 6));
        assert!(m.boxes.len() < 150, "merge should keep box count low: {}", m.boxes.len());
        let a = m.site_cells().find(|p| m.site_at(*p) == Some('A'));
        let b = m.site_cells().find(|p| m.site_at(*p) == Some('B'));
        assert!(a.is_some() && b.is_some());
        assert_eq!(m.site_at(m.t_spawns[0]), None);
    }
}
