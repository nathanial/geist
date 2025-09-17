use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::types::MaterialId;

#[derive(Clone, Debug)]
pub struct Material {
    #[allow(dead_code)]
    pub id: MaterialId,
    #[allow(dead_code)]
    pub key: String,
    pub texture_candidates: Vec<PathBuf>,
    pub render_tag: Option<String>,
}

#[derive(Default, Clone, Debug)]
pub struct MaterialCatalog {
    pub materials: Vec<Material>,
    pub by_key: HashMap<String, MaterialId>,
}

impl MaterialCatalog {
    pub fn new() -> Self {
        Self {
            materials: Vec::new(),
            by_key: HashMap::new(),
        }
    }

    pub fn get_id(&self, key: &str) -> Option<MaterialId> {
        self.by_key.get(key).copied()
    }

    pub fn get(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id.0 as usize)
    }

    pub fn from_toml_str(toml_str: &str) -> Result<Self, Box<dyn Error>> {
        let cfg: MaterialsConfig = toml::from_str(toml_str)?;
        let mut catalog = MaterialCatalog::new();
        let mut entries: Vec<(String, MaterialEntry)> = cfg.materials.into_iter().collect();
        // HashMap iteration order is nondeterministic; sort keys so MaterialId assignment is stable.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, entry) in entries {
            let (paths, render_tag) = match entry {
                MaterialEntry::Paths(v) => (v, None),
                MaterialEntry::Detail { paths, render_tag } => (paths, render_tag),
            };
            let id = MaterialId(catalog.materials.len() as u16);
            catalog.by_key.insert(key.clone(), id);
            catalog.materials.push(Material {
                id,
                key,
                texture_candidates: paths.into_iter().map(PathBuf::from).collect(),
                render_tag,
            });
        }
        Ok(catalog)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let s = fs::read_to_string(path)?;
        Self::from_toml_str(&s)
    }
}

// --- Config ---

#[derive(Deserialize)]
pub struct MaterialsConfig {
    pub materials: HashMap<String, MaterialEntry>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum MaterialEntry {
    // Simple: material = ["assets/blocks/foo.png", ...]
    Paths(Vec<String>),
    // Detailed: material = { paths = ["..."], render_tag = "leaves" }
    Detail {
        paths: Vec<String>,
        render_tag: Option<String>,
    },
}
