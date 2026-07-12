//! Versioned `VXCH` chunk codec: canonical palette + little-endian bit-packed palette indices.

use crate::generation::GENERATOR_VERSION;
use crate::{CHUNK_EDGE, CHUNK_VOLUME, Chunk, ChunkCoord, Material};
use std::fmt;

const MAGIC: &[u8; 4] = b"VXCH";
const FORMAT_VERSION: u16 = 1;
const HEADER_LEN: u16 = 76;
const ENCODING_UNIFORM: u8 = 0;
const ENCODING_PALETTE: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    UnknownMaterial(u16),
    CorruptHash,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated VXCH payload"),
            Self::InvalidMagic => formatter.write_str("invalid VXCH magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported VXCH version {version}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid VXCH header: {reason}"),
            Self::UnknownMaterial(id) => write!(formatter, "unknown material id {id}"),
            Self::CorruptHash => formatter.write_str("VXCH voxel hash mismatch"),
        }
    }
}

impl std::error::Error for CodecError {}

pub fn encode_chunk(chunk: &Chunk) -> Vec<u8> {
    let mut present = [false; Material::ALL.len()];
    for material in chunk.voxels() {
        present[usize::from(material.id())] = true;
    }
    let mut palette = Vec::with_capacity(Material::ALL.len());
    let mut palette_indices = [0u8; Material::ALL.len()];
    for material in Material::ALL {
        let id = material.id();
        if present[usize::from(id)] {
            palette_indices[usize::from(id)] = palette.len() as u8;
            palette.push(id);
        }
    }
    let uniform = palette.len() == 1;
    let bits = if uniform { 0 } else { bits_for(palette.len()) };
    let payload = if uniform {
        Vec::new()
    } else {
        pack_indices(chunk.voxels(), &palette_indices, bits)
    };
    let hash = hash_voxels(chunk.voxels());
    let coord = chunk.coord();
    let mut encoded =
        Vec::with_capacity(usize::from(HEADER_LEN) + palette.len() * 2 + payload.len());
    encoded.extend_from_slice(MAGIC);
    encoded.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    encoded.extend_from_slice(&HEADER_LEN.to_le_bytes());
    encoded.push(CHUNK_EDGE.trailing_zeros() as u8);
    encoded.push(if uniform {
        ENCODING_UNIFORM
    } else {
        ENCODING_PALETTE
    });
    encoded.extend_from_slice(&0u16.to_le_bytes());
    encoded.extend_from_slice(&coord.x.to_le_bytes());
    encoded.extend_from_slice(&coord.y.to_le_bytes());
    encoded.extend_from_slice(&coord.z.to_le_bytes());
    encoded.extend_from_slice(&Material::SCHEMA_VERSION.to_le_bytes());
    encoded.extend_from_slice(&0u16.to_le_bytes());
    encoded.extend_from_slice(&GENERATOR_VERSION.to_le_bytes());
    encoded.extend_from_slice(&(CHUNK_VOLUME as u32).to_le_bytes());
    encoded.extend_from_slice(&(palette.len() as u16).to_le_bytes());
    encoded.push(bits);
    encoded.push(0);
    encoded.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    encoded.extend_from_slice(hash.as_bytes());
    debug_assert_eq!(encoded.len(), usize::from(HEADER_LEN));
    for material in palette {
        encoded.extend_from_slice(&material.to_le_bytes());
    }
    encoded.extend_from_slice(&payload);
    encoded
}

