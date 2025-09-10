use geist_geom::Vec3;
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

fn approx_zero_scaled(val: f32, scale: f32, atol: f32, rtol: f32) -> bool {
    val.abs() <= atol + rtol * scale
}

fn bounded_f32() -> impl Strategy<Value = f32> {
    NORMAL.prop_filter("bounded", |v| v.abs() <= 1e6)
}

fn bounded_nonzero_f32() -> impl Strategy<Value = f32> {
    NORMAL.prop_filter("bounded_nonzero", |v| {
        let a = v.abs();
        a >= 1e-6 && a <= 1e6
    })
}

fn any_vec3() -> impl Strategy<Value = Vec3> {
    (bounded_f32(), bounded_f32(), bounded_f32())
        .prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn nondegenerate_vec3() -> impl Strategy<Value = Vec3> {
    (bounded_nonzero_f32(), bounded_nonzero_f32(), bounded_nonzero_f32())
        .prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

proptest! {
    // Addition commutativity: a + b == b + a (element-wise)
    #[test]
    fn vec3_add_commutative(
        a in any_vec3(),
        b in any_vec3(),
    ) {
        prop_assert!(vapprox(a + b, b + a, 1e-5));
    }

    // Distributive property of dot over addition: (a + b)·c = a·c + b·c
    #[test]
    fn vec3_dot_distributive(
        a in any_vec3(),
        b in any_vec3(),
        c in any_vec3(),
    ) {
        let left = (a + b).dot(c);
        let right = a.dot(c) + b.dot(c);
        prop_assert!(approx_abs_rel(left, right, 1e-6, 1e-5));
    }

    // Cross orthogonality: a·(a×b) = 0 and b·(a×b) = 0
    #[test]
    fn vec3_cross_orthogonal(
        a in nondegenerate_vec3(),
        b in nondegenerate_vec3(),
    ) {
        let c = a.cross(b);
        let scale_a = a.length() * c.length();
        let scale_b = b.length() * c.length();
        prop_assert!(approx_zero_scaled(a.dot(c), scale_a, 1e-6, 1e-5));
        prop_assert!(approx_zero_scaled(b.dot(c), scale_b, 1e-6, 1e-5));
    }

    // Cross anti-commutativity: a×b = -(b×a)  -> a×b + b×a ≈ 0
    #[test]
    fn vec3_cross_anticommutative(
        a in any_vec3(),
        b in any_vec3(),
    ) {
        let sum = a.cross(b) + b.cross(a);
        prop_assert!(vapprox(sum, Vec3::ZERO, 1e-3));
    }

    // Normalized length: |normalize(v)| = 1 for non-zero, else unchanged for zero vector
    #[test]
    fn vec3_normalized_length(
        v in nondegenerate_vec3(),
    ) {
        let len = v.length();
        let n = v.normalized();
        if len > 0.0 {
            prop_assert!(approx(n.length(), 1.0, 1e-3));
        } else {
            prop_assert!(vapprox(n, v, 1e-6));
        }
    }

    // Scalar roundtrip: (a * k) / k == a for k != 0
    #[test]
    fn vec3_scalar_roundtrip(
        a in any_vec3(),
        k in bounded_nonzero_f32(),
    ) {
        prop_assume!(k != 0.0);
        let r = (a * k) / k;
        prop_assert!(vapprox_abs_rel(r, a, 1e-6, 1e-5));
    }
}
