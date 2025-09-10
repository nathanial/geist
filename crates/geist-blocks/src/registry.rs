use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use super::config::{
    BlocksConfig, LightProfile, MaterialSelector, MaterialsDef, SeamPolicyCfg, SeamPolicyFlagsCfg,
    SeamPolicySimple, ShapeConfig, ShapeDetailed, SourceDirs,
};
use super::material::MaterialCatalog;
use super::types::{Block, BlockId, BlockState, FaceRole, MaterialId, Shape};

// Minimal duplication of mesher-facing enums to avoid a dependency from blocks → mesher.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum Face {
    PosY = 0,
    NegY = 1,
    PosX = 2,
    NegX = 3,
    PosZ = 4,
    NegZ = 5,
}
impl Face {
    #[inline]
    fn index(self) -> usize {
        self as usize
    }
}

/// Simple cardinal facing used by stairs and similar shapes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum Facing {
    North,
    South,
    West,
    East,
}
impl Facing {
    #[inline]
    fn from_str(s: &str) -> Facing {
        match s {
            "north" => Facing::North,
            "south" => Facing::South,
            "west" => Facing::West,
            "east" => Facing::East,
            _ => Facing::North,
        }
    }
}

#[derive(Default, Clone, Debug)]
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
        Self {
            materials: MaterialCatalog::new(),
            blocks: Vec::new(),
            by_name: HashMap::new(),
            unknown_block_id: None,
        }
    }

    #[inline]
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
            let id = def.id.unwrap_or(reg.blocks.len() as u16);
            let solid = def.solid.unwrap_or(true);
            let blocks_skylight = def.blocks_skylight.unwrap_or(solid);
            let propagates_light = def.propagates_light.unwrap_or(false);
            let emission = def.emission.unwrap_or(0);
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
                pre_shape_variants: Vec::new(),
                seam: match def.seam {
                    Some(SeamPolicyCfg::Simple(SeamPolicySimple::DontOccludeSame)) => SeamPolicy {
                        dont_occlude_same: true,
                        dont_project_fixups: false,
                    },
                    Some(SeamPolicyCfg::Simple(SeamPolicySimple::DontProjectFixups)) => {
                        SeamPolicy {
                            dont_occlude_same: false,
                            dont_project_fixups: true,
                        }
                    }
                    Some(SeamPolicyCfg::Simple(SeamPolicySimple::Default)) | None => SeamPolicy {
                        dont_occlude_same: false,
                        dont_project_fixups: false,
                    },
                    Some(SeamPolicyCfg::Flags(SeamPolicyFlagsCfg {
                        dont_occlude_same,
                        dont_project_fixups,
                    })) => SeamPolicy {
                        dont_occlude_same,
                        dont_project_fixups,
                    },
                },
                state_schema,
                state_fields,
                prop_index,
            };

            let (pre_top, pre_bottom, pre_side, pre_occ, pre_vars) = {
                let total_bits: u32 = ty.state_fields.iter().map(|f| f.bits).sum();
                let states_len: usize = if total_bits == 0 {
                    1
                } else {
                    1usize << total_bits.min(16)
                };
                let unknown_mid = reg.materials.get_id("unknown").unwrap_or(MaterialId(0));
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
                let mut vars: Vec<ShapeVariant> = Vec::with_capacity(states_len);
                for s in 0..states_len {
                    let state = s as BlockState;
                    let (m, var) = match &ty.shape {
                        Shape::Slab { half_from } | Shape::Stairs { half_from, .. } => {
                            let is_top = ty.state_prop_is_value(state, half_from, "top");
                            let posy = (!is_top) as u8;
                            let negy = (is_top) as u8;
                            let occ_mask = (posy << Face::PosY.index())
                                | (negy << Face::NegY.index())
                                | (1 << Face::PosX.index())
                                | (1 << Face::NegX.index())
                                | (1 << Face::PosZ.index())
                                | (1 << Face::NegZ.index());
                            let occ8 = match &ty.shape {
                                Shape::Slab { .. } => Some(occ_slab(is_top)),
                                Shape::Stairs { facing_from, .. } => {
                                    let facing = Facing::from_str(
                                        ty.state_prop_value(state, facing_from).unwrap_or("north"),
                                    );
                                    Some(occ_stairs(facing, is_top))
                                }
                                _ => None,
                            };
                            (
                                occ_mask,
                                ShapeVariant {
                                    occupancy: occ8,
                                    dynamic: None,
                                },
                            )
                        }
                        Shape::Pane => (
                            0,
                            ShapeVariant {
                                occupancy: None,
                                dynamic: Some(DynamicShape::Pane),
                            },
                        ),
                        Shape::Fence => (
                            0,
                            ShapeVariant {
                                occupancy: None,
                                dynamic: Some(DynamicShape::Fence),
                            },
                        ),
                        Shape::Gate { .. } => (
                            0,
                            ShapeVariant {
                                occupancy: None,
                                dynamic: Some(DynamicShape::Gate),
                            },
                        ),
                        Shape::Carpet => (
                            0,
                            ShapeVariant {
                                occupancy: None,
                                dynamic: Some(DynamicShape::Carpet),
                            },
                        ),
                        _ => {
                            if ty.is_solid(state) {
                                (
                                    0b11_1111,
                                    ShapeVariant {
                                        occupancy: None,
                                        dynamic: None,
                                    },
                                )
                            } else {
                                (
                                    0,
                                    ShapeVariant {
                                        occupancy: None,
                                        dynamic: None,
                                    },
                                )
                            }
                        }
                    };
                    occ.push(m);
                    vars.push(var);
                }
                (
                    fill_role(FaceRole::Top),
                    fill_role(FaceRole::Bottom),
                    fill_role(FaceRole::Side),
                    occ,
                    vars,
                )
            };
            ty.pre_mat_top = pre_top;
            ty.pre_mat_bottom = pre_bottom;
            ty.pre_mat_side = pre_side;
            ty.pre_occ_mask = pre_occ;
            ty.pre_shape_variants = pre_vars;
            if reg.blocks.len() <= id as usize {
                reg.blocks
                    .resize(id as usize + 1, BlockType::placeholder(id));
            }
            reg.blocks[id as usize] = ty;
        }

        if let Some(name) = unknown_name {
            reg.unknown_block_id = reg.id_by_name(&name);
        }
        reg.by_name = reg.blocks.iter().map(|t| (t.name.clone(), t.id)).collect();
        Ok(reg)
    }

    pub fn unknown_block_id_or_panic(&self) -> BlockId {
        if let Some(id) = self.unknown_block_id.or_else(|| self.id_by_name("unknown")) {
            id
        } else {
            panic!(
                "Unknown block fallback not configured. Set `unknown_block = \"<name>\"` in blocks.toml and define that block."
            )
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
    // Precomputed shape variant per state (for micro-grid based shapes)
    pub pre_shape_variants: Vec<ShapeVariant>,
    // Seam policy to control occlusion and fixup projection between neighbors
    pub seam: SeamPolicy,
    #[allow(dead_code)]
    pub state_schema: HashMap<String, Vec<String>>, // property name -> allowed values
    // Precomputed, sorted layout for fast state packing/unpacking
    pub state_fields: Vec<StateField>,
    pub prop_index: HashMap<String, usize>,
}

impl BlockType {
    fn placeholder(id: BlockId) -> Self {
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
            pre_shape_variants: vec![ShapeVariant::default()],
            seam: SeamPolicy {
                dont_occlude_same: false,
                dont_project_fixups: false,
            },
            state_schema: HashMap::new(),
            state_fields: Vec::new(),
            prop_index: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StateField {
    pub name: String,
    pub values: Vec<String>,
    pub bits: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ShapeVariant {
    pub occupancy: Option<u8>,
    pub dynamic: Option<DynamicShape>,
}

#[derive(Copy, Clone, Debug)]
pub enum DynamicShape {
    Pane,
    Fence,
    Gate,
    Carpet,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SeamPolicy {
    pub dont_occlude_same: bool,
    pub dont_project_fixups: bool,
}

#[derive(Clone, Debug)]
pub enum CompiledLight {
    Omni {
        attenuation: u8,
        max_range: Option<u16>,
    },
    Beam {
        straight_cost: u8,
        turn_cost: u8,
        vertical_cost: u8,
        source_dirs: SourceDirs,
        max_range: Option<u16>,
    },
}

fn compile_shape(shape: Option<ShapeConfig>) -> Shape {
    use super::config::{PropertyFrom, ShapeConfig::*, ShapeDetailed};
    match shape.unwrap_or(Simple("cube".into())) {
        Simple(k) => match k.as_str() {
            "cube" => Shape::Cube,
            "slab" => Shape::Slab {
                half_from: "half".into(),
            },
            "stairs" => Shape::Stairs {
                facing_from: "facing".into(),
                half_from: "half".into(),
            },
            "pane" => Shape::Pane,
            "fence" => Shape::Fence,
            "gate" => Shape::Gate {
                facing_from: "facing".into(),
                open_from: "open".into(),
            },
            "carpet" => Shape::Carpet,
            _ => Shape::None,
        },
        Detailed(ShapeDetailed {
            kind,
            axis,
            half,
            facing,
            open,
        }) => match kind.as_str() {
            "cube" => Shape::Cube,
            "axis_cube" => Shape::AxisCube {
                axis_from: axis
                    .map(|p: PropertyFrom| p.from)
                    .unwrap_or_else(|| "axis".to_string()),
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
            "pane" => Shape::Pane,
            "fence" => Shape::Fence,
            "gate" => Shape::Gate {
                facing_from: facing
                    .map(|p| p.from)
                    .unwrap_or_else(|| "facing".to_string()),
                open_from: open.map(|p| p.from).unwrap_or_else(|| "open".to_string()),
            },
            "carpet" => Shape::Carpet,
            _ => Shape::None,
        },
    }
}

#[inline]
fn bit2(x: usize, y: usize, z: usize) -> u8 {
    1u8 << (((y & 1) << 2) | ((z & 1) << 1) | (x & 1))
}

#[inline]
fn occ_slab(is_top: bool) -> u8 {
    let y = if is_top { 1 } else { 0 };
    bit2(0, y, 0) | bit2(1, y, 0) | bit2(0, y, 1) | bit2(1, y, 1)
}

#[inline]
fn occ_stairs(facing: Facing, is_top: bool) -> u8 {
    let y_major = if is_top { 1 } else { 0 };
    let y_minor = 1 - y_major;
    let full_layer =
        bit2(0, y_major, 0) | bit2(1, y_major, 0) | bit2(0, y_major, 1) | bit2(1, y_major, 1);
    let half_minor = match facing {
        Facing::North => bit2(0, y_minor, 0) | bit2(1, y_minor, 0),
        Facing::South => bit2(0, y_minor, 1) | bit2(1, y_minor, 1),
        Facing::West => bit2(0, y_minor, 0) | bit2(0, y_minor, 1),
        Facing::East => bit2(1, y_minor, 0) | bit2(1, y_minor, 1),
    };
    full_layer | half_minor
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
    let mut keys: Vec<&String> = schema.keys().collect();
    keys.sort();
    let mut offset: u32 = 0;
    let mut fields: Vec<StateField> = Vec::with_capacity(keys.len());
    for k in keys.into_iter() {
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
    #[inline]
    pub fn material_for_cached(&self, role: FaceRole, state: BlockState) -> MaterialId {
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
    #[inline]
    pub fn variant(&self, state: BlockState) -> &ShapeVariant {
        let len = self.pre_shape_variants.len();
        &self.pre_shape_variants[state as usize & (len - 1)]
    }
}
