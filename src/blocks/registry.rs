use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use super::config::{BlocksConfig, MaterialSelector, MaterialsDef, ShapeConfig, ShapeDetailed};
use super::material::MaterialCatalog;
use super::types::{Block, BlockId, BlockState, FaceRole, MaterialId, Shape};

#[derive(Clone, Debug)]
pub struct BlockType {
    pub id: BlockId,
    pub name: String,
    pub solid: bool,
    pub blocks_skylight: bool,
    pub propagates_light: bool,
    pub emission: u8,
    pub shape: Shape,
    pub materials: CompiledMaterials,
    pub state_schema: HashMap<String, Vec<String>>, // property name -> allowed values
    // Precomputed, sorted layout for fast state packing/unpacking
    pub state_fields: Vec<StateField>,
    pub prop_index: HashMap<String, usize>,
}

#[derive(Clone, Debug)]
pub struct StateField {
    pub name: String,
    pub values: Vec<String>,
    pub bits: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CompiledMaterials {
    pub all: Option<ResolvedSelector>,
    pub top: Option<ResolvedSelector>,
    pub bottom: Option<ResolvedSelector>,
    pub side: Option<ResolvedSelector>,
}

#[derive(Clone, Debug)]
pub enum ResolvedSelector {
    Fixed(MaterialId),
    By { by: String, map: HashMap<String, MaterialId> },
}

impl CompiledMaterials {
    pub fn material_for(&self, role: FaceRole, state: BlockState, ty: &BlockType) -> Option<MaterialId> {
        // Pick selector by face role with fallback to `all`
        let pick = match role {
            FaceRole::Top => self.top.as_ref().or(self.all.as_ref()),
            FaceRole::Bottom => self.bottom.as_ref().or(self.all.as_ref()),
            FaceRole::Side => self.side.as_ref().or(self.all.as_ref()),
            FaceRole::All => self.all.as_ref(),
        }?;
        match pick {
            ResolvedSelector::Fixed(id) => Some(*id),
            ResolvedSelector::By { by, map } => {
                if let Some(val) = ty.state_prop_value(state, by) {
                    map.get(val).copied()
                } else {
                    None
                }
            }
        }
    }

