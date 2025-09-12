use std::sync::OnceLock;
use crate::constants::{
    MICROGRID_STEPS,
    MICROGRID_LAST_IDX,
    BOXES_TABLE_SIZE,
    RECTS_TABLE_SIZE,
};

// Compact encodings for micro-grid assets
// Boxes are encoded as half-step coordinates in [0, 1, 2]: (x0,y0,z0,x1,y1,z1)
pub type MicroBox = [u8; 6];
// Rects on a 2x2 boundary plane: (u0, v0, du, dv) in half-steps [0,1,2]
pub type MicroRect = [u8; 4];

fn gen_boxes_for_occ(occ: u8) -> Vec<MicroBox> {
    // Recreate mesher's greedy decomposition exactly on a 2x2x2 occupancy grid.
    let mut out: Vec<MicroBox> = Vec::new();
    for y in 0..MICROGRID_STEPS {
        let mut grid = [[false; MICROGRID_STEPS]; MICROGRID_STEPS]; // [z][x]
        for z in 0..MICROGRID_STEPS {
            for x in 0..MICROGRID_STEPS {
                let bit = 1u8 << (((y & 1) << 2) | ((z & 1) << 1) | (x & 1));
                grid[z][x] = (occ & bit) != 0;
            }
        }
        let mut used = [[false; MICROGRID_STEPS]; MICROGRID_STEPS];
        for z in 0..MICROGRID_STEPS {
            for x in 0..MICROGRID_STEPS {
                if !grid[z][x] || used[z][x] {
                    continue;
                }
                let w = if x == 0 && grid[z][MICROGRID_LAST_IDX] && !used[z][MICROGRID_LAST_IDX] {
                    MICROGRID_STEPS
                } else {
                    1
                };
                let h = if z == 0 {
                    let mut ok = true;
                    for xi in x..(x + w) {
                        if !grid[MICROGRID_LAST_IDX][xi] || used[MICROGRID_LAST_IDX][xi] {
                            ok = false;
                            break;
                        }
                    }
                    if ok { MICROGRID_STEPS } else { 1 }
                } else {
                    1
                };
                for dz in 0..h {
                    for dx in 0..w {
                        used[z + dz][x + dx] = true;
                    }
                }
                out.push([
                    x as u8,
                    y as u8,
                    z as u8,
                    (x + w) as u8,
                    (y + 1) as u8,
                    (z + h) as u8,
                ]);
            }
        }
    }
    out
}

fn gen_rects_for_mask(mask: u8) -> Vec<MicroRect> {
    // Boundary emptiness greedy merge on a 2x2 (u,v) grid. bit=(v<<1)|u
    let mut grid = [[false; MICROGRID_STEPS]; MICROGRID_STEPS]; // [v][u]
    for v in 0..MICROGRID_STEPS {
        for u in 0..MICROGRID_STEPS {
            grid[v][u] = (mask & (1u8 << ((v << 1) | u))) != 0;
        }
    }
    let mut out: Vec<MicroRect> = Vec::new();
    let mut used = [[false; MICROGRID_STEPS]; MICROGRID_STEPS];
    for v in 0..MICROGRID_STEPS {
        for u in 0..MICROGRID_STEPS {
            if !grid[v][u] || used[v][u] {
                continue;
            }
            let w = if u == 0 && grid[v][MICROGRID_LAST_IDX] && !used[v][MICROGRID_LAST_IDX] {
                MICROGRID_STEPS
            } else {
                1
            };
            let h = if v == 0 {
                let mut ok = true;
                for ui in u..(u + w) {
                    if !grid[MICROGRID_LAST_IDX][ui] || used[MICROGRID_LAST_IDX][ui] {
                        ok = false;
                        break;
                    }
                }
                if ok { MICROGRID_STEPS } else { 1 }
            } else {
                1
            };
            for dv in 0..h {
                for du in 0..w {
                    used[v + dv][u + du] = true;
                }
            }
            out.push([u as u8, v as u8, w as u8, h as u8]);
        }
    }
    out
}

fn build_boxes_table() -> [Vec<MicroBox>; BOXES_TABLE_SIZE] {
    std::array::from_fn(|i| gen_boxes_for_occ(i as u8))
}
fn build_rects_table() -> [Vec<MicroRect>; RECTS_TABLE_SIZE] {
    std::array::from_fn(|i| gen_rects_for_mask(i as u8))
}

pub fn occ8_to_boxes(occ: u8) -> &'static [MicroBox] {
    static BOXES: OnceLock<[Vec<MicroBox>; BOXES_TABLE_SIZE]> = OnceLock::new();
    let t = BOXES.get_or_init(build_boxes_table);
    &t[occ as usize]
}

pub fn empty4_to_rects(mask: u8) -> &'static [MicroRect] {
    static RECTS: OnceLock<[Vec<MicroRect>; RECTS_TABLE_SIZE]> = OnceLock::new();
    let t = RECTS.get_or_init(build_rects_table);
    &t[mask as usize]
}
