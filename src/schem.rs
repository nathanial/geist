// Compatibility shim: re-export from geist-io crate
pub use geist_io::{
    count_blocks_in_file, find_unsupported_blocks_in_file, list_schematics_with_size,
    load_any_schematic_apply_edits, load_any_schematic_apply_into_structure,
    load_sponge_schem_apply_edits, SchematicEntry,
};
