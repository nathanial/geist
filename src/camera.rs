use raylib::prelude::*;

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

    pub fn up(&self) -> Vector3 {
        self.right().cross(self.forward()).normalized()
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
    pub fn update_look_only(&mut self, rl: &mut RaylibHandle, dt: f32) {
        // Toggle mouse capture with Tab
        if rl.is_key_pressed(KeyboardKey::KEY_TAB) {
            self.captured = !self.captured;
            if self.captured { rl.disable_cursor(); } else { rl.enable_cursor(); }
        }
        if self.captured {
            let md = rl.get_mouse_delta();
            self.yaw += md.x * self.mouse_sensitivity;
            self.pitch -= md.y * self.mouse_sensitivity;
            self.pitch = self.pitch.clamp(-89.9, 89.9);
        }
    }
}
