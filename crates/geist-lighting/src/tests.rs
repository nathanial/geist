use super::*;
use geist_blocks::config::{BlockDef, BlocksConfig, ShapeConfig};
use geist_blocks::material::MaterialCatalog;
use geist_blocks::types::Block;
use geist_world::ChunkCoord;

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
            name: "slab".into(),
            id: Some(3),
            solid: Some(true),
            blocks_skylight: Some(false),
            propagates_light: Some(true),
            emission: Some(0),
            light_profile: None,
            light: None,
            shape: Some(ShapeConfig::Simple("slab".into())),
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
        BlocksConfig {
            blocks,
            lighting: None,
            unknown_block: Some("unknown".into()),
        },
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
    ChunkBuf::from_blocks_local(ChunkCoord::new(cx, 0, cz), sx, sy, sz, blocks)
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
                assert!(!super::occ_bit(other_mask, x, y, z) || (x == ox && y == oy && z == oz));
            }
        }
    }
}

#[test]
fn skylight_and_block_passable_gates() {
    let reg = make_test_registry();
    let air = Block {
        id: reg.id_by_name("air").unwrap(),
        state: 0,
    };
    let stone = Block {
        id: reg.id_by_name("stone").unwrap(),
        state: 0,
    };
    let fence = Block {
        id: reg.id_by_name("fence").unwrap(),
        state: 0,
    };

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
    let mut tmp = b2.xn.to_vec();
    tmp[0] ^= 1;
    b2.xn = tmp.into();
    assert!(!super::equal_planes(&b, &b2));
}

#[test]
fn lightborders_y_planes_from_grid() {
    let sx = 3usize;
    let sy = 3usize;
    let sz = 2usize;
    let mut lg = LightGrid::new(sx, sy, sz);
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let v = (x as u8) + 10 * (y as u8) + 40 * (z as u8);
                let i = lg.idx(x, y, z);
                lg.block_light[i] = v;
                lg.skylight[i] = v.saturating_add(1);
                lg.beacon_light[i] = v.saturating_add(2);
            }
        }
    }
    let b = LightBorders::from_grid(&lg);
    // -Y plane uses y=0
    for z in 0..sz {
        for x in 0..sx {
            let ii = z * sx + x;
            assert_eq!(b.yn[ii], lg.block_light[lg.idx(x, 0, z)]);
            assert_eq!(b.sk_yn[ii], lg.skylight[lg.idx(x, 0, z)]);
            assert_eq!(b.bcn_yn[ii], lg.beacon_light[lg.idx(x, 0, z)]);
        }
    }
    // +Y plane uses y=sy-1
    for z in 0..sz {
        for x in 0..sx {
            let ii = z * sx + x;
            assert_eq!(b.yp[ii], lg.block_light[lg.idx(x, sy - 1, z)]);
            assert_eq!(b.sk_yp[ii], lg.skylight[lg.idx(x, sy - 1, z)]);
            assert_eq!(b.bcn_yp[ii], lg.beacon_light[lg.idx(x, sy - 1, z)]);
        }
    }
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
    lg.nb_xp_blk = Some(vec![77].into()); // index y*sz+z = 0
    lg.nb_xp_sky = Some(vec![10].into());
    lg.nb_xp_bcn = Some(vec![5].into());
    assert_eq!(lg.neighbor_light_max(sx - 1, 0, 0, 2), 77);

    // -X neighbor via xn
    lg.nb_xn_blk = Some(vec![66].into());
    lg.nb_xn_sky = Some(vec![3].into());
    lg.nb_xn_bcn = Some(vec![9].into());
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
    b.xp = vec![11; 1 * 2].into();
    b.sk_xp = vec![22; 1 * 2].into();
    b.bcn_xp = vec![33; 1 * 2].into();
    b.bcn_dir_xp = vec![1; 1 * 2].into();
    store.update_borders(ChunkCoord::new(-1, 0, 0), b.clone());
    let nb = store.get_neighbor_borders(ChunkCoord::new(0, 0, 0));
    assert_eq!(nb.xn.as_ref().unwrap(), &b.xp);
    assert_eq!(nb.sk_xn.as_ref().unwrap(), &b.sk_xp);
    assert_eq!(nb.bcn_xn.as_ref().unwrap(), &b.bcn_xp);
    assert_eq!(nb.bcn_dir_xn.as_ref().unwrap(), &b.bcn_dir_xp);

    // Update borders returns false when unchanged
    assert!(!store.update_borders(ChunkCoord::new(-1, 0, 0), b.clone()));
    // And true when changed
    let mut b_changed = b.clone();
    let mut tmp = b_changed.xp.to_vec();
    tmp[0] = 99;
    b_changed.xp = tmp.into();
    assert!(store.update_borders(ChunkCoord::new(-1, 0, 0), b_changed));

    // Micro neighbor mapping
    let mb = MicroBorders {
        xm_sk_neg: vec![1; 2 * 4].into(),
        xm_sk_pos: vec![2; 2 * 4].into(),
        ym_sk_neg: vec![3; 4 * 4].into(),
        ym_sk_pos: vec![4; 4 * 4].into(),
        zm_sk_neg: vec![5; 2 * 4].into(),
        zm_sk_pos: vec![6; 2 * 4].into(),
        xm_bl_neg: vec![7; 2 * 4].into(),
        xm_bl_pos: vec![8; 2 * 4].into(),
        ym_bl_neg: vec![9; 4 * 4].into(),
        ym_bl_pos: vec![10; 4 * 4].into(),
        zm_bl_neg: vec![11; 2 * 4].into(),
        zm_bl_pos: vec![12; 2 * 4].into(),
        xm: 4,
        ym: 2,
        zm: 4,
    };
    store.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb.clone());
    let nbm = store.get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0));
    // -X neighbor provides xm_*_neg/pos to our neg
    assert_eq!(nbm.xm_sk_neg.as_ref().unwrap(), &mb.xm_sk_pos);
    assert_eq!(nbm.xm_bl_neg.as_ref().unwrap(), &mb.xm_bl_pos);
    // +X neighbor mapping
    let mut mb2 = mb.clone();
    store.update_micro_borders(ChunkCoord::new(1, 0, 0), mb2.clone());
    let nbm2 = store.get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0));
    assert_eq!(nbm2.xm_sk_pos.as_ref().unwrap(), &mb2.xm_sk_neg);
    assert_eq!(nbm2.xm_bl_pos.as_ref().unwrap(), &mb2.xm_bl_neg);
    // -Z neighbor mapping
    store.update_micro_borders(ChunkCoord::new(0, 0, -1), mb.clone());
    let nbm3 = store.get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0));
    assert_eq!(nbm3.zm_sk_neg.as_ref().unwrap(), &mb.zm_sk_pos);
    assert_eq!(nbm3.zm_bl_neg.as_ref().unwrap(), &mb.zm_bl_pos);
    // +Z neighbor mapping
    store.update_micro_borders(ChunkCoord::new(0, 0, 1), mb2.clone());
    let nbm4 = store.get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0));
    assert_eq!(nbm4.zm_sk_pos.as_ref().unwrap(), &mb2.zm_sk_neg);
    assert_eq!(nbm4.zm_bl_pos.as_ref().unwrap(), &mb2.zm_bl_neg);
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
    let world = geist_world::World::new(1, 1, 1, 42, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    // All air chunk at (0,0)
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });

    // Seed coarse neighbor on -X via neighbor chunk (-1,0)'s +X plane
    let store = LightingStore::new(sx, sy, sz);
    let mut nb = LightBorders::new(sx, sy, sz);
    nb.xp = vec![200; sy * sz].into();
    store.update_borders(ChunkCoord::new(-1, 0, 0), nb);

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
    let world = geist_world::World::new(1, 1, 1, 7, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });

    let store = LightingStore::new(sx, sy, sz);
    // Provide both coarse and micro neighbors on -X; micro should win
    let mut coarse = LightBorders::new(sx, sy, sz);
    coarse.xp = vec![200; sy * sz].into();
    store.update_borders(ChunkCoord::new(-1, 0, 0), coarse);

    // Neighbor micro planes for chunk (-1,0): we need xm_bl_pos to be present (maps to our xm_bl_neg)
    let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
    let mut mb = MicroBorders {
        xm_sk_neg: vec![0; mys * mzs].into(),
        xm_sk_pos: vec![0; mys * mzs].into(),
        ym_sk_neg: vec![0; mzs * mxs].into(),
        ym_sk_pos: vec![0; mzs * mxs].into(),
        zm_sk_neg: vec![0; mys * mxs].into(),
        zm_sk_pos: vec![0; mys * mxs].into(),
        xm_bl_neg: vec![0; mys * mzs].into(),
        xm_bl_pos: vec![200; mys * mzs].into(),
        ym_bl_neg: vec![0; mzs * mxs].into(),
        ym_bl_pos: vec![0; mzs * mxs].into(),
        zm_bl_neg: vec![0; mys * mxs].into(),
        zm_bl_pos: vec![0; mys * mxs].into(),
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    // Publish neighbor micro borders for (-1,0)
    store.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb.clone());

    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    // With MICRO_BLOCK_ATTENUATION=16, expect 200-16=184 on x=0 edge
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(0, y, z)], 184);
        }
    }
}

