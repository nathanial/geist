use super::*;
use geist_blocks::config::{BlockDef, BlocksConfig, ShapeConfig};
use geist_blocks::material::MaterialCatalog;
use geist_blocks::types::Block;

fn make_test_registry() -> BlockRegistry {
    let materials = MaterialCatalog::new();
    let blocks = vec![
        BlockDef {
            name: "air".into(),
            id: Some(0),
            solid: Some(false),
            blocks_skylight: Some(false),
            propagates_light: Some(true),
            emission: Some(0),
            light_profile: None,
            light: None,
            shape: Some(ShapeConfig::Simple("cube".into())),
            materials: None,
            state_schema: None,
            seam: None,
        },
        BlockDef {
            name: "stone".into(),
            id: Some(1),
            solid: Some(true),
            blocks_skylight: Some(true),
            propagates_light: Some(false),
            emission: Some(0),
            light_profile: None,
            light: None,
            shape: Some(ShapeConfig::Simple("cube".into())),
            materials: None,
            state_schema: None,
            seam: None,
        },
        BlockDef {
            name: "fence".into(),
            id: Some(2),
            solid: Some(false),
            blocks_skylight: Some(false),
            propagates_light: Some(true),
            emission: Some(0),
            light_profile: None,
            light: None,
            shape: Some(ShapeConfig::Simple("fence".into())),
            materials: None,
            state_schema: None,
            seam: None,
        },
    ];
    BlockRegistry::from_configs(
        materials,
        BlocksConfig { blocks, lighting: None, unknown_block: Some("unknown".into()) },
    )
    .unwrap()
}

fn make_chunk_buf_with(
    reg: &BlockRegistry,
    cx: i32,
    cz: i32,
    sx: usize,
    sy: usize,
    sz: usize,
    fill: &dyn Fn(usize, usize, usize) -> Block,
) -> ChunkBuf {
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                blocks.push(fill(x, y, z));
            }
        }
    }
    ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks)
}

#[test]
fn occ_bit_indexing() {
    // Each bit should map to (x,y,z) in S=2 micro grid
    for x in 0..2 {
        for y in 0..2 {
            for z in 0..2 {
                let idx = ((y & 1) << 2) | ((z & 1) << 1) | (x & 1);
                let mask = 1u8 << idx;
                assert!(super::occ_bit(mask, x, y, z));
                // Neighbor bit should be false
                let other = (idx + 1) & 7;
                let other_mask = 1u8 << other;
                let ox = other & 1;
                let oy = (other >> 2) & 1;
                let oz = (other >> 1) & 1;
                assert!(
                    !super::occ_bit(other_mask, x, y, z) || (x == ox && y == oy && z == oz)
                );
            }
        }
    }
}

#[test]
fn skylight_and_block_passable_gates() {
    let reg = make_test_registry();
    let air = Block { id: reg.id_by_name("air").unwrap(), state: 0 };
    let stone = Block { id: reg.id_by_name("stone").unwrap(), state: 0 };
    let fence = Block { id: reg.id_by_name("fence").unwrap(), state: 0 };

    // skylight_transparent: air and fence (blocks_skylight=false) are transparent; stone is not
    assert!(super::skylight_transparent(air, &reg));
    assert!(super::skylight_transparent(fence, &reg));
    assert!(!super::skylight_transparent(stone, &reg));

    // block_light_passable: air and fence propagate; stone does not
    assert!(super::block_light_passable(air, &reg));
    assert!(super::block_light_passable(fence, &reg));
    assert!(!super::block_light_passable(stone, &reg));
}

