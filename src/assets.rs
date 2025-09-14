use std::path::{Path, PathBuf};

pub fn resolve_assets_root(cli: Option<String>) -> PathBuf {
    // Precedence: CLI flag -> GEIST_ASSETS env -> search nearby dirs -> CWD
    if let Some(p) = cli {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    if let Ok(p) = std::env::var("GEIST_ASSETS") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    // Search candidates: CWD, executable dir, crate root; climb up to 5 parents
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    for base in candidates {
        let mut cur = base.clone();
        for _ in 0..5 {
            let check = cur.join("assets/voxels/materials.toml");
            if check.exists() {
                return cur;
            }
            if let Some(parent) = cur.parent() {
                cur = parent.to_path_buf();
            } else {
                break;
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn materials_path(root: &Path) -> PathBuf {
    root.join("assets/voxels/materials.toml")
}

pub fn blocks_path(root: &Path) -> PathBuf {
    root.join("assets/voxels/blocks.toml")
}

pub fn hotbar_path(root: &Path) -> PathBuf {
    root.join("assets/voxels/hotbar.toml")
}

pub fn textures_dir(root: &Path) -> PathBuf {
    root.join("assets/blocks")
}

pub fn shaders_dir(root: &Path) -> PathBuf {
    root.join("assets/shaders")
}

pub fn schematics_dir(root: &Path) -> PathBuf {
    root.join("schematics")
}
