use serde::{Deserialize, Serialize};

/// Stable ids written into durable chunk payloads. Existing values must never be reassigned.
#[repr(u16)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Material {
    #[default]
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Snow = 5,
    Clay = 6,
    Basalt = 7,
    Wood = 8,
    Leaves = 9,
    Moss = 10,
    Limestone = 11,
    RedSand = 12,
}

impl Material {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn from_id(id: u16) -> Option<Self> {
        Some(match id {
            0 => Self::Air,
            1 => Self::Grass,
            2 => Self::Dirt,
            3 => Self::Stone,
            4 => Self::Sand,
            5 => Self::Snow,
            6 => Self::Clay,
            7 => Self::Basalt,
            8 => Self::Wood,
            9 => Self::Leaves,
            10 => Self::Moss,
            11 => Self::Limestone,
            12 => Self::RedSand,
            _ => return None,
        })
    }

    pub const fn id(self) -> u16 {
        self as u16
    }

    pub const fn is_solid(self) -> bool {
        !matches!(self, Self::Air)
    }
}
