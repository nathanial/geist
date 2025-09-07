pub mod material;
pub mod types;
pub mod config;
pub mod registry;

// Re-exports for convenience
pub use material::MaterialCatalog;
pub use types::{Block, Shape, FaceRole, MaterialId};
pub use registry::BlockRegistry;
