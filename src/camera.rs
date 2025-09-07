use raylib::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub normal: Vector3,
    pub distance: f32,
}

impl Plane {
    pub fn new(normal: Vector3, point: Vector3) -> Self {
        let normal = normal.normalized();
        Self {
            normal,
            distance: -normal.dot(point),
        }
    }

    pub fn distance_to_point(&self, point: Vector3) -> f32 {
        self.normal.dot(point) + self.distance
    }
}

pub struct Frustum {
    pub planes: [Plane; 6], // left, right, top, bottom, near, far
}

impl Frustum {
    pub fn contains_bounding_box(&self, bbox: &raylib::core::math::BoundingBox) -> bool {
        // Get the 8 corners of the bounding box
        let corners = [
            Vector3::new(bbox.min.x, bbox.min.y, bbox.min.z),
            Vector3::new(bbox.max.x, bbox.min.y, bbox.min.z),
            Vector3::new(bbox.min.x, bbox.max.y, bbox.min.z),
            Vector3::new(bbox.max.x, bbox.max.y, bbox.min.z),
            Vector3::new(bbox.min.x, bbox.min.y, bbox.max.z),
            Vector3::new(bbox.max.x, bbox.min.y, bbox.max.z),
            Vector3::new(bbox.min.x, bbox.max.y, bbox.max.z),
            Vector3::new(bbox.max.x, bbox.max.y, bbox.max.z),
        ];

        // Check each plane
        for plane in &self.planes {
            let mut all_outside = true;
            for corner in &corners {
                if plane.distance_to_point(*corner) >= 0.0 {
                    all_outside = false;
                    break;
                }
            }
            // If all corners are outside this plane, the box is outside the frustum
            if all_outside {
                return false;
            }
        }

        true
    }
}

pub struct FlyCamera {
    pub position: Vector3,
    pub yaw: f32,   // degrees
    pub pitch: f32, // degrees
    pub move_speed: f32,
    pub mouse_sensitivity: f32,
    pub captured: bool,
}

impl FlyCamera {
    pub fn new(position: Vector3) -> Self {
        Self {
            position,
            yaw: -45.0,
            pitch: -15.0,
            move_speed: 8.0,
            mouse_sensitivity: 0.1,
            captured: true,
        }
    }

    pub fn to_camera3d(&self) -> Camera3D {
        let forward = self.forward();
        Camera3D::perspective(
            self.position,
            self.position + forward,
            Vector3::new(0.0, 1.0, 0.0),
            70.0,
        )
    }

    pub fn calculate_frustum(&self, aspect_ratio: f32, near: f32, far: f32) -> Frustum {
        let fov_y = 70.0_f32.to_radians();
        let forward = self.forward();
        let right = self.right();
        let up = right.cross(forward).normalized();

        // Calculate dimensions at near and far planes
        let tan_half_fov = (fov_y * 0.5).tan();
        let near_height = 2.0 * near * tan_half_fov;
        let near_width = near_height * aspect_ratio;
        let far_height = 2.0 * far * tan_half_fov;
        let _far_width = far_height * aspect_ratio;

        // Calculate frustum corner points
        let nc = self.position + forward * near; // near center
        let fc = self.position + forward * far; // far center

        // Near corners
        let nlt = nc + up * (near_height * 0.5) - right * (near_width * 0.5); // near left top
        let nrt = nc + up * (near_height * 0.5) + right * (near_width * 0.5); // near right top
        let nlb = nc - up * (near_height * 0.5) - right * (near_width * 0.5); // near left bottom
        let nrb = nc - up * (near_height * 0.5) + right * (near_width * 0.5); // near right bottom

        // Create planes using cross products to get correct normals
        // Each plane normal points INWARD to the frustum

        // Near plane (normal points forward into frustum)
        let near_plane = Plane::new(forward, nc);

        // Far plane (normal points backward into frustum)
        let far_plane = Plane::new(-forward, fc);

        // Left plane (contains camera position, nlt, nlb)
        let left_edge1 = (nlt - self.position).normalized();
        let left_edge2 = (nlb - self.position).normalized();
        let left_normal = left_edge2.cross(left_edge1).normalized();
        let left_plane = Plane::new(left_normal, self.position);

        // Right plane (contains camera position, nrb, nrt)
        let right_edge1 = (nrb - self.position).normalized();
        let right_edge2 = (nrt - self.position).normalized();
        let right_normal = right_edge2.cross(right_edge1).normalized();
        let right_plane = Plane::new(right_normal, self.position);

        // Top plane (contains camera position, nrt, nlt)
        let top_edge1 = (nrt - self.position).normalized();
        let top_edge2 = (nlt - self.position).normalized();
        let top_normal = top_edge2.cross(top_edge1).normalized();
        let top_plane = Plane::new(top_normal, self.position);

        // Bottom plane (contains camera position, nlb, nrb)
        let bottom_edge1 = (nlb - self.position).normalized();
        let bottom_edge2 = (nrb - self.position).normalized();
        let bottom_normal = bottom_edge2.cross(bottom_edge1).normalized();
        let bottom_plane = Plane::new(bottom_normal, self.position);

        Frustum {
            planes: [
                left_plane,
                right_plane,
                top_plane,
                bottom_plane,
                near_plane,
                far_plane,
            ],
        }
    }

