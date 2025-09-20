use geist_blocks::config::{BlockDef, BlocksConfig, MaterialSelector, MaterialsDef, ShapeConfig};
use geist_blocks::material::MaterialCatalog;
use geist_blocks::registry::BlockRegistry;
use std::collections::HashMap;

#[test]
fn pack_state_roundtrip_fixed() {
    // Fixed schema with 3 properties and varied cardinalities
    let schema: HashMap<String, Vec<String>> = HashMap::from([
        ("p0".into(), vec!["a".into(), "b".into()]),
        ("p1".into(), vec!["u".into()]),
        ("p2".into(), vec!["x".into(), "y".into(), "z".into()]),
    ]);
    let materials = MaterialCatalog::new();
    let def = BlockDef {
        name: "t".into(),
        id: Some(0),
        solid: Some(true),
        blocks_skylight: Some(true),
        propagates_light: Some(false),
        emission: Some(0),
        light_profile: None,
        light: None,
        shape: None,
        materials: None,
        state_schema: Some(schema.clone()),
        seam: None,
    };
    let cfg = BlocksConfig {
        blocks: vec![def],
        lighting: None,
        unknown_block: None,
    };
    let reg = BlockRegistry::from_configs(materials, cfg).expect("registry");
    let ty = reg.get(0).unwrap();

    // Select subset of props
    let props = HashMap::from([
        ("p0".into(), "b".into()), // second value
        // omit p1 -> should default to first
        ("p2".into(), "z".into()), // third value
    ]);
    let state = ty.pack_state(&props);
    assert_eq!(ty.state_prop_value(state, "p0"), Some("b"));
    assert_eq!(ty.state_prop_value(state, "p1"), Some("u"));
    assert_eq!(ty.state_prop_value(state, "p2"), Some("z"));
}

#[test]
fn material_catalog_reserves_zero_id_for_sentinel() {
    let materials = MaterialCatalog::from_toml_str(
        r#"
        [materials]
        jungle_leaves = ["assets/blocks/leaves_jungle_opaque.png"]
        unknown = ["assets/blocks/unknown.png"]
    "#,
    )
    .unwrap();
    assert!(materials.materials[0].key.is_empty());
    let jungle = materials.get_id("jungle_leaves").unwrap();
    let unknown = materials.get_id("unknown").unwrap();
    assert!(jungle.0 > 0);
    assert!(unknown.0 > 0);
}

#[test]
fn material_cache_matches_dynamic_fixed() {
    use geist_blocks::types::FaceRole;
    let materials = MaterialCatalog::from_toml_str(
        r#"
        [materials]
        red = ["assets/blocks/red.png"]
        blue = ["assets/blocks/blue.png"]
        unknown = ["assets/blocks/unknown.png"]
    "#,
    )
    .unwrap();
    let schema = HashMap::from([(
        "material".to_string(),
        vec!["red".to_string(), "blue".to_string()],
    )]);
    let materials_def = MaterialsDef {
        all: None,
        top: None,
        bottom: None,
        side: Some(MaterialSelector::By {
            by: "material".into(),
            map: HashMap::from([("red".into(), "red".into()), ("blue".into(), "blue".into())]),
        }),
    };
    let def = BlockDef {
        name: "painted".into(),
        id: Some(1),
        solid: Some(true),
        blocks_skylight: Some(true),
        propagates_light: Some(false),
        emission: Some(0),
        light_profile: None,
        light: None,
        shape: Some(ShapeConfig::Simple("cube".into())),
        materials: Some(materials_def),
        state_schema: Some(schema.clone()),
        seam: None,
    };
    let cfg = BlocksConfig {
        blocks: vec![def],
        lighting: None,
        unknown_block: Some("unknown".into()),
    };
    let reg = BlockRegistry::from_configs(materials, cfg).expect("registry");
    let ty = reg.get(1).expect("block type");
    let red_state = ty.pack_state(&HashMap::from([("material".into(), "red".into())]));
    let blue_state = ty.pack_state(&HashMap::from([("material".into(), "blue".into())]));
    let dyn_red = ty
        .materials
        .material_for(FaceRole::Side, red_state, ty)
        .unwrap();
    let dyn_blue = ty
        .materials
        .material_for(FaceRole::Side, blue_state, ty)
        .unwrap();
    let cached_red = ty.material_for_cached(FaceRole::Side, red_state);
    let cached_blue = ty.material_for_cached(FaceRole::Side, blue_state);
    assert_eq!(dyn_red, cached_red);
    assert_eq!(dyn_blue, cached_blue);
}

#[test]
fn slab_occlusion_and_occupancy_half_fixed() {
    let materials = MaterialCatalog::from_toml_str(
        r#"
        [materials]
        unknown = ["assets/blocks/unknown.png"]
    "#,
    )
    .unwrap();
    let schema = HashMap::from([(
        "half".to_string(),
        vec!["bottom".to_string(), "top".to_string()],
    )]);
    let def = BlockDef {
        name: "slab".into(),
        id: Some(2),
        solid: Some(true),
        blocks_skylight: Some(false),
        propagates_light: Some(true),
        emission: Some(0),
        light_profile: None,
        light: None,
        shape: Some(ShapeConfig::Simple("slab".into())),
        materials: None,
        state_schema: Some(schema.clone()),
        seam: None,
    };
    let cfg = BlocksConfig {
        blocks: vec![def],
        lighting: None,
        unknown_block: Some("unknown".into()),
    };
    let reg = BlockRegistry::from_configs(materials, cfg).expect("registry");
    let ty = reg.get(2).expect("block type");
    let st_bottom = ty.pack_state(&HashMap::from([("half".into(), "bottom".into())]));
    let st_top = ty.pack_state(&HashMap::from([("half".into(), "top".into())]));
    let mask_bottom = ty.occlusion_mask_cached(st_bottom);
    let mask_top = ty.occlusion_mask_cached(st_top);
    let sides: u8 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5);
    assert_eq!(mask_bottom & sides, sides);
    assert_eq!(mask_top & sides, sides);
    assert_eq!((mask_bottom >> 0) & 1, 1);
    assert_eq!((mask_bottom >> 1) & 1, 0);
    assert_eq!((mask_top >> 0) & 1, 0);
    assert_eq!((mask_top >> 1) & 1, 1);
    let occ_bottom = ty.variant(st_bottom).occupancy.unwrap();
    let occ_top = ty.variant(st_top).occupancy.unwrap();
    assert_eq!(occ_bottom, 0x0F);
    assert_eq!(occ_top, 0xF0);
}
