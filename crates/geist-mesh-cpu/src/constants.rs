//! Shared constants for geist-mesh-cpu. Centralizes common magic numbers.

// Microgrid (2x2x2 half-step grid)
pub(crate) const MICROGRID_STEPS: usize = 2; // half-steps per axis in microgrid
pub(crate) const MICROGRID_LAST_IDX: usize = MICROGRID_STEPS - 1;
pub(crate) const MICRO_HALF_STEP_SIZE: f32 = 0.5; // world units per half-step

// Lookup table sizes for microgrid occupancy/emptiness encodings
pub(crate) const BOXES_TABLE_SIZE: usize = 256; // 2^8 occupancy patterns
pub(crate) const RECTS_TABLE_SIZE: usize = 16;  // 2^4 boundary emptiness patterns

// Bitset configuration (u64-based)
pub(crate) const BITS_PER_WORD: usize = 64;
pub(crate) const WORD_INDEX_SHIFT: usize = 6; // log2(64)
pub(crate) const WORD_INDEX_MASK: usize = 63; // (1<<6) - 1

// Colors
pub(crate) const OPAQUE_ALPHA: u8 = 255;
/// Visual-only lighting floor to avoid pitch-black faces in darkness.
/// Does not affect logical light propagation.
#[allow(dead_code)]
pub(crate) const VISUAL_LIGHT_MIN: u8 = 18; // ~7% brightness floor