#[test]
fn lightborders_from_grid_and_equal() {
    // Build a small grid and verify planes extracted correctly; test equal_planes too
    let sx = 3usize;
    let sy = 2usize;
    let sz = 2usize;
    let mut lg = LightGrid::new(sx, sy, sz);
    // Fill distinct values
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let v = (x as u8) + 10 * (y as u8) + 40 * (z as u8);
                let i = lg.idx(x, y, z);
                lg.block_light[i] = v;
                lg.skylight[i] = v.saturating_add(1);
                lg.beacon_light[i] = v.saturating_add(2);
                lg.beacon_dir[i] = 0; // neutral -> maps to face-specific dir in borders
            }
        }
    }
    let b = LightBorders::from_grid(&lg);
    // Check -X plane
    for y in 0..sy {
        for z in 0..sz {
            let ii = y * sz + z;
            assert_eq!(b.xn[ii], lg.block_light[lg.idx(0, y, z)]);
            assert_eq!(b.sk_xn[ii], lg.skylight[lg.idx(0, y, z)]);
            assert_eq!(b.bcn_xn[ii], lg.beacon_light[lg.idx(0, y, z)]);
            // With beacon_dir=0, -X dir plane encodes 2 (PosX) per impl
            assert_eq!(b.bcn_dir_xn[ii], 2);
        }
    }
    // Check +Z plane
    for x in 0..sx {
        for y in 0..sy {
            let ii = y * sx + x;
            assert_eq!(b.zp[ii], lg.block_light[lg.idx(x, y, sz - 1)]);
            assert_eq!(b.sk_zp[ii], lg.skylight[lg.idx(x, y, sz - 1)]);
            assert_eq!(b.bcn_zp[ii], lg.beacon_light[lg.idx(x, y, sz - 1)]);
            // With beacon_dir=0, +Z dir plane encodes 3 (NegZ) per impl
            assert_eq!(b.bcn_dir_zp[ii], 3);
        }
    }

    // equal_planes detects equality and inequality
    let mut b2 = LightBorders::from_grid(&lg);
    assert!(super::equal_planes(&b, &b2));
    b2.xn[0] ^= 1;
    assert!(!super::equal_planes(&b, &b2));
}

#[test]
fn neighbor_light_max_uses_neighbor_planes_on_bounds() {
    let sx = 2;
    let sy = 1;
    let sz = 1;
    let mut lg = LightGrid::new(sx, sy, sz);
    // No local light
    lg.block_light.fill(0);
    lg.skylight.fill(0);
    lg.beacon_light.fill(0);
    // Provide +X neighbor planes
    lg.nb_xp_blk = Some(vec![77]); // index y*sz+z = 0
    lg.nb_xp_sky = Some(vec![10]);
    lg.nb_xp_bcn = Some(vec![5]);
    assert_eq!(lg.neighbor_light_max(sx - 1, 0, 0, 2), 77);

    // -X neighbor via xn
    lg.nb_xn_blk = Some(vec![66]);
    lg.nb_xn_sky = Some(vec![3]);
    lg.nb_xn_bcn = Some(vec![9]);
    assert_eq!(lg.neighbor_light_max(0, 0, 0, 3), 66);

    // When neighbor plane is None, falls back to boundary cell value
    lg.nb_zp_blk = None;
    lg.nb_zp_sky = None;
    lg.nb_zp_bcn = None;
    let edge_i = lg.idx(0, 0, sz - 1);
    lg.block_light[edge_i] = 65;
    assert_eq!(lg.neighbor_light_max(0, 0, sz - 1, 4), 65);
}

#[test]
fn lightingstore_borders_and_micro_neighbors() {
    let store = LightingStore::new(2, 1, 2);
    // Insert neighbor at (-1,0) so current (0,0) sees xn from its xp
    let mut b = LightBorders::new(2, 1, 2);
    b.xp = vec![11; 1 * 2];
    b.sk_xp = vec![22; 1 * 2];
    b.bcn_xp = vec![33; 1 * 2];
    b.bcn_dir_xp = vec![1; 1 * 2];
    store.update_borders(-1, 0, b.clone());
    let nb = store.get_neighbor_borders(0, 0);
    assert_eq!(nb.xn.as_ref().unwrap(), &b.xp);
    assert_eq!(nb.sk_xn.as_ref().unwrap(), &b.sk_xp);
    assert_eq!(nb.bcn_xn.as_ref().unwrap(), &b.bcn_xp);
    assert_eq!(nb.bcn_dir_xn.as_ref().unwrap(), &b.bcn_dir_xp);

    // Update borders returns false when unchanged
    assert!(!store.update_borders(-1, 0, b.clone()));
    // And true when changed
    let mut b_changed = b.clone();
    b_changed.xp[0] = 99;
    assert!(store.update_borders(-1, 0, b_changed));

    // Micro neighbor mapping
    let mb = MicroBorders {
        xm_sk_neg: vec![1; 2 * 4],
        xm_sk_pos: vec![2; 2 * 4],
        ym_sk_neg: vec![3; 4 * 4],
        ym_sk_pos: vec![4; 4 * 4],
        zm_sk_neg: vec![5; 2 * 4],
        zm_sk_pos: vec![6; 2 * 4],
        xm_bl_neg: vec![7; 2 * 4],
        xm_bl_pos: vec![8; 2 * 4],
        ym_bl_neg: vec![9; 4 * 4],
        ym_bl_pos: vec![10; 4 * 4],
        zm_bl_neg: vec![11; 2 * 4],
        zm_bl_pos: vec![12; 2 * 4],
        xm: 4,
        ym: 2,
        zm: 4,
    };
    store.update_micro_borders(-1, 0, mb.clone());
    let nbm = store.get_neighbor_micro_borders(0, 0);
    // -X neighbor provides xm_*_neg/pos to our neg
    assert_eq!(nbm.xm_sk_neg.as_ref().unwrap(), &mb.xm_sk_pos);
    assert_eq!(nbm.xm_bl_neg.as_ref().unwrap(), &mb.xm_bl_pos);
}

