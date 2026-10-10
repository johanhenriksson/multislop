//! Procedural textures in the style of de_dust: sandstone blocks, frieze trim, flagstone floor, wooden and green
//! crates, arched doors and shutters. Generated at startup so the game stays a single binary.

use sdk::client::Command;

/// RGBA8 sRGB pixels with all mip levels, as `Command::Texture` takes them.
pub struct Texture {
    w: u32,
    h: u32,
    mips: u32,
    rgba: Vec<u8>,
}

impl Texture {
    pub fn command(self, id: u32) -> Command {
        Command::Texture { id, w: self.w, h: self.h, mips: self.mips, rgba: self.rgba }
    }
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth value noise that tiles every `period` cells.
fn noise(x: f32, y: f32, cell: f32, period: i32, seed: u32) -> f32 {
    let (fx, fy) = (x / cell, y / cell);
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let h = |dx: i32, dy: i32| hash((ix + dx).rem_euclid(period), (iy + dy).rem_euclid(period), seed);
    let top = h(0, 0) + (h(1, 0) - h(0, 0)) * sx;
    let bottom = h(0, 1) + (h(1, 1) - h(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// Builds a mipmapped sRGB texture from a color function of pixel position.
fn texture(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Texture {
    let mut level: Vec<[f32; 4]> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    let (mut lw, mut lh) = (w, h);
    let mut data: Vec<u8> = vec![];
    let mut mips = 0;
    loop {
        data.extend(level.iter().flat_map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)));
        mips += 1;
        if lw == 1 || lh == 1 {
            break;
        }
        // Box-filter the next mip level.
        let (nw, nh) = (lw / 2, lh / 2);
        level = (0..nh)
            .flat_map(|y| (0..nw).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = |dx: u32, dy: u32| level[((y * 2 + dy) * lw + x * 2 + dx) as usize];
                let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
                [0, 1, 2, 3].map(|i| (a[i] + b[i] + c[i] + d[i]) / 4.0)
            })
            .collect();
        (lw, lh) = (nw, nh);
    }
    Texture { w, h, mips, rgba: data }
}

fn rgb(r: f32, g: f32, b: f32, k: f32) -> [f32; 4] {
    [r * k, g * k, b * k, 1.0]
}

/// Sandstone wall, 4 m square: big blocks in staggered courses with mortar, each block a slightly different shade.
pub fn sandstone() -> Texture {
    texture(256, 256, |x, y| {
        let (bw, bh) = (64, 32);
        let row = y / bh;
        let shift = if row % 2 == 0 { 0 } else { bw / 2 };
        let col = (x + shift) / bw;
        let (bx, by) = ((x + shift) % bw, y % bh);
        let mortar = bx < 2 || by < 2;
        let block = 0.82 + 0.26 * hash(col as i32, row as i32, 1);
        let grain = 0.86 + 0.16 * noise(x as f32, y as f32, 3.0, 85, 2) + 0.1 * noise(x as f32, y as f32, 12.0, 21, 5);
        // Weathering: broad darker patches across blocks.
        let grime = 1.0 - 0.22 * noise(x as f32, y as f32, 64.0, 4, 3).powf(1.5);
        let edge = if by < 5 || bx < 4 { 0.9 } else { 1.0 };
        let k = if mortar { 0.55 } else { block * grain * edge * grime };
        rgb(0.8, 0.6, 0.38, k)
    })
}

/// Frieze band along wall tops, 4 m x 0.5 m: carved zigzag between thin moldings.
pub fn frieze() -> Texture {
    texture(256, 32, |x, y| {
        let molding = !(4..=27).contains(&y);
        let zig = ((x % 16) as f32 - 8.0).abs() * 1.6 + 9.0;
        let carved = (y as f32 - zig).abs() < 2.5 || (x % 16 == 0 && (8..24).contains(&y));
        let grain = 0.92 + 0.1 * noise(x as f32, y as f32, 3.0, 86, 4);
        match (molding, carved) {
            (true, _) => rgb(0.76, 0.58, 0.38, 0.85 * grain),
            (_, true) => rgb(0.4, 0.22, 0.13, grain),
            _ => rgb(0.78, 0.6, 0.4, grain),
        }
    })
}

/// Ground, 4 m square: sand with worn flagstones (Voronoi cells) showing through.
pub fn ground() -> Texture {
    let points: Vec<(f32, f32)> = (0..14).map(|i| (hash(i, 0, 7) * 256.0, hash(i, 1, 7) * 256.0)).collect();
    texture(256, 256, move |x, y| {
        let (px, py) = (x as f32, y as f32);
        let (mut d1, mut d2, mut id) = (f32::MAX, f32::MAX, 0);
        for (i, &(cx, cy)) in points.iter().enumerate() {
            // Wrap so the pattern tiles.
            let dx = (px - cx).abs().min(256.0 - (px - cx).abs());
            let dy = (py - cy).abs().min(256.0 - (py - cy).abs());
            let d = (dx * dx + dy * dy).sqrt();
            if d < d1 {
                (d2, d1, id) = (d1, d, i);
            } else if d < d2 {
                d2 = d;
            }
        }
        // Sand drifts over the stones; joints only show where the sand is thin.
        let sand = noise(px, py, 40.0, 6, 9);
        let crack = d2 - d1 < 1.6 && sand < 0.55;
        let stone = 0.88 + 0.12 * hash(id as i32, 3, 8);
        let grain = 0.86 + 0.18 * noise(px, py, 2.0, 128, 10) + 0.06 * noise(px, py, 8.0, 32, 11);
        let base = if sand > 0.55 { 1.0 } else { stone };
        let k = if crack { 0.82 } else { grain * base };
        rgb(0.82, 0.6, 0.36, k)
    })
}

/// Wooden crate face, 1 m square: frame, vertical planks and an X brace.
pub fn wood_crate() -> Texture {
    texture(64, 64, |x, y| {
        let (fx, fy) = (x as f32 / 63.0, y as f32 / 63.0);
        let frame = !(6..58).contains(&x) || !(6..58).contains(&y);
        let brace = (fx - fy).abs() < 0.08 || (fx + fy - 1.0).abs() < 0.08;
        let gap = x % 13 == 0;
        let grain = 0.88 + 0.14 * noise(x as f32, y as f32 * 0.2, 2.0, 32, 11);
        let k = match (frame || brace, gap) {
            (true, _) => 0.85 * grain,
            (_, true) => 0.55,
            _ => 1.0 * grain,
        };
        rgb(0.66, 0.5, 0.3, k)
    })
}

/// Green ammo crate face, 1 m square: ribbed metal with a hazard stripe along the bottom.
pub fn ammo_crate() -> Texture {
    texture(64, 64, |x, y| {
        let rib = y % 16 < 2 || x < 3 || x > 60;
        if y > 54 {
            let stripe = ((x + y) / 4) % 2 == 0;
            return if stripe { rgb(0.85, 0.7, 0.15, 1.0) } else { rgb(0.1, 0.1, 0.1, 1.0) };
        }
        let grain = 0.92 + 0.1 * noise(x as f32, y as f32, 4.0, 16, 12);
        rgb(0.4, 0.52, 0.35, if rib { 0.7 } else { grain })
    })
}

/// Arched wooden door, 1.6 x 2.6 m: dark planks with iron bands; outside the arch is transparent.
pub fn door() -> Texture {
    texture(64, 104, |x, y| {
        let (cx, r) = (32.0, 32.0);
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - r);
        if y < 32 && dx * dx + dy * dy > r * r {
            return [0.0; 4];
        }
        let band = (20..24).contains(&y) || (70..74).contains(&y);
        let gap = x % 11 == 0;
        let grain = 0.85 + 0.2 * noise(x as f32, y as f32 * 0.15, 2.0, 32, 13);
        let frame = dx * dx + dy * dy > (r - 3.0) * (r - 3.0) && y < 32 || x < 3 || x > 60;
        let k = if band { 0.45 } else if gap || frame { 0.6 } else { grain };
        rgb(0.42, 0.29, 0.17, k)
    })
}

/// Soft white clouds on transparent, tiling.
pub fn clouds() -> Texture {
    texture(256, 256, |x, y| {
        let (px, py) = (x as f32, y as f32);
        let n = noise(px, py, 64.0, 4, 20) * 0.5 + noise(px, py, 32.0, 8, 21) * 0.3 + noise(px, py, 12.0, 21, 22) * 0.2;
        let a = ((n - 0.45) * 3.0).clamp(0.0, 0.9);
        [1.0, 1.0, 1.0, a]
    })
}

/// A splash of blood: a ragged blob with spatters flung around it, transparent elsewhere. Dark red, wet and glossy
/// once lit.
pub fn blood() -> Texture {
    // Spatter droplets: position (from the center, in pixels) and radius.
    let drops: Vec<(f32, f32, f32)> = (0..18)
        .map(|i| {
            let (a, d) = (hash(i, 1, 41) * std::f32::consts::TAU, 30.0 + hash(i, 2, 41) * 30.0);
            (a.cos() * d, a.sin() * d, 1.5 + hash(i, 3, 41) * 4.0)
        })
        .collect();
    texture(128, 128, |x, y| {
        let (px, py) = (x as f32 - 64.0, y as f32 - 64.0);
        let (r, a) = ((px * px + py * py).sqrt(), py.atan2(px));
        // A wobbly edge: noise around the rim plus a few longer streaks.
        let rim = noise(64.0 + a.cos() * 40.0, 64.0 + a.sin() * 40.0, 10.0, 13, 42);
        let streak = ((a * 5.0 + 1.3).sin() * (a * 3.0).cos()).max(0.0).powi(6);
        let edge = 24.0 + 12.0 * rim + 22.0 * streak;
        let blob = ((edge - r) / 3.0).clamp(0.0, 1.0);
        let spatter = drops.iter().map(|&(dx, dy, rad)| ((rad - ((px - dx).powi(2) + (py - dy).powi(2)).sqrt()) / 1.2).clamp(0.0, 1.0)).fold(0.0, f32::max);
        let alpha = blob.max(spatter);
        // Darker and thicker in the middle, brighter where thin.
        let shade = 0.55 + 0.45 * (r / edge).min(1.0) + 0.1 * noise(x as f32, y as f32, 8.0, 16, 43);
        [0.42 * shade, 0.02, 0.03, alpha]
    })
}

