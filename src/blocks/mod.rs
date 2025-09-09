// Compatibility shim re-exporting from `geist-blocks` crate.
pub use geist_blocks::{MaterialCatalog, BlockRegistry};
pub use geist_blocks::types::{Block, FaceRole, MaterialId, Shape};

// Preserve old submodule paths like `crate::blocks::registry::...`
pub mod registry { pub use geist_blocks::registry::*; }
pub mod material { pub use geist_blocks::material::*; }
pub mod config { pub use geist_blocks::config::*; }
pub mod types { pub use geist_blocks::types::*; }
