use std::f32::consts::TAU;

use geist_geom::Vec3;

#[derive(Clone, Copy, Debug)]
pub struct DayLightSample {
    pub phase: f32,
    pub sky_scale: f32,
    pub brightness: f32,
    pub surface_sky: [f32; 3],
    pub sun_dir: Vec3,
    pub sun_visible: bool,
}

impl DayLightSample {
    #[inline]
    pub fn skylight_max(&self) -> u8 {
        (self.brightness.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

pub struct DayCycle {
    time: f32,
    day_length: f32,
}

impl DayCycle {
    pub fn new(day_length: f32) -> Self {
        Self {
            time: 0.0,
            day_length: day_length.max(1.0),
        }
    }

    pub fn advance(&mut self, dt: f32) -> DayLightSample {
        self.time = (self.time + dt).rem_euclid(self.day_length);
        self.sample()
    }

    pub fn sample(&self) -> DayLightSample {
        let frac = if self.day_length > 0.0 {
            (self.time / self.day_length).rem_euclid(1.0)
        } else {
            0.0
        };
        let phase = frac * TAU;
        let sky_scale = 0.5 * (1.0 + phase.sin());
        let brightness = sky_scale.powf(1.5);
        let day_sky = [210.0 / 255.0, 221.0 / 255.0, 235.0 / 255.0];
        let night_sky = [10.0 / 255.0, 12.0 / 255.0, 20.0 / 255.0];
        let base_sky = [
            night_sky[0] + (day_sky[0] - night_sky[0]) * brightness,
            night_sky[1] + (day_sky[1] - night_sky[1]) * brightness,
            night_sky[2] + (day_sky[2] - night_sky[2]) * brightness,
        ];
        let warm_tint = [1.0, 0.63, 0.32];
        let twilight = phase.cos().abs().powf(3.0);
        let warm_strength = (0.35 * twilight * sky_scale).clamp(0.0, 0.5);
        let surface_sky = [
            base_sky[0] * (1.0 - warm_strength) + warm_tint[0] * warm_strength,
            base_sky[1] * (1.0 - warm_strength) + warm_tint[1] * warm_strength,
            base_sky[2] * (1.0 - warm_strength) + warm_tint[2] * warm_strength,
        ];
        // Incline the sun slightly on Z so the path arcs across the sky instead of a flat plane.
        let raw_dir = Vec3::new(phase.cos(), (phase.sin() * 1.05).clamp(-1.0, 1.0), 0.25);
        let sun_dir = raw_dir.normalized();
        DayLightSample {
            phase,
            sky_scale,
            brightness,
            surface_sky,
            sun_dir,
            sun_visible: sun_dir.y > 0.0,
        }
    }
}
