#[cfg(feature = "mcworld")]
use bedrock_hematite_nbt as bnbt;

#[cfg(feature = "mcworld")]
use std::io::Read;

#[cfg(feature = "mcworld")]
use zip::ZipArchive;

#[cfg(feature = "mcworld")]
use std::fs::File;

#[cfg(feature = "mcworld")]
use std::path::Path;

#[cfg(feature = "mcworld")]
#[derive(Debug)]
struct McStructure {
    size: (i32, i32, i32),
    // palette of block names (states ignored for now)
    palette: Vec<String>,
    // flattened indices into palette, length = sx*sy*sz
    indices: Vec<usize>,
}

#[cfg(feature = "mcworld")]
fn parse_mcstructure_le(reader: &mut dyn std::io::Read) -> Result<McStructure, String> {
    // Bedrock .mcstructure is an NBT-like little-endian format.
    // We'll parse it as a generic NBT compound and extract the bits we need.
    let nbt = bnbt::from_reader_le(reader).map_err(|e| format!("nbt: {}", e))?;
    let root = match nbt {
        bnbt::Value::Compound(c) => c,
        _ => return Err("root not a compound".into()),
    };
    // size: int array [x,y,z]
    let size = match root.get("size") {
        Some(bnbt::Value::List(xs)) => {
            let mut arr = [0i32; 3];
            for (i, it) in xs.iter().take(3).enumerate() {
                arr[i] = match it {
                    bnbt::Value::Int(v) => *v,
                    _ => 0,
                };
            }
            (arr[0], arr[1], arr[2])
        }
        _ => (0, 0, 0),
    };
    if size.0 <= 0 || size.1 <= 0 || size.2 <= 0 {
        return Err("invalid size in mcstructure".into());
    }

    // structure.palette[0].block_palette: list of compounds with name and states
    let mut palette: Vec<String> = Vec::new();
    let mut indices: Vec<usize> = Vec::new();
    if let Some(bnbt::Value::Compound(structure)) = root.get("structure") {
        if let Some(bnbt::Value::List(pals)) = structure.get("palette") {
            // We only read the first layer's palettes and indices
            if let Some(bnbt::Value::Compound(p0)) = pals.get(0) {
                // block_palette
                if let Some(bnbt::Value::List(bp)) = p0.get("block_palette") {
                    for ent in bp {
                        if let bnbt::Value::Compound(c) = ent {
                            if let Some(bnbt::Value::String(name)) = c.get("name") {
                                palette.push(name.clone());
                            }
                        }
                    }
                }
                // block_indices is an array of int arrays with RLE chunks; flatten
                if let Some(bnbt::Value::List(bis)) = p0.get("block_indices") {
                    // Each entry is a list of ints; we concatenate in order
                    for part in bis {
                        if let bnbt::Value::List(li) = part {
                            for v in li {
                                if let bnbt::Value::Int(ix) = v {
                                    indices.push((*ix) as usize);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Ensure size matches
    let expect = (size.0 as usize) * (size.1 as usize) * (size.2 as usize);
    if indices.len() != expect {
        // Some variants encode in layers differently; do a best-effort truncate/pad
        if indices.len() < expect {
            indices.resize(expect, 0);
        } else {
            indices.truncate(expect);
        }
    }
    Ok(McStructure {
        size,
        palette,
        indices,
    })
}

#[cfg(feature = "mcworld")]
pub fn load_mcworlds_in_dir(
    dir: &Path,
    base_y: i32,
    edits: &mut crate::edit::EditStore,
) -> Result<Vec<(String, (i32, i32, i32), (i32, i32, i32))>, String> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(dir).map_err(|e| format!("read_dir {:?}: {}", dir, e))? {
        let ent = ent.map_err(|e| format!("read_dir entry: {}", e))?;
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        if p.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("mcworld".to_string())
        {
            continue;
        }
        let f = File::open(&p).map_err(|e| format!("open {:?}: {}", p, e))?;
        let mut zip = ZipArchive::new(f).map_err(|e| format!("zip {:?}: {}", p, e))?;
        // Look for structures/*.mcstructure entries and import each
        for i in 0..zip.len() {
            let name = zip
                .by_index(i)
                .map_err(|e| e.to_string())?
                .name()
                .to_string()
                .replace('\\', "/");
            if !name.to_lowercase().ends_with(".mcstructure") {
                continue;
            }
            if !name.contains("structures/") {
                continue;
            }
            let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .map_err(|e| format!("read {}: {}", name, e))?;
            let mut cur = std::io::Cursor::new(buf);
            match parse_mcstructure_le(&mut cur) {
                Ok(ms) => {
                    // Place at world center progressively (0, base_y, 0) offset grid
                    let (sx, sy, sz) = ms.size;
                    // Convert palette to our blocks
                    let mut blocks: Vec<crate::voxel::Block> = Vec::with_capacity(ms.indices.len());
                    for &ix in &ms.indices {
                        let bname = ms
                            .palette
                            .get(ix)
                            .cloned()
                            .unwrap_or_else(|| "minecraft:air".to_string());
                        let b = super::schem::map_palette_key_to_block_opt(&bname)
                            .unwrap_or(crate::voxel::Block::Unknown);
                        blocks.push(b);
                    }
                    // Stamp as a local buffer at origin; then emit edits
                    let mut placed = 0usize;
                    for y in 0..sy {
                        for z in 0..sz {
                            for x in 0..sx {
                                let i = (y as usize) * (sz as usize) * (sx as usize)
                                    + (z as usize) * (sx as usize)
                                    + (x as usize);
                                let b = blocks[i];
                                if b != crate::voxel::Block::Air {
                                    edits.set(x as i32, base_y + y as i32, z as i32, b);
                                    placed += 1;
                                }
                            }
                        }
                    }
                    log::info!(
                        "Imported mcworld {:?} structure {} ({}x{}x{}) blocks={} at y={}.",
                        p,
                        name,
                        sx,
                        sy,
                        sz,
                        placed,
                        base_y
                    );
                    out.push((
                        format!("{}!{}", p.display(), name),
                        (0, base_y, 0),
                        (sx, sy, sz),
                    ));
                }
                Err(e) => {
                    log::warn!("Failed parsing mcstructure in {:?} ({}): {}", p, name, e);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(not(feature = "mcworld"))]
pub fn load_mcworlds_in_dir(
    _dir: &std::path::Path,
    _base_y: i32,
    _edits: &mut crate::edit::EditStore,
) -> Result<Vec<(String, (i32, i32, i32), (i32, i32, i32))>, String> {
    Err("mcworld support not enabled; recompile with --features mcworld".into())
}
