use raylib::prelude::*;

use crate::blocks::{Block, BlockRegistry};
use crate::voxel::World;

#[derive(Debug)]
pub struct Walker {
    pub pos: Vector3, // feet position (x,z at center, y at feet)
    pub vel: Vector3,
    pub on_ground: bool,
    pub yaw: f32,    // degrees (use camera yaw)
    pub height: f32, // standing height (eye at pos.y + eye_height)
    pub eye_height: f32,
    pub radius: f32,     // horizontal radius
    pub speed: f32,      // walk speed (units/s)
    pub run_mult: f32,   // when LeftShift held
    pub jump_speed: f32, // initial jump velocity
    pub gravity: f32,    // negative
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

    pub fn eye_position(&self) -> Vector3 {
        Vector3::new(self.pos.x, self.pos.y + self.eye_height, self.pos.z)
    }

    #[inline]
    fn is_solid_for_collision(reg: &BlockRegistry, b: Block) -> bool {
        reg.get(b.id).map(|t| t.is_solid(b.state)).unwrap_or(false)
    }

    fn aabb_collides_with<F>(&self, reg: &BlockRegistry, sample: &F, pos: Vector3) -> bool
    where
        F: Fn(i32, i32, i32) -> Block,
    {
        let rx = self.radius;
        let rz = self.radius;
        let h = self.height;
        let min_x = (pos.x - rx).floor() as i32;
        let max_x = (pos.x + rx).floor() as i32;
        let min_y = (pos.y).floor() as i32;
        let max_y = (pos.y + h).floor() as i32;
        let min_z = (pos.z - rz).floor() as i32;
        let max_z = (pos.z + rz).floor() as i32;
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                for x in min_x..=max_x {
                    let b = sample(x, y, z);
                    if Self::is_solid_for_collision(reg, b) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn move_axis<F>(&mut self, reg: &BlockRegistry, sample: &F, axis: usize, amt: f32) -> f32
    where
        F: Fn(i32, i32, i32) -> Block,
    {
        if amt == 0.0 {
            return 0.0;
        }
        const STEP_RES: f32 = 0.05;
        let mut moved = 0.0_f32;
        let step = STEP_RES * amt.signum();
        let mut remaining = amt;
        while remaining.abs() > 0.0001 {
            let s = if remaining.abs() < step.abs() {
                remaining
            } else {
                step
            };
            let mut p = self.pos;
            match axis {
                0 => p.x += s,
                1 => p.y += s,
                _ => p.z += s,
            };
            if self.aabb_collides_with(reg, sample, p) {
                break; // collision
            } else {
                self.pos = p;
                moved += s;
                remaining -= s;
            }
        }
        moved
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_with_sampler<F>(
        &mut self,
        rl: &mut raylib::RaylibHandle,
        sample: &F,
        _world: &World,
        reg: &BlockRegistry,
        dt: f32,
        yaw: f32,
        platform_velocity: Option<Vector3>,
    ) where
        F: Fn(i32, i32, i32) -> Block,
    {
        self.yaw = yaw;
        // Input wishdir (XZ plane) based on yaw
        let yaw_rad = self.yaw.to_radians();
        let fwd = Vector3::new(yaw_rad.cos(), 0.0, yaw_rad.sin()).normalized();
        let right = fwd.cross(Vector3::up());
        let mut wish = Vector3::zero();
        if rl.is_key_down(KeyboardKey::KEY_W) {
            wish += fwd;
        }
        if rl.is_key_down(KeyboardKey::KEY_S) {
            wish -= fwd;
        }
        if rl.is_key_down(KeyboardKey::KEY_A) {
            wish -= right;
        }
        if rl.is_key_down(KeyboardKey::KEY_D) {
            wish += right;
        }
        if wish.length() > 0.0 {
            wish = wish.normalized();
        }
        let run = if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) {
            self.run_mult
        } else {
            1.0
        };

        // Horizontal motion is kinematic toward wishdir (simple, responsive)
        let target_v = wish * self.speed * run;
        let horiz = Vector3::new(target_v.x, 0.0, target_v.z);

        // Add platform velocity if provided (for moving structures)
        let platform_vel = platform_velocity.unwrap_or(Vector3::zero());
        let total_horiz = horiz + Vector3::new(platform_vel.x, 0.0, platform_vel.z);

        // Gravity and jumping
        // Ground check: test a slightly larger offset down for stability
        let mut below = self.pos;
        below.y -= 0.10;
        self.on_ground = self.aabb_collides_with(reg, sample, below);
        if self.on_ground {
            // Reset vertical velocity and allow jump
            if self.vel.y < 0.0 {
                self.vel.y = 0.0;
            }
            if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
                self.vel.y = self.jump_speed;
                self.on_ground = false;
            }
        } else {
            self.vel.y += self.gravity * dt;
        }

        // Apply movement with collision; order depends on vertical motion
        // Include platform velocity in movement
        let dx = total_horiz.x * dt;
        let dz = total_horiz.z * dt;
        let dy = (self.vel.y + platform_vel.y) * dt;
        let moved_y = if dy > 0.0 {
            // Ascending (jump/climb): move up first, then horizontal
            let my = self.move_axis(reg, sample, 1, dy);
            self.move_axis(reg, sample, 0, dx);
            self.move_axis(reg, sample, 2, dz);
            my
        } else {
            // Descending / grounded: hug terrain by doing horizontal first
            self.move_axis(reg, sample, 0, dx);
            self.move_axis(reg, sample, 2, dz);
            self.move_axis(reg, sample, 1, dy)
        };
        // Land
        if dy < 0.0 && moved_y.abs() < dy.abs() * 0.5 {
            self.on_ground = true;
            self.vel.y = 0.0;
        }

        // Clamp only the minimum vertical; allow going above world ceiling (for flying structures)
        self.pos.y = self.pos.y.max(0.0);
    }

    // No back-compat path: the walker updates only via an explicit sampler tied to loaded chunk buffers.
}
