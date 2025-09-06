use std::path::Path;
use std::path::PathBuf;
use std::fs;

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

        // Common cubes
        "minecraft:cobblestone" => Some(Block::Cobblestone),
        "minecraft:mossy_cobblestone" => Some(Block::MossyCobblestone),
        "minecraft:stone_bricks" => Some(Block::StoneBricks),
        "minecraft:chiseled_stone_bricks" => Some(Block::StoneBricks),
        "minecraft:mossy_stone_bricks" => Some(Block::MossyStoneBricks),
        "minecraft:bricks" => Some(Block::Brick),
        "minecraft:granite" => Some(Block::Granite),
        "minecraft:diorite" => Some(Block::Diorite),
        "minecraft:andesite" => Some(Block::Andesite),
        "minecraft:polished_granite" => Some(Block::PolishedGranite),
        "minecraft:polished_diorite" => Some(Block::PolishedDiorite),
        "minecraft:polished_andesite" => Some(Block::PolishedAndesite),
        "minecraft:gravel" => Some(Block::Gravel),
        "minecraft:smooth_stone" => Some(Block::SmoothStone),
        "minecraft:sandstone" => Some(Block::Sandstone),
        "minecraft:smooth_sandstone" => Some(Block::SmoothSandstone),
        "minecraft:cut_sandstone" => Some(Block::SmoothSandstone),
        "minecraft:red_sandstone" => Some(Block::RedSandstone),
        "minecraft:smooth_red_sandstone" => Some(Block::SmoothRedSandstone),
        "minecraft:cut_red_sandstone" => Some(Block::SmoothRedSandstone),
        "minecraft:quartz_block" => Some(Block::QuartzBlock),
        "minecraft:chiseled_quartz_block" => Some(Block::QuartzBlock),
        "minecraft:quartz_pillar" => Some(Block::QuartzBlock),
        "minecraft:lapis_block" => Some(Block::LapisBlock),
        "minecraft:coal_block" => Some(Block::CoalBlock),
        "minecraft:prismarine_bricks" => Some(Block::PrismarineBricks),
        "minecraft:nether_bricks" => Some(Block::NetherBricks),

        // Planks
        "minecraft:oak_planks" => Some(Block::Planks(TreeSpecies::Oak)),
        "minecraft:birch_planks" => Some(Block::Planks(TreeSpecies::Birch)),
        "minecraft:spruce_planks" => Some(Block::Planks(TreeSpecies::Spruce)),
        "minecraft:jungle_planks" => Some(Block::Planks(TreeSpecies::Jungle)),
        "minecraft:acacia_planks" => Some(Block::Planks(TreeSpecies::Acacia)),
        "minecraft:dark_oak_planks" => Some(Block::Planks(TreeSpecies::DarkOak)),

        // Dirt-like variants
        "minecraft:coarse_dirt" => Some(Block::CoarseDirt),
        "minecraft:podzol" => Some(Block::Podzol),

        // Bookshelf
        "minecraft:bookshelf" => Some(Block::Bookshelf),
        "minecraft:chiseled_bookshelf" => Some(Block::Bookshelf),

        // Logs with bark all around; approximate with logs
        "minecraft:oak_wood" => Some(Block::Wood(TreeSpecies::Oak)),
        "minecraft:birch_wood" => Some(Block::Wood(TreeSpecies::Birch)),
        "minecraft:spruce_wood" => Some(Block::Wood(TreeSpecies::Spruce)),
        "minecraft:jungle_wood" => Some(Block::Wood(TreeSpecies::Jungle)),
        "minecraft:acacia_wood" => Some(Block::Wood(TreeSpecies::Acacia)),
        "minecraft:dark_oak_wood" => Some(Block::Wood(TreeSpecies::DarkOak)),

        // Ores -> approximate as stone for now
        "minecraft:coal_ore" => Some(Block::Stone),
        "minecraft:iron_ore" => Some(Block::Stone),
        "minecraft:gold_ore" => Some(Block::Stone),
        "minecraft:copper_ore" => Some(Block::Stone),
        "minecraft:redstone_ore" => Some(Block::Stone),
        "minecraft:lapis_ore" => Some(Block::Stone),
        "minecraft:diamond_ore" => Some(Block::Stone),
        "minecraft:emerald_ore" => Some(Block::Stone),
        "minecraft:quartz_ore" => Some(Block::Stone),

        // Other dirt-likes
        "minecraft:rooted_dirt" => Some(Block::Dirt),

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
    map_palette_key_to_block_opt(key).unwrap_or(Block::Air)
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

pub fn count_blocks_in_file(path: &Path) -> Result<Vec<(String, u64)>, String> {
    let (schem, _meta) = mc_schem::Schematic::from_file(
        path.to_str().ok_or_else(|| "invalid path".to_string())?,
    )
    .map_err(|e| format!("parse schem: {e}"))?;

    let shape = schem.shape();
    let mut counts: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for x in 0..shape[0] {
        for y in 0..shape[1] {
            for z in 0..shape[2] {
                if let Some(b) = schem.first_block_at([x, y, z]) {
                    if b.is_air() || b.is_structure_void() { continue; }
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
                if ext == "schem" {
                    let (schem, _meta) = mc_schem::Schematic::from_file(
                        p.to_str().ok_or_else(|| "invalid path".to_string())?,
                    )
                    .map_err(|e| format!("parse schem {:?}: {}", p, e))?;
                    let shape = schem.shape();
                    out.push(SchematicEntry {
                        path: p,
                        size: (shape[0] as i32, shape[1] as i32, shape[2] as i32),
                    });
                }
            }
        }
    }
    Ok(out)
}
