use geist_blocks::{Block, BlockRegistry};
use geist_chunk::generate_chunk_buffer;
use geist_lighting::{LightBorders, LightingStore};
use geist_mesh_cpu::{ChunkMeshCPU, NeighborsLoaded, build_chunk_wcc_cpu_buf};
use geist_render_raylib::{ChunkRender, TextureCache, upload_chunk_mesh};
use geist_world::voxel::{
    World, WorldGenMode, build_showcase_entries, build_showcase_stairs_cluster,
};
use raylib::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::SnapArgs;

struct CpuGpuChunk {
    cpu: ChunkMeshCPU,
    gpu: ChunkRender,
}

#[derive(Clone)]
struct ItemInfo {
    label: String,
    wx: i32,
    wy: i32,
    wz: i32,
    block: Block,
}

pub fn run_showcase_snapshots(args: SnapArgs) {
    // Init raylib (hidden window not guaranteed on all builds; use small titled window)
    unsafe { raylib::ffi::SetTraceLogLevel(7) };
    let (mut rl, thread) = raylib::init()
        .size(args.width.max(64), args.height.max(64))
        .title("Geist Showcase Snapshots")
        .msaa_4x()
        .build();
    unsafe { raylib::ffi::SetTraceLogLevel(7) };

    // Load registry
    let reg = std::sync::Arc::new(
        BlockRegistry::load_from_paths("assets/voxels/materials.toml", "assets/voxels/blocks.toml")
            .unwrap_or_else(|e| {
                log::warn!("Failed to load voxel registry ({}), using empty.", e);
                BlockRegistry::new()
            }),
    );

    // World
    let world = std::sync::Arc::new(World::new(
        args.chunks_x,
        args.chunks_z,
        args.chunk_size_x,
        args.chunk_size_y,
        args.chunk_size_z,
        args.seed,
        WorldGenMode::Showcase,
    ));
    // Load worldgen config if present
    if Path::new(&args.world_config).exists() {
        match geist_world::worldgen::load_params_from_path(Path::new(&args.world_config)) {
            Ok(p) => world.update_worldgen_params(p),
            Err(e) => log::warn!(
                "Failed to load worldgen config {}: {}",
                args.world_config,
                e
            ),
        }
    }

    // Prepare lighting + texture cache
    let lighting = std::sync::Arc::new(LightingStore::new(
        args.chunk_size_x,
        args.chunk_size_y,
        args.chunk_size_z,
    ));
    let mut tex_cache = TextureCache::new();

    // Build all chunks (CPU+GPU)
    let mut chunks: HashMap<(i32, i32), CpuGpuChunk> = HashMap::new();
    let mut borders_to_publish: Vec<((i32, i32), LightBorders)> = Vec::new();
    let neighbors_loaded = NeighborsLoaded {
        neg_x: true,
        pos_x: true,
        neg_z: true,
        pos_z: true,
    };
    for cz in 0..(args.chunks_z as i32) {
        for cx in 0..(args.chunks_x as i32) {
            let buf = generate_chunk_buffer(&world, cx, cz, &reg);
            if let Some((cpu, lb)) = build_chunk_wcc_cpu_buf(
                &buf,
                Some(&lighting),
                &world,
                None,
                neighbors_loaded,
                cx,
                cz,
                &reg,
            ) {
                // Keep a CPU copy for geometry dump
                let cpu_copy = ChunkMeshCPU {
                    cx: cpu.cx,
                    cz: cpu.cz,
                    bbox: cpu.bbox,
                    parts: cpu.parts.clone(),
                };
                if let Some(cr) =
                    upload_chunk_mesh(&mut rl, &thread, cpu, &mut tex_cache, &reg.materials)
                {
                    if let Some(b) = lb {
                        borders_to_publish.push(((cx, cz), b));
                    }
                    chunks.insert(
                        (cx, cz),
                        CpuGpuChunk {
                            cpu: cpu_copy,
                            gpu: cr,
                        },
                    );
                }
            }
        }
    }
    // Publish light borders so optional second pass could sample neighbors if needed
    for ((cx, cz), lb) in borders_to_publish.into_iter() {
        lighting.update_borders(cx, cz, lb);
    }

    // Prepare output dir
    let out_root = PathBuf::from(&args.out_dir);
    let _ = fs::create_dir_all(&out_root);

    // Build list of items: main row + stairs cluster
    let mut items: Vec<ItemInfo> = Vec::new();
    let params = { world.gen_params.read().map(|g| g.clone()).ok() };
    if let Some(p) = params {
        let mut row_y =
            (world.chunk_size_y as f32 * p.platform_y_ratio + p.platform_y_offset).round() as i32;
        row_y = row_y.clamp(1, world.chunk_size_y as i32 - 2);
        let cz_mid = (world.world_size_z() as i32) / 2;
        // Main row
        let entries = build_showcase_entries(&reg);
        if !entries.is_empty() {
            let spacing = 2i32;
            let row_len = (entries.len() as i32) * spacing - 1;
            let cx_mid = (world.world_size_x() as i32) / 2;
            let start_x = cx_mid - row_len / 2;
            for (i, e) in entries.iter().enumerate() {
                let bx = start_x + (i as i32) * spacing;
                items.push(ItemInfo {
                    label: e.label.clone(),
                    wx: bx,
                    wy: row_y,
                    wz: cz_mid,
                    block: e.block,
                });
            }
        }
        // Stairs cluster
        let placements = build_showcase_stairs_cluster(&reg);
        if !placements.is_empty() {
            let base_z = cz_mid + 3;
            let max_dx = placements.iter().map(|p| p.dx).max().unwrap_or(0);
            let cluster_w = max_dx + 1;
            let cx_mid = (world.world_size_x() as i32) / 2;
            let start_x = cx_mid - cluster_w / 2;
            for p in placements {
                let bx = start_x + p.dx;
                let bz = base_z + p.dz;
                items.push(ItemInfo {
                    label: p.label.clone(),
                    wx: bx,
                    wy: row_y,
                    wz: bz,
                    block: p.block,
                });
            }
        }
    }

    // Camera setup params
    let radius = 3.25f32;
    let fov_y = 40.0f32;
    rl.set_target_fps(60);

    // Collect XML manifest data in memory then write
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<showcase>\n");

    for item in &items {
        // Prepare subfolder
        let folder_name = sanitize_name(&item.label);
        let item_dir = out_root.join(&folder_name);
        let _ = fs::create_dir_all(&item_dir);

        // Camera target at block center
        let target = Vector3::new(
            item.wx as f32 + 0.5,
            item.wy as f32 + 0.5,
            item.wz as f32 + 0.5,
        );
        let mut shot_files: Vec<String> = Vec::new();
        let n = args.angles.max(1);
        for k in 0..n {
            let ang = (k as f32) * std::f32::consts::TAU / (n as f32);
            let dir = Vector3::new(ang.cos(), 0.0, ang.sin());
            let pos = target + dir * radius + Vector3::new(0.0, radius * 0.25, 0.0);
            let mut cam = Camera3D::perspective(pos, target, Vector3::up(), fov_y);

            // Draw scene
            let mut d = rl.begin_drawing(&thread);
            d.clear_background(Color::RAYWHITE);
            {
                let mut d3 = d.begin_mode3D(cam);
                // Draw all chunk models
                for cr in chunks.values() {
                    for (_mid, model) in &cr.gpu.parts {
                        d3.draw_model(model, Vector3::zero(), 1.0, Color::WHITE);
                    }
                }
            }
            // End frame, then capture pixels from screen and export to target path
            let fname = format!("angle_{:02}.png", k);
            let fpath = item_dir.join(&fname);
            drop(d);
            let img = rl.load_image_from_screen(&thread);
            img.export_image(fpath.to_string_lossy().as_ref());
            shot_files.push(format!("{}/{}", folder_name, fname));
        }

        // Dump faces for this block from its chunk CPU mesh
        let sx = world.chunk_size_x as i32;
        let sz = world.chunk_size_z as i32;
        let cx = item.wx.div_euclid(sx);
        let cz = item.wz.div_euclid(sz);
        let lx = item.wx.rem_euclid(sx) as i32;
        let lz = item.wz.rem_euclid(sz) as i32;
        let at_edge_xn = lx == 0;
        let at_edge_xp = lx == sx - 1;
        let at_edge_zn = lz == 0;
        let at_edge_zp = lz == sz - 1;
        let crosses_chunk_boundary = at_edge_xn || at_edge_xp || at_edge_zn || at_edge_zp;

        let mut faces: Vec<(String, [f32; 3], [[f32; 3]; 4])> = Vec::new();
        if let Some(c) = chunks.get(&(cx, cz)) {
            collect_block_faces(
                &c.cpu,
                &reg,
                item.wx as f32,
                item.wy as f32,
                item.wz as f32,
                &mut faces,
            );
        }

        // Resolve block name and state props
        let (block_name, state_props) = match reg.get(item.block.id) {
            Some(ty) => {
                let mut props: Vec<(String, String)> = Vec::new();
                for f in &ty.state_fields {
                    let v = ty.state_prop_value(item.block.state, &f.name).unwrap_or("");
                    props.push((f.name.clone(), v.to_string()));
                }
                (ty.name.clone(), props)
            }
            None => (format!("id:{}", item.block.id), Vec::new()),
        };

        // Write XML entry
        let mut entry = String::new();
        entry.push_str(&format!(
            "  <element label=\"{}\" block=\"{}\" block_id=\"{}\" state=\"{}\" wx=\"{}\" wy=\"{}\" wz=\"{}\" cx=\"{}\" cz=\"{}\" lx=\"{}\" lz=\"{}\" edge_xn=\"{}\" edge_xp=\"{}\" edge_zn=\"{}\" edge_zp=\"{}\" crosses_chunk_boundary=\"{}\">\n",
            escape_xml(&item.label),
            escape_xml(&block_name),
            item.block.id,
            item.block.state,
            item.wx,
            item.wy,
            item.wz,
            cx,
            cz,
            lx,
            lz,
            at_edge_xn,
            at_edge_xp,
            at_edge_zn,
            at_edge_zp,
            crosses_chunk_boundary
        ));
        // Screenshots
        entry.push_str("    <screenshots>\n");
        for s in &shot_files {
            entry.push_str(&format!("      <img path=\"{}\"/>\n", escape_xml(s)));
        }
        entry.push_str("    </screenshots>\n");
        // State props
        if !state_props.is_empty() {
            entry.push_str("    <state>\n");
            for (k, v) in state_props {
                entry.push_str(&format!(
                    "      <prop name=\"{}\" value=\"{}\"/>\n",
                    escape_xml(&k),
                    escape_xml(&v)
                ));
            }
            entry.push_str("    </state>\n");
        }
        // Faces
        entry.push_str("    <faces>\n");
        for (mat, n, verts) in faces {
            entry.push_str(&format!(
                "      <face material=\"{}\" nx=\"{:.6}\" ny=\"{:.6}\" nz=\"{:.6}\">\n",
                escape_xml(&mat),
                n[0],
                n[1],
                n[2]
            ));
            for v in &verts {
                entry.push_str(&format!(
                    "        <v x=\"{:.6}\" y=\"{:.6}\" z=\"{:.6}\"/>\n",
                    v[0], v[1], v[2]
                ));
            }
            entry.push_str("      </face>\n");
        }
        entry.push_str("    </faces>\n");
        entry.push_str("  </element>\n");
        xml.push_str(&entry);
    }

    xml.push_str("</showcase>\n");
    let manifest_path = out_root.join("manifest.xml");
    if let Ok(mut f) = fs::File::create(&manifest_path) {
        let _ = f.write_all(xml.as_bytes());
        let _ = f.flush();
        println!("Wrote {}", manifest_path.display());
    } else {
        eprintln!("Failed to write {}", manifest_path.display());
    }
}

