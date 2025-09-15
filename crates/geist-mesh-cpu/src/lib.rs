//! CPU meshing crate: watertight per-face mesher and helpers (engine-only).
#![forbid(unsafe_code)]

pub mod microgrid_tables;

mod build;
mod chunk;
mod constants;
mod emit;
mod face;
mod mesh_build;
mod neighbors;
mod parity;
mod util;

pub use build::{
    build_chunk_wcc_cpu_buf, build_chunk_wcc_cpu_buf_with_light, build_voxel_body_cpu_buf,
};
pub use chunk::ChunkMeshCPU;
pub use face::{Face, SIDE_NEIGHBORS};
pub use mesh_build::MeshBuild;
pub use neighbors::NeighborsLoaded;
pub use parity::ParityMesher;
pub use util::is_full_cube;
