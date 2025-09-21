use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
}

impl ChunkCoord {
    #[inline]
    pub const fn new(cx: i32, cy: i32, cz: i32) -> Self {
        Self { cx, cy, cz }
    }

    #[inline]
    pub fn with_y(self, cy: i32) -> Self {
        Self { cy, ..self }
    }

    #[inline]
    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self {
            cx: self.cx + dx,
            cy: self.cy + dy,
            cz: self.cz + dz,
        }
    }

    #[inline]
    pub fn distance_sq(self, other: ChunkCoord) -> i64 {
        let dx = i64::from(self.cx - other.cx);
        let dy = i64::from(self.cy - other.cy);
        let dz = i64::from(self.cz - other.cz);
        dx * dx + dy * dy + dz * dz
    }
}

impl From<(i32, i32, i32)> for ChunkCoord {
    fn from(value: (i32, i32, i32)) -> Self {
        Self::new(value.0, value.1, value.2)
    }
}

impl From<ChunkCoord> for (i32, i32, i32) {
    fn from(value: ChunkCoord) -> Self {
        (value.cx, value.cy, value.cz)
    }
}
