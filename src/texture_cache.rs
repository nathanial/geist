use std::collections::HashMap;

pub struct TextureCache {
    pub map: HashMap<String, raylib::core::texture::Texture2D>,
}

impl TextureCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get_ref(&self, key: &str) -> Option<&raylib::core::texture::Texture2D> {
        self.map.get(key)
    }

    pub fn replace_loaded(&mut self, key: String, tex: raylib::core::texture::Texture2D) {
        self.map.insert(key, tex);
    }
}
