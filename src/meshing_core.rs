use crate::chunkbuf::ChunkBuf;
use crate::mesher::MeshBuild;
use crate::meshutil::Face;
use raylib::prelude::*;
use std::collections::HashMap;
use std::hash::Hash;

// Generic greedy-rectangle sweep over a 2D mask. The mask is width*height laid out row-major.
// For each maximal rectangle of identical Some(code), calls `emit(x, y, w, h, code)` once.
#[inline]
fn greedy_rects<K: Copy + Eq + Hash>(
    width: usize,
    height: usize,
    mask: &mut [Option<(K, u8)>],
    mut emit: impl FnMut(usize, usize, usize, usize, (K, u8)),
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
            while x + w < width && mask[y * width + (x + w)] == code && !used[y * width + (x + w)] {
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

#[inline]
fn apply_min_light(l: u8, min: Option<u8>) -> u8 {
    if let Some(m) = min { l.max(m) } else { l }
}

// Core greedy meshing builder used by both world and local meshers.
// The `face_info` closure decides visibility and lighting per face; it must return None if the
// face is not visible. `flip_v[face]` controls V flipping for that face (0..5).
pub fn build_mesh_core<K, F>(
    buf: &ChunkBuf,
    base_x: i32,
    base_z: i32,
    flip_v: [bool; 6],
    min_light: Option<u8>,
    mut face_info: F,
) -> HashMap<K, MeshBuild>
where
    K: Copy + Eq + Hash,
    F: FnMut(usize, usize, usize, Face, crate::blocks::Block) -> Option<(K, u8)>,
{
    let sx = buf.sx;
    let sy = buf.sy;
    let sz = buf.sz;
    let mut builds: HashMap<K, MeshBuild> = HashMap::new();

    // +Y faces
    for y in 0..sy {
        let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, Face::PosY, here) {
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
            let lv = apply_min_light(codev.1, min_light);
            let rgba = [lv, lv, lv, 255];
            mb.add_face_rect(
                Face::PosY,
                Vector3::new(fx, fy, fz),
                u1,
                v1,
                flip_v[Face::PosY.index()],
                rgba,
            );
        });
    }

    // -Y faces
    for y in 0..sy {
        let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sz];
        for z in 0..sz {
            for x in 0..sx {
                let here = buf.get_local(x, y, z);
                if let Some((fm, l)) = face_info(x, y, z, Face::NegY, here) {
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
            let lv = apply_min_light(codev.1, min_light);
            let rgba = [lv, lv, lv, 255];
            mb.add_face_rect(
                Face::NegY,
                Vector3::new(fx, fy, fz),
                u1,
                v1,
                flip_v[Face::NegY.index()],
                rgba,
            );
        });
    }

    // X planes (±X faces)
    for x in 0..sx {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(K, u8)>> = vec![None; sz * sy];
            for z in 0..sz {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { Face::PosX } else { Face::NegX };
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
                let lv = apply_min_light(codev.1, min_light);
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosX } else { Face::NegX };
                mb.add_face_rect(
                    face,
                    Vector3::new(fx, fy, fz),
                    u1,
                    v1,
                    flip_v[face.index()],
                    rgba,
                );
            });
        }
    }

    // Z planes (±Z faces)
    for z in 0..sz {
        for &pos in &[false, true] {
            let mut mask: Vec<Option<(K, u8)>> = vec![None; sx * sy];
            for x in 0..sx {
                for y in 0..sy {
                    let here = buf.get_local(x, y, z);
                    let face = if pos { Face::PosZ } else { Face::NegZ };
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
                let lv = apply_min_light(codev.1, min_light);
                let rgba = [lv, lv, lv, 255];
                let face = if pos { Face::PosZ } else { Face::NegZ };
                mb.add_face_rect(
                    face,
                    Vector3::new(fx, fy, fz),
                    u1,
                    v1,
                    flip_v[face.index()],
                    rgba,
                );
            });
        }
    }

    builds
}
