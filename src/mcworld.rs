
#[cfg(not(feature = "mcworld"))]
pub fn load_mcworlds_in_dir(
    _dir: &std::path::Path,
    _base_y: i32,
    _edits: &mut crate::edit::EditStore,
) -> Result<Vec<(String, (i32, i32, i32), (i32, i32, i32))>, String> {
    Err("mcworld support not enabled; recompile with --features mcworld".into())
}
