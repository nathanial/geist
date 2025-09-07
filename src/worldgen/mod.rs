use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct WorldGenConfig {
    #[serde(default = "default_mode")] pub mode: Mode,
    #[serde(default)] pub flat: Flat,
    #[serde(default)] pub height: Height,
    #[serde(default)] pub surface: Surface,
    #[serde(default)] pub carvers: Carvers,
    #[serde(default)] pub trees: Trees,
    #[serde(default)] pub features: Vec<FeatureRule>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Normal, Flat }

fn default_mode() -> Mode { Mode::Normal }

#[derive(Clone, Debug, Deserialize)]
pub struct Flat { #[serde(default = "default_flat_thickness")] pub thickness: i32 }
fn default_flat_thickness() -> i32 { 1 }
impl Default for Flat { fn default() -> Self { Self { thickness: 1 } } }

#[derive(Clone, Debug, Deserialize)]
pub struct Height {
    #[serde(default = "default_height_freq")] pub frequency: f32,
    #[serde(default = "default_min_y_ratio")] pub min_y_ratio: f32,
    #[serde(default = "default_max_y_ratio")] pub max_y_ratio: f32,
}
fn default_height_freq() -> f32 { 0.02 }
fn default_min_y_ratio() -> f32 { 0.15 }
fn default_max_y_ratio() -> f32 { 0.70 }
impl Default for Height { fn default() -> Self { Self { frequency: default_height_freq(), min_y_ratio: default_min_y_ratio(), max_y_ratio: default_max_y_ratio() } } }

#[derive(Clone, Debug, Deserialize)]
pub struct Surface {
    #[serde(default = "default_snow_thr")] pub snow_threshold: f32,
    #[serde(default = "default_sand_thr")] pub sand_threshold: f32,
    #[serde(default = "default_topsoil")] pub topsoil_thickness: i32,
    #[serde(default = "default_top_names")] pub top: TopNames,
    #[serde(default = "default_sub_names")] pub subsoil: SubsoilNames,
}
#[derive(Clone, Debug, Deserialize)]
pub struct TopNames { pub high: String, pub low: String, pub mid: String }
#[derive(Clone, Debug, Deserialize)]
pub struct SubsoilNames { pub near_surface: String, pub deep: String }
fn default_snow_thr() -> f32 { 0.62 }
fn default_sand_thr() -> f32 { 0.20 }
fn default_topsoil() -> i32 { 3 }
fn default_top_names() -> TopNames { TopNames { high: "snow".into(), low: "sand".into(), mid: "grass".into() } }
fn default_sub_names() -> SubsoilNames { SubsoilNames { near_surface: "dirt".into(), deep: "stone".into() } }
impl Default for Surface { fn default() -> Self { Self { snow_threshold: default_snow_thr(), sand_threshold: default_sand_thr(), topsoil_thickness: default_topsoil(), top: default_top_names(), subsoil: default_sub_names() } } }

#[derive(Clone, Debug, Deserialize)]
pub struct Carvers {
    #[serde(default = "default_carvers_enable")] pub enable: bool,
    #[serde(default = "default_y_scale")] pub y_scale: f32,
    #[serde(default = "default_eps_base")] pub eps_base: f32,
    #[serde(default = "default_eps_add")] pub eps_add: f32,
    #[serde(default = "default_warp_xy")] pub warp_xy: f32,
    #[serde(default = "default_warp_y")] pub warp_y: f32,
    #[serde(default = "default_room_cell")] pub room_cell: f32,
    #[serde(default = "default_room_thr_base")] pub room_thr_base: f32,
    #[serde(default = "default_room_thr_add")] pub room_thr_add: f32,
    #[serde(default = "default_soil_min")] pub soil_min: f32,
    #[serde(default = "default_min_y")] pub min_y: f32,
    #[serde(default = "default_glow_prob")] pub glow_prob: f32,
    #[serde(default)] pub tunnel: Fractal,
    #[serde(default)] pub warp: Fractal,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Fractal { #[serde(default = "d_oct")] pub octaves: i32, #[serde(default = "d_pers")] pub persistence: f32, #[serde(default = "d_lac")] pub lacunarity: f32, #[serde(default = "d_scale")] pub scale: f32 }
fn d_oct() -> i32 { 4 }
fn d_pers() -> f32 { 0.55 }
fn d_lac() -> f32 { 2.0 }
fn d_scale() -> f32 { 140.0 }
fn default_carvers_enable() -> bool { true }
fn default_y_scale() -> f32 { 1.6 }
fn default_eps_base() -> f32 { 0.04 }
fn default_eps_add() -> f32 { 0.08 }
fn default_warp_xy() -> f32 { 5.0 }
fn default_warp_y() -> f32 { 2.5 }
fn default_room_cell() -> f32 { 120.0 }
fn default_room_thr_base() -> f32 { 0.12 }
fn default_room_thr_add() -> f32 { 0.12 }
fn default_soil_min() -> f32 { 3.5 }
fn default_min_y() -> f32 { 2.0 }
fn default_glow_prob() -> f32 { 0.0009 }
impl Default for Fractal { fn default() -> Self { Self { octaves: d_oct(), persistence: d_pers(), lacunarity: d_lac(), scale: 220.0 } } }
impl Default for Carvers { fn default() -> Self { Self { enable: true, y_scale: default_y_scale(), eps_base: default_eps_base(), eps_add: default_eps_add(), warp_xy: default_warp_xy(), warp_y: default_warp_y(), room_cell: default_room_cell(), room_thr_base: default_room_thr_base(), room_thr_add: default_room_thr_add(), soil_min: default_soil_min(), min_y: default_min_y(), glow_prob: default_glow_prob(), tunnel: Fractal { octaves: 4, persistence: 0.55, lacunarity: 2.0, scale: 140.0 }, warp: Fractal { octaves: 3, persistence: 0.6, lacunarity: 2.0, scale: 220.0 } } } }

#[derive(Clone, Debug, Deserialize)]
pub struct Trees {
    #[serde(default = "default_tree_prob")] pub probability: f32,
    #[serde(default = "default_trunk_min")] pub trunk_min: i32,
    #[serde(default = "default_trunk_max")] pub trunk_max: i32,
    #[serde(default = "default_leaf_radius")] pub leaf_radius: i32,
}
fn default_tree_prob() -> f32 { 0.02 }
fn default_trunk_min() -> i32 { 4 }
fn default_trunk_max() -> i32 { 6 }
fn default_leaf_radius() -> i32 { 2 }
impl Default for Trees { fn default() -> Self { Self { probability: default_tree_prob(), trunk_min: default_trunk_min(), trunk_max: default_trunk_max(), leaf_radius: default_leaf_radius() } } }

// Flattened params used in tight loops (snapshot of config)
#[derive(Clone, Debug)]
pub struct WorldGenParams {
    pub mode_flat_thickness: i32,
    pub height_frequency: f32,
    pub min_y_ratio: f32,
    pub max_y_ratio: f32,
    pub snow_threshold: f32,
    pub sand_threshold: f32,
    pub topsoil_thickness: i32,
    pub top_high: String,
    pub top_low: String,
    pub top_mid: String,
    pub sub_near: String,
    pub sub_deep: String,
    pub carvers_enable: bool,
    pub y_scale: f32,
    pub eps_base: f32,
    pub eps_add: f32,
    pub warp_xy: f32,
    pub warp_y: f32,
    pub room_cell: f32,
    pub room_thr_base: f32,
    pub room_thr_add: f32,
    pub soil_min: f32,
    pub min_y: f32,
    pub glow_prob: f32,
    pub tunnel: Fractal,
    pub warp: Fractal,
    pub tree_probability: f32,
    pub trunk_min: i32,
    pub trunk_max: i32,
    pub leaf_radius: i32,
    pub features: Vec<FeatureRule>,
}

impl WorldGenParams {
    pub fn from_config(cfg: &WorldGenConfig) -> Self {
        Self {
            mode_flat_thickness: cfg.flat.thickness,
            height_frequency: cfg.height.frequency,
            min_y_ratio: cfg.height.min_y_ratio,
            max_y_ratio: cfg.height.max_y_ratio,
            snow_threshold: cfg.surface.snow_threshold,
            sand_threshold: cfg.surface.sand_threshold,
            topsoil_thickness: cfg.surface.topsoil_thickness,
            top_high: cfg.surface.top.high.clone(),
            top_low: cfg.surface.top.low.clone(),
            top_mid: cfg.surface.top.mid.clone(),
            sub_near: cfg.surface.subsoil.near_surface.clone(),
            sub_deep: cfg.surface.subsoil.deep.clone(),
            carvers_enable: cfg.carvers.enable,
            y_scale: cfg.carvers.y_scale,
            eps_base: cfg.carvers.eps_base,
            eps_add: cfg.carvers.eps_add,
            warp_xy: cfg.carvers.warp_xy,
            warp_y: cfg.carvers.warp_y,
            room_cell: cfg.carvers.room_cell,
            room_thr_base: cfg.carvers.room_thr_base,
            room_thr_add: cfg.carvers.room_thr_add,
            soil_min: cfg.carvers.soil_min,
            min_y: cfg.carvers.min_y,
            glow_prob: cfg.carvers.glow_prob,
            tunnel: cfg.carvers.tunnel.clone(),
            warp: cfg.carvers.warp.clone(),
            tree_probability: cfg.trees.probability,
            trunk_min: cfg.trees.trunk_min,
            trunk_max: cfg.trees.trunk_max,
            leaf_radius: cfg.trees.leaf_radius,
            features: cfg.features.clone(),
        }
    }
}

impl Default for WorldGenParams {
    fn default() -> Self {
        Self::from_config(&WorldGenConfig::default())
    }
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            flat: Flat::default(),
            height: Height::default(),
            surface: Surface::default(),
            carvers: Carvers::default(),
            trees: Trees::default(),
            features: Vec::new(),
        }
    }
}

pub fn load_params_from_path(path: &std::path::Path) -> Result<WorldGenParams, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
    let cfg: WorldGenConfig = toml::from_str(&s).map_err(|e| format!("parse error: {}", e))?;
    Ok(WorldGenParams::from_config(&cfg))
}

// --- Feature Rules (Phase 2) ---

#[derive(Clone, Debug, Deserialize, Default)]
pub struct FeatureWhen {
    #[serde(default)] pub base_in: Vec<String>,
    #[serde(default)] pub base_not_in: Vec<String>,
    #[serde(default)] pub y_min: Option<i32>,
    #[serde(default)] pub y_max: Option<i32>,
    #[serde(default)] pub below_height_offset: Option<i32>,
    #[serde(default)] pub in_carved: Option<bool>,
    #[serde(default)] pub near_solid: Option<bool>,
    #[serde(default)] pub chance: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FeaturePlace { pub block: String }

#[derive(Clone, Debug, Deserialize)]
pub struct FeatureRule {
    #[serde(default)] pub name: Option<String>,
    #[serde(default)] pub when: FeatureWhen,
    pub place: FeaturePlace,
}
