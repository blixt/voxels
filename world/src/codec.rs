//! Versioned `VXCH` chunk codec: canonical palette + little-endian bit-packed palette indices.

use crate::{CHUNK_EDGE, CHUNK_VOLUME, Chunk, ChunkCoord, Material, WorldSourceIdentityHash};
use std::fmt;

const MAGIC: &[u8; 4] = b"VXCH";
const FORMAT_VERSION: u16 = 2;
const HEADER_LEN: u16 = 104;
const ENCODING_UNIFORM: u8 = 0;
const ENCODING_PALETTE: u8 = 1;
const CONTENT_HASH_DOMAIN: &[u8] = b"voxels-vxch-content-v2\0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    SourceIdentityMismatch,
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
            Self::SourceIdentityMismatch => formatter.write_str("VXCH source identity mismatch"),
            Self::UnknownMaterial(id) => write!(formatter, "unknown material id {id}"),
            Self::CorruptHash => formatter.write_str("VXCH semantic content hash mismatch"),
        }
    }
}

impl std::error::Error for CodecError {}

pub fn encode_chunk(chunk: &Chunk, source_identity_hash: WorldSourceIdentityHash) -> Vec<u8> {
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
    let coord = chunk.coord();
    let hash = content_hash(chunk, source_identity_hash);
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
    encoded.extend_from_slice(source_identity_hash.as_bytes());
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

pub fn decode_chunk(
    bytes: &[u8],
    expected_source_identity_hash: WorldSourceIdentityHash,
) -> Result<Chunk, CodecError> {
    if bytes.len() < 8 {
        return Err(CodecError::Truncated);
    }
    if &bytes[0..4] != MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    let version = read_u16(bytes, 4)?;
    if version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    if bytes.len() < usize::from(HEADER_LEN) {
        return Err(CodecError::Truncated);
    }
    if read_u16(bytes, 6)? != HEADER_LEN {
        return Err(CodecError::InvalidHeader("unexpected header length"));
    }
    if bytes[8] != CHUNK_EDGE.trailing_zeros() as u8 {
        return Err(CodecError::InvalidHeader("unexpected chunk edge"));
    }
    let encoding = bytes[9];
    if bytes[28..60] != expected_source_identity_hash.as_bytes()[..] {
        return Err(CodecError::SourceIdentityMismatch);
    }
    if read_u16(bytes, 10)? != 0 || read_u16(bytes, 26)? != 0 || bytes[67] != 0 {
        return Err(CodecError::InvalidHeader("reserved fields must be zero"));
    }
    if read_u16(bytes, 24)? != Material::SCHEMA_VERSION {
        return Err(CodecError::InvalidHeader("material schema mismatch"));
    }
    if read_u32(bytes, 60)? as usize != CHUNK_VOLUME {
        return Err(CodecError::InvalidHeader("unexpected voxel count"));
    }
    let coord = ChunkCoord::new(
        read_i32(bytes, 12)?,
        read_i32(bytes, 16)?,
        read_i32(bytes, 20)?,
    );
    if !coord.is_world_representable() {
        return Err(CodecError::InvalidHeader(
            "chunk coordinate outside voxel grid",
        ));
    }
    let palette_len = usize::from(read_u16(bytes, 64)?);
    if palette_len == 0 || palette_len > Material::ALL.len() {
        return Err(CodecError::InvalidHeader("invalid palette length"));
    }
    let bits = bytes[66];
    let payload_len = read_u32(bytes, 68)? as usize;
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
    let chunk = Chunk::from_voxels(coord, voxels)
        .ok_or(CodecError::InvalidHeader("voxel count mismatch"))?;
    let expected_hash = &bytes[72..104];
    if content_hash(&chunk, expected_source_identity_hash).as_bytes() != expected_hash {
        return Err(CodecError::CorruptHash);
    }
    Ok(chunk)
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

fn content_hash(chunk: &Chunk, source_identity_hash: WorldSourceIdentityHash) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CONTENT_HASH_DOMAIN);
    hasher.update(source_identity_hash.as_bytes());
    let coord = chunk.coord();
    hasher.update(&coord.x.to_le_bytes());
    hasher.update(&coord.y.to_le_bytes());
    hasher.update(&coord.z.to_le_bytes());
    hasher.update(&Material::SCHEMA_VERSION.to_le_bytes());
    for material in chunk.voxels() {
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
    use crate::{
        ProceduralWorldSource, VoxelCoord, WorldProduct, WorldProductBatch, WorldProductPriority,
        WorldProductRequest, WorldSourceEngine, WorldSourceIdentity,
    };

    fn identity(seed: u64) -> WorldSourceIdentityHash {
        WorldSourceIdentity::procedural_v16(seed).identity_hash()
    }

    fn generated_chunk(seed: u64, coord: ChunkCoord) -> (WorldSourceIdentityHash, Chunk) {
        let source = ProceduralWorldSource::new(seed);
        let identity = source.source_identity_hash();
        let chunk = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleChunk,
                requests: vec![WorldProductRequest::ChunkWithHalo(coord)],
            })
            .expect("procedural chunk generation succeeds")
            .items
            .into_iter()
            .find_map(|item| match item.result {
                Ok(WorldProduct::Chunk(snapshot)) => Some(snapshot.chunk),
                Ok(_) | Err(_) => None,
            })
            .expect("batch contains requested chunk");
        (identity, chunk)
    }

    #[test]
    fn uniform_chunk_round_trips_compactly() {
        let chunk = Chunk::filled(ChunkCoord::new(-4, 2, 9), Material::Basalt);
        let identity = identity(7);
        let encoded = encode_chunk(&chunk, identity);
        assert_eq!(encoded.len(), usize::from(HEADER_LEN) + 2);
        assert_eq!(decode_chunk(&encoded, identity), Ok(chunk));
    }

    #[test]
    fn water_material_round_trips_through_the_versioned_palette() {
        let mut chunk = Chunk::empty(ChunkCoord::new(3, 0, -2));
        chunk.set(4, 10, 7, Material::Water);
        chunk.set(4, 9, 7, Material::Sand);
        let identity = identity(7);
        let encoded = encode_chunk(&chunk, identity);
        let decoded =
            decode_chunk(&encoded, identity).expect("valid current-version Water payload");
        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get(4, 10, 7), Material::Water);
    }

    #[test]
    fn glow_crystal_round_trips_as_an_ordinary_opaque_voxel() {
        let mut chunk = Chunk::empty(ChunkCoord::new(-162, 0, 103));
        chunk.set(4, 10, 7, Material::GlowCrystal);
        let identity = identity(7);
        let encoded = encode_chunk(&chunk, identity);
        let decoded =
            decode_chunk(&encoded, identity).expect("valid current-version crystal payload");
        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get(4, 10, 7), Material::GlowCrystal);
        assert_eq!(
            Material::GlowCrystal.render_layer(),
            crate::RenderLayer::Opaque
        );
    }

    #[test]
    fn generated_chunk_round_trips_through_palette_bits() {
        let (identity, chunk) = generated_chunk(42, ChunkCoord::new(0, 0, 0));
        let encoded = encode_chunk(&chunk, identity);
        assert_eq!(
            blake3::hash(&encoded).to_hex().to_string(),
            "589a4a71d72470d025d919528bbf29c883c093b582394b04830da500c88ac622"
        );
        assert_eq!(encoded.len(), 12_402);
        assert!(encoded.len() < CHUNK_VOLUME * size_of::<u16>());
        assert_eq!(decode_chunk(&encoded, identity), Ok(chunk));
    }

    #[test]
    fn corruption_is_detected() {
        let (identity, chunk) = generated_chunk(7, ChunkCoord::new(1, 0, 1));
        let mut encoded = encode_chunk(&chunk, identity);
        let last = encoded.len() - 1;
        encoded[last] ^= 0x40;
        assert!(decode_chunk(&encoded, identity).is_err());
    }

    #[test]
    fn malformed_lengths_are_rejected_before_allocation() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let identity = identity(7);
        let mut encoded = encode_chunk(&chunk, identity);
        let impossible_palette_len = Material::ALL.len() as u16 + 1;
        encoded[64..66].copy_from_slice(&impossible_palette_len.to_le_bytes());
        assert_eq!(
            decode_chunk(&encoded, identity),
            Err(CodecError::InvalidHeader("invalid palette length"))
        );
    }

    #[test]
    fn chunk_coordinates_outside_the_voxel_grid_are_rejected() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let identity = identity(7);
        let mut encoded = encode_chunk(&chunk, identity);
        let invalid = i32::MAX.div_euclid(CHUNK_EDGE as i32) + 1;
        encoded[12..16].copy_from_slice(&invalid.to_le_bytes());
        assert_eq!(
            decode_chunk(&encoded, identity),
            Err(CodecError::InvalidHeader(
                "chunk coordinate outside voxel grid"
            ))
        );
    }

    #[test]
    fn source_identity_mismatch_is_rejected_before_dynamic_lengths() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let encoded_identity = identity(7);
        let mut encoded = encode_chunk(&chunk, encoded_identity);
        encoded[64..66].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            decode_chunk(&encoded, identity(8)),
            Err(CodecError::SourceIdentityMismatch)
        );
    }

    #[test]
    fn source_identity_changes_the_envelope_for_identical_voxels() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let first = encode_chunk(&chunk, identity(7));
        let second = encode_chunk(&chunk, identity(8));
        assert_ne!(first, second);
    }

    #[test]
    fn coordinate_corruption_is_covered_by_the_content_hash() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let identity = identity(7);
        let mut encoded = encode_chunk(&chunk, identity);
        encoded[12..16].copy_from_slice(&1_i32.to_le_bytes());
        assert_eq!(
            decode_chunk(&encoded, identity),
            Err(CodecError::CorruptHash)
        );
    }

    #[test]
    fn every_material_id_round_trips_at_negative_coordinates() {
        let coord = ChunkCoord::new(-1, -1, -1);
        let mut chunk = Chunk::empty(coord);
        for (index, material) in Material::ALL.into_iter().enumerate() {
            chunk.set(index, 0, 0, material);
        }
        let identity = identity(7);
        let encoded = encode_chunk(&chunk, identity);
        assert_eq!(decode_chunk(&encoded, identity), Ok(chunk));
    }

    #[test]
    fn material_schema_and_legacy_v1_envelopes_are_rejected() {
        let chunk = Chunk::filled(ChunkCoord::new(0, 0, 0), Material::Stone);
        let identity = identity(7);
        let mut wrong_schema = encode_chunk(&chunk, identity);
        wrong_schema[24..26].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            decode_chunk(&wrong_schema, identity),
            Err(CodecError::InvalidHeader("material schema mismatch"))
        );

        let mut legacy_version = vec![0; 78];
        legacy_version[0..4].copy_from_slice(MAGIC);
        legacy_version[4..6].copy_from_slice(&1_u16.to_le_bytes());
        legacy_version[6..8].copy_from_slice(&76_u16.to_le_bytes());
        assert_eq!(
            decode_chunk(&legacy_version, identity),
            Err(CodecError::UnsupportedVersion(1))
        );
    }

    #[test]
    fn canonical_world_edge_chunks_round_trip() {
        let identity = identity(7);
        for coord in [
            VoxelCoord::new(i32::MIN, 0, 0).chunk(),
            VoxelCoord::new(i32::MAX, 0, 0).chunk(),
        ] {
            let chunk = Chunk::filled(coord, Material::Basalt);
            let encoded = encode_chunk(&chunk, identity);
            assert_eq!(decode_chunk(&encoded, identity), Ok(chunk));
        }
    }
}
