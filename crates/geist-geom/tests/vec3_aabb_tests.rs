use geist_geom::{Aabb, Vec3};

fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn vec3_approx_eq(a: Vec3, b: Vec3, eps: f32) -> bool {
    approx_eq(a.x, b.x, eps) && approx_eq(a.y, b.y, eps) && approx_eq(a.z, b.z, eps)
}

#[test]
fn vec3_constants() {
    assert!(vec3_approx_eq(Vec3::ZERO, Vec3::new(0.0, 0.0, 0.0), 1e-6));
    assert!(vec3_approx_eq(Vec3::UP, Vec3::new(0.0, 1.0, 0.0), 1e-6));
}

#[test]
fn vec3_add_sub() {
    let a = Vec3::new(1.0, 2.0, 3.0);
    let b = Vec3::new(-4.0, 5.0, -6.0);
    let c = a + b;
    assert!(vec3_approx_eq(c, Vec3::new(-3.0, 7.0, -3.0), 1e-6));

    let d = c - a;
    assert!(vec3_approx_eq(d, b, 1e-6));
}

#[test]
fn vec3_add_assign_sub_assign() {
    let mut v = Vec3::new(1.0, 1.0, 1.0);
    v += Vec3::new(2.0, 3.0, 4.0);
    assert!(vec3_approx_eq(v, Vec3::new(3.0, 4.0, 5.0), 1e-6));

    v -= Vec3::new(1.0, 2.0, 3.0);
    assert!(vec3_approx_eq(v, Vec3::new(2.0, 2.0, 2.0), 1e-6));
}

#[test]
fn vec3_scalar_mul_div() {
    let v = Vec3::new(1.5, -2.0, 4.0);
    let m = v * 2.0;
    assert!(vec3_approx_eq(m, Vec3::new(3.0, -4.0, 8.0), 1e-6));

    let d = m / 2.0;
    assert!(vec3_approx_eq(d, v, 1e-6));
}

#[test]
fn vec3_dot_length_normalized() {
    let v = Vec3::new(3.0, 4.0, 0.0);
    assert!(approx_eq(v.dot(v), 25.0, 1e-6));
    assert!(approx_eq(v.length(), 5.0, 1e-6));

    let n = v.normalized();
    assert!(approx_eq(n.length(), 1.0, 1e-6));
    assert!(vec3_approx_eq(n, Vec3::new(0.6, 0.8, 0.0), 1e-6));

    // Zero vector normalization should be a no-op (not NaN, unchanged)
    let z = Vec3::ZERO;
    let zn = z.normalized();
    assert!(vec3_approx_eq(zn, Vec3::ZERO, 1e-6));
    assert!(approx_eq(zn.length(), 0.0, 1e-6));
}

#[test]
fn vec3_cross_properties() {
    let i = Vec3::new(1.0, 0.0, 0.0);
    let j = Vec3::new(0.0, 1.0, 0.0);
    let k = Vec3::new(0.0, 0.0, 1.0);

    // Basis cross products
    assert!(vec3_approx_eq(i.cross(j), k, 1e-6));
    assert!(vec3_approx_eq(j.cross(k), i, 1e-6));
    assert!(vec3_approx_eq(k.cross(i), j, 1e-6));

    // Cross result is orthogonal to both inputs
    let a = Vec3::new(2.0, -1.0, 3.0);
    let b = Vec3::new(-4.0, 0.5, 1.0);
    let c = a.cross(b);
    assert!(approx_eq(a.dot(c), 0.0, 1e-6));
    assert!(approx_eq(b.dot(c), 0.0, 1e-6));
}

#[test]
fn aabb_new() {
    let min = Vec3::new(-1.0, 0.0, 1.0);
    let max = Vec3::new(2.0, 3.0, 4.0);
    let aabb = Aabb::new(min, max);
    assert!(vec3_approx_eq(aabb.min, min, 1e-6));
    assert!(vec3_approx_eq(aabb.max, max, 1e-6));
}

