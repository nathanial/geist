use geist_geom::{Aabb, Vec3};
use proptest::prelude::*;
use proptest::num::f32::NORMAL;
use proptest::strategy::Strategy;

fn approx(a: f32, b: f32, eps: f32) -> bool { (a - b).abs() <= eps }
fn vapprox(a: Vec3, b: Vec3, eps: f32) -> bool {
    approx(a.x, b.x, eps) && approx(a.y, b.y, eps) && approx(a.z, b.z, eps)
}
fn approx_abs_rel(a: f32, b: f32, atol: f32, rtol: f32) -> bool {
    let diff = (a - b).abs();
    let scale = a.abs().max(b.abs());
    diff <= atol + rtol * scale
}
fn vapprox_abs_rel(a: Vec3, b: Vec3, atol: f32, rtol: f32) -> bool {
    approx_abs_rel(a.x, b.x, atol, rtol)
        && approx_abs_rel(a.y, b.y, atol, rtol)
        && approx_abs_rel(a.z, b.z, atol, rtol)
}

fn bounded_f32() -> impl Strategy<Value = f32> {
    NORMAL.prop_filter("bounded", |v| v.is_finite() && v.abs() <= 1e6)
}
fn arb_vec3() -> impl Strategy<Value = Vec3> {
    (bounded_f32(), bounded_f32(), bounded_f32())
        .prop_map(|(x, y, z)| Vec3::new(x, y, z))
}
fn arb_aabb() -> impl Strategy<Value = Aabb> {
    (arb_vec3(), arb_vec3())
        .prop_map(|(min, max)| Aabb::new(min, max))
}

fn small_f32() -> impl Strategy<Value = f32> {
    bounded_f32().prop_map(|v| v % 1_000.0)
}

fn small_vec3() -> impl Strategy<Value = Vec3> {
    (small_f32(), small_f32(), small_f32())
        .prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn arb_nondegenerate_aabb() -> impl Strategy<Value = Aabb> {
    (arb_vec3(), arb_vec3())
        .prop_filter("extent not tiny", |(min, max)| {
            let e = *max - *min;
            e.x.abs() >= 1e-3 && e.y.abs() >= 1e-3 && e.z.abs() >= 1e-3
        })
        .prop_map(|(min, max)| Aabb::new(min, max))
}

proptest! {
    // Midpoint translates with the box
    #[test]
    fn aabb_midpoint_translation(a in arb_aabb(), t in arb_vec3()) {
        let m = (a.min + a.max) / 2.0;
        let b = Aabb::new(a.min + t, a.max + t);
        let m2 = (b.min + b.max) / 2.0;
        prop_assert!(vapprox_abs_rel(m2, m + t, 1e-6, 1e-5));
    }

    // Extents scale linearly with uniform scalar multiplication
    #[test]
    fn aabb_extents_scale_linear(a in arb_aabb(), k in bounded_f32()) {
        let e = a.max - a.min;
        let b = Aabb::new(a.min * k, a.max * k);
        let e2 = b.max - b.min;
        prop_assert!(vapprox_abs_rel(e2, e * k, 1e-6, 1e-5));
    }

    // Swapping min/max twice returns original
    #[test]
    fn aabb_swap_twice_identity(a in arb_aabb()) {
        let b = Aabb::new(a.max, a.min);
        let c = Aabb::new(b.max, b.min);
        prop_assert_eq!(a, c);
    }

    // Extents symmetry: (max - min) == - (min - max)
    #[test]
    fn aabb_extents_symmetry(a in arb_aabb()) {
        let e1 = a.max - a.min;
        let e2 = a.min - a.max;
        prop_assert!(vapprox(e1, e2 * -1.0, 1e-6));
    }
}
