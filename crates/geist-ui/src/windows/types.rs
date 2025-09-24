use raylib::prelude::Vector2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowId {
    DebugTabs,
    DiagnosticsTabs,
    Minimap,
    ChunkVoxels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Pin,
}

impl WindowButton {
    pub const MAX_VISIBLE: usize = 3;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    pub const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub fn contains(&self, point: Vector2) -> bool {
        point.x >= self.x as f32
            && point.x <= (self.x + self.w) as f32
            && point.y >= self.y as f32
            && point.y <= (self.y + self.h) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitRegion {
    None,
    TitleBar,
    TitleBarButton(WindowButton),
    Resize(ResizeHandle),
    Content,
}
