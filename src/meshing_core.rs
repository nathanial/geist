use crate::chunkbuf::ChunkBuf;
use crate::mesher::{FaceMaterial, MeshBuild};
use raylib::prelude::*;
use std::collections::HashMap;

// Generic greedy-rectangle sweep over a 2D mask. The mask is width*height laid out row-major.
// For each maximal rectangle of identical Some(code), calls `emit(x, y, w, h, code)` once.
#[inline]
fn greedy_rects(
    width: usize,
    height: usize,
    mask: &mut [Option<(FaceMaterial, u8)>],
    mut emit: impl FnMut(usize, usize, usize, usize, (FaceMaterial, u8)),
) {
    let mut used = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let code = mask[idx];
            if code.is_none() || used[idx] {
                continue;
            }
            let mut w = 1;
            while x + w < width
                && mask[y * width + (x + w)] == code
                && !used[y * width + (x + w)]
            {
                w += 1;
            }
            let mut h = 1;
            'expand: while y + h < height {
                for i in 0..w {
                    let j = (y + h) * width + (x + i);
                    if mask[j] != code || used[j] {
                        break 'expand;
                    }
                }
                h += 1;
            }
            emit(x, y, w, h, code.unwrap());
            for yy in 0..h {
                for xx in 0..w {
                    used[(y + yy) * width + (x + xx)] = true;
                }
            }
        }
    }
}

// Core greedy meshing builder used by both world and local meshers.
// The `face_info` closure decides visibility and lighting per face; it must return None if the
// face is not visible. `flip_v[face]` controls V flipping for that face (0..5).
pub fn build_mesh_core<F>(
    buf: &ChunkBuf,
    base_x: i32,
    base_z: i32,
    flip_v: [bool; 6],
    min_light: Option<u8>,
    mut face_info: F,
) -> HashMap<FaceMaterial, MeshBuild>
where
    F: FnMut(usize, usize, usize, usize, crate::voxel::Block) -> Option<(FaceMaterial, u8)>,
{
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let mut builds: HashMap<FaceMaterial, MeshBuild> = HashMap::new();

    // +Y faces
    for y in 0..sy {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, 0, here) {
                    mask[z * sx + x] = Some((fm, l));
                }
            }
        }
        greedy_rects(sx, sz, &mut mask, |x, z, w, h, codev| {
            let fx = (base_x + x as i32) as f32;
            let fz = (base_z + z as i32) as f32;
            let fy = (y as f32) + 1.0;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let mut lv = codev.1;
            if let Some(m) = min_light {
                lv = lv.max(m);
            }
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz),
                Vector3::new(fx + u1, fy, fz),
                Vector3::new(fx + u1, fy, fz + v1),
                Vector3::new(fx, fy, fz + v1),
                Vector3::new(0.0, 1.0, 0.0),
                u1,
                v1,
                flip_v[0],
                rgba,
            );
        });
    }

    // -Y faces
    for y in 0..sy {
        let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, 1, here) {
                    mask[z * sx + x] = Some((fm, l));
                }
            }
        }
        greedy_rects(sx, sz, &mut mask, |x, z, w, h, codev| {
            let fx = (base_x + x as i32) as f32;
            let fz = (base_z + z as i32) as f32;
            let fy = y as f32;
            let u1 = w as f32;
            let v1 = h as f32;
            let mb = builds.entry(codev.0).or_default();
            let mut lv = codev.1;
            if let Some(m) = min_light {
                lv = lv.max(m);
            }
            let rgba = [lv, lv, lv, 255];
            mb.add_quad(
                Vector3::new(fx, fy, fz + v1),
                Vector3::new(fx + u1, fy, fz + v1),
                Vector3::new(fx + u1, fy, fz),
                Vector3::new(fx, fy, fz),
                Vector3::new(0.0, -1.0, 0.0),
                u1,
                v1,
                flip_v[1],
                rgba,
            );
        });
    }

    // X planes (±X faces)
    for x in 0..sx {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sz * sy];
            for z in 0..sz {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { 2 } else { 3 };
                    if let Some((fm, l)) = face_info(x, y, z, face, here) {
                        mask[y * sz + z] = Some((fm, l));
                    }
                }
            }
            greedy_rects(sz, sy, &mut mask, |z, y, w, h, codev| {
                let fx = (base_x + x as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32;
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let mut lv = codev.1;
                if let Some(m) = min_light {
                    lv = lv.max(m);
                }
                let rgba = [lv, lv, lv, 255];
                if !pos {
                    // -X
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(-1.0, 0.0, 0.0),
                        u1,
                        v1,
                        flip_v[3],
                        rgba,
                    );
                } else {
                    // +X
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz + u1),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx, fy, fz + u1),
                        Vector3::new(1.0, 0.0, 0.0),
                        u1,
                        v1,
                        flip_v[2],
                        rgba,
                    );
                }
            });
        }
    }

    // Z planes (±Z faces)
    for z in 0..sz {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(FaceMaterial, u8)>> = vec![None; sx * sy];
            for x in 0..sx {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { 4 } else { 5 };
                    if let Some((fm, l)) = face_info(x, y, z, face, here) {
                        mask[y * sx + x] = Some((fm, l));
                    }
                }
            }
            greedy_rects(sx, sy, &mut mask, |x, y, w, h, codev| {
                let fx = (base_x + x as i32) as f32;
                let fy = y as f32;
                let fz = (base_z + z as i32) as f32 + if pos { 1.0 } else { 0.0 };
                let u1 = w as f32;
                let v1 = h as f32;
                let mb = builds.entry(codev.0).or_default();
                let mut lv = codev.1;
                if let Some(m) = min_light {
                    lv = lv.max(m);
                }
                let rgba = [lv, lv, lv, 255];
                if !pos {
                    // -Z
                    mb.add_quad(
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(0.0, 0.0, -1.0),
                        u1,
                        v1,
                        flip_v[5],
                        rgba,
                    );
                } else {
                    // +Z
                    mb.add_quad(
                        Vector3::new(fx + u1, fy + v1, fz),
                        Vector3::new(fx, fy + v1, fz),
                        Vector3::new(fx, fy, fz),
                        Vector3::new(fx + u1, fy, fz),
                        Vector3::new(0.0, 0.0, 1.0),
                        u1,
                        v1,
                        flip_v[4],
                        rgba,
                    );
                }
            });
        }
    }

    builds
}

