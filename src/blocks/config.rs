use serde::Deserialize;
use std::collections::HashMap;

// Top-level blocks config file
#[derive(Deserialize, Debug)]
pub struct BlocksConfig {
    pub blocks: Vec<BlockDef>,
    #[serde(default)]
    pub lighting: Option<LightingConfig>,
    // Optional name of a block to use as the default unknown/fallback block
    // when a requested block is unavailable. If absent or not found, fallbacks
    // will use `air`.
    #[serde(default)]
    pub unknown_block: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BlockDef {
    pub name: String,
    #[serde(default)]
    pub id: Option<u16>,
    #[serde(default)]
    pub solid: Option<bool>,
    #[serde(default)]
    pub blocks_skylight: Option<bool>,
    #[serde(default)]
    pub propagates_light: Option<bool>,
    #[serde(default)]
    pub emission: Option<u8>,

    // Optional lighting behavior configuration
    #[serde(default)]
    pub light_profile: Option<String>,
    #[serde(default)]
    pub light: Option<LightProfile>,

    #[serde(default)]
    pub shape: Option<ShapeConfig>,

    #[serde(default)]
    pub materials: Option<MaterialsDef>,

    #[serde(default)]
    pub state_schema: Option<HashMap<String, Vec<String>>>,

    // Optional seam policy for meshing across neighbors
    #[serde(default)]
    pub seam: Option<SeamPolicyCfg>,
}

// Shape config supports either a simple string ("cube") or a detailed table
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ShapeConfig {
    Simple(String),
    Detailed(ShapeDetailed),
}

#[derive(Deserialize, Debug, Clone)]
pub struct ShapeDetailed {
    pub kind: String,
    #[serde(default)]
    pub axis: Option<PropertyFrom>,
    #[serde(default)]
    pub half: Option<PropertyFrom>,
    #[serde(default)]
    pub facing: Option<PropertyFrom>,
    #[serde(default)]
    pub open: Option<PropertyFrom>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PropertyFrom {
    pub from: String,
}

// Materials mapping: supports all/top/bottom/side, each can be a key or by-property map
#[derive(Deserialize, Debug, Clone, Default)]
pub struct MaterialsDef {
    #[serde(default)]
    pub all: Option<MaterialSelector>,
    #[serde(default)]
    pub top: Option<MaterialSelector>,
    #[serde(default)]
    pub bottom: Option<MaterialSelector>,
    #[serde(default)]
    pub side: Option<MaterialSelector>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum MaterialSelector {
    Key(String),
    By {
        by: String,
        #[serde(default)]
        map: HashMap<String, String>,
    },
}

// Top-level lighting config with reusable profiles
#[derive(Deserialize, Debug, Clone, Default)]
pub struct LightingConfig {
    #[serde(default)]
    pub profiles: HashMap<String, LightProfile>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum LightProfile {
    Omni {
        #[serde(default = "default_omni_atten")]
        attenuation: u8,
        #[serde(default)]
        max_range: Option<u16>,
    },
    Beam {
        #[serde(default = "default_beam_straight")]
        straight_cost: u8,
        #[serde(default = "default_beam_turn")]
        turn_cost: u8,
        #[serde(default = "default_beam_vertical")]
        vertical_cost: u8,
        #[serde(default = "default_source_dirs")]
        source_dirs: SourceDirs,
        #[serde(default)]
        max_range: Option<u16>,
    },
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SourceDirs {
    Horizontal,
    Vertical,
    Any,
}

// Configurable seam policies for neighbor occlusion/fixups
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SeamPolicyCfg {
    Default,
    DontOccludeSame,
    DontProjectFixups,
}

fn default_omni_atten() -> u8 {
    32
}
fn default_beam_straight() -> u8 {
    1
}
fn default_beam_turn() -> u8 {
    32
}
fn default_beam_vertical() -> u8 {
    32
}
fn default_source_dirs() -> SourceDirs {
    SourceDirs::Horizontal
}