    pub fn material_for_props(
        &self,
        role: FaceRole,
        props: &std::collections::HashMap<String, String>,
    ) -> Option<MaterialId> {
        let pick = match role {
            FaceRole::Top => self.top.as_ref().or(self.all.as_ref()),
            FaceRole::Bottom => self.bottom.as_ref().or(self.all.as_ref()),
            FaceRole::Side => self.side.as_ref().or(self.all.as_ref()),
            FaceRole::All => self.all.as_ref(),
        }?;
        match pick {
            ResolvedSelector::Fixed(id) => Some(*id),
            ResolvedSelector::By { by, map } => {
                if let Some(val) = props.get(by) {
                    map.get(val).copied()
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct BlockRegistry {
    pub materials: MaterialCatalog,
    pub blocks: Vec<BlockType>,
    pub by_name: HashMap<String, BlockId>,
}

impl BlockRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn get(&self, id: BlockId) -> Option<&BlockType> {
        self.blocks.get(id as usize)
    }

    pub fn id_by_name(&self, name: &str) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    pub fn load_from_paths(materials_path: impl AsRef<Path>, blocks_path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let materials = MaterialCatalog::from_path(materials_path)?;
        let blocks_toml = fs::read_to_string(blocks_path)?;
        let blocks_cfg: BlocksConfig = toml::from_str(&blocks_toml)?;
        Self::from_configs(materials, blocks_cfg)
    }

    pub fn from_configs(materials: MaterialCatalog, cfg: BlocksConfig) -> Result<Self, Box<dyn Error>> {
        let mut reg = BlockRegistry { materials, blocks: Vec::new(), by_name: HashMap::new() };
        for def in cfg.blocks.into_iter() {
            let id = match def.id {
                Some(id) => id,
                None => reg.blocks.len() as u16,
            };
            let solid = def.solid.unwrap_or(true);
            let blocks_skylight = def.blocks_skylight.unwrap_or(solid);
            let propagates_light = def.propagates_light.unwrap_or(false);
            let emission = def.emission.unwrap_or(0);
            let shape = compile_shape(def.shape);
            let mats = compile_materials(&reg.materials, def.materials);
            let state_schema = def.state_schema.unwrap_or_default();
            let (state_fields, prop_index) = compute_state_layout(&state_schema);

            let ty = BlockType {
                id,
                name: def.name,
                solid,
                blocks_skylight,
                propagates_light,
                emission,
                shape,
                materials: mats,
                state_schema,
                state_fields,
                prop_index,
            };
            if reg.blocks.len() <= id as usize {
                reg.blocks.resize(id as usize + 1, BlockType {
                    id,
                    name: String::new(),
                    solid: false,
                    blocks_skylight: false,
                    propagates_light: false,
                    emission: 0,
                    shape: Shape::None,
                    materials: CompiledMaterials::default(),
                    state_schema: HashMap::new(),
                    state_fields: Vec::new(),
                    prop_index: HashMap::new(),
                });
            }
            reg.blocks[id as usize] = ty.clone();
            reg.by_name.insert(ty.name.clone(), id);
        }
        Ok(reg)
    }
}

fn compile_shape(shape: Option<ShapeConfig>) -> Shape {
    match shape {
        None => Shape::Cube,
        Some(ShapeConfig::Simple(s)) => match s.as_str() {
            "cube" => Shape::Cube,
            _ => Shape::None,
        },
        Some(ShapeConfig::Detailed(ShapeDetailed { kind, axis, half, facing })) => match kind.as_str() {
            "cube" => Shape::Cube,
            "axis_cube" => Shape::AxisCube { axis_from: axis.map(|p| p.from).unwrap_or_else(|| "axis".to_string()) },
            "slab" => Shape::Slab { half_from: half.map(|p| p.from).unwrap_or_else(|| "half".to_string()) },
            "stairs" => Shape::Stairs {
                facing_from: facing.map(|p| p.from).unwrap_or_else(|| "facing".to_string()),
                half_from: half.map(|p| p.from).unwrap_or_else(|| "half".to_string()),
            },
            _ => Shape::None,
        },
    }
}

fn compile_materials(matcat: &MaterialCatalog, mats: Option<MaterialsDef>) -> CompiledMaterials {
    fn resolve_selector(matcat: &MaterialCatalog, sel: &MaterialSelector) -> Option<ResolvedSelector> {
        match sel {
            MaterialSelector::Key(k) => matcat.get_id(k).map(ResolvedSelector::Fixed),
            MaterialSelector::By { by, map } => {
                let mut out: HashMap<String, MaterialId> = HashMap::new();
                for (k, v) in map.iter() {
                    if let Some(id) = matcat.get_id(v) {
                        out.insert(k.clone(), id);
                    }
                }
                Some(ResolvedSelector::By { by: by.clone(), map: out })
            }
        }
    }

    let mut out = CompiledMaterials::default();
    if let Some(m) = mats {
        if let Some(ref all) = m.all { out.all = resolve_selector(matcat, all); }
        if let Some(ref top) = m.top { out.top = resolve_selector(matcat, top); }
        if let Some(ref bottom) = m.bottom { out.bottom = resolve_selector(matcat, bottom); }
        if let Some(ref side) = m.side { out.side = resolve_selector(matcat, side); }
    }
    out
}

fn compute_state_layout(schema: &HashMap<String, Vec<String>>) -> (Vec<StateField>, HashMap<String, usize>) {
    if schema.is_empty() {
        return (Vec::new(), HashMap::new());
    }
    let mut keys: Vec<&str> = schema.keys().map(|s| s.as_str()).collect();
    keys.sort_unstable();
    let mut fields: Vec<StateField> = Vec::with_capacity(keys.len());
    let mut offset: u32 = 0;
    for k in keys {
        let vals = schema.get(k).cloned().unwrap_or_default();
        let vlen = vals.len() as u32;
        let bits: u32 = if vlen <= 1 { 0 } else { 32 - (vlen - 1).leading_zeros() };
        fields.push(StateField { name: k.to_string(), values: vals, bits, offset });
        offset = offset.saturating_add(bits);
    }
    let mut index: HashMap<String, usize> = HashMap::with_capacity(fields.len());
    for (i, f) in fields.iter().enumerate() {
        index.insert(f.name.clone(), i);
    }
    (fields, index)
}

// Convenience helpers that mirror the future Block API; these delegate to BlockType
impl BlockType {
    pub fn is_solid(&self, _state: BlockState) -> bool { self.solid }
    pub fn blocks_skylight(&self, _state: BlockState) -> bool { self.blocks_skylight }
    pub fn propagates_light(&self, _state: BlockState) -> bool { self.propagates_light }
    pub fn light_emission(&self, _state: BlockState) -> u8 { self.emission }
    pub fn debug_name(&self) -> &str { &self.name }

    // Decode a named property from the block state using the declared state_schema.
    // Bit packing order is stable and derived by sorting property names ascending,
    // assigning each field just enough bits to encode its allowed values (ceil(log2(len))).
    pub fn state_prop_value<'a>(&'a self, state: BlockState, prop: &str) -> Option<&'a str> {
        if self.state_fields.is_empty() {
            return None;
        }
        let &i = self.prop_index.get(prop)?;
        let f = &self.state_fields[i];
        if f.bits == 0 {
            return f.values.get(0).map(|s| s.as_str());
        }
        let mask: u32 = if f.bits >= 32 { u32::MAX } else { (1u32 << f.bits) - 1 };
        let idx: usize = (((state as u32) >> f.offset) & mask) as usize;
        f.values.get(idx).map(|s| s.as_str())
    }

    pub fn state_prop_is_value(&self, state: BlockState, prop: &str, expect: &str) -> bool {
        self.state_prop_value(state, prop) == Some(expect)
    }

    // Pack a set of named property values into a BlockState according to this type's state_schema.
    // Unknown properties or values default to 0. Packing order matches decoding (sorted keys).
    pub fn pack_state(&self, props: &std::collections::HashMap<String, String>) -> BlockState {
        if self.state_fields.is_empty() {
            return 0;
        }
        let mut acc: u32 = 0;
        for f in &self.state_fields {
            if f.bits == 0 { continue; }
            let sel_idx: u32 = match props.get(&f.name) {
                Some(val) => f.values.iter().position(|s| s == val).unwrap_or(0) as u32,
                None => 0,
            };
            acc |= (sel_idx & ((1u32 << f.bits) - 1)) << f.offset;
        }
        acc as BlockState
    }
}

impl BlockRegistry {
    pub fn make_block_by_name(
        &self,
        name: &str,
        props: Option<&std::collections::HashMap<String, String>>,
    ) -> Option<Block> {
        let id = self.id_by_name(name)?;
        let state = if let Some(p) = props {
            self.get(id).map(|ty| ty.pack_state(p)).unwrap_or(0)
        } else {
            0
        };
        Some(Block { id, state })
    }
}