pub fn decode_chunk(bytes: &[u8]) -> Result<Chunk, CodecError> {
    if bytes.len() < usize::from(HEADER_LEN) {
        return Err(CodecError::Truncated);
    }
    if &bytes[0..4] != MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    let version = read_u16(bytes, 4)?;
    if version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    if read_u16(bytes, 6)? != HEADER_LEN {
        return Err(CodecError::InvalidHeader("unexpected header length"));
    }
    if bytes[8] != CHUNK_EDGE.trailing_zeros() as u8 {
        return Err(CodecError::InvalidHeader("unexpected chunk edge"));
    }
    let encoding = bytes[9];
    if read_u16(bytes, 24)? != Material::SCHEMA_VERSION {
        return Err(CodecError::InvalidHeader("material schema mismatch"));
    }
    if read_u32(bytes, 28)? != GENERATOR_VERSION {
        return Err(CodecError::InvalidHeader("generator version mismatch"));
    }
    if read_u32(bytes, 32)? as usize != CHUNK_VOLUME {
        return Err(CodecError::InvalidHeader("unexpected voxel count"));
    }
    let palette_len = usize::from(read_u16(bytes, 36)?);
    if palette_len == 0 || palette_len > CHUNK_VOLUME {
        return Err(CodecError::InvalidHeader("invalid palette length"));
    }
    let bits = bytes[38];
    let payload_len = read_u32(bytes, 40)? as usize;
    let expected_len = usize::from(HEADER_LEN)
        .checked_add(palette_len * 2)
        .and_then(|length| length.checked_add(payload_len))
        .ok_or(CodecError::InvalidHeader("length overflow"))?;
    if bytes.len() < expected_len {
        return Err(CodecError::Truncated);
    }
    if bytes.len() != expected_len {
        return Err(CodecError::InvalidHeader("trailing bytes"));
    }
    let mut palette = Vec::with_capacity(palette_len);
    let mut cursor = usize::from(HEADER_LEN);
    for _ in 0..palette_len {
        let id = read_u16(bytes, cursor)?;
        palette.push(Material::from_id(id).ok_or(CodecError::UnknownMaterial(id))?);
        cursor += 2;
    }
    let voxels = match encoding {
        ENCODING_UNIFORM if palette_len == 1 && bits == 0 && payload_len == 0 => {
            vec![palette[0]; CHUNK_VOLUME]
        }
        ENCODING_PALETTE if palette_len > 1 && bits == bits_for(palette_len) => {
            let required = (CHUNK_VOLUME * usize::from(bits)).div_ceil(8);
            if payload_len != required {
                return Err(CodecError::InvalidHeader("invalid packed payload length"));
            }
            unpack_indices(&bytes[cursor..], &palette, bits)?
        }
        _ => return Err(CodecError::InvalidHeader("invalid encoding parameters")),
    };
    let expected_hash = &bytes[44..76];
    if hash_voxels(&voxels).as_bytes() != expected_hash {
        return Err(CodecError::CorruptHash);
    }
    let coord = ChunkCoord::new(
        read_i32(bytes, 12)?,
        read_i32(bytes, 16)?,
        read_i32(bytes, 20)?,
    );
    Chunk::from_voxels(coord, voxels).ok_or(CodecError::InvalidHeader("voxel count mismatch"))
}

fn bits_for(palette_len: usize) -> u8 {
    usize::BITS.saturating_sub((palette_len - 1).leading_zeros()) as u8
}

fn pack_indices(
    voxels: &[Material],
    palette_indices: &[u8; Material::ALL.len()],
    bits: u8,
) -> Vec<u8> {
    let mut packed = Vec::with_capacity((voxels.len() * usize::from(bits)).div_ceil(8));
    let mut accumulator = 0u64;
    let mut held = 0u32;
    for material in voxels {
        let index = u64::from(palette_indices[usize::from(material.id())]);
        accumulator |= index << held;
        held += u32::from(bits);
        while held >= 8 {
            packed.push(accumulator as u8);
            accumulator >>= 8;
            held -= 8;
        }
    }
    if held > 0 {
        packed.push(accumulator as u8);
    }
    packed
}

