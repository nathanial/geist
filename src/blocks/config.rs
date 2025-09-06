use std::collections::HashMap;
use serde::Deserialize;

// Top-level blocks config file
#[derive(Deserialize, Debug)]
pub struct BlocksConfig {
    pub blocks: Vec<BlockDef>,
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
    pub emission: Option<u8>,

    #[serde(default)]
    pub shape: Option<ShapeConfig>,

    #[serde(default)]
    pub materials: Option<MaterialsDef>,

    #[serde(default)]
    pub state_schema: Option<HashMap<String, Vec<String>>>,
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
    By { by: String, #[serde(default)] map: HashMap<String, String> },
}

