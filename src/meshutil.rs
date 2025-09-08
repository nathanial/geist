use raylib::prelude::*;

use crate::blocks::{Block, BlockRegistry, FaceRole, Shape};

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
    #[inline]
    pub fn index(self) -> usize { self as usize }

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

    #[inline]
    pub fn normal(self) -> Vector3 {
        match self {
            Face::PosY => Vector3::new(0.0, 1.0, 0.0),
            Face::NegY => Vector3::new(0.0, -1.0, 0.0),
            Face::PosX => Vector3::new(1.0, 0.0, 0.0),
            Face::NegX => Vector3::new(-1.0, 0.0, 0.0),
            Face::PosZ => Vector3::new(0.0, 0.0, 1.0),
            Face::NegZ => Vector3::new(0.0, 0.0, -1.0),
        }
    }

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

    #[inline]
    pub fn role(self) -> FaceRole {
        match self {
            Face::PosY => FaceRole::Top,
            Face::NegY => FaceRole::Bottom,
            _ => FaceRole::Side,
        }
    }
}

#[inline]
pub fn is_full_cube(reg: &BlockRegistry, nb: Block) -> bool {
    reg.get(nb.id)
        .map(|t| matches!(t.shape, Shape::Cube | Shape::AxisCube { .. }))
        .unwrap_or(false)
}