#[test]
fn compute_light_with_borders_buf_vertical_neighbor_seeding() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let world = geist_world::World::new(1, 1, 1, 9, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });

    let store = LightingStore::new(sx, sy, sz);
    let mut below = LightBorders::new(sx, sy, sz);
    below.yp = vec![200; sx * sz].into();
    store.update_borders(ChunkCoord::new(0, -1, 0), below);
    let mut above = LightBorders::new(sx, sy, sz);
    above.yn = vec![180; sx * sz].into();
    store.update_borders(ChunkCoord::new(0, 1, 0), above);

    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    for z in 0..sz {
        for x in 0..sx {
            assert_eq!(lg.block_light[lg.idx(x, 0, z)], 168);
            assert_eq!(lg.block_light[lg.idx(x, sy - 1, z)], 148);
        }
    }
}

#[test]
fn lightgrid_compute_with_borders_buf_vertical_neighbor_seeding() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let store = LightingStore::new(sx, sy, sz);
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });
    let mut below = LightBorders::new(sx, sy, sz);
    below.yp = vec![200; sx * sz].into();
    store.update_borders(ChunkCoord::new(0, -1, 0), below);
    let mut above = LightBorders::new(sx, sy, sz);
    above.yn = vec![180; sx * sz].into();
    store.update_borders(ChunkCoord::new(0, 1, 0), above);

    let lg = LightGrid::compute_with_borders_buf(&buf, &store, &reg);
    for z in 0..sz {
        for x in 0..sx {
            assert_eq!(lg.block_light[lg.idx(x, 0, z)], 168);
            assert_eq!(lg.block_light[lg.idx(x, sy - 1, z)], 148);
        }
    }
}

