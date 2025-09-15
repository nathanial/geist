//! CPU meshing crate: watertight per-face mesher and helpers (engine-only).
#![forbid(unsafe_code)]

pub mod microgrid_tables;

mod constants;
mod face;
mod mesh_build;
mod neighbors;
mod chunk;
mod util;
mod emit;
mod wcc;
#[cfg(feature = "parity_mesher")]
mod parity;
mod build;

pub use face::{Face, SIDE_NEIGHBORS};
pub use mesh_build::MeshBuild;
pub use neighbors::NeighborsLoaded;
pub use chunk::ChunkMeshCPU;
pub use wcc::WccMesher;
pub use build::{build_chunk_wcc_cpu_buf, build_chunk_wcc_cpu_buf_with_light, build_voxel_body_cpu_buf};
pub use util::is_full_cube;
