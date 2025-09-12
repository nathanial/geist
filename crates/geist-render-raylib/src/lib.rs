//! Raylib-based GPU rendering utilities: conversions, upload, textures, shaders.
// Unsafe is required for Raylib mesh/model upload operations in this crate.

use geist_blocks::MaterialCatalog;
use geist_mesh_cpu::ChunkMeshCPU;
use raylib::prelude::*;
use std::collections::HashMap;

pub mod conv {
    use geist_geom::{Aabb, Vec3};

    pub fn vec3_to_rl(v: Vec3) -> raylib::prelude::Vector3 {
        raylib::prelude::Vector3::new(v.x, v.y, v.z)
    }

    pub fn vec3_from_rl(v: raylib::prelude::Vector3) -> Vec3 {
        Vec3 {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }

    pub fn aabb_to_rl(bb: Aabb) -> raylib::core::math::BoundingBox {
        raylib::core::math::BoundingBox::new(vec3_to_rl(bb.min), vec3_to_rl(bb.max))
    }

    pub fn aabb_from_rl(bb: raylib::core::math::BoundingBox) -> Aabb {
        Aabb {
            min: Vec3 {
                x: bb.min.x,
                y: bb.min.y,
                z: bb.min.z,
            },
            max: Vec3 {
                x: bb.max.x,
                y: bb.max.y,
                z: bb.max.z,
            },
        }
    }
}

pub struct TextureCache {
    pub map: HashMap<String, raylib::core::texture::Texture2D>,
}

impl TextureCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn get_ref(&self, key: &str) -> Option<&raylib::core::texture::Texture2D> {
        self.map.get(key)
    }
    pub fn replace_loaded(&mut self, key: String, tex: raylib::core::texture::Texture2D) {
        self.map.insert(key, tex);
    }
}

pub struct ChunkRender {
    pub cx: i32,
    pub cz: i32,
    pub bbox: raylib::core::math::BoundingBox,
    pub parts: Vec<(geist_blocks::types::MaterialId, raylib::core::models::Model)>,
    pub leaf_tint: Option<[f32; 3]>,
}