#[test]
fn sample_face_local_s2_fallback_respects_neighbor_coverage() {
    let reg = make_test_registry();
    // 2x2x1 chunk: left column air, right column stone
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, 2, 2, 1, &|x, _, _| Block {
        id: if x == 0 { air_id } else { stone_id },
        state: 0,
    });

    let mut lg = LightGrid::new(2, 2, 1);
    // Set local at (0,0,0) to 10, and its +X neighbor (1,0,0) to 0 initially
    let i000 = lg.idx(0, 0, 0);
    lg.block_light[i000] = 10;
    let i100 = lg.idx(1, 0, 0);
    lg.block_light[i100] = 0;
    // Also set (0,1,0) to 60 to test fallback sampling for open neighbor
    let i010 = lg.idx(0, 1, 0);
    lg.block_light[i010] = 60;

    // From (0,0,0) towards +X where neighbor is stone: fully covered -> return local only
    let v_solid = lg.sample_face_local_s2(&buf, &reg, 0, 0, 0, 2 /* +X into stone */);
    assert_eq!(v_solid, 10);

    // From (1,0,0) towards -X where neighbor is air: fallback samples (0,0,0) and (0,1,0) -> max=60
    let v_open = lg.sample_face_local_s2(&buf, &reg, 1, 0, 0, 3 /* -X into air */);
    assert_eq!(v_open, 60);
}

use geist_world::WorldGenMode;

#[test]
fn compute_with_borders_buf_seeds_from_coarse_neighbors() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let world = geist_world::World::new(1, 1, sx, sy, sz, 42, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    // All air chunk at (0,0)
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block { id: air_id, state: 0 });

    // Seed coarse neighbor on -X via neighbor chunk (-1,0)'s +X plane
    let store = LightingStore::new(sx, sy, sz);
    let mut nb = LightBorders::new(sx, sy, sz);
    nb.xp = vec![200; sy * sz];
    store.update_borders(-1, 0, nb);

    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    // Expect V-atten on x=0 edge where V=200 atten=32
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(0, y, z)], 168);
        }
    }
    // Interior spreads by micro BFS: next macro cell gets one extra micro step attenuation (168-16=152) on micro x=1, and another step to reach macro x=1 (152-16=136)
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(sx - 1, y, z)], 136);
        }
    }

    // Borders from grid reflect edge values
    let b = LightBorders::from_grid(&lg);
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(b.xn[y * sz + z], 168);
        }
    }
}

#[test]
fn compute_with_borders_buf_micro_neighbors_take_precedence() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let world = geist_world::World::new(1, 1, sx, sy, sz, 7, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block { id: air_id, state: 0 });

    let store = LightingStore::new(sx, sy, sz);
    // Provide both coarse and micro neighbors on -X; micro should win
    let mut coarse = LightBorders::new(sx, sy, sz);
    coarse.xp = vec![200; sy * sz];
    store.update_borders(-1, 0, coarse);

    // Neighbor micro planes for chunk (-1,0): we need xm_bl_pos to be present (maps to our xm_bl_neg)
    let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
    let mut mb = MicroBorders {
        xm_sk_neg: vec![0; mys * mzs],
        xm_sk_pos: vec![0; mys * mzs],
        ym_sk_neg: vec![0; mzs * mxs],
        ym_sk_pos: vec![0; mzs * mxs],
        zm_sk_neg: vec![0; mys * mxs],
        zm_sk_pos: vec![0; mys * mxs],
        xm_bl_neg: vec![0; mys * mzs],
        xm_bl_pos: vec![200; mys * mzs],
        ym_bl_neg: vec![0; mzs * mxs],
        ym_bl_pos: vec![0; mzs * mxs],
        zm_bl_neg: vec![0; mys * mxs],
        zm_bl_pos: vec![0; mys * mxs],
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    // Publish neighbor micro borders for (-1,0)
    store.update_micro_borders(-1, 0, mb.clone());

    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    // With MICRO_BLOCK_ATTENUATION=16, expect 200-16=184 on x=0 edge
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(0, y, z)], 184);
        }
    }
}

