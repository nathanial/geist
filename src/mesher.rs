// Compatibility shim re-exporting mesher CPU/GPU APIs.

pub use geist_mesh_cpu::{
    build_chunk_greedy_cpu_buf,
    build_voxel_body_cpu_buf,
    ChunkMeshCPU,
    MeshBuild,
    NeighborsLoaded,
};

pub use geist_render_raylib::{
    upload_chunk_mesh,
    ChunkRender,
};

