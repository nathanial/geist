use std::path::Path;

use crate::voxel::{Block, TreeSpecies};

// Map a Sponge palette key like "minecraft:oak_log[axis=y]" to our Block
fn map_palette_key_to_block(key: &str) -> Block {
    // Strip states suffix if present
    let base = key.split('[').next().unwrap_or(key);
    match base {
        "minecraft:air" => Block::Air,
        "minecraft:stone" => Block::Stone,
        "minecraft:dirt" => Block::Dirt,
        "minecraft:grass_block" => Block::Grass,
        "minecraft:sand" => Block::Sand,
        "minecraft:snow_block" => Block::Snow,
        "minecraft:glowstone" => Block::Glowstone,
        "minecraft:beacon" => Block::Beacon,

        // Logs
        "minecraft:oak_log" => Block::Wood(TreeSpecies::Oak),
        "minecraft:birch_log" => Block::Wood(TreeSpecies::Birch),
        "minecraft:spruce_log" => Block::Wood(TreeSpecies::Spruce),
        "minecraft:jungle_log" => Block::Wood(TreeSpecies::Jungle),
        "minecraft:acacia_log" => Block::Wood(TreeSpecies::Acacia),
        "minecraft:dark_oak_log" => Block::Wood(TreeSpecies::DarkOak),

        // Leaves
        "minecraft:oak_leaves" => Block::Leaves(TreeSpecies::Oak),
        "minecraft:birch_leaves" => Block::Leaves(TreeSpecies::Birch),
        "minecraft:spruce_leaves" => Block::Leaves(TreeSpecies::Spruce),
        "minecraft:jungle_leaves" => Block::Leaves(TreeSpecies::Jungle),
        "minecraft:acacia_leaves" => Block::Leaves(TreeSpecies::Acacia),
        "minecraft:dark_oak_leaves" => Block::Leaves(TreeSpecies::DarkOak),

        // Common transparent/decoration blocks -> approximate as Air to avoid overfilling
        "minecraft:glass" | "minecraft:glass_pane" | "minecraft:torch" | "minecraft:lantern"
        | "minecraft:water" | "minecraft:lava" => Block::Air,

        _ => Block::Stone, // Fallback: keep structure shape visible
    }
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
                    if mapped == Block::Air { continue; }
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
