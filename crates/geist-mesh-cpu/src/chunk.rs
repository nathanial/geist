use geist_blocks::types::MaterialId;
use geist_geom::Aabb;
use std::collections::HashMap;

use crate::mesh_build::MeshBuild;
use geist_world::ChunkCoord;

pub struct ChunkMeshCPU {
    pub coord: ChunkCoord,
    pub bbox: Aabb,
    pub parts: HashMap<MaterialId, MeshBuild>,
}