#[test]
fn vertical_neighbor_planes_map_to_lower_chunk() {
    let sx = 2usize;
    let sy = 2usize;
    let sz = 2usize;
    let store = LightingStore::new(sx, sy, sz);
    let top_coord = ChunkCoord::new(0, 1, 0);
    let bottom_coord = ChunkCoord::new(0, 0, 0);

    // Coarse -Y planes from the upper chunk map to +Y neighbors below.
    let mut coarse = LightBorders::new(sx, sy, sz);
    let yn_vals: Vec<u8> = (0..(sx * sz)).map(|i| (i as u8) + 40).collect();
    coarse.yn = yn_vals.clone().into();
    store.update_borders(top_coord, coarse.clone());

    let nb = store.get_neighbor_borders(bottom_coord);
    assert_eq!(
        nb.yp.as_ref().unwrap().as_ref(),
        coarse.yn.as_ref(),
        "upper chunk -Y coarse plane not exposed as lower +Y neighbor",
    );

    // Micro S=2 planes should follow the same mapping.
    let mxs = sx * 2;
    let mys = sy * 2;
    let mzs = sz * 2;
    let zeros = vec![0u8; mys * mzs];
    let ym_sk_neg: Vec<u8> = (0..(mzs * mxs)).map(|i| (i as u8) + 5).collect();
    let ym_bl_neg: Vec<u8> = (0..(mzs * mxs)).map(|i| (i as u8) + 80).collect();
    let micro = MicroBorders {
        xm_sk_neg: zeros.clone().into(),
        xm_sk_pos: zeros.clone().into(),
        ym_sk_neg: ym_sk_neg.clone().into(),
        ym_sk_pos: vec![0; mzs * mxs].into(),
        zm_sk_neg: zeros.clone().into(),
        zm_sk_pos: zeros.clone().into(),
        xm_bl_neg: zeros.clone().into(),
        xm_bl_pos: zeros.clone().into(),
        ym_bl_neg: ym_bl_neg.clone().into(),
        ym_bl_pos: vec![0; mzs * mxs].into(),
        zm_bl_neg: zeros.clone().into(),
        zm_bl_pos: zeros.into(),
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    store.update_micro_borders(top_coord, micro);
    let nbm = store.get_neighbor_micro_borders(bottom_coord);
    assert_eq!(
        nbm.ym_sk_pos.as_ref().unwrap().as_ref(),
        &ym_sk_neg[..],
        "upper chunk -Y micro skylight plane not mapped to lower +Y neighbor",
    );
    assert_eq!(
        nbm.ym_bl_pos.as_ref().unwrap().as_ref(),
        &ym_bl_neg[..],
        "upper chunk -Y micro block plane not mapped to lower +Y neighbor",
    );
}

#[test]
fn micro_skylight_open_above_and_blocked() {
    let reg = make_test_registry();
    let sx = 1;
    let sy = 2;
    let sz = 1;
    let world = geist_world::World::new(1, 1, 1, 1, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();

    // Case 1: all air -> skylight fills both macro cells
    let buf_air = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });
    let store = LightingStore::new(sx, sy, sz);
    let lg_air = super::compute_light_with_borders_buf(&buf_air, &store, &reg, &world);
    assert_eq!(lg_air.skylight[lg_air.idx(0, 0, 0)], 255);
    assert_eq!(lg_air.skylight[lg_air.idx(0, 1, 0)], 255);

    // Case 2: stone on top blocks skylight below
    let buf_blocked = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, y, _| Block {
        id: if y == sy - 1 { stone_id } else { air_id },
        state: 0,
    });
    let lg_blk = super::compute_light_with_borders_buf(&buf_blocked, &store, &reg, &world);
    assert_eq!(lg_blk.skylight[lg_blk.idx(0, 1, 0)], 0); // top is stone
    assert_eq!(lg_blk.skylight[lg_blk.idx(0, 0, 0)], 0); // below stays dark
}

#[test]
fn skylight_neighbors_coarse_and_micro_precedence() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2; // top layer will be a roof to block local skylight
    let sz = 2;
    let world = geist_world::World::new(1, 1, 1, 3, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    // Bottom layer air, top layer stone: blocks skylight-from-above seeding locally
    let stone_id = reg.id_by_name("stone").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, y, _| Block {
        id: if y == sy - 1 { stone_id } else { air_id },
        state: 0,
    });

    // Coarse skylight seeding from -X
    let store = LightingStore::new(sx, sy, sz);
    let mut nb = LightBorders::new(sx, sy, sz);
    nb.sk_xp = vec![200; sy * sz].into(); // neighbor (-1,0)'s +X skylight
    store.update_borders(ChunkCoord::new(-1, 0, 0), nb);
    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    // Edge x=0 attenuated by COARSE_SEAM_ATTENUATION=32
    for z in 0..sz {
        assert_eq!(lg.skylight[lg.idx(0, 0, z)], 168);
    }
    // Interior macro x=1 gets two micro steps (16 each) -> 136
    for z in 0..sz {
        assert_eq!(lg.skylight[lg.idx(sx - 1, 0, z)], 136);
    }

    // Micro neighbor skylight should override coarse planes
    let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
    let mut mb = MicroBorders {
        xm_sk_neg: vec![0; mys * mzs].into(),
        xm_sk_pos: vec![200; mys * mzs].into(), // maps to our xm_sk_neg
        ym_sk_neg: vec![0; mzs * mxs].into(),
        ym_sk_pos: vec![0; mzs * mxs].into(),
        zm_sk_neg: vec![0; mys * mxs].into(),
        zm_sk_pos: vec![0; mys * mxs].into(),
        xm_bl_neg: vec![0; mys * mzs].into(),
        xm_bl_pos: vec![0; mys * mzs].into(),
        ym_bl_neg: vec![0; mzs * mxs].into(),
        ym_bl_pos: vec![0; mzs * mxs].into(),
        zm_bl_neg: vec![0; mys * mxs].into(),
        zm_bl_pos: vec![0; mys * mxs].into(),
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    let store2 = LightingStore::new(sx, sy, sz);
    // Publish both coarse and micro for (-1,0)
    let mut coarse = LightBorders::new(sx, sy, sz);
    coarse.sk_xp = vec![200; sy * sz].into();
    store2.update_borders(ChunkCoord::new(-1, 0, 0), coarse);
    store2.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb.clone());
    let lg2 = super::compute_light_with_borders_buf(&buf, &store2, &reg, &world);
    for z in 0..sz {
        assert_eq!(lg2.skylight[lg2.idx(0, 0, z)], 184); // 200-16 using micro seam
    }
}

#[test]
fn emitters_seed_micro_and_remove() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 1;
    let sz = 1;
    let world = geist_world::World::new(1, 1, 1, 5, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });

    let store = LightingStore::new(sx, sy, sz);
    // Place an emitter at (0,0,0) world coords with level 200
    store.add_emitter_world(0, 0, 0, 200);
    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    assert_eq!(lg.block_light[lg.idx(0, 0, 0)], 200);
    // Neighbor macro +X should get 200-16 = 184 via micro propagation
    assert_eq!(lg.block_light[lg.idx(1, 0, 0)], 184);

    // Removing the emitter clears the light next recompute
    store.remove_emitter_world(0, 0, 0);
    let lg_off = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    assert_eq!(lg_off.block_light[lg_off.idx(0, 0, 0)], 0);
    assert_eq!(lg_off.block_light[lg_off.idx(1, 0, 0)], 0);
}

