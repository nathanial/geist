use std::fs;
use std::path::{Path, PathBuf};
use serde::Deserialize;

use crate::blocks::{Block as RtBlock, BlockRegistry};

// Map a Sponge palette key like "minecraft:oak_log[axis=y]" to our Block
fn base_from_key(key: &str) -> &str {
    key.split('[').next().unwrap_or(key)
}

// axis_from_key removed with legacy enum mappings; axis is handled via registry state where needed

fn state_value<'a>(key: &'a str, name: &str) -> Option<&'a str> {
    if let Some(start) = key.find('[') {
        if let Some(end) = key[start + 1..].find(']') {
            let inner = &key[start + 1..start + 1 + end];
            for part in inner.split(',') {
                let part = part.trim();
                if let Some(val) = part.strip_prefix(&format!("{}=", name)) {
                    return Some(val);
                }
            }
        }
    }
    None
}

// --- Config-driven palette translator to runtime blocks ---

#[derive(Deserialize, Debug, Clone)]
struct PaletteMapConfig { rules: Vec<PaletteRule> }

#[derive(Deserialize, Debug, Clone)]
struct PaletteRule { from: String, to: ToDef }

#[derive(Deserialize, Debug, Clone)]
struct ToDef { name: String, #[serde(default)] state: std::collections::HashMap<String, String> }

fn load_palette_map() -> Option<PaletteMapConfig> {
    let path = std::path::Path::new("assets/voxels/palette_map.toml");
    let s = fs::read_to_string(path).ok()?;
    toml::from_str::<PaletteMapConfig>(&s).ok()
}

fn runtime_from_palette_key_with_lut(
    reg: &BlockRegistry,
    key: &str,
    lut: &std::collections::HashMap<String, ToDef>,
) -> Option<RtBlock> {
    let base = base_from_key(key);
    let to = lut.get(key).or_else(|| lut.get(base))?;
    // Start with rule-provided state
    let mut state = to.state.clone();
    // Supplement state from palette key attributes when relevant
    if to.name == "slab" {
        if let Some(t) = state_value(key, "type").or_else(|| state_value(key, "half")) {
            state.entry("half".to_string()).or_insert_with(|| match t {
                "top" => "top".to_string(),
                _ => "bottom".to_string(), // treat double/others as bottom for now
            });
        }
    } else if to.name == "stairs" {
        if let Some(h) = state_value(key, "half") {
            state.entry("half".to_string()).or_insert_with(|| match h {
                "top" => "top".to_string(),
                _ => "bottom".to_string(),
            });
        }
        if let Some(f) = state_value(key, "facing") {
            state.entry("facing".to_string()).or_insert_with(|| f.to_string());
        }
    }
    reg.make_block_by_name(&to.name, Some(&state))
}


pub fn load_any_schematic_apply_edits(
    path: &Path,
    origin: (i32, i32, i32),
    edits: &mut crate::edit::EditStore,
    reg: &BlockRegistry,
) -> Result<(usize, usize, usize), String> {
    let ext = path
        .extension()
        .and_then(|e| Some(e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    if ext == "schem" || ext == "schematic" {
        // Unified config-driven path using mc_schem palette keys + palette_map.toml
        load_sponge_schem_apply_edits(path, origin, edits, reg)
    } else {
        Err(format!("unsupported schematic extension: {:?}", path))
    }
}

pub fn load_sponge_schem_apply_edits(
    path: &Path,
    origin: (i32, i32, i32),
    edits: &mut crate::edit::EditStore,
    reg: &BlockRegistry,
) -> Result<(usize, usize, usize), String> {
    // Load via mc_schem high-level API
    let (schem, _meta) =
        mc_schem::Schematic::from_file(path.to_str().ok_or_else(|| "invalid path".to_string())?)
            .map_err(|e| format!("parse schem: {e}"))?;

    // Enclosing shape in global xyz
    let shape = schem.shape();
    let (sx, sy, sz) = (shape[0] as usize, shape[1] as usize, shape[2] as usize);
    let (ox, oy, oz) = origin;

    // Build a fast lookup table for palette mapping to avoid re-parsing TOML per block
    let lut: std::collections::HashMap<String, ToDef> = if let Some(cfg) = load_palette_map() {
        cfg.rules
            .into_iter()
            .map(|r| (r.from, r.to))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    for x in 0..shape[0] {
        for y in 0..shape[1] {
            for z in 0..shape[2] {
                if let Some(b) = schem.first_block_at([x, y, z]) {
                    if b.is_air() || b.is_structure_void() {
                        continue;
                    }
                    let key = b.full_id(); // like "minecraft:oak_log[axis=y]"
                    // Config-driven translator only; skip if no rule is present
                    let maybe_rt = if lut.is_empty() {
                        None
                    } else {
                        runtime_from_palette_key_with_lut(reg, &key, &lut)
                    };
                    let wx = ox + x;
                    let wy = oy + y;
                    let wz = oz + z;
                    // Fallback to configured unknown block when unmapped; panic if not configured
                    let rt = maybe_rt.unwrap_or_else(|| RtBlock { id: reg.unknown_block_id_or_panic(), state: 0 });
                    if rt.id != reg.id_by_name("air").unwrap_or(0) {
                        edits.set(wx, wy, wz, rt);
                    }
                }
            }
        }
    }

    Ok((sx, sy, sz))
}

pub fn find_unsupported_blocks_in_file(path: &Path) -> Result<Vec<String>, String> {
    let (schem, _meta) =
        mc_schem::Schematic::from_file(path.to_str().ok_or_else(|| "invalid path".to_string())?)
            .map_err(|e| format!("parse schem: {e}"))?;

    // Build a set of supported ids from palette_map.toml
    let rules = load_palette_map()
        .map(|cfg| cfg.rules.into_iter().map(|r| r.from).collect::<std::collections::HashSet<String>>())
        .unwrap_or_default();

    // Use full palette across all regions
    let (palette, _lut) = schem.full_palette();
    let mut unsupported: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (blk, _hash) in palette {
        if blk.is_air() || blk.is_structure_void() {
            continue;
        }
        let id = blk.full_id();
        let base = base_from_key(&id);
        if !(rules.contains(&id) || rules.contains(base)) {
            unsupported.insert(base.to_string());
        }
    }
    Ok(unsupported.into_iter().collect())
}

pub fn count_blocks_in_file(path: &Path) -> Result<Vec<(String, u64)>, String> {
    let (schem, _meta) =
        mc_schem::Schematic::from_file(path.to_str().ok_or_else(|| "invalid path".to_string())?)
            .map_err(|e| format!("parse schem: {e}"))?;

    let shape = schem.shape();
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for x in 0..shape[0] {
        for y in 0..shape[1] {
            for z in 0..shape[2] {
                if let Some(b) = schem.first_block_at([x, y, z]) {
                    if b.is_air() || b.is_structure_void() {
                        continue;
                    }
                    let key = b.full_id();
                    let id = base_from_key(&key).to_string();
                    *counts.entry(id).or_insert(0) += 1;
                }
            }
        }
    }
    Ok(counts.into_iter().collect())
}

#[derive(Clone, Debug)]
pub struct SchematicEntry {
    pub path: PathBuf,
    pub size: (i32, i32, i32),
}

pub fn list_schematics_with_size(dir: &Path) -> Result<Vec<SchematicEntry>, String> {
    let mut out = Vec::new();
    let rd = fs::read_dir(dir).map_err(|e| format!("read_dir {:?}: {}", dir, e))?;
    for ent in rd {
        let ent = ent.map_err(|e| format!("read_dir entry: {}", e))?;
        let p = ent.path();
        if p.is_file() {
            if let Some(ext) = p.extension() {
                let ext_s = ext.to_string_lossy();
                if ext_s.eq_ignore_ascii_case("schem") {
                    match mc_schem::Schematic::from_file(
                        p.to_str().ok_or_else(|| "invalid path".to_string())?,
                    ) {
                        Ok((schem, _meta)) => {
                            let shape = schem.shape();
                            out.push(SchematicEntry {
                                path: p,
                                size: (shape[0] as i32, shape[1] as i32, shape[2] as i32),
                            });
                        }
                        Err(e) => return Err(format!("parse schem {:?}: {}", p, e)),
                    }
                } else if ext_s.eq_ignore_ascii_case("schematic") {
                    // Try mc_schem for legacy .schematic as well; if it fails, skip sizing
                    match mc_schem::Schematic::from_file(
                        p.to_str().ok_or_else(|| "invalid path".to_string())?,
                    ) {
                        Ok((schem, _meta)) => {
                            let shape = schem.shape();
                            out.push(SchematicEntry {
                                path: p,
                                size: (shape[0] as i32, shape[1] as i32, shape[2] as i32),
                            });
                        }
                        Err(e) => {
                            log::warn!("parse schem {:?}: {}", p, e);
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}