    pub fn forward(&self) -> Vector3 {
        let yaw_rad = self.yaw.to_radians();
        let pitch_rad = self.pitch.to_radians();
        Vector3::new(
            yaw_rad.cos() * pitch_rad.cos(),
            pitch_rad.sin(),
            yaw_rad.sin() * pitch_rad.cos(),
        )
        .normalized()
    }

    pub fn right(&self) -> Vector3 {
        self.forward().cross(Vector3::up()).normalized()
    }

    pub fn update(&mut self, rl: &mut RaylibHandle, dt: f32) {
        // Toggle mouse capture with Tab
        if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
            self.captured = !self.captured;
            if self.captured {
                rl.disable_cursor();
            } else {
                rl.enable_cursor();
            }
        }

        if self.captured {
            // Mouse look
            let md = rl.get_mouse_delta();
            self.yaw += md.x * self.mouse_sensitivity;
            self.pitch -= md.y * self.mouse_sensitivity;
            self.pitch = self.pitch.clamp(-89.9, 89.9);
        }

        // Movement
        let mut wish_dir = Vector3::zero();
        let f = self.forward();
        let r = self.right();
        if rl.is_key_down(KeyboardKey::KEY_W) {
            wish_dir += f;
        }
        if rl.is_key_down(KeyboardKey::KEY_S) {
            wish_dir -= f;
        }
        if rl.is_key_down(KeyboardKey::KEY_A) {
            wish_dir -= r;
        }
        if rl.is_key_down(KeyboardKey::KEY_D) {
            wish_dir += r;
        }
        if rl.is_key_down(KeyboardKey::KEY_E) {
            wish_dir += Vector3::up();
        }
        if rl.is_key_down(KeyboardKey::KEY_Q) {
            wish_dir -= Vector3::up();
        }
        if wish_dir.length() > 0.0 {
            wish_dir = wish_dir.normalized();
            let speed = if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) {
                self.move_speed * 3.0
            } else {
                self.move_speed
            };
            self.position += wish_dir * speed * dt;
        }
    }

    // Update only mouse-look/capture; leave translation to an external controller (e.g., Walker)
    pub fn update_look_only(&mut self, rl: &mut RaylibHandle, _dt: f32) {
        // Toggle mouse capture with Tab
        if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
            self.captured = !self.captured;
            if self.captured {
                rl.disable_cursor();
            } else {
                rl.enable_cursor();
            }
        }
        if self.captured {
            let md = rl.get_mouse_delta();
            self.yaw += md.x * self.mouse_sensitivity;
            self.pitch -= md.y * self.mouse_sensitivity;
            self.pitch = self.pitch.clamp(-89.9, 89.9);
        }
    }
}
