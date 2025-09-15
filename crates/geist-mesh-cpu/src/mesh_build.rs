use geist_geom::Vec3;

use crate::face::Face;

#[derive(Default, Clone)]
pub struct MeshBuild {
    pub pos: Vec<f32>,
    pub norm: Vec<f32>,
    pub uv: Vec<f32>,
    pub idx: Vec<u16>,
    pub col: Vec<u8>,
}

impl MeshBuild {
    /// Clears all arrays but retains capacity for reuse across frames.
    #[inline]
    pub fn clear_keep_capacity(&mut self) {
        self.pos.clear();
        self.norm.clear();
        self.uv.clear();
        self.idx.clear();
        self.col.clear();
    }
    /// Pre-reserve capacity for approximately `n_quads` quads worth of data.
    #[inline]
    pub fn reserve_quads(&mut self, n_quads: usize) {
        // 4 vertices per quad
        self.pos.reserve(n_quads * 4 * 3);
        self.norm.reserve(n_quads * 4 * 3);
        self.uv.reserve(n_quads * 4 * 2);
        self.col.reserve(n_quads * 4 * 4);
        self.idx.reserve(n_quads * 6);
    }
    /// Appends a quad (two triangles) with normals, UVs derived from world-space and color.
    /// Prefer `add_quad_uv` when explicit UVs are available.
    pub fn add_quad(
        &mut self,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        n: Vec3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        // Default relative UVs (legacy)
        let uvs = [(0.0, 0.0), (0.0, v1), (u1, v1), (u1, 0.0)];
        self.add_quad_uv(a, b, c, d, n, uvs, flip_v, rgba);
    }

    /// Appends a quad with explicit per-vertex UVs.
    pub fn add_quad_uv(
        &mut self,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        n: Vec3,
        mut uvs: [(f32, f32); 4],
        _flip_v: bool,
        rgba: [u8; 4],
    ) {
        let base = self.pos.len() as u32 / 3;
        let mut vs = [a, d, c, b];
        let e1 = Vec3 {
            x: vs[1].x - vs[0].x,
            y: vs[1].y - vs[0].y,
            z: vs[1].z - vs[0].z,
        };
        let e2 = Vec3 {
            x: vs[2].x - vs[0].x,
            y: vs[2].y - vs[0].y,
            z: vs[2].z - vs[0].z,
        };
        let cross = Vec3 {
            x: e1.y * e2.z - e1.z * e2.y,
            y: e1.z * e2.x - e1.x * e2.z,
            z: e1.x * e2.y - e1.y * e2.x,
        };
        if (cross.x * n.x + cross.y * n.y + cross.z * n.z) < 0.0 {
            vs.swap(1, 3);
            uvs.swap(1, 3);
        }
        // Flip V axis so textures aren't upside-down (top-left origin vs bottom-left)
        for uv in &mut uvs {
            uv.1 = -uv.1;
        }
        for i in 0..4 {
            self.pos.extend_from_slice(&[vs[i].x, vs[i].y, vs[i].z]);
            self.norm.extend_from_slice(&[n.x, n.y, n.z]);
            self.uv.extend_from_slice(&[uvs[i].0, uvs[i].1]);
            self.col
                .extend_from_slice(&[rgba[0], rgba[1], rgba[2], rgba[3]]);
        }
        self.idx.extend_from_slice(&[
            base as u16,
            (base + 1) as u16,
            (base + 2) as u16,
            base as u16,
            (base + 2) as u16,
            (base + 3) as u16,
        ]);
    }

    /// Emits a face-aligned rectangle for the given face at `origin` with size `(u1,v1)`.
    pub fn add_face_rect(
        &mut self,
        face: Face,
        origin: Vec3,
        u1: f32,
        v1: f32,
        flip_v: bool,
        rgba: [u8; 4],
    ) {
        let n = face.normal();
        let (a, b, c, d) = match face {
            Face::PosY => (
                origin,
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + v1,
                },
            ),
            Face::NegY => (
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z + v1,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                origin,
            ),
            Face::PosX => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z + u1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                origin,
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + u1,
                },
            ),
            Face::NegX => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z + u1,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y,
                    z: origin.z + u1,
                },
                origin,
            ),
            Face::PosZ => (
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                origin,
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
            ),
            Face::NegZ => (
                Vec3 {
                    x: origin.x,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y + v1,
                    z: origin.z,
                },
                Vec3 {
                    x: origin.x + u1,
                    y: origin.y,
                    z: origin.z,
                },
                origin,
            ),
        };
        // Derive absolute UVs from world-space coordinates per face orientation
        let uv_from = |p: Vec3| match face {
            Face::PosY | Face::NegY => (p.x, p.z),
            Face::PosX | Face::NegX => (p.z, p.y),
            Face::PosZ | Face::NegZ => (p.x, p.y),
        };
        let uvs = [uv_from(a), uv_from(d), uv_from(c), uv_from(b)];
        self.add_quad_uv(a, b, c, d, n, uvs, flip_v, rgba);
    }

    /// Returns a slice of interleaved vertex positions (x,y,z per vertex).
    pub fn positions(&self) -> &[f32] {
        &self.pos
    }
    /// Returns a slice of interleaved vertex normals (x,y,z per vertex).
    pub fn normals(&self) -> &[f32] {
        &self.norm
    }
}