#[test]
fn lightingstore_clear_chunk_and_all_borders() {
    let store = LightingStore::new(2, 2, 2);
    // Seed (-1,0) coarse borders so (0,0) has -X neighbor planes
    let mut b = LightBorders::new(2, 2, 2);
    b.xp = vec![50; 4].into();
    store.update_borders(ChunkCoord::new(-1, 0, 0), b);
    // Seed micro borders at (-1,0)
    let mb = MicroBorders {
        xm_sk_neg: vec![1; 4].into(),
        xm_sk_pos: vec![2; 4].into(),
        ym_sk_neg: vec![3; 8].into(),
        ym_sk_pos: vec![4; 8].into(),
        zm_sk_neg: vec![5; 4].into(),
        zm_sk_pos: vec![6; 4].into(),
        xm_bl_neg: vec![7; 4].into(),
        xm_bl_pos: vec![8; 4].into(),
        ym_bl_neg: vec![9; 8].into(),
        ym_bl_pos: vec![10; 8].into(),
        zm_bl_neg: vec![11; 4].into(),
        zm_bl_pos: vec![12; 4].into(),
        xm: 4,
        ym: 4,
        zm: 4,
    };
    store.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb);
    // Add an emitter in chunk (-1,0)
    store.add_emitter_world(-1, 0, 0, 100);
    // Verify present
    assert!(
        store
            .get_neighbor_borders(ChunkCoord::new(0, 0, 0))
            .xn
            .is_some()
    );
    assert!(
        store
            .get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0))
            .xm_sk_neg
            .is_some()
    );
    assert!(
        !store
            .emitters_for_chunk(ChunkCoord::new(-1, 0, 0))
            .is_empty()
    );

    // Clear that chunk only
    store.clear_chunk(ChunkCoord::new(-1, 0, 0));
    let nb = store.get_neighbor_borders(ChunkCoord::new(0, 0, 0));
    assert!(nb.xn.is_none());
    let nbm = store.get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0));
    assert!(nbm.xm_sk_neg.is_none());
    assert!(
        store
            .emitters_for_chunk(ChunkCoord::new(-1, 0, 0))
            .is_empty()
    );

    // Repopulate borders at multiple neighbors and then clear all borders only
    let mut b2 = LightBorders::new(2, 2, 2);
    b2.zp = vec![77; 4].into();
    store.update_borders(ChunkCoord::new(0, 0, -1), b2); // provides zn to (0,0)
    assert!(
        store
            .get_neighbor_borders(ChunkCoord::new(0, 0, 0))
            .zn
            .is_some()
    );
    store.clear_all_borders();
    let nb_after = store.get_neighbor_borders(ChunkCoord::new(0, 0, 0));
    assert!(nb_after.zn.is_none());
}

