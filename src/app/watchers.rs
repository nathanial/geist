use raylib::prelude::*;

use super::App;

impl App {
    pub fn process_texture_file_events(
        &mut self,
        rl: &mut raylib::prelude::RaylibHandle,
        thread: &raylib::prelude::RaylibThread,
    ) {
        use std::collections::HashSet;
        let mut changed: HashSet<String> = HashSet::new();
        for p in self.tex_event_rx.try_iter() {
            let canon = std::fs::canonicalize(&p)
                .ok()
                .map(|pb| pb.to_string_lossy().to_string())
                .unwrap_or(p);
            changed.insert(canon);
        }
        if changed.is_empty() {
            return;
        }
        log::info!("Texture changes detected: {} file(s)", changed.len());
        for p in &changed {
            log::debug!(" - {}", p);
        }
        // Helper to choose material path like upload path
        let choose_path = |mid: geist_blocks::types::MaterialId| -> Option<String> {
            self.reg.materials.get(mid).and_then(|mdef| {
                let candidates: Vec<String> = mdef
                    .texture_candidates
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let chosen = candidates
                    .iter()
                    .find(|p| std::path::Path::new(p.as_str()).exists())
                    .cloned()
                    .or_else(|| candidates.first().cloned());
                chosen.map(|s| {
                    std::fs::canonicalize(&s)
                        .ok()
                        .map(|pb| pb.to_string_lossy().to_string())
                        .unwrap_or(s)
                })
            })
        };
        // Reload any changed paths into cache
        for path in changed.iter() {
            if let Ok(tex) = rl.load_texture(thread, path) {
                tex.set_texture_filter(thread, raylib::consts::TextureFilter::TEXTURE_FILTER_POINT);
                tex.set_texture_wrap(thread, raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT);
                self.tex_cache.replace_loaded(path.clone(), tex);
                log::debug!("reloaded texture {}", path);
            } else {
                log::warn!("failed to reload texture {}", path);
            }
        }
        let mut rebound: std::collections::HashMap<String, usize> = Default::default();
        // Rebind textures on existing chunk renders
        for (_k, cr) in self.renders.iter_mut() {
            for part in cr.parts.iter_mut() {
                let Some(path) = choose_path(part.mid) else {
                    continue;
                };
                if !changed.contains(&path) {
                    continue;
                }
                if let Some(mat) = {
                    use raylib::prelude::RaylibModel;
                    part.model.materials_mut().get_mut(0)
                } {
                    if let Some(tex) = self.tex_cache.get_ref(&path) {
                        mat.set_material_texture(
                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                            tex,
                        );
                        *rebound.entry(path.clone()).or_insert(0) += 1;
                    } else if let Ok(t) = rl.load_texture(thread, &path) {
                        t.set_texture_filter(
                            thread,
                            raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                        );
                        t.set_texture_wrap(
                            thread,
                            raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                        );
                        self.tex_cache.replace_loaded(path.clone(), t);
                        if let Some(tex) = self.tex_cache.get_ref(&path) {
                            mat.set_material_texture(
                                raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                tex,
                            );
                            *rebound.entry(path.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        // Rebind for structure renders as well
        for (_id, cr) in self.structure_renders.iter_mut() {
            for part in cr.parts.iter_mut() {
                let Some(path) = choose_path(part.mid) else {
                    continue;
                };
                if !changed.contains(&path) {
                    continue;
                }
                if let Some(mat) = {
                    use raylib::prelude::RaylibModel;
                    part.model.materials_mut().get_mut(0)
                } {
                    if let Some(tex) = self.tex_cache.get_ref(&path) {
                        mat.set_material_texture(
                            raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                            tex,
                        );
                        *rebound.entry(path.clone()).or_insert(0) += 1;
                    } else if let Ok(t) = rl.load_texture(thread, &path) {
                        t.set_texture_filter(
                            thread,
                            raylib::consts::TextureFilter::TEXTURE_FILTER_POINT,
                        );
                        t.set_texture_wrap(
                            thread,
                            raylib::consts::TextureWrap::TEXTURE_WRAP_REPEAT,
                        );
                        self.tex_cache.replace_loaded(path.clone(), t);
                        if let Some(tex) = self.tex_cache.get_ref(&path) {
                            mat.set_material_texture(
                                raylib::consts::MaterialMapIndex::MATERIAL_MAP_ALBEDO,
                                tex,
                            );
                            *rebound.entry(path.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        if rebound.is_empty() {
            log::info!("Texture reload complete; no active models referenced changed textures");
        } else {
            for (p, n) in rebound {
                log::info!("Rebound {} on {} material(s)", p, n);
            }
        }
    }

    pub fn process_worldgen_file_events(&mut self) {
        let mut changed = false;
        for _ in self.worldgen_event_rx.try_iter() {
            changed = true;
        }
        if !changed {
            return;
        }
        let path = std::path::Path::new(&self.world_config_path);
        if !path.exists() {
            log::warn!("worldgen config missing: {}", self.world_config_path);
            return;
        }
        match geist_world::worldgen::load_params_from_path(path) {
            Ok(params) => {
                self.gs.world.update_worldgen_params(params);
                log::info!("worldgen config reloaded from {}", self.world_config_path);
                log::info!("Existing chunks unchanged; new gen uses updated params");
                self.worldgen_dirty = true;
            }
            Err(e) => {
                log::warn!(
                    "worldgen config reload failed ({}): {}",
                    self.world_config_path,
                    e
                );
            }
        }
    }

    pub fn take_worldgen_dirty(&mut self) -> bool {
        if self.worldgen_dirty {
            self.worldgen_dirty = false;
            true
        } else {
            false
        }
    }
}
