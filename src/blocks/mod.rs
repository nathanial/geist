pub mod material;
pub mod types;
pub mod config;
pub mod registry;

// Re-exports for convenience
pub use material::{Material, MaterialCatalog};
pub use types::{Block, BlockId, BlockState, Shape, FaceRole, MaterialId};
pub use registry::{BlockRegistry, BlockType};