#[test]
fn atlas_border_rings_match_neighbors() {
    // Build a tiny grid and explicit neighbor planes; verify atlas rings match exactly.
    let (sx, sy, sz) = (3usize, 2usize, 4usize);
    let mut lg = LightGrid::new(sx, sy, sz);
    // Interior doesn't matter for ring checks; leave zeros.
    // Compose NeighborBorders with distinct patterns per plane and channel
    let mut nb = NeighborBorders::empty(sx, sy, sz);
    let mut xn_blk = vec![0u8; sy * sz];
    let mut xn_sky = vec![0u8; sy * sz];
    let mut xn_bcn = vec![0u8; sy * sz];
    let mut xp_blk = vec![0u8; sy * sz];
    let mut xp_sky = vec![0u8; sy * sz];
    let mut xp_bcn = vec![0u8; sy * sz];
    let mut zn_blk = vec![0u8; sy * sx];
    let mut zn_sky = vec![0u8; sy * sx];
    let mut zn_bcn = vec![0u8; sy * sx];
    let mut zp_blk = vec![0u8; sy * sx];
    let mut zp_sky = vec![0u8; sy * sx];
    let mut zp_bcn = vec![0u8; sy * sx];
    let mut yn_blk = vec![0u8; sx * sz];
    let mut yn_sky = vec![0u8; sx * sz];
    let mut yn_bcn = vec![0u8; sx * sz];
    let mut yp_blk = vec![0u8; sx * sz];
    let mut yp_sky = vec![0u8; sx * sz];
    let mut yp_bcn = vec![0u8; sx * sz];
    // Fill plane patterns
    for y in 0..sy {
        for z in 0..sz {
            let ii = y * sz + z;
            xn_blk[ii] = 10 + (y as u8) * 3 + (z as u8);
            xn_sky[ii] = 20 + (y as u8) * 3 + (z as u8);
            xn_bcn[ii] = 30 + (y as u8) * 3 + (z as u8);
            xp_blk[ii] = 40 + (y as u8) * 3 + (z as u8);
            xp_sky[ii] = 50 + (y as u8) * 3 + (z as u8);
            xp_bcn[ii] = 60 + (y as u8) * 3 + (z as u8);
        }
        for x in 0..sx {
            let ii = y * sx + x;
            zn_blk[ii] = 70 + (y as u8) * 5 + (x as u8);
            zn_sky[ii] = 80 + (y as u8) * 5 + (x as u8);
            zn_bcn[ii] = 90 + (y as u8) * 5 + (x as u8);
            zp_blk[ii] = 100 + (y as u8) * 5 + (x as u8);
            zp_sky[ii] = 110 + (y as u8) * 5 + (x as u8);
            zp_bcn[ii] = 120 + (y as u8) * 5 + (x as u8);
        }
    }
    for z in 0..sz {
        for x in 0..sx {
            let ii = z * sx + x;
            yn_blk[ii] = 130 + (z as u8) * 5 + (x as u8);
            yn_sky[ii] = 140 + (z as u8) * 5 + (x as u8);
            yn_bcn[ii] = 150 + (z as u8) * 5 + (x as u8);
            yp_blk[ii] = 160 + (z as u8) * 5 + (x as u8);
            yp_sky[ii] = 170 + (z as u8) * 5 + (x as u8);
            yp_bcn[ii] = 180 + (z as u8) * 5 + (x as u8);
        }
    }
    nb.xn = Some(xn_blk.into());
    nb.sk_xn = Some(xn_sky.into());
    nb.bcn_xn = Some(xn_bcn.into());
    nb.xp = Some(xp_blk.into());
    nb.sk_xp = Some(xp_sky.into());
    nb.bcn_xp = Some(xp_bcn.into());
    nb.zn = Some(zn_blk.into());
    nb.sk_zn = Some(zn_sky.into());
    nb.bcn_zn = Some(zn_bcn.into());
    nb.zp = Some(zp_blk.into());
    nb.sk_zp = Some(zp_sky.into());
    nb.bcn_zp = Some(zp_bcn.into());
    nb.yn = Some(yn_blk.into());
    nb.sk_yn = Some(yn_sky.into());
    nb.bcn_yn = Some(yn_bcn.into());
    nb.yp = Some(yp_blk.into());
    nb.sk_yp = Some(yp_sky.into());
    nb.bcn_yp = Some(yp_bcn.into());
    let atlas = super::pack_light_grid_atlas_with_neighbors(&lg, &nb);
    let width = atlas.width;
    let grid_cols = atlas.grid_cols;
    let tile_w = atlas.sx; // sx + 2
    let tile_h = atlas.sz; // sz + 2
    let at = |x: usize, y: usize| -> (u8, u8, u8) {
        let di = (y * width + x) * 4;
        (atlas.data[di + 0], atlas.data[di + 1], atlas.data[di + 2])
    };
    let total_slices = atlas.sy;
    let inner_sy = total_slices.saturating_sub(2);
    for slice in 0..total_slices {
        let tx = slice % grid_cols;
        let ty = slice / grid_cols;
        let ox = tx * tile_w;
        let oy = ty * tile_h;
        if slice == 0 {
            for z in 0..sz {
                for x in 0..sx {
                    let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                    let ii = z * sx + x;
                    assert_eq!(
                        r,
                        nb.yn.as_ref().unwrap()[ii],
                        "yn.r mismatch z={} x={}",
                        z,
                        x
                    );
                    assert_eq!(
                        g,
                        nb.sk_yn.as_ref().unwrap()[ii],
                        "yn.g mismatch z={} x={}",
                        z,
                        x
                    );
                    assert_eq!(
                        b,
                        nb.bcn_yn.as_ref().unwrap()[ii],
                        "yn.b mismatch z={} x={}",
                        z,
                        x
                    );
                }
            }
            continue;
        }
        if slice == inner_sy + 1 {
            for z in 0..sz {
                for x in 0..sx {
                    let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                    let ii = z * sx + x;
                    assert_eq!(
                        r,
                        nb.yp.as_ref().unwrap()[ii],
                        "yp.r mismatch z={} x={}",
                        z,
                        x
                    );
                    assert_eq!(
                        g,
                        nb.sk_yp.as_ref().unwrap()[ii],
                        "yp.g mismatch z={} x={}",
                        z,
                        x
                    );
                    assert_eq!(
                        b,
                        nb.bcn_yp.as_ref().unwrap()[ii],
                        "yp.b mismatch z={} x={}",
                        z,
                        x
                    );
                }
            }
            continue;
        }
        let y = slice - 1;
        for z in 0..sz {
            let (r, g, b) = at(ox + 0, oy + 1 + z);
            let ii = y * sz + z;
            assert_eq!(
                r,
                nb.xn.as_ref().unwrap()[ii],
                "xn.r mismatch y={} z={}",
                y,
                z
            );
            assert_eq!(
                g,
                nb.sk_xn.as_ref().unwrap()[ii],
                "xn.g mismatch y={} z={}",
                y,
                z
            );
            assert_eq!(
                b,
                nb.bcn_xn.as_ref().unwrap()[ii],
                "xn.b mismatch y={} z={}",
                y,
                z
            );
        }
        for z in 0..sz {
            let (r, g, b) = at(ox + (sx + 1), oy + 1 + z);
            let ii = y * sz + z;
            assert_eq!(
                r,
                nb.xp.as_ref().unwrap()[ii],
                "xp.r mismatch y={} z={}",
                y,
                z
            );
            assert_eq!(
                g,
                nb.sk_xp.as_ref().unwrap()[ii],
                "xp.g mismatch y={} z={}",
                y,
                z
            );
            assert_eq!(
                b,
                nb.bcn_xp.as_ref().unwrap()[ii],
                "xp.b mismatch y={} z={}",
                y,
                z
            );
        }
        for x in 0..sx {
            let (r, g, b) = at(ox + 1 + x, oy + 0);
            let ii = y * sx + x;
            assert_eq!(
                r,
                nb.zn.as_ref().unwrap()[ii],
                "zn.r mismatch y={} x={}",
                y,
                x
            );
            assert_eq!(
                g,
                nb.sk_zn.as_ref().unwrap()[ii],
                "zn.g mismatch y={} x={}",
                y,
                x
            );
            assert_eq!(
                b,
                nb.bcn_zn.as_ref().unwrap()[ii],
                "zn.b mismatch y={} x={}",
                y,
                x
            );
        }
        for x in 0..sx {
            let (r, g, b) = at(ox + 1 + x, oy + (sz + 1));
            let ii = y * sx + x;
            assert_eq!(
                r,
                nb.zp.as_ref().unwrap()[ii],
                "zp.r mismatch y={} x={}",
                y,
                x
            );
            assert_eq!(
                g,
                nb.sk_zp.as_ref().unwrap()[ii],
                "zp.g mismatch y={} x={}",
                y,
                x
            );
            assert_eq!(
                b,
                nb.bcn_zp.as_ref().unwrap()[ii],
                "zp.b mismatch y={} x={}",
                y,
                x
            );
        }
    }
}

