use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct WorldGenConfig {
    #[serde(default = "default_mode")]
    #[allow(dead_code)]
    pub mode: Mode,
    #[serde(default)]
    pub flat: Flat,
    #[serde(default)]
    pub platform: Platform,
    #[serde(default)]
    pub height: Height,
    #[serde(default)]
    pub surface: Surface,
    #[serde(default)]
    pub carvers: Carvers,
    #[serde(default)]
    pub trees: Trees,
    #[serde(default)]
    pub features: Vec<FeatureRule>,
    #[serde(default)]
    pub biomes: Biomes,
    #[serde(default)]
    pub water: Water,
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            flat: Flat::default(),
            platform: Platform::default(),
            height: Height::default(),
            surface: Surface::default(),
            carvers: Carvers::default(),
            trees: Trees::default(),
            features: Vec::new(),
            biomes: Biomes::default(),
            water: Water::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Normal,
    Flat,
}

fn default_mode() -> Mode {
    Mode::Normal
}

#[derive(Clone, Debug, Deserialize)]
pub struct Flat {
    #[serde(default = "default_flat_thickness")]
    pub thickness: i32,
}
fn default_flat_thickness() -> i32 {
    1
}
impl Default for Flat {
    fn default() -> Self {
        Self { thickness: 1 }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Platform {
    #[serde(default = "default_platform_y_ratio")]
    pub y_ratio: f32,
    #[serde(default = "default_platform_y_offset")]
    pub y_offset: f32,
}
fn default_platform_y_ratio() -> f32 {
    0.70
}
fn default_platform_y_offset() -> f32 {
    16.0
}
impl Default for Platform {
    fn default() -> Self {
        Self {
            y_ratio: default_platform_y_ratio(),
            y_offset: default_platform_y_offset(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Height {
    #[serde(default = "default_height_freq")]
    pub frequency: f32,
    #[serde(default = "default_min_y_ratio")]
    pub min_y_ratio: f32,
    #[serde(default = "default_max_y_ratio")]
    pub max_y_ratio: f32,
}
fn default_height_freq() -> f32 {
    0.02
}
fn default_min_y_ratio() -> f32 {
    0.15
}
fn default_max_y_ratio() -> f32 {
    0.70
}
impl Default for Height {
    fn default() -> Self {
        Self {
            frequency: default_height_freq(),
            min_y_ratio: default_min_y_ratio(),
            max_y_ratio: default_max_y_ratio(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Surface {
    #[serde(default = "default_snow_thr")]
    pub snow_threshold: f32,
    #[serde(default = "default_sand_thr")]
    pub sand_threshold: f32,
    #[serde(default = "default_topsoil")]
    pub topsoil_thickness: i32,
    #[serde(default = "default_top_names")]
    pub top: TopNames,
    #[serde(default = "default_sub_names")]
    pub subsoil: SubsoilNames,
}
#[derive(Clone, Debug, Deserialize)]
pub struct TopNames {
    pub high: String,
    pub low: String,
    pub mid: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SubsoilNames {
    pub near_surface: String,
    pub deep: String,
}
fn default_snow_thr() -> f32 {
    0.62
}
fn default_sand_thr() -> f32 {
    0.22
}
fn default_topsoil() -> i32 {
    3
}
fn default_top_names() -> TopNames {
    TopNames {
        high: "snow".into(),
        low: "sand".into(),
        mid: "grass".into(),
    }
}
fn default_sub_names() -> SubsoilNames {
    SubsoilNames {
        near_surface: "dirt".into(),
        deep: "stone".into(),
    }
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            snow_threshold: default_snow_thr(),
            sand_threshold: default_sand_thr(),
            topsoil_thickness: default_topsoil(),
            top: default_top_names(),
            subsoil: default_sub_names(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Water {
    #[serde(default = "default_water_enable")]
    pub enable: bool,
    #[serde(default = "default_water_level_ratio")]
    pub level_ratio: f32,
}
fn default_water_enable() -> bool { true }
fn default_water_level_ratio() -> f32 { 0.33 }
impl Default for Water {
    fn default() -> Self {
        Self { enable: true, level_ratio: default_water_level_ratio() }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Carvers {
    #[serde(default = "default_carvers_enable")]
    pub enable: bool,
    #[serde(default = "default_y_scale")]
    pub y_scale: f32,
    #[serde(default = "default_eps_base")]
    pub eps_base: f32,
    #[serde(default = "default_eps_add")]
    pub eps_add: f32,
    #[serde(default = "default_warp_xy")]
    pub warp_xy: f32,
    #[serde(default = "default_warp_y")]
    pub warp_y: f32,
    #[serde(default = "default_room_cell")]
    pub room_cell: f32,
    #[serde(default = "default_room_thr_base")]
    pub room_thr_base: f32,
    #[serde(default = "default_room_thr_add")]
    pub room_thr_add: f32,
    #[serde(default = "default_soil_min")]
    pub soil_min: f32,
    #[serde(default = "default_min_y")]
    pub min_y: f32,
    #[serde(default = "default_glow_prob")]
    pub glow_prob: f32,
    #[serde(default)]
    pub tunnel: Fractal,
    #[serde(default)]
    pub warp: Fractal,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Fractal {
    #[serde(default = "d_oct")]
    pub octaves: i32,
    #[serde(default = "d_pers")]
    pub persistence: f32,
    #[serde(default = "d_lac")]
    pub lacunarity: f32,
    #[serde(default = "d_scale")]
    pub scale: f32,
}
fn d_oct() -> i32 {
    4
}
fn d_pers() -> f32 {
    0.55
}
fn d_lac() -> f32 {
    2.0
}
fn d_scale() -> f32 {
    140.0
}
fn default_carvers_enable() -> bool {
    true
}
fn default_y_scale() -> f32 {
    1.6
}
fn default_eps_base() -> f32 {
    0.04
}
fn default_eps_add() -> f32 {
    0.08
}
fn default_warp_xy() -> f32 {
    5.0
}
fn default_warp_y() -> f32 {
    2.5
}
fn default_room_cell() -> f32 {
    120.0
}
fn default_room_thr_base() -> f32 {
    0.12
}
fn default_room_thr_add() -> f32 {
    0.12
}
fn default_soil_min() -> f32 {
    3.5
}
fn default_min_y() -> f32 {
    2.0
}
fn default_glow_prob() -> f32 {
    0.0009
}
impl Default for Fractal {
    fn default() -> Self {
        Self {
            octaves: d_oct(),
            persistence: d_pers(),
            lacunarity: d_lac(),
            scale: 220.0,
        }
    }
}
impl Default for Carvers {
    fn default() -> Self {
        Self {
            enable: true,
            y_scale: default_y_scale(),
            eps_base: default_eps_base(),
            eps_add: default_eps_add(),
            warp_xy: default_warp_xy(),
            warp_y: default_warp_y(),
            room_cell: default_room_cell(),
            room_thr_base: default_room_thr_base(),
            room_thr_add: default_room_thr_add(),
            soil_min: default_soil_min(),
            min_y: default_min_y(),
            glow_prob: default_glow_prob(),
            tunnel: Fractal {
                octaves: 4,
                persistence: 0.55,
                lacunarity: 2.0,
                scale: 140.0,
            },
            warp: Fractal {
                octaves: 3,
                persistence: 0.6,
                lacunarity: 2.0,
                scale: 220.0,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Trees {
    #[serde(default = "default_tree_prob")]
    pub probability: f32,
    #[serde(default = "default_trunk_min")]
    pub trunk_min: i32,
    #[serde(default = "default_trunk_max")]
    pub trunk_max: i32,
    #[serde(default = "default_leaf_radius")]
    pub leaf_radius: i32,
}
fn default_tree_prob() -> f32 {
    0.02
}
fn default_trunk_min() -> i32 {
    4
}
fn default_trunk_max() -> i32 {
    6
}
fn default_leaf_radius() -> i32 {
    2
}
impl Default for Trees {
    fn default() -> Self {
        Self {
            probability: default_tree_prob(),
            trunk_min: default_trunk_min(),
            trunk_max: default_trunk_max(),
            leaf_radius: default_leaf_radius(),
        }
    }
}

// Flattened params used in tight loops (snapshot of config)
#[derive(Clone, Debug)]
pub struct WorldGenParams {
    #[allow(dead_code)]
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
    pub biomes: Option<BiomesParams>,
    // Platform controls (for flying structures)
    pub platform_y_ratio: f32,
    pub platform_y_offset: f32,
    pub water_enable: bool,
    pub water_level_ratio: f32,
}

impl WorldGenParams {
    pub fn default() -> Self {
        Self::from_config(&WorldGenConfig::default())
    }
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
            biomes: if cfg.biomes.enable {
                Some(BiomesParams::from(&cfg.biomes))
            } else {
                None
            },
            platform_y_ratio: cfg.platform.y_ratio,
            platform_y_offset: cfg.platform.y_offset,
            water_enable: cfg.water.enable,
            water_level_ratio: cfg.water.level_ratio,
        }
    }
}

pub fn load_params_from_path(path: &Path) -> Result<WorldGenParams, Box<dyn Error>> {
    let s = fs::read_to_string(path)?;
    let cfg: WorldGenConfig = toml::from_str(&s)?;
    Ok(WorldGenParams::from_config(&cfg))
}

#[derive(Clone, Debug, Deserialize)]
pub struct FeatureRule {
    #[serde(default)]
    #[allow(dead_code)]
    pub name: Option<String>,
    #[serde(default)]
    pub when: FeatureWhen,
    pub place: FeaturePlace,
}

// --- Biomes (Phase 3) ---

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Biomes {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub temp: Climate2D,
    #[serde(default)]
    pub moisture: Climate2D,
    #[serde(default)]
    pub biomes: Vec<BiomeDef>,
    #[serde(default = "d_one")]
    pub scale_x: f32,
    #[serde(default = "d_one")]
    pub scale_z: f32,
    #[serde(default)]
    pub debug_pack_all: bool,
    #[serde(default = "d_cell")]
    pub debug_cell_size: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Climate2D {
    #[serde(default = "default_climate_freq")]
    pub frequency: f32,
}
fn default_climate_freq() -> f32 {
    0.01
}
impl Default for Climate2D {
    fn default() -> Self {
        Self {
            frequency: default_climate_freq(),
        }
    }
}
fn d_one() -> f32 {
    1.0
}
fn d_cell() -> i32 {
    32
}

#[derive(Clone, Debug, Deserialize)]
pub struct BiomeDef {
    pub name: String,
    #[serde(default)]
    pub temp_min: Option<f32>,
    #[serde(default)]
    pub temp_max: Option<f32>,
    #[serde(default)]
    pub moisture_min: Option<f32>,
    #[serde(default)]
    pub moisture_max: Option<f32>,
    #[serde(default)]
    pub top_block: Option<String>,
    #[serde(default)]
    pub species_weights: std::collections::HashMap<String, f32>,
    #[serde(default)]
    pub tree_density: Option<f32>,
    #[serde(default)]
    pub leaf_tint: Option<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct BiomesParams {
    pub temp_freq: f32,
    pub moisture_freq: f32,
    pub defs: Vec<BiomeDefParam>,
    pub scale_x: f32,
    pub scale_z: f32,
    pub debug_pack_all: bool,
    pub debug_cell_size: i32,
}

#[derive(Clone, Debug)]
pub struct BiomeDefParam {
    pub name: String,
    pub temp_min: f32,
    pub temp_max: f32,
    pub moisture_min: f32,
    pub moisture_max: f32,
    pub top_block: Option<String>,
    pub species_weights: std::collections::HashMap<String, f32>,
    pub tree_density: Option<f32>,
    pub leaf_tint: Option<[f32; 3]>,
}

impl BiomesParams {
    pub fn from(cfg: &Biomes) -> Self {
        let defs = cfg
            .biomes
            .iter()
            .map(|b| BiomeDefParam {
                name: b.name.clone(),
                temp_min: b.temp_min.unwrap_or(0.0),
                temp_max: b.temp_max.unwrap_or(1.0),
                moisture_min: b.moisture_min.unwrap_or(0.0),
                moisture_max: b.moisture_max.unwrap_or(1.0),
                top_block: b.top_block.clone(),
                species_weights: b.species_weights.clone(),
                tree_density: b.tree_density,
                leaf_tint: b.leaf_tint,
            })
            .collect();
        Self {
            temp_freq: cfg.temp.frequency,
            moisture_freq: cfg.moisture.frequency,
            defs,
            scale_x: cfg.scale_x,
            scale_z: cfg.scale_z,
            debug_pack_all: cfg.debug_pack_all,
            debug_cell_size: cfg.debug_cell_size,
        }
    }
}

// Feature condition and placement types
#[derive(Clone, Debug, Deserialize, Default)]
pub struct FeatureWhen {
    #[serde(default)]
    pub base_in: Vec<String>,
    #[serde(default)]
    pub base_not_in: Vec<String>,
    #[serde(default)]
    pub y_min: Option<i32>,
    #[serde(default)]
    pub y_max: Option<i32>,
    #[serde(default)]
    pub below_height_offset: Option<i32>,
    #[serde(default)]
    pub in_carved: Option<bool>,
    #[serde(default)]
    pub near_solid: Option<bool>,
    #[serde(default)]
    pub chance: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FeaturePlace {
    pub block: String,
}
