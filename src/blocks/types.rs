use serde::{Deserialize, Serialize};

// Compact voxel representation used at runtime
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub state: BlockState,
}

pub type BlockId = u16;
pub type BlockState = u16;

impl Block {
    pub const AIR: Block = Block { id: 0, state: 0 };
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct MaterialId(pub u16);

// Used by cube-like shapes to resolve which material to apply
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FaceRole {
    All,
    Top,
    Bottom,
    Side,
}

// Shape abstraction used by the mesher to select emitters
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Cube,
    AxisCube { axis_from: String },
    Slab { half_from: String },
    Stairs { facing_from: String, half_from: String },
    None,
}

