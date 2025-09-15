use geist_lighting::{LightBorders, LightingStore};
use proptest::prelude::*;

fn dims() -> impl Strategy<Value = (usize, usize, usize)> {
    (1usize..=3, 1usize..=3, 1usize..=3)
}

fn planes_for_dims(
    sx: usize,
    sy: usize,
    sz: usize,
) -> impl Strategy<Value = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> {
    let a = prop::collection::vec(any::<u8>(), sy * sz);
    let b = prop::collection::vec(any::<u8>(), sy * sz);
    let c = prop::collection::vec(any::<u8>(), sy * sx);
    let d = prop::collection::vec(any::<u8>(), sy * sx);
    let e = prop::collection::vec(any::<u8>(), sx * sz);
    let f = prop::collection::vec(any::<u8>(), sx * sz);
    (a, b, c, d, e, f)
}

proptest! {
    // Neighbor mapping: store maps neighbor planes to the opposite faces
    #[test]
    fn neighbor_borders_mapping(((sx,sy,sz), (xn,xp,zn,zp,yn,yp)) in dims().prop_flat_map(|d| {
        let (sx,sy,sz) = d;
        planes_for_dims(sx,sy,sz).prop_map(move |p| (d, p))
    })) {
        let store = LightingStore::new(sx, sy, sz);

        // Left neighbor provides its +X to our -X
        let mut left = LightBorders::new(sx, sy, sz);
        left.xp = xn.clone().into();
        prop_assert!(store.update_borders(-1, 0, left));

        // Right neighbor provides its -X to our +X
        let mut right = LightBorders::new(sx, sy, sz);
        right.xn = xp.clone().into();
        prop_assert!(store.update_borders(1, 0, right));

        // Front neighbor (negative Z) provides its +Z to our -Z
        let mut front = LightBorders::new(sx, sy, sz);
        front.zp = zn.clone().into();
        prop_assert!(store.update_borders(0, -1, front));

        // Back neighbor (positive Z) provides its -Z to our +Z
        let mut back = LightBorders::new(sx, sy, sz);
        back.zn = zp.clone().into();
        prop_assert!(store.update_borders(0, 1, back));

        // No vertical neighbors managed; skip yn/yp mapping

        let nb = store.get_neighbor_borders(0, 0);
        prop_assert_eq!(nb.xn.as_ref().unwrap().as_ref(), &xn[..]);
        prop_assert_eq!(nb.xp.as_ref().unwrap().as_ref(), &xp[..]);
        prop_assert_eq!(nb.zn.as_ref().unwrap().as_ref(), &zn[..]);
        prop_assert_eq!(nb.zp.as_ref().unwrap().as_ref(), &zp[..]);
    }
}
