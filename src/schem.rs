use std::path::Path;

use crate::voxel::{Block, TreeSpecies};

// Map a Sponge palette key like "minecraft:oak_log[axis=y]" to our Block
fn base_from_key(key: &str) -> &str { key.split('[').next().unwrap_or(key) }

fn map_palette_key_to_block_opt(key: &str) -> Option<Block> {
    // Strip states suffix if present
    let base = base_from_key(key);
    match base {
        "minecraft:air" => Some(Block::Air),
        "minecraft:stone" => Some(Block::Stone),
        "minecraft:dirt" => Some(Block::Dirt),
        "minecraft:grass_block" => Some(Block::Grass),
        "minecraft:sand" => Some(Block::Sand),
        "minecraft:snow_block" => Some(Block::Snow),
        "minecraft:glowstone" => Some(Block::Glowstone),
        "minecraft:beacon" => Some(Block::Beacon),

        // Logs
        "minecraft:oak_log" => Some(Block::Wood(TreeSpecies::Oak)),
        "minecraft:birch_log" => Some(Block::Wood(TreeSpecies::Birch)),
        "minecraft:spruce_log" => Some(Block::Wood(TreeSpecies::Spruce)),
        "minecraft:jungle_log" => Some(Block::Wood(TreeSpecies::Jungle)),
        "minecraft:acacia_log" => Some(Block::Wood(TreeSpecies::Acacia)),
        "minecraft:dark_oak_log" => Some(Block::Wood(TreeSpecies::DarkOak)),

        // Leaves
        "minecraft:oak_leaves" => Some(Block::Leaves(TreeSpecies::Oak)),
        "minecraft:birch_leaves" => Some(Block::Leaves(TreeSpecies::Birch)),
        "minecraft:spruce_leaves" => Some(Block::Leaves(TreeSpecies::Spruce)),
        "minecraft:jungle_leaves" => Some(Block::Leaves(TreeSpecies::Jungle)),
        "minecraft:acacia_leaves" => Some(Block::Leaves(TreeSpecies::Acacia)),
        "minecraft:dark_oak_leaves" => Some(Block::Leaves(TreeSpecies::DarkOak)),

        // Common transparent/decoration blocks -> approximate as Air to avoid overfilling
        // Treat these as unsupported (None) rather than Air; we'll skip them in building if needed
        "minecraft:glass" | "minecraft:glass_pane" | "minecraft:torch" | "minecraft:lantern"
        | "minecraft:water" | "minecraft:lava" => None,

        _ => None,
    }
}

fn map_palette_key_to_block(key: &str) -> Block {
    // Fallback to stone to preserve shape during build
    map_palette_key_to_block_opt(key).unwrap_or(Block::Stone)
}

pub fn load_sponge_schem_apply_edits(
    path: &Path,
    origin: (i32, i32, i32),
    edits: &mut crate::edit::EditStore,
) -> Result<(usize, usize, usize), String> {
    // Load via mc_schem high-level API
    let (schem, _meta) = mc_schem::Schematic::from_file(
        path.to_str().ok_or_else(|| "invalid path".to_string())?,
    )
    .map_err(|e| format!("parse schem: {e}"))?;

    // Enclosing shape in global xyz
    let shape = schem.shape();
    let (sx, sy, sz) = (shape[0] as usize, shape[1] as usize, shape[2] as usize);
    let (ox, oy, oz) = origin;

    for x in 0..shape[0] {
        for y in 0..shape[1] {
            for z in 0..shape[2] {
                if let Some(b) = schem.first_block_at([x, y, z]) {
                    if b.is_air() || b.is_structure_void() { continue; }
                    let key = b.full_id(); // like "minecraft:oak_log[axis=y]"
                    let mapped = map_palette_key_to_block(&key);
                    let wx = ox + x;
                    let wy = oy + y;
                    let wz = oz + z;
                    edits.set(wx, wy, wz, mapped);
                }
            }
        }
    }

    Ok((sx, sy, sz))
}

pub fn find_unsupported_blocks_in_file(path: &Path) -> Result<Vec<String>, String> {
    let (schem, _meta) = mc_schem::Schematic::from_file(
        path.to_str().ok_or_else(|| "invalid path".to_string())?,
    )
    .map_err(|e| format!("parse schem: {e}"))?;

    // Use full palette across all regions
    let (palette, _lut) = schem.full_palette();
    let mut unsupported: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (blk, _hash) in palette {
        if blk.is_air() || blk.is_structure_void() { continue; }
        let id = blk.full_id();
        if map_palette_key_to_block_opt(&id).is_none() {
            // Record only the base id without attributes to reduce duplicates
            unsupported.insert(base_from_key(&id).to_string());
        }
    }
    Ok(unsupported.into_iter().collect())
}