#[test]
fn atlas_interior_and_missing_ring_corners() {
    // Verify interior pixels copy LightGrid values, and missing rings/corners remain zero.
    let (sx, sy, sz) = (3usize, 2usize, 2usize);
    let mut lg = LightGrid::new(sx, sy, sz);
    for y in 0..sy {
        for z in 0..sz {
            for x in 0..sx {
                let v = (x as u8) + 10 * (y as u8) + 100 * (z as u8);
                let i = lg.idx(x, y, z);
                lg.block_light[i] = v;
                lg.skylight[i] = v.saturating_add(1);
                lg.beacon_light[i] = v.saturating_add(2);
                lg.beacon_dir[i] = 0;
            }
        }
    }
    // Only provide +X neighbor planes; others are None
    let mut nb = NeighborBorders::empty(sx, sy, sz);
    let mut xp_blk = vec![0u8; sy * sz];
    let mut xp_sky = vec![0u8; sy * sz];
    let mut xp_bcn = vec![0u8; sy * sz];
    for y in 0..sy {
        for z in 0..sz {
            let ii = y * sz + z;
            xp_blk[ii] = 200 + (ii as u8);
            xp_sky[ii] = 150 + (ii as u8);
            xp_bcn[ii] = 50 + (ii as u8);
        }
    }
    nb.xp = Some(xp_blk.into());
    nb.sk_xp = Some(xp_sky.into());
    nb.bcn_xp = Some(xp_bcn.into());

    let atlas = super::pack_light_grid_atlas_with_neighbors(&lg, &nb);
    let width = atlas.width;
    let grid_cols = atlas.grid_cols;
    let tile_w = atlas.sx;
    let tile_h = atlas.sz;
    let at = |x: usize, y: usize| -> (u8, u8, u8) {
        let di = (y * width + x) * 4;
        (atlas.data[di + 0], atlas.data[di + 1], atlas.data[di + 2])
    };
    let total_slices = atlas.sy;
    let inner_sy = total_slices.saturating_sub(2);
    for slice in 0..total_slices {
        let tx = slice % grid_cols;
        let ty = slice / grid_cols;
        let ox = tx * tile_w;
        let oy = ty * tile_h;
        if slice == 0 || slice == inner_sy + 1 {
            for z in 0..sz {
                for x in 0..sx {
                    let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                    assert_eq!(
                        (r, g, b),
                        (0, 0, 0),
                        "Y plane not zero at slice={} z={} x={}",
                        slice,
                        z,
                        x
                    );
                }
            }
            let corners = [
                (ox + 0, oy + 0),
                (ox + (sx + 1), oy + 0),
                (ox + 0, oy + (sz + 1)),
                (ox + (sx + 1), oy + (sz + 1)),
            ];
            for &(cx, cy) in &corners {
                let (r, g, b) = at(cx, cy);
                assert_eq!(
                    (r, g, b),
                    (0, 0, 0),
                    "corner not zero at Y slice {} pos=({}, {})",
                    slice,
                    cx,
                    cy
                );
            }
            continue;
        }
        let y = slice - 1;
        for z in 0..sz {
            for x in 0..sx {
                let (r, g, b) = at(ox + 1 + x, oy + 1 + z);
                let i = lg.idx(x, y, z);
                assert_eq!(
                    (r, g, b),
                    (lg.block_light[i], lg.skylight[i], lg.beacon_light[i]),
                    "interior mismatch at x={},y={},z={}",
                    x,
                    y,
                    z
                );
            }
        }
        for z in 0..sz {
            let (r, g, b) = at(ox + (sx + 1), oy + 1 + z);
            let ii = y * sz + z;
            assert_eq!(r, nb.xp.as_ref().unwrap()[ii]);
            assert_eq!(g, nb.sk_xp.as_ref().unwrap()[ii]);
            assert_eq!(b, nb.bcn_xp.as_ref().unwrap()[ii]);
        }
        for z in 0..sz {
            let (r, g, b) = at(ox + 0, oy + 1 + z);
            assert_eq!((r, g, b), (0, 0, 0), "-X ring not zero at y={}, z={}", y, z);
        }
        for x in 0..sx {
            let (r0, g0, b0) = at(ox + 1 + x, oy + 0);
            let (r1, g1, b1) = at(ox + 1 + x, oy + (sz + 1));
            assert_eq!(
                (r0, g0, b0),
                (0, 0, 0),
                "-Z ring not zero at y={}, x={}",
                y,
                x
            );
            assert_eq!(
                (r1, g1, b1),
                (0, 0, 0),
                "+Z ring not zero at y={}, x={}",
                y,
                x
            );
        }
        let corners = [
            (ox + 0, oy + 0),
            (ox + (sx + 1), oy + 0),
            (ox + 0, oy + (sz + 1)),
            (ox + (sx + 1), oy + (sz + 1)),
        ];
        for &(cx, cy) in &corners {
            let (r, g, b) = at(cx, cy);
            assert_eq!(
                (r, g, b),
                (0, 0, 0),
                "corner not zero at tile origin ({},{})",
                cx,
                cy
            );
        }
    }
}

#[test]
fn seam_symmetry_block_and_sky_z_plus_minus_with_micro_override() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let world = geist_world::World::new(1, 1, 1, 10, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();
    // Roofed air to suppress local skylight seeding
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, y, _| Block {
        id: if y == sy - 1 { stone_id } else { air_id },
        state: 0,
    });
    // -Z coarse neighbors
    let store = LightingStore::new(sx, sy, sz);
    let mut nb = LightBorders::new(sx, sy, sz);
    nb.zp = vec![200; sy * sx].into();
    nb.sk_zp = vec![200; sy * sx].into();
    store.update_borders(ChunkCoord::new(0, 0, -1), nb);
    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    for x in 0..sx {
        assert_eq!(lg.block_light[lg.idx(x, 0, 0)], 168);
        assert_eq!(lg.skylight[lg.idx(x, 0, 0)], 168);
    }
    for x in 0..sx {
        assert_eq!(lg.block_light[lg.idx(x, 0, sz - 1)], 136);
        assert_eq!(lg.skylight[lg.idx(x, 0, sz - 1)], 136);
    }
    // +Z micro neighbors override
    let store2 = LightingStore::new(sx, sy, sz);
    // coarse as well but micro should win
    let mut nb2 = LightBorders::new(sx, sy, sz);
    nb2.zn = vec![200; sy * sx].into();
    nb2.sk_zn = vec![200; sy * sx].into();
    store2.update_borders(ChunkCoord::new(0, 0, 1), nb2);
    // Micro neighbor at +Z: zm_*_neg maps to our zm_*_pos
    let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
    let mb = MicroBorders {
        xm_sk_neg: vec![0; mys * mzs].into(),
        xm_sk_pos: vec![0; mys * mzs].into(),
        ym_sk_neg: vec![0; mzs * mxs].into(),
        ym_sk_pos: vec![0; mzs * mxs].into(),
        zm_sk_neg: vec![200; mys * mxs].into(),
        zm_sk_pos: vec![0; mys * mxs].into(),
        xm_bl_neg: vec![0; mys * mzs].into(),
        xm_bl_pos: vec![0; mys * mzs].into(),
        ym_bl_neg: vec![0; mzs * mxs].into(),
        ym_bl_pos: vec![0; mzs * mxs].into(),
        zm_bl_neg: vec![200; mys * mxs].into(),
        zm_bl_pos: vec![0; mys * mxs].into(),
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    store2.update_micro_borders(ChunkCoord::new(0, 0, 1), mb);
    let lg2 = super::compute_light_with_borders_buf(&buf, &store2, &reg, &world);
    for x in 0..sx {
        assert_eq!(lg2.block_light[lg2.idx(x, 0, sz - 1)], 184);
        assert_eq!(lg2.skylight[lg2.idx(x, 0, sz - 1)], 184);
    }
}

