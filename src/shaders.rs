use raylib::prelude::*;

pub struct LeavesShader {
    pub shader: raylib::shaders::WeakShader,
    pub loc_fog_color: i32,
    pub loc_fog_start: i32,
    pub loc_fog_end: i32,
    pub loc_camera_pos: i32,
    pub loc_palette0: i32,
    pub loc_palette1: i32,
    pub loc_palette2: i32,
    pub loc_palette3: i32,
    pub loc_strength: i32,
}

impl LeavesShader {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let vs = "assets/shaders/voxel_fog_textured.vs";
        let fs = "assets/shaders/voxel_fog_leaves.fs";
        let shader_strong = rl.load_shader(thread, Some(vs), Some(fs));
        let shader = unsafe { shader_strong.make_weak() };
        // Query uniforms (WeakShader implements RaylibShader)
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        let loc_palette0 = shader.get_shader_location("palette0");
        let loc_palette1 = shader.get_shader_location("palette1");
        let loc_palette2 = shader.get_shader_location("palette2");
        let loc_palette3 = shader.get_shader_location("palette3");
        let loc_strength = shader.get_shader_location("autumnStrength");
        let mut s = Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
            loc_palette0,
            loc_palette1,
            loc_palette2,
            loc_palette3,
            loc_strength,
        };
        // Default palette from old code
        s.set_autumn_palette(
            [0.905, 0.678, 0.161],
            [0.847, 0.451, 0.122],
            [0.710, 0.200, 0.153],
            [0.431, 0.231, 0.039],
            1.0,
        );
        Some(s)
    }

    pub fn set_autumn_palette(
        &mut self,
        p0: [f32; 3],
        p1: [f32; 3],
        p2: [f32; 3],
        p3: [f32; 3],
        strength: f32,
    ) {
        if self.loc_palette0 >= 0 {
            self.shader.set_shader_value(self.loc_palette0, p0);
        }
        if self.loc_palette1 >= 0 {
            self.shader.set_shader_value(self.loc_palette1, p1);
        }
        if self.loc_palette2 >= 0 {
            self.shader.set_shader_value(self.loc_palette2, p2);
        }
        if self.loc_palette3 >= 0 {
            self.shader.set_shader_value(self.loc_palette3, p3);
        }
        if self.loc_strength >= 0 {
            self.shader.set_shader_value(self.loc_strength, strength);
        }
    }

    pub fn update_frame_uniforms(
        &mut self,
        camera_pos: Vector3,
        fog_color: [f32; 3],
        fog_start: f32,
        fog_end: f32,
    ) {
        if self.loc_fog_color >= 0 {
            self.shader.set_shader_value(self.loc_fog_color, fog_color);
        }
        if self.loc_fog_start >= 0 {
            self.shader.set_shader_value(self.loc_fog_start, fog_start);
        }
        if self.loc_fog_end >= 0 {
            self.shader.set_shader_value(self.loc_fog_end, fog_end);
        }
        if self.loc_camera_pos >= 0 {
            let cam = [camera_pos.x, camera_pos.y, camera_pos.z];
            self.shader.set_shader_value(self.loc_camera_pos, cam);
        }
    }
}

pub struct FogShader {
    pub shader: raylib::shaders::WeakShader,
    pub loc_fog_color: i32,
    pub loc_fog_start: i32,
    pub loc_fog_end: i32,
    pub loc_camera_pos: i32,
}

impl FogShader {
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread) -> Option<Self> {
        let vs = "assets/shaders/voxel_fog_textured.vs";
        let fs = "assets/shaders/voxel_fog_textured.fs";
        let shader_strong = rl.load_shader(thread, Some(vs), Some(fs));
        let shader = unsafe { shader_strong.make_weak() };
        let loc_fog_color = shader.get_shader_location("fogColor");
        let loc_fog_start = shader.get_shader_location("fogStart");
        let loc_fog_end = shader.get_shader_location("fogEnd");
        let loc_camera_pos = shader.get_shader_location("cameraPos");
        Some(Self {
            shader,
            loc_fog_color,
            loc_fog_start,
            loc_fog_end,
            loc_camera_pos,
        })
    }

    pub fn update_frame_uniforms(
        &mut self,
        camera_pos: Vector3,
        fog_color: [f32; 3],
        fog_start: f32,
        fog_end: f32,
    ) {
        if self.loc_fog_color >= 0 {
            self.shader.set_shader_value(self.loc_fog_color, fog_color);
        }
        if self.loc_fog_start >= 0 {
            self.shader.set_shader_value(self.loc_fog_start, fog_start);
        }
        if self.loc_fog_end >= 0 {
            self.shader.set_shader_value(self.loc_fog_end, fog_end);
        }
        if self.loc_camera_pos >= 0 {
            let cam = [camera_pos.x, camera_pos.y, camera_pos.z];
            self.shader.set_shader_value(self.loc_camera_pos, cam);
        }
    }
}
