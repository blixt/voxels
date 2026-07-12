use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderLayer {
    Empty,
    Opaque,
    Translucent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialEmission {
    /// Linear-sRGB radiance tint consumed by host renderers.
    pub color_linear: [f32; 3],
    pub intensity: f32,
    pub radius_metres: f32,
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
    /// Stable emissive cave mineral. This remains an ordinary opaque, editable voxel.
    GlowCrystal = 14,
}

impl Material {
    pub const SCHEMA_VERSION: u16 = 3;
    pub const ALL: [Self; 15] = [
        Self::Air,
        Self::Grass,
        Self::Dirt,
        Self::Stone,
        Self::Sand,
        Self::Snow,
        Self::Clay,
        Self::Basalt,
        Self::Wood,
        Self::Leaves,
        Self::Moss,
        Self::Limestone,
        Self::RedSand,
        Self::Water,
        Self::GlowCrystal,
    ];

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
            14 => Self::GlowCrystal,
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

    pub const fn emission(self) -> Option<MaterialEmission> {
        match self {
            Self::GlowCrystal => Some(MaterialEmission {
                color_linear: [0.010, 0.477, 0.911],
                intensity: 2.4,
                radius_metres: 3.2,
            }),
            _ => None,
        }
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

    #[test]
    fn stable_material_catalog_is_dense_and_id_ordered() {
        assert_eq!(Material::SCHEMA_VERSION, 3);
        assert_eq!(Material::Water.id(), 13);
        assert_eq!(Material::GlowCrystal.id(), 14);
        for (id, material) in Material::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(material.id()), id);
            assert_eq!(Material::from_id(id as u16), Some(material));
        }
    }

    #[test]
    fn emissive_metadata_is_generic_bounded_and_linear() {
        for material in Material::ALL {
            let Some(emission) = material.emission() else {
                continue;
            };
            assert!(
                emission
                    .color_linear
                    .into_iter()
                    .all(|channel| (0.0..=1.0).contains(&channel))
            );
            assert!((0.0..=8.0).contains(&emission.intensity));
            assert!((0.1..=8.0).contains(&emission.radius_metres));
        }
        assert!(Material::GlowCrystal.emission().is_some());
        assert!(Material::Basalt.emission().is_none());
    }
}
