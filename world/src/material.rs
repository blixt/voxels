use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderLayer {
    Empty,
    Opaque,
    Translucent,
}

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
    Water = 13,
}

impl Material {
    pub const SCHEMA_VERSION: u16 = 2;

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
            13 => Self::Water,
            _ => return None,
        })
    }

    pub const fn id(self) -> u16 {
        self as u16
    }

    pub const fn render_layer(self) -> RenderLayer {
        match self {
            Self::Air => RenderLayer::Empty,
            Self::Water => RenderLayer::Translucent,
            _ => RenderLayer::Opaque,
        }
    }

    pub const fn is_collidable(self) -> bool {
        !matches!(self, Self::Air | Self::Water)
    }

    pub const fn is_renderable(self) -> bool {
        !matches!(self.render_layer(), RenderLayer::Empty)
    }

    pub const fn occludes_ambient(self) -> bool {
        matches!(self.render_layer(), RenderLayer::Opaque)
    }

    pub const fn is_fluid(self) -> bool {
        matches!(self, Self::Water)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_is_renderable_without_blocking_motion() {
        assert!(Material::Water.is_renderable());
        assert!(!Material::Water.is_collidable());
        assert!(!Material::Water.occludes_ambient());
        assert!(Material::Water.is_fluid());
        assert_eq!(Material::Water.render_layer(), RenderLayer::Translucent);
        assert_eq!(
            Material::from_id(Material::Water.id()),
            Some(Material::Water)
        );
    }
}
