pub mod config;
pub mod material;
pub mod registry;
pub mod types;

// Re-exports for convenience
pub use material::MaterialCatalog;
pub use registry::BlockRegistry;
pub use types::{Block, FaceRole, MaterialId, Shape};