pub fn upload_chunk_mesh(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    cpu: ChunkMeshCPU,
    tex_cache: &mut TextureCache,
    mats: &MaterialCatalog,
) -> Option<ChunkRender> {
    let mut parts_gpu = Vec::new();
    for (mid, mb) in cpu.parts.into_iter() {
        let total_verts = mb.pos.len() / 3;
        if total_verts == 0 {
            continue;
        }
        let max_verts: usize = 65000;
        let total_quads = total_verts / 4;
        let max_quads = max_verts / 4;
        let mut q = 0usize;
        while q < total_quads {
            let take_q = (total_quads - q).min(max_quads);
            let v_start = q * 4;
            let v_count = take_q * 4;
            let mut raw: raylib::ffi::Mesh = unsafe { std::mem::zeroed() };
            raw.vertexCount = v_count as i32;
            raw.triangleCount = (take_q * 2) as i32;
            unsafe {
                let pos_start = v_start * 3;
                let pos_end = pos_start + v_count * 3;
                let norm_start = v_start * 3;
                let norm_end = norm_start + v_count * 3;
                let uv_start = v_start * 2;
                let uv_end = uv_start + v_count * 2;
                let col_start = v_start * 4;
                let col_end = col_start + v_count * 4;
                let vbytes = (v_count * 3 * std::mem::size_of::<f32>()) as u32;
                let nbytes = (v_count * 3 * std::mem::size_of::<f32>()) as u32;
                let tbytes = (v_count * 2 * std::mem::size_of::<f32>()) as u32;
                let cbytes = (v_count * 4 * std::mem::size_of::<u8>()) as u32;
                let ibytes = (take_q * 6 * std::mem::size_of::<u16>()) as u32;
                raw.vertices = raylib::ffi::MemAlloc(vbytes) as *mut f32;
                raw.normals = raylib::ffi::MemAlloc(nbytes) as *mut f32;
                raw.texcoords = raylib::ffi::MemAlloc(tbytes) as *mut f32;
                raw.colors = raylib::ffi::MemAlloc(cbytes) as *mut u8;
                raw.indices = raylib::ffi::MemAlloc(ibytes) as *mut u16;
                std::ptr::copy_nonoverlapping(
                    mb.pos[pos_start..pos_end].as_ptr(),
                    raw.vertices,
                    v_count * 3,
                );
                std::ptr::copy_nonoverlapping(
                    mb.norm[norm_start..norm_end].as_ptr(),
                    raw.normals,
                    v_count * 3,
                );
                std::ptr::copy_nonoverlapping(
                    mb.uv[uv_start..uv_end].as_ptr(),
                    raw.texcoords,
                    v_count * 2,
                );
                std::ptr::copy_nonoverlapping(
                    mb.col[col_start..col_end].as_ptr(),
                    raw.colors,
                    v_count * 4,
                );
                let idx_ptr = raw.indices;
                let mut write = 0usize;
                for i in 0..take_q {
                    let base = (i * 4) as u16;
                    let tri = [base, base + 1, base + 2, base, base + 2, base + 3];
                    let dst = idx_ptr.add(write);
                    std::ptr::copy_nonoverlapping(tri.as_ptr(), dst, 6);
                    write += 6;
                }
            }
            let mut mesh = unsafe { raylib::core::models::Mesh::from_raw(raw) };
            unsafe {
                mesh.upload(false);
            }
            let model = rl
                .load_model_from_mesh(thread, unsafe { mesh.make_weak() })
                .ok()?;
            let mut model = model;
            if let Some(mat) = model.materials_mut().get_mut(0) {
                if let Some(mdef) = mats.get(mid) {
                    let candidates: Vec<String> = mdef
                        .texture_candidates
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let chosen: Option<String> = candidates
                        .iter()
                        .find(|p| std::path::Path::new(p.as_str()).exists())
                        .cloned()
                        .or_else(|| candidates.first().cloned());
                    if let Some(path) = chosen {
                        let key = std::fs::canonicalize(&path)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or(path);
                        use std::collections::hash_map::Entry;
                        match tex_cache.map.entry(key.clone()) {
                            Entry::Occupied(e) => {
                                let tex = e.into_mut();
                                mat.set_material_texture(
                                    raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                    tex,
                                );
                            }
                            Entry::Vacant(v) => {
                                if let Ok(t) = rl.load_texture(thread, &key) {
                                    t.set_texture_filter(
                                        thread,
                                        raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                                    );
                                    t.set_texture_wrap(
                                        thread,
                                        raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                                    );
                                    v.insert(t);
                                    if let Some(tex) = tex_cache.get_ref(&key) {
                                        mat.set_material_texture(
                                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                            tex,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            parts_gpu.push((mid, model));
            q += take_q;
        }
    }
    Some(ChunkRender {
        cx: cpu.cx,
        cz: cpu.cz,
        bbox: conv::aabb_to_rl(cpu.bbox),
        parts: parts_gpu,
        leaf_tint: None,
    })
}

pub struct LeavesShader {
    pub shader: raylib::shaders::WeakShader,
    pub loc_fog_color: i32,
    pub loc_fog_start: i32,
    pub loc_fog_end: i32,
    pub loc_camera_pos: i32,
    pub loc_palette0: i32,
    pub loc_palette1: i32,
    pub loc_palette2: i32,
    pub loc_palette3: i32,
    pub loc_strength: i32,
}

impl LeavesShader {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let vs = "assets/shaders/voxel_fog_textured.vs";
        let fs = "assets/shaders/voxel_fog_leaves.fs";
        let shader_strong = rl.load_shader(thread, Some(vs), Some(fs));
        let shader = unsafe { shader_strong.make_weak() };
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        let loc_palette0 = shader.get_shader_location("palette0");
        let loc_palette1 = shader.get_shader_location("palette1");
        let loc_palette2 = shader.get_shader_location("palette2");
        let loc_palette3 = shader.get_shader_location("palette3");
        let loc_strength = shader.get_shader_location("autumnStrength");
        let mut s = Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
            loc_palette0,
            loc_palette1,
            loc_palette2,
            loc_palette3,
            loc_strength,
        };
        s.set_autumn_palette(
            [0.905, 0.678, 0.161],
            [0.847, 0.451, 0.122],
            [0.710, 0.200, 0.153],
            [0.431, 0.231, 0.039],
            1.0,
        );
        Some(s)
    }
    pub fn load_with_base(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        base: &std::path::Path,
    ) -> Option<Self> {
        let vs = base.join("assets/shaders/voxel_fog_textured.vs");
        let fs = base.join("assets/shaders/voxel_fog_leaves.fs");
        let shader_strong = rl.load_shader(
            thread,
            Some(vs.to_string_lossy().as_ref()),
            Some(fs.to_string_lossy().as_ref()),
        );
        let shader = unsafe { shader_strong.make_weak() };
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        let loc_palette0 = shader.get_shader_location("palette0");
        let loc_palette1 = shader.get_shader_location("palette1");
        let loc_palette2 = shader.get_shader_location("palette2");
        let loc_palette3 = shader.get_shader_location("palette3");
        let loc_strength = shader.get_shader_location("autumnStrength");
        let mut s = Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
            loc_palette0,
            loc_palette1,
            loc_palette2,
            loc_palette3,
            loc_strength,
        };
        s.set_autumn_palette(
            [0.905, 0.678, 0.161],
            [0.847, 0.451, 0.122],
            [0.710, 0.200, 0.153],
            [0.431, 0.231, 0.039],
            1.0,
        );
        Some(s)
    }
    pub fn set_autumn_palette(
        &mut self,
        p0: [f32; 3],
        p1: [f32; 3],
        p2: [f32; 3],
        p3: [f32; 3],
        strength: f32,
    ) {
        if self.loc_palette0 >= 0 {
            self.shader.set_shader_value(self.loc_palette0, p0);
        }
        if self.loc_palette1 >= 0 {
            self.shader.set_shader_value(self.loc_palette1, p1);
        }
        if self.loc_palette2 >= 0 {
            self.shader.set_shader_value(self.loc_palette2, p2);
        }
        if self.loc_palette3 >= 0 {
            self.shader.set_shader_value(self.loc_palette3, p3);
        }
        if self.loc_strength >= 0 {
            self.shader.set_shader_value(self.loc_strength, strength);
        }
    }
    pub fn update_frame_uniforms(
        &mut self,
        camera_pos: Vector3,
        fog_color: [f32; 3],
        fog_start: f32,
        fog_end: f32,
    ) {
        if self.loc_fog_color >= 0 {
            self.shader.set_shader_value(self.loc_fog_color, fog_color);
        }
        if self.loc_fog_start >= 0 {
            self.shader.set_shader_value(self.loc_fog_start, fog_start);
        }
        if self.loc_fog_end >= 0 {
            self.shader.set_shader_value(self.loc_fog_end, fog_end);
        }
        if self.loc_camera_pos >= 0 {
            let cam = [camera_pos.x, camera_pos.y, camera_pos.z];
            self.shader.set_shader_value(self.loc_camera_pos, cam);
        }
    }
}

pub struct FogShader {
    pub shader: raylib::shaders::WeakShader,
    pub loc_fog_color: i32,
    pub loc_fog_start: i32,
    pub loc_fog_end: i32,
    pub loc_camera_pos: i32,
}

impl FogShader {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let vs = "assets/shaders/voxel_fog_textured.vs";
        let fs = "assets/shaders/voxel_fog_textured.fs";
        let shader_strong = rl.load_shader(thread, Some(vs), Some(fs));
        let shader = unsafe { shader_strong.make_weak() };
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        Some(Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
        })
    }
    pub fn load_with_base(
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        base: &std::path::Path,
    ) -> Option<Self> {
        let vs = base.join("assets/shaders/voxel_fog_textured.vs");
        let fs = base.join("assets/shaders/voxel_fog_textured.fs");
        let shader_strong = rl.load_shader(
            thread,
            Some(vs.to_string_lossy().as_ref()),
            Some(fs.to_string_lossy().as_ref()),
        );
        let shader = unsafe { shader_strong.make_weak() };
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        Some(Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
        })
    }
    pub fn update_frame_uniforms(
        &mut self,
        camera_pos: Vector3,
        fog_color: [f32; 3],
        fog_start: f32,
        fog_end: f32,
    ) {
        if self.loc_fog_color >= 0 {
            self.shader.set_shader_value(self.loc_fog_color, fog_color);
        }
        if self.loc_fog_start >= 0 {
            self.shader.set_shader_value(self.loc_fog_start, fog_start);
        }
        if self.loc_fog_end >= 0 {
            self.shader.set_shader_value(self.loc_fog_end, fog_end);
        }
        if self.loc_camera_pos >= 0 {
            let cam = [camera_pos.x, camera_pos.y, camera_pos.z];
            self.shader.set_shader_value(self.loc_camera_pos, cam);
        }
    }
}
