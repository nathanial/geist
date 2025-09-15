use geist_blocks::types::Block;
use geist_chunk::ChunkBuf;
use proptest::prelude::*;

fn dim() -> impl Strategy<Value = usize> {
    1usize..=8
}

fn small_i32() -> impl Strategy<Value = i32> {
    -1_000_000i32..=1_000_000
}

proptest! {
    // idx maps each (x,y,z) within bounds to unique in-range indices
    #[test]
    fn idx_is_unique_and_in_range(cx in small_i32(), cz in small_i32(), sx in dim(), sy in dim(), sz in dim()) {
        let expect = sx*sy*sz;
        let blocks = vec![Block::AIR; expect];
        let buf = ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks);

        let mut seen = vec![false; expect];
        for y in 0..sy { for z in 0..sz { for x in 0..sx {
            let i = buf.idx(x,y,z);
            prop_assert!(i < expect);
            prop_assert!(!seen[i]);
            seen[i] = true;
        }}}
        // All indices hit exactly once
        prop_assert!(seen.into_iter().all(|b| b));
    }

    // get_local reads from linearized storage at idx
    #[test]
    fn get_local_matches_linear(cx in small_i32(), cz in small_i32(), sx in dim(), sy in dim(), sz in dim()) {
        let expect = sx*sy*sz;
        let blocks = (0..expect).map(|i| Block { id: i as u16, state: ! (i as u16)}).collect();
        let buf = ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks);
        for y in 0..sy { for z in 0..sz { for x in 0..sx {
            let i = buf.idx(x,y,z);
            prop_assert_eq!(buf.get_local(x,y,z), buf.blocks[i]);
        }}}
    }

    // contains_world matches spec and aligns with get_world
    #[test]
    fn contains_world_and_get_world_agree(cx in small_i32(), cz in small_i32(), sx in dim(), sy in dim(), sz in dim()) {
        let expect = sx*sy*sz;
        let blocks = (0..expect).map(|i| Block { id: (i%65535) as u16, state: (i*31 % 65535) as u16}).collect();
        let buf = ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, blocks);

        let x0 = cx * sx as i32;
        let z0 = cz * sz as i32;

        // Sample a mix of inside/outside positions
        let candidates = vec![
            (x0,               0,                z0),
            (x0 + sx as i32-1, sy as i32-1,     z0 + sz as i32-1),
            (x0 - 1,           0,                z0),
            (x0 + sx as i32,   0,                z0),
            (x0,              -1,                z0),
            (x0,               sy as i32,        z0),
            (x0,               0,                z0 - 1),
            (x0,               0,                z0 + sz as i32),
        ];

        for (wx,wy,wz) in candidates {
            let spec = wy >= 0 && wy < sy as i32 && wx >= x0 && wx < x0 + sx as i32 && wz >= z0 && wz < z0 + sz as i32;
            prop_assert_eq!(buf.contains_world(wx,wy,wz), spec);
            match buf.get_world(wx,wy,wz) {
                None => prop_assert!(!spec),
                Some(b) => {
                    prop_assert!(spec);
                    let lx = (wx - x0) as usize; let ly = wy as usize; let lz = (wz - z0) as usize;
                    prop_assert_eq!(b, buf.get_local(lx, ly, lz));
                }
            }
        }
    }

    // from_blocks_local resizes or preserves to exact length
    #[test]
    fn from_blocks_local_resizes(cx in small_i32(), cz in small_i32(), sx in dim(), sy in dim(), sz in dim()) {
        let expect = sx*sy*sz;
        // exact length preserved
        let buf_ok = ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, vec![Block::AIR; expect]);
        prop_assert_eq!(buf_ok.blocks.len(), expect);
        // wrong length resized to expected
        let wrong_len = expect.saturating_sub(1);
        let buf_resized = ChunkBuf::from_blocks_local(cx, cz, sx, sy, sz, vec![Block::AIR; wrong_len]);
        prop_assert_eq!(buf_resized.blocks.len(), expect);
    }
}