fn collect_block_faces(
    cpu: &ChunkMeshCPU,
    reg: &BlockRegistry,
    wx: f32,
    wy: f32,
    wz: f32,
    out: &mut Vec<(String, [f32; 3], [[f32; 3]; 4])>,
) {
    let eps = 1.0e-4f32;
    let bx = [wx, wx + 1.0];
    let by = [wy, wy + 1.0];
    let bz = [wz, wz + 1.0];
    let nearly_eq = |a: f32, b: f32| (a - b).abs() <= 1.0e-3;
    let overlaps =
        |a0: f32, a1: f32, b0: f32, b1: f32| a0.min(a1) <= b1 + eps && a1.max(a0) >= b0 - eps;
    for (mid, mb) in &cpu.parts {
        let mat_key = reg
            .materials
            .get(*mid)
            .map(|m| m.key.clone())
            .unwrap_or_else(|| format!("mat:{}", mid.0));
        let verts = &mb.positions();
        let norms = &mb.normals();
        let total_quads = verts.len() / (3 * 4);
        for i in 0..total_quads {
            let vbase = i * 4 * 3;
            let v = [
                [verts[vbase + 0], verts[vbase + 1], verts[vbase + 2]],
                [verts[vbase + 3], verts[vbase + 4], verts[vbase + 5]],
                [verts[vbase + 6], verts[vbase + 7], verts[vbase + 8]],
                [verts[vbase + 9], verts[vbase + 10], verts[vbase + 11]],
            ];
            let nbase = i * 4 * 3;
            let n = [norms[nbase + 0], norms[nbase + 1], norms[nbase + 2]];
            // Determine primary axis by normal
            let ax = n[0].abs();
            let ay = n[1].abs();
            let az = n[2].abs();
            let axis = if ax > ay && ax > az {
                0
            } else if ay > az {
                1
            } else {
                2
            };
            // Compute rectangle bounds on the two non-normal axes
            let mut x_min = f32::INFINITY;
            let mut x_max = f32::NEG_INFINITY;
            let mut y_min = f32::INFINITY;
            let mut y_max = f32::NEG_INFINITY;
            let mut z_min = f32::INFINITY;
            let mut z_max = f32::NEG_INFINITY;
            for p in &v {
                if p[0] < x_min {
                    x_min = p[0];
                }
                if p[0] > x_max {
                    x_max = p[0];
                }
                if p[1] < y_min {
                    y_min = p[1];
                }
                if p[1] > y_max {
                    y_max = p[1];
                }
                if p[2] < z_min {
                    z_min = p[2];
                }
                if p[2] > z_max {
                    z_max = p[2];
                }
            }
            // Plane coordinate to check coincidence with block face plane
            let plane_ok = match axis {
                0 => {
                    // X-constant plane; must match either wx or wx+1
                    let x0 = (v[0][0] + v[1][0] + v[2][0] + v[3][0]) * 0.25;
                    nearly_eq(x0, bx[0]) || nearly_eq(x0, bx[1])
                }
                1 => {
                    let y0 = (v[0][1] + v[1][1] + v[2][1] + v[3][1]) * 0.25;
                    nearly_eq(y0, by[0]) || nearly_eq(y0, by[1])
                }
                _ => {
                    let z0 = (v[0][2] + v[1][2] + v[2][2] + v[3][2]) * 0.25;
                    nearly_eq(z0, bz[0]) || nearly_eq(z0, bz[1])
                }
            };
            if !plane_ok {
                continue;
            }
            // Overlap test in the other two axes
            let intersects = match axis {
                0 => overlaps(y_min, y_max, by[0], by[1]) && overlaps(z_min, z_max, bz[0], bz[1]),
                1 => overlaps(x_min, x_max, bx[0], bx[1]) && overlaps(z_min, z_max, bz[0], bz[1]),
                _ => overlaps(x_min, x_max, bx[0], bx[1]) && overlaps(y_min, y_max, by[0], by[1]),
            };
            if !intersects {
                continue;
            }
            out.push((mat_key.clone(), n, v));
        }
    }
}

fn sanitize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch == ' ' {
            out.push('_');
        } else {
            out.push('_');
        }
    }
    out
}

fn escape_xml(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            _ => c.to_string(),
        })
        .collect::<String>()
}
