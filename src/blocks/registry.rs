use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use super::config::{
    BlocksConfig, LightProfile, MaterialSelector, MaterialsDef, ShapeConfig, ShapeDetailed,
    SourceDirs,
};
use super::material::MaterialCatalog;
use super::types::{Block, BlockId, BlockState, FaceRole, MaterialId, Shape};
use crate::meshutil::Face;

#[derive(Clone, Debug)]
pub struct BlockType {
    #[allow(dead_code)]
    pub id: BlockId,
    pub name: String,
    pub solid: bool,
    pub blocks_skylight: bool,
    pub propagates_light: bool,
    pub emission: u8,
    pub light: CompiledLight,
    pub shape: Shape,
    pub materials: CompiledMaterials,
    // Precomputed role->material lookup per state (fast path for mesher)
    pub pre_mat_top: Vec<MaterialId>,
    pub pre_mat_bottom: Vec<MaterialId>,
    pub pre_mat_side: Vec<MaterialId>,
    // Precomputed occlusion mask per state (6 bits in Face order)
    pub pre_occ_mask: Vec<u8>,
    #[allow(dead_code)]
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
    By {
        by: String,
        map: HashMap<String, MaterialId>,
    },
}

impl CompiledMaterials {
    pub fn material_for(
        &self,
        role: FaceRole,
        state: BlockState,
        ty: &BlockType,
    ) -> Option<MaterialId> {
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

    #[allow(dead_code)]
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
    pub unknown_block_id: Option<BlockId>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: BlockId) -> Option<&BlockType> {
        self.blocks.get(id as usize)
    }

    pub fn id_by_name(&self, name: &str) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    pub fn load_from_paths(
        materials_path: impl AsRef<Path>,
        blocks_path: impl AsRef<Path>,
    ) -> Result<Self, Box<dyn Error>> {
        let materials = MaterialCatalog::from_path(materials_path)?;
        let blocks_toml = fs::read_to_string(blocks_path)?;
        let blocks_cfg: BlocksConfig = toml::from_str(&blocks_toml)?;
        Self::from_configs(materials, blocks_cfg)
    }

