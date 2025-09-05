use raylib::prelude::*;

use crate::voxel::{World, Block};

#[derive(Debug)]
pub struct Walker {
    pub pos: Vector3,     // feet position (x,z at center, y at feet)
    pub vel: Vector3,
    pub on_ground: bool,
    pub yaw: f32,         // degrees (use camera yaw)
    pub height: f32,      // standing height (eye at pos.y + eye_height)
    pub eye_height: f32,
    pub radius: f32,      // horizontal radius
    pub speed: f32,       // walk speed (units/s)
    pub run_mult: f32,    // when LeftShift held
    pub jump_speed: f32,  // initial jump velocity
    pub gravity: f32,     // negative
}

impl Walker {
    pub fn new(spawn: Vector3) -> Self {
        Self {
            pos: spawn,
            vel: Vector3::zero(),
            on_ground: false,
            yaw: -45.0,
            height: 1.75,
            eye_height: 1.60,
            radius: 0.35,
            speed: 5.0,
            run_mult: 1.6,
            jump_speed: 7.5,
            gravity: -25.0,
        }
    }

    pub fn eye_position(&self) -> Vector3 { Vector3::new(self.pos.x, self.pos.y + self.eye_height, self.pos.z) }

    #[inline]
    fn is_solid_for_collision(b: Block) -> bool {
        match b {
            Block::Air => false,
            Block::Leaves(_) => false, // allow walking through foliage
            _ => true,
        }
    }

    fn aabb_collides(&self, world: &World, pos: Vector3) -> bool {
        let rx = self.radius; let rz = self.radius; let h = self.height;
        let min_x = (pos.x - rx).floor() as i32;
        let max_x = (pos.x + rx).floor() as i32;
        let min_y = (pos.y).floor() as i32;
        let max_y = (pos.y + h).floor() as i32;
        let min_z = (pos.z - rz).floor() as i32;
        let max_z = (pos.z + rz).floor() as i32;
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = world.block_at(x, y, z);
                    if Self::is_solid_for_collision(b) { return true; }
                }
            }
        }
        false
    }

    fn move_axis(&mut self, world: &World, axis: usize, amt: f32) -> f32 {
        if amt == 0.0 { return 0.0; }
        let mut moved = 0.0_f32;
        let step = 0.05_f32 * amt.signum();
        let mut remaining = amt;
        while remaining.abs() > 0.0001 {
            let s = if remaining.abs() < step.abs() { remaining } else { step };
            let mut p = self.pos;
            match axis { 0 => p.x += s, 1 => p.y += s, _ => p.z += s };
            if self.aabb_collides(world, p) {
                // Step-up heuristic for horizontal axes
                if axis != 1 && self.on_ground {
                    let mut up = self.pos; up.y += 0.6; // try stepping up a half-block
                    let mut p2 = up; match axis { 0 => p2.x += s, _ => p2.z += s };
                    if !self.aabb_collides(world, up) && !self.aabb_collides(world, p2) {
                        self.pos = p2; moved += s; remaining -= s; continue;
                    }
                }
                break; // collision
            } else {
                self.pos = p; moved += s; remaining -= s;
            }
        }
        moved
    }

    pub fn update(&mut self, rl: &mut raylib::RaylibHandle, world: &World, dt: f32, yaw: f32) {
        self.yaw = yaw;
        // Input wishdir (XZ plane) based on yaw
        let yaw_rad = self.yaw.to_radians();
        let fwd = Vector3::new(yaw_rad.cos(), 0.0, yaw_rad.sin()).normalized();
        let right = fwd.cross(Vector3::up());
        let mut wish = Vector3::zero();
        if rl.is_key_down(KeyboardKey::KEY_W) { wish += fwd; }
        if rl.is_key_down(KeyboardKey::KEY_S) { wish -= fwd; }
        if rl.is_key_down(KeyboardKey::KEY_A) { wish -= right; }
        if rl.is_key_down(KeyboardKey::KEY_D) { wish += right; }
        if wish.length() > 0.0 { wish = wish.normalized(); }
        let run = if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) { self.run_mult } else { 1.0 };

        // Horizontal motion is kinematic toward wishdir (simple, responsive)
        let target_v = wish * self.speed * run;
        let mut horiz = Vector3::new(target_v.x, 0.0, target_v.z);

        // Gravity and jumping
        // Ground check: test a small offset down
        let mut below = self.pos; below.y -= 0.05;
        self.on_ground = self.aabb_collides(world, below);
        if self.on_ground {
            // Reset vertical velocity and allow jump
            if self.vel.y < 0.0 { self.vel.y = 0.0; }
            if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
                self.vel.y = self.jump_speed; self.on_ground = false;
            }
        } else {
            self.vel.y += self.gravity * dt;
        }

        // Apply movement with collision (X, Z, then Y)
        let dx = horiz.x * dt; let dz = horiz.z * dt; let dy = self.vel.y * dt;
        self.move_axis(world, 0, dx);
        self.move_axis(world, 2, dz);
        let moved_y = self.move_axis(world, 1, dy);
        // Land
        if dy < 0.0 && moved_y.abs() < dy.abs() * 0.5 { self.on_ground = true; self.vel.y = 0.0; }

        // Clamp within world bounds
        let max_x = (world.world_size_x() as f32) - 0.001;
        let max_z = (world.world_size_z() as f32) - 0.001;
        let max_y = (world.world_size_y() as f32) - self.height - 0.001;
        self.pos.x = self.pos.x.clamp(0.001, max_x);
        self.pos.z = self.pos.z.clamp(0.001, max_z);
        self.pos.y = self.pos.y.clamp(0.0, max_y.max(0.0));
    }
}

