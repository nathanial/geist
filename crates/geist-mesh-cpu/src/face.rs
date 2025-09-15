use geist_blocks::types::FaceRole;
use geist_geom::Vec3;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Face {
    PosY = 0,
    NegY = 1,
    PosX = 2,
    NegX = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
    /// Returns the `[0..6)` index of this face.
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Converts a face index `[0..6)` back into a `Face` value.
    /// Falls back to `PosY` for out-of-range indices.
    #[inline]
    pub fn from_index(i: usize) -> Face {
        match i {
            0 => Face::PosY,
            1 => Face::NegY,
            2 => Face::PosX,
            3 => Face::NegX,
            4 => Face::PosZ,
            5 => Face::NegZ,
            _ => Face::PosY,
        }
    }

    /// Returns the unit-normal vector for this face.
    #[inline]
    pub fn normal(self) -> Vec3 {
        match self {
            Face::PosY => Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            Face::NegY => Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            Face::PosX => Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Face::NegX => Vec3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Face::PosZ => Vec3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            Face::NegZ => Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
        }
    }

    /// Returns the integer grid delta `(dx,dy,dz)` when stepping out of this face.
    #[inline]
    pub fn delta(self) -> (i32, i32, i32) {
        match self {
            Face::PosY => (0, 1, 0),
            Face::NegY => (0, -1, 0),
            Face::PosX => (1, 0, 0),
            Face::NegX => (-1, 0, 0),
            Face::PosZ => (0, 0, 1),
            Face::NegZ => (0, 0, -1),
        }
    }

    /// Classifies the face into top/bottom/side role for material lookup.
    #[inline]
    pub fn role(self) -> FaceRole {
        match self {
            Face::PosY => FaceRole::Top,
            Face::NegY => FaceRole::Bottom,
            _ => FaceRole::Side,
        }
    }
}

/// Neighbor offsets used for thin connector geometry on the four lateral sides.
pub const SIDE_NEIGHBORS: [(i32, i32, Face, f32, f32); 4] = [
    (-1, 0, Face::PosX, 0.0, 0.0),
    (1, 0, Face::NegX, 1.0, 0.0),
    (0, -1, Face::PosZ, 0.0, 0.0),
    (0, 1, Face::NegZ, 0.0, 1.0),
];