    pub fn from_configs(
        materials: MaterialCatalog,
        cfg: BlocksConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let mut reg = BlockRegistry {
            materials,
            blocks: Vec::new(),
            by_name: HashMap::new(),
            unknown_block_id: None,
        };
        let unknown_name = cfg.unknown_block.clone();
        let profiles: HashMap<String, LightProfile> = cfg
            .lighting
            .as_ref()
            .map(|l| l.profiles.clone())
            .unwrap_or_default();
        for def in cfg.blocks.into_iter() {
            let id = match def.id {
                Some(id) => id,
                None => reg.blocks.len() as u16,
            };
            let solid = def.solid.unwrap_or(true);
            let blocks_skylight = def.blocks_skylight.unwrap_or(solid);
            let propagates_light = def.propagates_light.unwrap_or(false);
            let emission = def.emission.unwrap_or(0);
            // Resolve lighting behavior: inline spec > profile reference > default omni(atten=32)
            let light: CompiledLight = match def.light.or_else(|| {
                def.light_profile
                    .as_ref()
                    .and_then(|n| profiles.get(n).cloned())
            }) {
                Some(LightProfile::Omni {
                    attenuation,
                    max_range,
                }) => CompiledLight::Omni {
                    attenuation,
                    max_range,
                },
                Some(LightProfile::Beam {
                    straight_cost,
                    turn_cost,
                    vertical_cost,
                    source_dirs,
                    max_range,
                }) => CompiledLight::Beam {
                    straight_cost,
                    turn_cost,
                    vertical_cost,
                    source_dirs,
                    max_range,
                },
                None => CompiledLight::Omni {
                    attenuation: 32,
                    max_range: None,
                },
            };
            let shape = compile_shape(def.shape);
            let mats = compile_materials(&reg.materials, def.materials);
            let state_schema = def.state_schema.unwrap_or_default();
            let (state_fields, prop_index) = compute_state_layout(&state_schema);

            let mut ty = BlockType {
                id,
                name: def.name,
                solid,
                blocks_skylight,
                propagates_light,
                emission,
                light,
                shape,
                materials: mats,
                pre_mat_top: Vec::new(),
                pre_mat_bottom: Vec::new(),
                pre_mat_side: Vec::new(),
                pre_occ_mask: Vec::new(),
                state_schema,
                state_fields,
                prop_index,
            };
            // Precompute face materials per state for fast lookup
            let (pre_top, pre_bottom, pre_side, pre_occ) = {
                let total_bits: u32 = ty.state_fields.iter().map(|f| f.bits).sum();
                let states_len: usize = if total_bits == 0 {
                    1
                } else {
                    let tb = total_bits.min(16);
                    1usize << tb
                };
                let unknown_mid = reg
                    .materials
                    .get_id("unknown")
                    .unwrap_or(MaterialId(0));
                let fill_role = |role: FaceRole| -> Vec<MaterialId> {
                    let mut v = Vec::with_capacity(states_len);
                    for s in 0..states_len {
                        let state = s as BlockState;
                        let id = ty
                            .materials
                            .material_for(role, state, &ty)
                            .unwrap_or(unknown_mid);
                        v.push(id);
                    }
                    v
                };
                let mut occ = Vec::with_capacity(states_len);
                for s in 0..states_len {
                    let state = s as BlockState;
                    let m = match &ty.shape {
                        Shape::Slab { half_from } | Shape::Stairs { half_from, .. } => {
                            let is_top = ty.state_prop_is_value(state, half_from, "top");
                            let posy = (!is_top) as u8;
                            let negy = (is_top) as u8;
                            (posy << Face::PosY.index())
                                | (negy << Face::NegY.index())
                                | (1 << Face::PosX.index())
                                | (1 << Face::NegX.index())
                                | (1 << Face::PosZ.index())
                                | (1 << Face::NegZ.index())
                        }
                        _ => {
                            if ty.is_solid(state) {
                                0b11_1111
                            } else {
                                0
                            }
                        }
                    };
                    occ.push(m);
                }
                (
                    fill_role(FaceRole::Top),
                    fill_role(FaceRole::Bottom),
                    fill_role(FaceRole::Side),
                    occ,
                )
            };
            ty.pre_mat_top = pre_top;
            ty.pre_mat_bottom = pre_bottom;
            ty.pre_mat_side = pre_side;
            ty.pre_occ_mask = pre_occ;
            if reg.blocks.len() <= id as usize {
                reg.blocks.resize(
                    id as usize + 1,
                    BlockType {
                        id,
                        name: String::new(),
                        solid: false,
                        blocks_skylight: false,
                        propagates_light: false,
                        emission: 0,
                        light: CompiledLight::Omni {
                            attenuation: 32,
                            max_range: None,
                        },
                        shape: Shape::None,
                        materials: CompiledMaterials::default(),
                        pre_mat_top: vec![MaterialId(0)],
                        pre_mat_bottom: vec![MaterialId(0)],
                        pre_mat_side: vec![MaterialId(0)],
                        pre_occ_mask: vec![0],
                        state_schema: HashMap::new(),
                        state_fields: Vec::new(),
                        prop_index: HashMap::new(),
                    },
                );
            }
            reg.blocks[id as usize] = ty.clone();
            reg.by_name.insert(ty.name.clone(), id);
        }
        // Resolve unknown/fallback block id if configured
        if let Some(name) = unknown_name {
            reg.unknown_block_id = reg.id_by_name(&name);
        }
        Ok(reg)
    }
}

#[derive(Clone, Debug)]
pub enum CompiledLight {
    Omni {
        attenuation: u8,
        #[allow(dead_code)]
        max_range: Option<u16>,
    },
    Beam {
        straight_cost: u8,
        turn_cost: u8,
        vertical_cost: u8,
        source_dirs: SourceDirs,
        #[allow(dead_code)]
        max_range: Option<u16>,
    },
}

fn compile_shape(shape: Option<ShapeConfig>) -> Shape {
    match shape {
        None => Shape::Cube,
        Some(ShapeConfig::Simple(s)) => match s.as_str() {
            "cube" => Shape::Cube,
            _ => Shape::None,
        },
        Some(ShapeConfig::Detailed(ShapeDetailed {
            kind,
            axis,
            half,
            facing,
        })) => match kind.as_str() {
            "cube" => Shape::Cube,
            "axis_cube" => Shape::AxisCube {
                axis_from: axis.map(|p| p.from).unwrap_or_else(|| "axis".to_string()),
            },
            "slab" => Shape::Slab {
                half_from: half.map(|p| p.from).unwrap_or_else(|| "half".to_string()),
            },
            "stairs" => Shape::Stairs {
                facing_from: facing
                    .map(|p| p.from)
                    .unwrap_or_else(|| "facing".to_string()),
                half_from: half.map(|p| p.from).unwrap_or_else(|| "half".to_string()),
            },
            _ => Shape::None,
        },
    }
}

