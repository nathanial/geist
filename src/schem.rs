use std::path::{Path, PathBuf};
use std::fs;

use crate::voxel::{Block, TerracottaColor, TreeSpecies};

// Map a Sponge palette key like "minecraft:oak_log[axis=y]" to our Block
fn base_from_key(key: &str) -> &str { key.split('[').next().unwrap_or(key) }

fn axis_from_key(key: &str) -> Option<crate::voxel::Axis> {
    if let Some(start) = key.find('[') {
        if let Some(end) = key[start + 1..].find(']') {
            let inner = &key[start + 1..start + 1 + end];
            for part in inner.split(',') {
                let part = part.trim();
                if let Some(val) = part.strip_prefix("axis=") {
                    return match val {
                        "x" => Some(crate::voxel::Axis::X),
                        "y" => Some(crate::voxel::Axis::Y),
                        "z" => Some(crate::voxel::Axis::Z),
                        _ => None,
                    };
                }
            }
        }
    }
    None
}

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
        "minecraft:quartz_pillar" => Some(Block::QuartzPillar(axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:lapis_block" => Some(Block::LapisBlock),
        "minecraft:coal_block" => Some(Block::CoalBlock),
        "minecraft:prismarine_bricks" => Some(Block::PrismarineBricks),
        "minecraft:nether_bricks" => Some(Block::NetherBricks),
        "minecraft:end_stone" => Some(Block::EndStone),
        "minecraft:end_stone_bricks" => Some(Block::EndStoneBricks),

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

        // Logs with bark all around; approximate with logs for now
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

        // Logs with axis
        "minecraft:oak_log" => Some(Block::LogAxis(TreeSpecies::Oak, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:birch_log" => Some(Block::LogAxis(TreeSpecies::Birch, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:spruce_log" => Some(Block::LogAxis(TreeSpecies::Spruce, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:jungle_log" => Some(Block::LogAxis(TreeSpecies::Jungle, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:acacia_log" => Some(Block::LogAxis(TreeSpecies::Acacia, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),
        "minecraft:dark_oak_log" => Some(Block::LogAxis(TreeSpecies::DarkOak, axis_from_key(key).unwrap_or(crate::voxel::Axis::Y))),

        // Leaves
        "minecraft:oak_leaves" => Some(Block::Leaves(TreeSpecies::Oak)),
        "minecraft:birch_leaves" => Some(Block::Leaves(TreeSpecies::Birch)),
        "minecraft:spruce_leaves" => Some(Block::Leaves(TreeSpecies::Spruce)),
        "minecraft:jungle_leaves" => Some(Block::Leaves(TreeSpecies::Jungle)),
        "minecraft:acacia_leaves" => Some(Block::Leaves(TreeSpecies::Acacia)),
        "minecraft:dark_oak_leaves" => Some(Block::Leaves(TreeSpecies::DarkOak)),

        // Terracotta (hardened clay)
        "minecraft:terracotta" => Some(Block::TerracottaPlain),
        "minecraft:white_terracotta" => Some(Block::Terracotta(TerracottaColor::White)),
        "minecraft:orange_terracotta" => Some(Block::Terracotta(TerracottaColor::Orange)),
        "minecraft:magenta_terracotta" => Some(Block::Terracotta(TerracottaColor::Magenta)),
        "minecraft:light_blue_terracotta" => Some(Block::Terracotta(TerracottaColor::LightBlue)),
        "minecraft:yellow_terracotta" => Some(Block::Terracotta(TerracottaColor::Yellow)),
        "minecraft:lime_terracotta" => Some(Block::Terracotta(TerracottaColor::Lime)),
        "minecraft:pink_terracotta" => Some(Block::Terracotta(TerracottaColor::Pink)),
        "minecraft:gray_terracotta" => Some(Block::Terracotta(TerracottaColor::Gray)),
        "minecraft:light_gray_terracotta" => Some(Block::Terracotta(TerracottaColor::LightGray)),
        "minecraft:cyan_terracotta" => Some(Block::Terracotta(TerracottaColor::Cyan)),
        "minecraft:purple_terracotta" => Some(Block::Terracotta(TerracottaColor::Purple)),
        "minecraft:blue_terracotta" => Some(Block::Terracotta(TerracottaColor::Blue)),
        "minecraft:brown_terracotta" => Some(Block::Terracotta(TerracottaColor::Brown)),
        "minecraft:green_terracotta" => Some(Block::Terracotta(TerracottaColor::Green)),
        "minecraft:red_terracotta" => Some(Block::Terracotta(TerracottaColor::Red)),
        "minecraft:black_terracotta" => Some(Block::Terracotta(TerracottaColor::Black)),

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

// Fallback: MCEdit/old WorldEdit .schematic loader via NBT (fastnbt + optional gzip)
#[derive(Debug, serde::Deserialize)]
struct MCSchematicNBT {
    Width: i16,
    Height: i16,
    Length: i16,
    Blocks: Vec<u8>,
    Data: Vec<u8>,
    #[serde(default)]
    AddBlocks: Vec<u8>,
    #[serde(default)]
    WEOffsetX: i32,
    #[serde(default)]
    WEOffsetY: i32,
    #[serde(default)]
    WEOffsetZ: i32,
}

fn nbt_schematic_from_file(path: &Path) -> Result<MCSchematicNBT, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {:?}: {}", path, e))?;
    let mut data = Vec::new();
    f.read_to_end(&mut data).map_err(|e| format!("read {:?}: {}", path, e))?;
    // Try gzip-decompress
    let mut gz = flate2::read::GzDecoder::new(&data[..]);
    let mut dec = Vec::new();
    match std::io::Read::read_to_end(&mut gz, &mut dec) {
        Ok(_) => match from_bytes::<MCSchematicNBT>(&dec) {
            Ok(n) => Ok(n),
            Err(e) => {
                // Try raw
                from_bytes::<MCSchematicNBT>(&data)
                    .map_err(|e2| format!("parse NBT {:?}: {} / {}", path, e, e2))
            }
        },
        Err(_) => from_bytes::<MCSchematicNBT>(&data)
            .map_err(|e| format!("parse NBT {:?}: {}", path, e)),
    }
}

fn numeric_id_to_block(id: u16, data: u8) -> Block {
    use crate::voxel::{Block::*, TreeSpecies::*};
    match id {
        0 => Air,
        1 => Stone,
        2 => Grass,
        3 => Dirt,
        4 => Cobblestone,
        5 => {
            let sp = match (data & 0x7) as u8 { // lower 3 bits
                0 => Oak,
                1 => Spruce,
                2 => Birch,
                3 => Jungle,
                4 => Acacia,
                5 => DarkOak,
                _ => Oak,
            };
            Planks(sp)
        }
        12 => Sand,
        13 => Gravel,
        17 => { // Logs (include axis when available)
            let sp = match (data & 0x3) as u8 { // bottom 2 bits species in older versions
                0 => Oak,
                1 => Spruce,
                2 => Birch,
                3 => Jungle,
                _ => Oak,
            };
            let axis_bits = data & 0xC;
            match axis_bits {
                0x0 => LogAxis(sp, crate::voxel::Axis::Y),
                0x4 => LogAxis(sp, crate::voxel::Axis::X),
                0x8 => LogAxis(sp, crate::voxel::Axis::Z),
                0xC => Wood(sp), // bark on all sides
                _ => LogAxis(sp, crate::voxel::Axis::Y),
            }
        }
        18 => { // Leaves (ignore decay flags)
            let sp = match (data & 0x3) as u8 {
                0 => Oak,
                1 => Spruce,
                2 => Birch,
                3 => Jungle,
                _ => Oak,
            };
            Leaves(sp)
        }
        22 => LapisBlock,
        24 => { // Sandstone variants
            match data as u8 {
                2 => SmoothSandstone,
                _ => Sandstone,
            }
        }
        45 => Brick,
        47 => Bookshelf,
        80 => Snow,
        89 => Glowstone,
        98 => { // Stone bricks variants
            match data as u8 {
                1 => MossyStoneBricks,
                _ => StoneBricks,
            }
        }
        112 => NetherBricks,
        155 => QuartzBlock,
        159 => { // stained hardened clay (terracotta) colored variants
            let c = match (data & 0x0F) as u8 {
                0 => TerracottaColor::White,
                1 => TerracottaColor::Orange,
                2 => TerracottaColor::Magenta,
                3 => TerracottaColor::LightBlue,
                4 => TerracottaColor::Yellow,
                5 => TerracottaColor::Lime,
                6 => TerracottaColor::Pink,
                7 => TerracottaColor::Gray,
                8 => TerracottaColor::LightGray,
                9 => TerracottaColor::Cyan,
                10 => TerracottaColor::Purple,
                11 => TerracottaColor::Blue,
                12 => TerracottaColor::Brown,
                13 => TerracottaColor::Green,
                14 => TerracottaColor::Red,
                15 => TerracottaColor::Black,
                _ => TerracottaColor::White,
            };
            Terracotta(c)
        }
        168 => PrismarineBricks, // approximate all variants
        172 => TerracottaPlain, // hardened clay (terracotta base)
        173 => CoalBlock,
        179 => { // Red sandstone
            match data as u8 {
                2 => SmoothRedSandstone,
                _ => RedSandstone,
            }
        }
        121 => EndStone, // end stone legacy id
        _ => Air,
    }
}

pub fn load_mcedit_schematic_apply_edits(
    path: &Path,
    origin: (i32, i32, i32),
    edits: &mut crate::edit::EditStore,
) -> Result<(usize, usize, usize), String> {
    let nbt = nbt_schematic_from_file(path)?;
    let w = nbt.Width as i32;
    let h = nbt.Height as i32;
    let l = nbt.Length as i32;
    let blocks = &nbt.Blocks;
    let data = &nbt.Data;
    let add = &nbt.AddBlocks;
    let (mut ox, mut oy, mut oz) = origin;
    // Apply WEOffset if present (serde default is 0 when missing)
    ox += nbt.WEOffsetX;
    oy += nbt.WEOffsetY;
    oz += nbt.WEOffsetZ;
    let total = (w as usize) * (h as usize) * (l as usize);
    if blocks.len() != total || data.len() != total {
        log::warn!(".schematic arrays size mismatch: blocks={}, data={}, expected={}", blocks.len(), data.len(), total);
    }
    // Helper to read high 4 bits for id i
    let high4 = |i: usize| -> u16 {
        if add.is_empty() { return 0; }
        let half = i >> 1;
        let byte = add.get(half).copied().unwrap_or(0) as u16;
        if (i & 1) == 0 { (byte & 0x0F) as u16 } else { ((byte >> 4) & 0x0F) as u16 }
    };
    // Index order: (y * Length + z) * Width + x
    let mut unsupported: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    for y in 0..h {
        for z in 0..l {
            for x in 0..w {
                let i = (y as usize) * (l as usize) * (w as usize) + (z as usize) * (w as usize) + (x as usize);
                let id_low = *blocks.get(i).unwrap_or(&0) as u16;
                let id = id_low | (high4(i) << 8);
                let dv = *data.get(i).unwrap_or(&0) as u8;
                let b = numeric_id_to_block(id, dv);
                if matches!(b, Block::Air) {
                    if id != 0 { unsupported.insert(id); }
                } else {
                    edits.set(ox + x, oy + y, oz + z, b);
                }
            }
        }
    }
    if !unsupported.is_empty() {
        let ids: Vec<String> = unsupported.iter().map(|v| v.to_string()).collect();
        log::info!(".schematic unsupported numeric block IDs encountered (mapped to air): {}", ids.join(", "));
    }
    Ok((w as usize, h as usize, l as usize))
}

pub fn load_any_schematic_apply_edits(
    path: &Path,
    origin: (i32, i32, i32),
    edits: &mut crate::edit::EditStore,
) -> Result<(usize, usize, usize), String> {
    let ext = path.extension().and_then(|e| Some(e.to_string_lossy().to_lowercase())).unwrap_or_default();
    if ext == "schem" {
        load_sponge_schem_apply_edits(path, origin, edits)
    } else if ext == "schematic" {
        match load_mcedit_schematic_apply_edits(path, origin, edits) {
            Ok(s) => Ok(s),
            Err(e) => {
                // As a last resort, try mc_schem parser if available
                match mc_schem::Schematic::from_file(path.to_str().ok_or_else(|| "invalid path".to_string())?) {
                    Ok(_s) => load_sponge_schem_apply_edits(path, origin, edits),
                    Err(_) => Err(e),
                }
            }
        }
    } else {
        Err(format!("unsupported schematic extension: {:?}", path))
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
                let ext_s = ext.to_string_lossy();
                if ext_s.eq_ignore_ascii_case("schem") {
                    match mc_schem::Schematic::from_file(p.to_str().ok_or_else(|| "invalid path".to_string())?) {
                        Ok((schem, _meta)) => {
                            let shape = schem.shape();
                            out.push(SchematicEntry { path: p, size: (shape[0] as i32, shape[1] as i32, shape[2] as i32) });
                        }
                        Err(e) => return Err(format!("parse schem {:?}: {}", p, e)),
                    }
                } else if ext_s.eq_ignore_ascii_case("schematic") {
                    // Fallback to NBT to get sizes even if mc_schem cannot parse due to missing tags
                    let nbt = nbt_schematic_from_file(&p)?;
                    out.push(SchematicEntry { path: p, size: (nbt.Width as i32, nbt.Height as i32, nbt.Length as i32) });
                }
            }
        }
    }
    Ok(out)
}
use fastnbt::from_bytes;