#[test]
fn seam_symmetry_block_and_sky_x_plus_with_micro_override() {
    let reg = make_test_registry();
    let sx = 2;
    let sy = 2;
    let sz = 2;
    let world = geist_world::World::new(1, 1, 1, 11, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, y, _| Block {
        id: if y == sy - 1 { stone_id } else { air_id },
        state: 0,
    });
    // +X coarse neighbors
    let store = LightingStore::new(sx, sy, sz);
    let mut nb = LightBorders::new(sx, sy, sz);
    nb.xn = vec![200; sy * sz].into();
    nb.sk_xn = vec![200; sy * sz].into();
    store.update_borders(ChunkCoord::new(1, 0, 0), nb);
    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    // Only bottom layer (y=0) is air; top is stone and remains dark
    for z in 0..sz {
        assert_eq!(lg.block_light[lg.idx(sx - 1, 0, z)], 168);
        assert_eq!(lg.skylight[lg.idx(sx - 1, 0, z)], 168);
    }
    for z in 0..sz {
        assert_eq!(lg.block_light[lg.idx(0, 0, z)], 136);
        assert_eq!(lg.skylight[lg.idx(0, 0, z)], 136);
    }
    // -X micro neighbors override
    let store2 = LightingStore::new(sx, sy, sz);
    let mut nb2 = LightBorders::new(sx, sy, sz);
    nb2.xp = vec![200; sy * sz].into();
    nb2.sk_xp = vec![200; sy * sz].into();
    store2.update_borders(ChunkCoord::new(-1, 0, 0), nb2);
    let (mxs, mys, mzs) = (sx * 2, sy * 2, sz * 2);
    let mb = MicroBorders {
        xm_sk_neg: vec![0; mys * mzs].into(),
        xm_sk_pos: vec![200; mys * mzs].into(), // maps to our xm_sk_neg
        ym_sk_neg: vec![0; mzs * mxs].into(),
        ym_sk_pos: vec![0; mzs * mxs].into(),
        zm_sk_neg: vec![0; mys * mxs].into(),
        zm_sk_pos: vec![0; mys * mxs].into(),
        xm_bl_neg: vec![0; mys * mzs].into(),
        xm_bl_pos: vec![200; mys * mzs].into(),
        ym_bl_neg: vec![0; mzs * mxs].into(),
        ym_bl_pos: vec![0; mzs * mxs].into(),
        zm_bl_neg: vec![0; mys * mxs].into(),
        zm_bl_pos: vec![0; mys * mxs].into(),
        xm: mxs,
        ym: mys,
        zm: mzs,
    };
    store2.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb);
    let lg2 = super::compute_light_with_borders_buf(&buf, &store2, &reg, &world);
    for z in 0..sz {
        assert_eq!(lg2.block_light[lg2.idx(0, 0, z)], 184);
        assert_eq!(lg2.skylight[lg2.idx(0, 0, z)], 184);
    }
}

#[test]
fn coarse_x_seam_uses_world_space_y() {
    let reg = make_test_registry();
    let sx = 2usize;
    let sy = 2usize;
    let sz = 2usize;
    let world = geist_world::World::new(2, 2, 1, 21, WorldGenMode::Flat { thickness: 2 });
    let air_id = reg.id_by_name("air").unwrap();
    let coord = ChunkCoord::new(0, 1, 0);
    let mut blocks = Vec::with_capacity(sx * sy * sz);
    for _y in 0..sy {
        for _z in 0..sz {
            for _x in 0..sx {
                blocks.push(Block { id: air_id, state: 0 });
            }
        }
    }
    let buf = ChunkBuf::from_blocks_local(coord, sx, sy, sz, blocks);
    let store = LightingStore::new(sx, sy, sz);
    let mut neighbor = LightBorders::new(sx, sy, sz);
    neighbor.xn = vec![200; sy * sz].into();
    store.update_borders(ChunkCoord::new(1, 1, 0), neighbor);

    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    for y in 0..sy {
        for z in 0..sz {
            assert_eq!(lg.block_light[lg.idx(sx - 1, y, z)], 168);
        }
    }
}

#[test]
fn beacons_are_ignored_in_micro_path() {
    let reg = make_test_registry();
    let sx = 1;
    let sy = 1;
    let sz = 1;
    let world = geist_world::World::new(1, 1, 1, 6, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let buf = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });
    let store = LightingStore::new(sx, sy, sz);
    store.add_beacon_world(0, 0, 0, 200);
    let lg = super::compute_light_with_borders_buf(&buf, &store, &reg, &world);
    assert_eq!(lg.beacon_light[lg.idx(0, 0, 0)], 0);
}

#[test]
fn sample_face_local_s2_uses_neighbor_micro_planes() {
    let reg = make_test_registry();
    let sx = 1;
    let sy = 1;
    let sz = 1;
    let world = geist_world::World::new(2, 1, 1, 9, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();

    // Compute neighbor (1,0) first so its micro planes are available
    let buf_nb = make_chunk_buf_with(&reg, 1, 0, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });
    let store = LightingStore::new(sx, sy, sz);
    let _lg_nb = super::compute_light_with_borders_buf(&buf_nb, &store, &reg, &world);

    // Now current (0,0) is stone (no local micro skylight)
    let buf_me = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: stone_id,
        state: 0,
    });
    let lg_me = super::compute_light_with_borders_buf(&buf_me, &store, &reg, &world);
    // Sampling +X across seam should use neighbor micro skylight (255)
    let val = lg_me.sample_face_local_s2(&buf_me, &reg, 0, 0, 0, 2);
    assert_eq!(val, 255);
}

#[test]
fn skylight_transparent_s2_gates_full_cube_vs_micro_occ() {
    let reg = make_test_registry();
    let air = Block {
        id: reg.id_by_name("air").unwrap(),
        state: 0,
    };
    let stone = Block {
        id: reg.id_by_name("stone").unwrap(),
        state: 0,
    };
    let slab = Block {
        id: reg.id_by_name("slab").unwrap(),
        state: 0,
    };
    assert!(super::skylight_transparent_s2(air, &reg));
    assert!(!super::skylight_transparent_s2(stone, &reg));
    // Slab has micro occupancy, which should be skylight transparent for BFS
    assert!(super::skylight_transparent_s2(slab, &reg));
}

#[test]
fn skylight_transparent_s2_dynamic_shapes() {
    let reg = make_test_registry();
    let air = Block {
        id: reg.id_by_name("air").unwrap(),
        state: 0,
    };
    let fence = Block {
        id: reg.id_by_name("fence").unwrap(),
        state: 0,
    };
    assert!(super::skylight_transparent_s2(air, &reg));
    assert!(super::skylight_transparent_s2(fence, &reg));
}