fn unpack_indices(
    payload: &[u8],
    palette: &[Material],
    bits: u8,
) -> Result<Vec<Material>, CodecError> {
    let mask = (1u64 << bits) - 1;
    let mut voxels = Vec::with_capacity(CHUNK_VOLUME);
    let mut accumulator = 0u64;
    let mut held = 0u32;
    let mut cursor = 0usize;
    while voxels.len() < CHUNK_VOLUME {
        while held < u32::from(bits) {
            let byte = payload.get(cursor).ok_or(CodecError::Truncated)?;
            accumulator |= u64::from(*byte) << held;
            held += 8;
            cursor += 1;
        }
        let index = (accumulator & mask) as usize;
        voxels.push(
            *palette
                .get(index)
                .ok_or(CodecError::InvalidHeader("palette index out of range"))?,
        );
        accumulator >>= bits;
        held -= u32::from(bits);
    }
    Ok(voxels)
}

fn hash_voxels(voxels: &[Material]) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for material in voxels {
        hasher.update(&material.id().to_le_bytes());
    }
    hasher.finalize()
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, CodecError> {
    let value = bytes.get(offset..offset + 2).ok_or(CodecError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CodecError> {
    let value = bytes.get(offset..offset + 4).ok_or(CodecError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, CodecError> {
    let value = bytes.get(offset..offset + 4).ok_or(CodecError::Truncated)?;
    Ok(i32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Generator;

    #[test]
    fn uniform_chunk_round_trips_compactly() {
        let chunk = Chunk::filled(ChunkCoord::new(-4, 2, 9), Material::Basalt);
        let encoded = encode_chunk(&chunk);
        assert_eq!(encoded.len(), usize::from(HEADER_LEN) + 2);
        assert_eq!(decode_chunk(&encoded), Ok(chunk));
    }

    #[test]
    fn water_material_round_trips_through_the_versioned_palette() {
        let mut chunk = Chunk::empty(ChunkCoord::new(3, 0, -2));
        chunk.set(4, 10, 7, Material::Water);
        chunk.set(4, 9, 7, Material::Sand);
        let encoded = encode_chunk(&chunk);
        let decoded = decode_chunk(&encoded).expect("valid current-version Water payload");
        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get(4, 10, 7), Material::Water);
    }

    #[test]
    fn glow_crystal_round_trips_as_an_ordinary_opaque_voxel() {
        let mut chunk = Chunk::empty(ChunkCoord::new(-162, 0, 103));
        chunk.set(4, 10, 7, Material::GlowCrystal);
        let encoded = encode_chunk(&chunk);
        let decoded = decode_chunk(&encoded).expect("valid current-version crystal payload");
        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get(4, 10, 7), Material::GlowCrystal);
        assert_eq!(
            Material::GlowCrystal.render_layer(),
            crate::RenderLayer::Opaque
        );
    }

    #[test]
    fn generated_chunk_round_trips_through_palette_bits() {
        let chunk = Generator::new(42).generate_chunk(ChunkCoord::new(0, 0, 0));
        let encoded = encode_chunk(&chunk);
        assert_eq!(
            blake3::hash(&encoded).to_hex().to_string(),
            "3c3accd05e44cc9ecb8ec291e56b579fe52e42df2682c171e005292bc39b37cc"
        );
        assert!(encoded.len() < CHUNK_VOLUME * size_of::<u16>());
        assert_eq!(decode_chunk(&encoded), Ok(chunk));
    }

    #[test]
    fn corruption_is_detected() {
        let chunk = Generator::new(7).generate_chunk(ChunkCoord::new(1, 0, 1));
        let mut encoded = encode_chunk(&chunk);
        let last = encoded.len() - 1;
        encoded[last] ^= 0x40;
        assert!(decode_chunk(&encoded).is_err());
    }

    #[test]
    fn malformed_lengths_are_rejected_before_allocation() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let mut encoded = encode_chunk(&chunk);
        encoded[36..38].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            decode_chunk(&encoded),
            Err(CodecError::InvalidHeader("invalid palette length"))
        );
    }
}
