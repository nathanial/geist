#[derive(Clone, Copy, Debug, Default)]
pub struct NeighborsLoaded {
    pub neg_x: bool,
    pub pos_x: bool,
    pub neg_y: bool,
    pub pos_y: bool,
    pub neg_z: bool,
    pub pos_z: bool,
}

impl NeighborsLoaded {
    #[inline]
    pub const fn empty() -> Self {
        Self {
            neg_x: false,
            pos_x: false,
            neg_y: false,
            pos_y: false,
            neg_z: false,
            pos_z: false,
        }
    }

    #[inline]
    pub const fn horizontal(neg_x: bool, pos_x: bool, neg_z: bool, pos_z: bool) -> Self {
        Self {
            neg_x,
            pos_x,
            neg_y: false,
            pos_y: false,
            neg_z,
            pos_z,
        }
    }

    #[inline]
    pub const fn with_vertical(mut self, neg_y: bool, pos_y: bool) -> Self {
        self.neg_y = neg_y;
        self.pos_y = pos_y;
        self
    }

    #[inline]
    pub const fn from_bools(
        neg_x: bool,
        pos_x: bool,
        neg_y: bool,
        pos_y: bool,
        neg_z: bool,
        pos_z: bool,
    ) -> Self {
        Self {
            neg_x,
            pos_x,
            neg_y,
            pos_y,
            neg_z,
            pos_z,
        }
    }
}
