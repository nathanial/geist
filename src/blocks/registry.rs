use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use super::config::{BlockDef, BlocksConfig, MaterialSelector, MaterialsDef, PropertyFrom, ShapeConfig, ShapeDetailed};
use super::material::MaterialCatalog;
use super::types::{Block, BlockId, BlockState, FaceRole, MaterialId, Shape};

#[derive(Clone, Debug)]
pub struct BlockType {
    pub id: BlockId,
    pub name: String,
    pub solid: bool,
    pub blocks_skylight: bool,
    pub emission: u8,
    pub shape: Shape,
    pub materials: CompiledMaterials,
    pub state_schema: HashMap<String, Vec<String>>, // property name -> allowed values
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
    pub fn material_for(&self, role: FaceRole, _state: BlockState, _ty: &BlockType) -> Option<MaterialId> {
        // NOTE: state decoding not yet implemented; return fixed or "all" as available.
        let pick = match role {
            FaceRole::Top => self.top.as_ref().or(self.all.as_ref()),
            FaceRole::Bottom => self.bottom.as_ref().or(self.all.as_ref()),
            FaceRole::Side => self.side.as_ref().or(self.all.as_ref()),
            FaceRole::All => self.all.as_ref(),
        };
        match pick {
            Some(ResolvedSelector::Fixed(id)) => Some(*id),
            Some(ResolvedSelector::By { .. }) => {
                // Until state decoding is wired, cannot resolve by-property
                None
            }
            None => None,
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
            let name = def.name.clone();
            let solid = def.solid.unwrap_or(true);
            let blocks_skylight = def.blocks_skylight.unwrap_or(solid);
            let emission = def.emission.unwrap_or(0);
            let shape = compile_shape(def.shape);
            let mats = compile_materials(&reg.materials, def.materials);
            let state_schema = def.state_schema.unwrap_or_default();

            let ty = BlockType { id, name: def.name, solid, blocks_skylight, emission, shape, materials: mats, state_schema };
            if reg.blocks.len() <= id as usize {
                reg.blocks.resize(id as usize + 1, BlockType {
                    id,
                    name: String::new(),
                    solid: false,
                    blocks_skylight: false,
                    emission: 0,
                    shape: Shape::None,
                    materials: CompiledMaterials::default(),
                    state_schema: HashMap::new(),
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

// Convenience helpers that mirror the future Block API; these delegate to BlockType
impl BlockType {
    pub fn is_solid(&self, _state: BlockState) -> bool { self.solid }
    pub fn blocks_skylight(&self, _state: BlockState) -> bool { self.blocks_skylight }
    pub fn light_emission(&self, _state: BlockState) -> u8 { self.emission }
    pub fn debug_name(&self) -> &str { &self.name }
}

