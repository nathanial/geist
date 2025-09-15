use geist_blocks::types::MaterialId;
use geist_geom::Aabb;
use std::collections::HashMap;

use crate::mesh_build::MeshBuild;

pub struct ChunkMeshCPU {
    pub cx: i32,
    pub cz: i32,
    pub bbox: Aabb,
    pub parts: HashMap<MaterialId, MeshBuild>,
}