fn compile_materials(matcat: &MaterialCatalog, mats: Option<MaterialsDef>) -> CompiledMaterials {
    fn resolve_selector(
        matcat: &MaterialCatalog,
        sel: &MaterialSelector,
    ) -> Option<ResolvedSelector> {
        match sel {
            MaterialSelector::Key(k) => matcat.get_id(k).map(ResolvedSelector::Fixed),
            MaterialSelector::By { by, map } => {
                let mut out: HashMap<String, MaterialId> = HashMap::new();
                for (k, v) in map.iter() {
                    if let Some(id) = matcat.get_id(v) {
                        out.insert(k.clone(), id);
                    }
                }
                Some(ResolvedSelector::By {
                    by: by.clone(),
                    map: out,
                })
            }
        }
    }

    let mut out = CompiledMaterials::default();
    if let Some(m) = mats {
        if let Some(ref all) = m.all {
            out.all = resolve_selector(matcat, all);
        }
        if let Some(ref top) = m.top {
            out.top = resolve_selector(matcat, top);
        }
        if let Some(ref bottom) = m.bottom {
            out.bottom = resolve_selector(matcat, bottom);
        }
        if let Some(ref side) = m.side {
            out.side = resolve_selector(matcat, side);
        }
    }
    out
}

fn compute_state_layout(
    schema: &HashMap<String, Vec<String>>,
) -> (Vec<StateField>, HashMap<String, usize>) {
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
        let bits: u32 = if vlen <= 1 {
            0
        } else {
            32 - (vlen - 1).leading_zeros()
        };
        fields.push(StateField {
            name: k.to_string(),
            values: vals,
            bits,
            offset,
        });
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
    pub fn is_solid(&self, _state: BlockState) -> bool {
        self.solid
    }
    pub fn blocks_skylight(&self, _state: BlockState) -> bool {
        self.blocks_skylight
    }
    pub fn propagates_light(&self, _state: BlockState) -> bool {
        self.propagates_light
    }
    pub fn light_emission(&self, _state: BlockState) -> u8 {
        self.emission
    }
    #[allow(dead_code)]
    pub fn debug_name(&self) -> &str {
        &self.name
    }

    pub fn light_is_beam(&self) -> bool {
        matches!(self.light, CompiledLight::Beam { .. })
    }

    pub fn omni_attenuation(&self) -> u8 {
        match self.light {
            CompiledLight::Omni { attenuation, .. } => attenuation,
            _ => 32,
        }
    }

    pub fn beam_params(&self) -> (u8, u8, u8, SourceDirs) {
        match self.light {
            CompiledLight::Beam {
                straight_cost,
                turn_cost,
                vertical_cost,
                source_dirs,
                ..
            } => (straight_cost, turn_cost, vertical_cost, source_dirs),
            _ => (1, 32, 32, SourceDirs::Horizontal),
        }
    }

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
            return f.values.first().map(|s| s.as_str());
        }
        let mask: u32 = if f.bits >= 32 {
            u32::MAX
        } else {
            (1u32 << f.bits) - 1
        };
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
            if f.bits == 0 {
                continue;
            }
            let sel_idx: u32 = match props.get(&f.name) {
                Some(val) => f.values.iter().position(|s| s == val).unwrap_or(0) as u32,
                None => 0,
            };
            acc |= (sel_idx & ((1u32 << f.bits) - 1)) << f.offset;
        }
        acc as BlockState
    }
}

impl BlockType {
    #[inline]
    pub fn material_for_cached(&self, role: FaceRole, state: BlockState) -> MaterialId {
        // Precomputed arrays cover the encoded state space (2^sum(bits)), or length 1 when no state.
        // Index by state masked to len-1 (len is power-of-two).
        match role {
            FaceRole::Top => {
                let len = self.pre_mat_top.len();
                self.pre_mat_top[state as usize & (len - 1)]
            }
            FaceRole::Bottom => {
                let len = self.pre_mat_bottom.len();
                self.pre_mat_bottom[state as usize & (len - 1)]
            }
            FaceRole::Side | FaceRole::All => {
                let len = self.pre_mat_side.len();
                self.pre_mat_side[state as usize & (len - 1)]
            }
        }
    }

    #[inline]
    pub fn occlusion_mask_cached(&self, state: BlockState) -> u8 {
        let len = self.pre_occ_mask.len();
        self.pre_occ_mask[state as usize & (len - 1)]
    }
}

impl BlockRegistry {
    pub fn unknown_block_id_or_panic(&self) -> BlockId {
        if let Some(id) = self.unknown_block_id.or_else(|| self.id_by_name("unknown")) {
            id
        } else {
            panic!(
                "Unknown block fallback not configured. Set `unknown_block = \"<name>\"` in blocks.toml and define that block."
            );
        }
    }

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