#[test]
fn neighbor_light_max_y_fallback_uses_boundary_value() {
    let mut lg = LightGrid::new(2, 2, 2);
    // Put value at +Y boundary cell
    // For Y faces, there are no coarse neighbor planes; implementation returns 0
    let v = 70u8;
    let idx = lg.idx(1, lg.sy - 1, 1);
    lg.block_light[idx] = v;
    assert_eq!(lg.neighbor_light_max(1, lg.sy - 1, 1, 0), 0);
}

#[test]
fn sample_face_local_s2_uses_neighbor_micro_planes_z() {
    let reg = make_test_registry();
    let sx = 1;
    let sy = 1;
    let sz = 1;
    let world = geist_world::World::new(1, 1, 2, 12, WorldGenMode::Flat { thickness: 0 });
    let air_id = reg.id_by_name("air").unwrap();
    let stone_id = reg.id_by_name("stone").unwrap();
    // Compute neighbor (0,1) first so its micro planes are available
    let buf_nb = make_chunk_buf_with(&reg, 0, 1, sx, sy, sz, &|_, _, _| Block {
        id: air_id,
        state: 0,
    });
    let store = LightingStore::new(sx, sy, sz);
    let _lg_nb = super::compute_light_with_borders_buf(&buf_nb, &store, &reg, &world);
    // Now current (0,0) is stone
    let buf_me = make_chunk_buf_with(&reg, 0, 0, sx, sy, sz, &|_, _, _| Block {
        id: stone_id,
        state: 0,
    });
    let lg_me = super::compute_light_with_borders_buf(&buf_me, &store, &reg, &world);
    // Sampling +Z across seam should use neighbor micro skylight (255)
    let val = lg_me.sample_face_local_s2(&buf_me, &reg, 0, 0, 0, 4);
    assert_eq!(val, 255);
}

#[test]
fn clear_all_borders_does_not_clear_micro_planes() {
    let store = LightingStore::new(2, 1, 2);
    // Seed micro neighbor at (-1,0)
    let mb = MicroBorders {
        xm_sk_neg: vec![1; 2 * 4].into(),
        xm_sk_pos: vec![2; 2 * 4].into(),
        ym_sk_neg: vec![3; 4 * 4].into(),
        ym_sk_pos: vec![4; 4 * 4].into(),
        zm_sk_neg: vec![5; 2 * 4].into(),
        zm_sk_pos: vec![6; 2 * 4].into(),
        xm_bl_neg: vec![7; 2 * 4].into(),
        xm_bl_pos: vec![8; 2 * 4].into(),
        ym_bl_neg: vec![9; 4 * 4].into(),
        ym_bl_pos: vec![10; 4 * 4].into(),
        zm_bl_neg: vec![11; 2 * 4].into(),
        zm_bl_pos: vec![12; 2 * 4].into(),
        xm: 4,
        ym: 2,
        zm: 4,
    };
    store.update_micro_borders(ChunkCoord::new(-1, 0, 0), mb);
    assert!(
        store
            .get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0))
            .xm_sk_neg
            .is_some()
    );
    // Add coarse borders and then clear them
    let mut b = LightBorders::new(2, 1, 2);
    b.xp = vec![1; 2].into();
    store.update_borders(ChunkCoord::new(-1, 0, 0), b);
    assert!(
        store
            .get_neighbor_borders(ChunkCoord::new(0, 0, 0))
            .xn
            .is_some()
    );
    store.clear_all_borders();
    assert!(
        store
            .get_neighbor_borders(ChunkCoord::new(0, 0, 0))
            .xn
            .is_none()
    );
    // Micro planes still present
    assert!(
        store
            .get_neighbor_micro_borders(ChunkCoord::new(0, 0, 0))
            .xm_sk_neg
            .is_some()
    );
}

#[test]
fn can_cross_face_s2_basic_blocking_and_open() {
    use geist_blocks::config::{SeamPolicyCfg, SeamPolicySimple};
    let reg = {
        let materials = MaterialCatalog::new();
        let mut blocks = vec![
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
                name: "slab".into(),
                id: Some(3),
                solid: Some(true),
                blocks_skylight: Some(false),
                propagates_light: Some(true),
                emission: Some(0),
                light_profile: None,
                light: None,
                shape: Some(ShapeConfig::Simple("slab".into())),
                materials: None,
                state_schema: None,
                seam: None,
            },
            // Slab with dont_occlude_same: should permit face openness when both sides are the same
            BlockDef {
                name: "slab_same".into(),
                id: Some(5),
                solid: Some(true),
                blocks_skylight: Some(false),
                propagates_light: Some(true),
                emission: Some(0),
                light_profile: None,
                light: None,
                shape: Some(ShapeConfig::Simple("slab".into())),
                materials: None,
                state_schema: None,
                seam: Some(SeamPolicyCfg::Simple(SeamPolicySimple::DontOccludeSame)),
            },
        ];
        BlockRegistry::from_configs(
            materials,
            BlocksConfig {
                blocks: blocks.drain(..).collect(),
                lighting: None,
                unknown_block: Some("unknown".into()),
            },
        )
        .unwrap()
    };
    let slab_id = reg.id_by_name("slab").unwrap();
    let slab_same_id = reg.id_by_name("slab_same").unwrap();
    // 2x2x1 chunk; we’ll test +X faces between x=0 and x=1
    let buf_slab_air = make_chunk_buf_with(&reg, 0, 0, 2, 2, 1, &|x, y, _| Block {
        id: if x == 0 { slab_id } else { 0 },
        state: 0,
    });
    // From x=0 slab to x=1 air: some micro face cells open => can cross
    assert!(super::can_cross_face_s2(&buf_slab_air, &reg, 0, 0, 0, 2));
    // Slab to slab with DontOccludeSame across +X should be considered open (ignores neighbor solid on same type)
    let buf_same = make_chunk_buf_with(&reg, 0, 0, 2, 2, 1, &|_, _, _| Block {
        id: slab_same_id,
        state: 0,
    });
    assert!(super::can_cross_face_s2(&buf_same, &reg, 0, 0, 0, 2));
    // Stone to stone (full cubes) is blocked
    let stone_id = reg.id_by_name("stone").unwrap();
    let buf_stone = make_chunk_buf_with(&reg, 0, 0, 2, 2, 1, &|_, _, _| Block {
        id: stone_id,
        state: 0,
    });
    assert!(!super::can_cross_face_s2(&buf_stone, &reg, 0, 0, 0, 2));
}
