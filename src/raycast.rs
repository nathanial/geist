use raylib::prelude::Vector3;

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub bx: i32,
    pub by: i32,
    pub bz: i32,
    pub px: i32,
    pub py: i32,
    pub pz: i32,
    pub nx: i32,
    pub ny: i32,
    pub nz: i32,
}

#[inline]
fn inv_or_max(v: f32) -> f32 {
    if v.abs() < 1e-8 { f32::MAX } else { 1.0 / v.abs() }
}

pub fn raycast_first_hit_with_face<F>(origin: Vector3, dir: Vector3, max_dist: f32, mut is_solid: F) -> Option<RayHit>
where
    F: FnMut(i32, i32, i32) -> bool,
{
    let mut d = dir;
    let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    if len < 1e-6 { return None; }
    d.x /= len; d.y /= len; d.z /= len;

    let mut vx = origin.x.floor() as i32;
    let mut vy = origin.y.floor() as i32;
    let mut vz = origin.z.floor() as i32;

    let stepx = if d.x > 0.0 { 1 } else if d.x < 0.0 { -1 } else { 0 };
    let stepy = if d.y > 0.0 { 1 } else if d.y < 0.0 { -1 } else { 0 };
    let stepz = if d.z > 0.0 { 1 } else if d.z < 0.0 { -1 } else { 0 };

    let invx = inv_or_max(d.x);
    let invy = inv_or_max(d.y);
    let invz = inv_or_max(d.z);
    let tdx = if stepx == 0 { f32::MAX } else { invx };
    let tdy = if stepy == 0 { f32::MAX } else { invy };
    let tdz = if stepz == 0 { f32::MAX } else { invz };

    let fx = origin.x - origin.x.floor();
    let fy = origin.y - origin.y.floor();
    let fz = origin.z - origin.z.floor();
    let mut tmx = if stepx > 0 { (1.0 - fx) * invx } else if stepx < 0 { fx * invx } else { f32::MAX };
    let mut tmy = if stepy > 0 { (1.0 - fy) * invy } else if stepy < 0 { fy * invy } else { f32::MAX };
    let mut tmz = if stepz > 0 { (1.0 - fz) * invz } else if stepz < 0 { fz * invz } else { f32::MAX };

    let mut prevx = vx; let mut prevy = vy; let mut prevz = vz;
    let mut t = 0.0f32;

    for _ in 0..512 {
        if t > max_dist { break; }
        if is_solid(vx, vy, vz) {
            // Determine face normal from step between prev and current
            let dx = vx - prevx; let dy = vy - prevy; let dz = vz - prevz;
            let (mut nx, mut ny, mut nz) = (0, 0, 0);
            if dx == 1 { nx = -1; } else if dx == -1 { nx = 1; }
            else if dy == 1 { ny = -1; } else if dy == -1 { ny = 1; }
            else if dz == 1 { nz = -1; } else if dz == -1 { nz = 1; }
            return Some(RayHit { bx: vx, by: vy, bz: vz, px: prevx, py: prevy, pz: prevz, nx, ny, nz });
        }
        prevx = vx; prevy = vy; prevz = vz;
        // Step through smallest tMax
        if tmx < tmy {
            if tmx < tmz { vx += stepx; t = tmx; tmx += tdx; } else { vz += stepz; t = tmz; tmz += tdz; }
        } else {
            if tmy < tmz { vy += stepy; t = tmy; tmy += tdy; } else { vz += stepz; t = tmz; tmz += tdz; }
        }
    }
    None
}

