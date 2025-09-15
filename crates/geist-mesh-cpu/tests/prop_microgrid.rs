use geist_mesh_cpu::microgrid_tables::{empty4_to_rects, occ8_to_boxes};
use proptest::prelude::*;

proptest! {
    // The union of boxes covers exactly the occupied 2x2 cells per y-layer with no overlap.
    #[test]
    fn occ8_boxes_cover_exact_mask(occ in any::<u8>()) {
        let boxes = occ8_to_boxes(occ);
        // For each layer y=0,1, build a 2x2 coverage grid
        for y in 0..2u8 {
            let mut cov = [[0u8;2];2]; // counts per cell [z][x]
            for b in boxes {
                let (x0,y0,z0,x1,_y1,z1) = (b[0],b[1],b[2],b[3],b[4],b[5]);
                if y0 != y { continue; }
                for z in z0..z1 { for x in x0..x1 { if z<2 && x<2 {
                    cov[z as usize][x as usize] += 1;
                }}}
            }
            for z in 0..2u8 { for x in 0..2u8 {
                let bit = 1u8 << (((y & 1) << 2) | ((z & 1) << 1) | (x & 1));
                let want = if (occ & bit) != 0 { 1 } else { 0 };
                // Each true cell covered exactly once; false cells uncovered
                prop_assert_eq!(cov[z as usize][x as usize], want);
            }}
        }
    }

    // The union of rects covers exactly the set bits in 2x2 plane with no overlap.
    #[test]
    fn empty4_rects_cover_exact_mask(mask in any::<u8>()) {
        let m = mask & 0x0F; // only 4 bits used
        let rects = empty4_to_rects(m);
        let mut cov = [[0u8;2];2]; // [v][u]
        for r in rects {
            let (u0,v0,du,dv) = (r[0],r[1],r[2],r[3]);
            for v in v0..(v0+dv) { for u in u0..(u0+du) { if v<2 && u<2 {
                cov[v as usize][u as usize] += 1;
            }}}
        }
        for v in 0..2u8 { for u in 0..2u8 {
            let bit = 1u8 << ((v<<1)|u);
            let want = if (m & bit) != 0 { 1 } else { 0 };
            prop_assert_eq!(cov[v as usize][u as usize], want);
        }}
    }
}
