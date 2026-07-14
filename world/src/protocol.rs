//! Versioned transport-neutral wire envelopes for authoritative world products.
//!
//! WebSocket messages use this framing today. The same complete byte envelopes can later ride a
//! reliable WebTransport stream without changing request identity, source validation, or bulk
//! product codecs. Rust enum layout and serde output are deliberately not part of the contract.

use crate::{
    ChunkCoord, ChunkSnapshot, Material, MeshingHalo, ModelIdentity, SourceDeviceRequirement,
    SurfaceBounds, SurfaceLodLevel, SurfacePatch, SurfaceQuad, SurfaceRegion, SurfaceTileCoord,
    SurfaceTileMesh, SurfaceTileSnapshot, WaterPatch, WaterTileMesh, WorldId, WorldManifest,
    WorldProductPriority, WorldSourceError, WorldSourceIdentity, WorldSourceIdentityHash,
    WorldSourceKind, codec,
};
use std::fmt;

pub const PROTOCOL_MAGIC: &[u8; 4] = b"VXWP";
pub const PROTOCOL_VERSION: u16 = 2;
pub const FRAME_HEADER_BYTES: usize = 24;
pub const MAX_PROTOCOL_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNKS_PER_BATCH: usize = 256;
pub const MAX_SURFACE_TILES_PER_BATCH: usize = 32;
pub const MAX_PLAYER_NAME_BYTES: usize = 32;
const MAX_SURFACE_QUADS_PER_TILE: usize = 65_535;
const MAX_SURFACE_PATCHES_PER_TILE: usize = 64;
const SURFACE_SNAPSHOT_MAGIC: &[u8; 4] = b"VXST";
const SURFACE_SNAPSHOT_VERSION: u16 = 1;

const KIND_OPEN_WORLD: u16 = 1;
const KIND_WORLD_OPENED: u16 = 2;
const KIND_CHUNK_BATCH: u16 = 3;
const KIND_CHUNK_BATCH_RESULT: u16 = 4;
const KIND_CANCEL: u16 = 5;
const KIND_ERROR: u16 = 6;
const KIND_SURFACE_TILE_BATCH: u16 = 7;
const KIND_SURFACE_TILE_BATCH_RESULT: u16 = 8;
const FLAG_NONE: u16 = 0;
const RESERVED: u16 = 0;
const HALO_HASH_DOMAIN: &[u8] = b"voxels-wire-halo-v1\0";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorldCapabilities(u64);

impl WorldCapabilities {
    pub const CANONICAL_CHUNKS: Self = Self(1 << 0);
    pub const SURFACE_LOD: Self = Self(1 << 1);
    pub const SERVER_EDITS: Self = Self(1 << 2);
    pub const ENVIRONMENT: Self = Self(1 << 3);
    pub const SURFACE_SEARCH: Self = Self(1 << 4);
    pub const SKYLINE_LANDMARKS: Self = Self(1 << 5);
    pub const AUTHORED_ROUTES: Self = Self(1 << 6);
    pub const CINDER_VAULT: Self = Self(1 << 7);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, capability: Self) -> bool {
        self.0 & capability.0 == capability.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

macro_rules! opaque_uuid_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            pub fn from_uuid_str(value: &str) -> Option<Self> {
                parse_uuid_bytes(value).map(Self)
            }

            pub fn is_nil(self) -> bool {
                self.0 == [0; 16]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for (index, byte) in self.0.iter().enumerate() {
                    if matches!(index, 4 | 6 | 8 | 10) {
                        formatter.write_str("-")?;
                    }
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    };
}

opaque_uuid_id!(BrowserUserId);
opaque_uuid_id!(PlayerId);

/// Browser-local identity claim used until authenticated server accounts exist.
///
/// The opaque IDs are durable keys; `player_name` is only a bounded local label. A remote server
/// must authenticate the user and verify player ownership instead of trusting these client values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerIdentity {
    pub browser_user_id: BrowserUserId,
    pub player_id: PlayerId,
    pub player_name: String,
}

impl PlayerIdentity {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.browser_user_id.is_nil() {
            return Err("browser user id is nil");
        }
        if self.player_id.is_nil() {
            return Err("player id is nil");
        }
        if !valid_player_name(&self.player_name) {
            return Err("player name must be 1-32 lowercase ASCII letters, digits, '_' or '-'");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenWorld {
    pub max_in_flight_batches: u16,
    pub identity: PlayerIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint {
    pub x: i32,
    pub z: i32,
    pub height: i32,
    pub water_level: Option<i32>,
    pub material: Material,
    pub region: SurfaceRegion,
    pub moisture: f32,
    pub temperature: f32,
    pub ridge: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldOpened {
    pub manifest: WorldManifest,
    pub capabilities: WorldCapabilities,
    pub recommended_in_flight_batches: u16,
    pub identity: PlayerIdentity,
    pub spawn: SpawnPoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkBatchRequest {
    pub request_id: u64,
    pub priority: WorldProductPriority,
    pub coords: Vec<ChunkCoord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkBatchItem {
    pub coord: ChunkCoord,
    pub result: Result<ChunkSnapshot, WorldSourceError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkBatchResult {
    pub request_id: u64,
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<ChunkBatchItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileBatchRequest {
    pub request_id: u64,
    pub priority: WorldProductPriority,
    /// Ordered coarse-to-fine by the caller. Each tile remains independently useful and
    /// cancellable; a later finer level replaces geometry rather than changing world truth.
    pub coords: Vec<SurfaceTileCoord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileBatchItem {
    pub coord: SurfaceTileCoord,
    pub result: Result<SurfaceTileSnapshot, WorldSourceError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileBatchResult {
    pub request_id: u64,
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<SurfaceTileBatchItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u16),
    UnexpectedMessageKind(u16),
    InvalidHeader(&'static str),
    InvalidPayload(&'static str),
    InvalidUtf8,
    UnknownEnum(&'static str, u64),
    LimitExceeded(&'static str),
    ChunkCodec(codec::CodecError),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated VXWP frame"),
            Self::InvalidMagic => formatter.write_str("invalid VXWP magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported VXWP version {version}")
            }
            Self::UnexpectedMessageKind(kind) => {
                write!(formatter, "unexpected VXWP message kind {kind}")
            }
            Self::InvalidHeader(reason) => write!(formatter, "invalid VXWP header: {reason}"),
            Self::InvalidPayload(reason) => write!(formatter, "invalid VXWP payload: {reason}"),
            Self::InvalidUtf8 => formatter.write_str("invalid UTF-8 in VXWP payload"),
            Self::UnknownEnum(name, value) => write!(formatter, "unknown {name} value {value}"),
            Self::LimitExceeded(limit) => write!(formatter, "VXWP {limit} limit exceeded"),
            Self::ChunkCodec(error) => write!(formatter, "VXWP chunk payload: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<codec::CodecError> for ProtocolError {
    fn from(error: codec::CodecError) -> Self {
        Self::ChunkCodec(error)
    }
}

struct Frame<'a> {
    kind: u16,
    request_id: u64,
    payload: &'a [u8],
}

pub fn encode_open_world(open: &OpenWorld) -> Result<Vec<u8>, ProtocolError> {
    if open.max_in_flight_batches == 0 {
        return Err(ProtocolError::InvalidPayload(
            "client requested a zero request window",
        ));
    }
    validate_player_identity(&open.identity)?;
    let mut payload = Vec::with_capacity(52 + open.identity.player_name.len());
    push_u16(&mut payload, open.max_in_flight_batches);
    payload.extend_from_slice(open.identity.browser_user_id.as_bytes());
    payload.extend_from_slice(open.identity.player_id.as_bytes());
    push_string(&mut payload, &open.identity.player_name);
    Ok(encode_frame(KIND_OPEN_WORLD, 0, &payload))
}

pub fn decode_open_world(bytes: &[u8]) -> Result<OpenWorld, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_OPEN_WORLD)?;
    if frame.request_id != 0 {
        return Err(ProtocolError::InvalidPayload("invalid OpenWorld body"));
    }
    let mut cursor = Cursor::new(frame.payload);
    let max_in_flight_batches = cursor.u16()?;
    if max_in_flight_batches == 0 {
        return Err(ProtocolError::InvalidPayload(
            "client requested a zero request window",
        ));
    }
    let identity = PlayerIdentity {
        browser_user_id: BrowserUserId::from_bytes(cursor.array()?),
        player_id: PlayerId::from_bytes(cursor.array()?),
        player_name: cursor.string()?,
    };
    validate_player_identity(&identity)?;
    cursor.finish()?;
    Ok(OpenWorld {
        max_in_flight_batches,
        identity,
    })
}

pub fn encode_world_opened(opened: &WorldOpened) -> Vec<u8> {
    let manifest = encode_manifest(&opened.manifest);
    let mut payload = Vec::with_capacity(manifest.len() + 64);
    push_u32(&mut payload, manifest.len() as u32);
    payload.extend_from_slice(&manifest);
    push_u64(&mut payload, opened.capabilities.bits());
    push_u16(&mut payload, opened.recommended_in_flight_batches);
    payload.extend_from_slice(opened.identity.browser_user_id.as_bytes());
    payload.extend_from_slice(opened.identity.player_id.as_bytes());
    push_string(&mut payload, &opened.identity.player_name);
    push_i32(&mut payload, opened.spawn.x);
    push_i32(&mut payload, opened.spawn.z);
    push_i32(&mut payload, opened.spawn.height);
    push_i32(&mut payload, opened.spawn.water_level.unwrap_or(i32::MIN));
    push_u16(&mut payload, opened.spawn.material.id());
    payload.push(surface_region_id(opened.spawn.region));
    payload.push(0);
    push_f32(&mut payload, opened.spawn.moisture);
    push_f32(&mut payload, opened.spawn.temperature);
    push_f32(&mut payload, opened.spawn.ridge);
    encode_frame(KIND_WORLD_OPENED, 0, &payload)
}

pub fn decode_world_opened(bytes: &[u8]) -> Result<WorldOpened, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_WORLD_OPENED)?;
    if frame.request_id != 0 {
        return Err(ProtocolError::InvalidPayload(
            "WorldOpened request id must be zero",
        ));
    }
    let mut cursor = Cursor::new(frame.payload);
    let manifest_len = cursor.u32()? as usize;
    let manifest = decode_manifest(cursor.bytes(manifest_len)?)?;
    let capabilities = WorldCapabilities::from_bits(cursor.u64()?);
    let recommended_in_flight_batches = cursor.u16()?;
    if recommended_in_flight_batches == 0 {
        return Err(ProtocolError::InvalidPayload(
            "server recommended a zero request window",
        ));
    }
    let identity = PlayerIdentity {
        browser_user_id: BrowserUserId::from_bytes(cursor.array()?),
        player_id: PlayerId::from_bytes(cursor.array()?),
        player_name: cursor.string()?,
    };
    validate_player_identity(&identity)?;
    let x = cursor.i32()?;
    let z = cursor.i32()?;
    let height = cursor.i32()?;
    let water = cursor.i32()?;
    let material_id = cursor.u16()?;
    let material = Material::from_id(material_id).ok_or(ProtocolError::UnknownEnum(
        "material",
        u64::from(material_id),
    ))?;
    let region = decode_surface_region(cursor.u8()?)?;
    if cursor.u8()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved spawn byte is nonzero",
        ));
    }
    let moisture = cursor.f32()?;
    let temperature = cursor.f32()?;
    let ridge = cursor.f32()?;
    if ![moisture, temperature, ridge]
        .into_iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    {
        return Err(ProtocolError::InvalidPayload(
            "spawn fields are not finite and normalized",
        ));
    }
    cursor.finish()?;
    Ok(WorldOpened {
        manifest,
        capabilities,
        recommended_in_flight_batches,
        identity,
        spawn: SpawnPoint {
            x,
            z,
            height,
            water_level: (water != i32::MIN).then_some(water),
            material,
            region,
            moisture,
            temperature,
            ridge,
        },
    })
}

pub fn encode_chunk_batch(request: &ChunkBatchRequest) -> Result<Vec<u8>, ProtocolError> {
    if request.coords.is_empty() || request.coords.len() > MAX_CHUNKS_PER_BATCH {
        return Err(ProtocolError::LimitExceeded("chunk batch"));
    }
    let mut payload = Vec::with_capacity(4 + request.coords.len() * 12);
    payload.push(request.priority as u8);
    payload.push(0);
    push_u16(&mut payload, request.coords.len() as u16);
    for (index, coord) in request.coords.iter().enumerate() {
        if !coord.is_world_representable() {
            return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
        }
        if request.coords[..index].contains(coord) {
            return Err(ProtocolError::InvalidPayload("duplicate chunk coordinate"));
        }
        push_i32(&mut payload, coord.x);
        push_i32(&mut payload, coord.y);
        push_i32(&mut payload, coord.z);
    }
    Ok(encode_frame(KIND_CHUNK_BATCH, request.request_id, &payload))
}

pub fn decode_chunk_batch(bytes: &[u8]) -> Result<ChunkBatchRequest, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_CHUNK_BATCH)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    let mut cursor = Cursor::new(frame.payload);
    let priority = decode_priority(cursor.u8()?)?;
    if cursor.u8()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved request byte is nonzero",
        ));
    }
    let count = usize::from(cursor.u16()?);
    if count == 0 || count > MAX_CHUNKS_PER_BATCH {
        return Err(ProtocolError::LimitExceeded("chunk batch"));
    }
    let mut coords = Vec::with_capacity(count);
    for _ in 0..count {
        let coord = ChunkCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?);
        if !coord.is_world_representable() {
            return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
        }
        if coords.contains(&coord) {
            return Err(ProtocolError::InvalidPayload("duplicate chunk coordinate"));
        }
        coords.push(coord);
    }
    cursor.finish()?;
    Ok(ChunkBatchRequest {
        request_id: frame.request_id,
        priority,
        coords,
    })
}

pub fn encode_chunk_batch_result(result: &ChunkBatchResult) -> Result<Vec<u8>, ProtocolError> {
    if result.request_id == 0
        || result.items.is_empty()
        || result.items.len() > MAX_CHUNKS_PER_BATCH
    {
        return Err(ProtocolError::LimitExceeded("chunk result batch"));
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(result.source_identity_hash.as_bytes());
    push_u16(&mut payload, result.items.len() as u16);
    push_u16(&mut payload, 0);
    for item in &result.items {
        push_i32(&mut payload, item.coord.x);
        push_i32(&mut payload, item.coord.y);
        push_i32(&mut payload, item.coord.z);
        match &item.result {
            Ok(snapshot) => {
                if snapshot.chunk.coord() != item.coord
                    || snapshot.meshing_halo.coord() != item.coord
                    || snapshot.source_identity_hash != result.source_identity_hash
                {
                    return Err(ProtocolError::InvalidPayload(
                        "chunk result key or identity mismatch",
                    ));
                }
                push_u16(&mut payload, 0);
                push_u16(&mut payload, 0);
                let encoded = encode_chunk_snapshot(snapshot);
                push_u32(&mut payload, encoded.len() as u32);
                payload.extend_from_slice(&encoded);
            }
            Err(error) => {
                push_u16(&mut payload, encode_world_source_error(*error));
                push_u16(&mut payload, 0);
                push_u32(&mut payload, 0);
            }
        }
    }
    if payload.len() + FRAME_HEADER_BYTES > MAX_PROTOCOL_FRAME_BYTES {
        return Err(ProtocolError::LimitExceeded("frame bytes"));
    }
    Ok(encode_frame(
        KIND_CHUNK_BATCH_RESULT,
        result.request_id,
        &payload,
    ))
}

pub fn decode_chunk_batch_result(bytes: &[u8]) -> Result<ChunkBatchResult, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_CHUNK_BATCH_RESULT)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    let mut cursor = Cursor::new(frame.payload);
    let source_identity_hash = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    let count = usize::from(cursor.u16()?);
    if count == 0 || count > MAX_CHUNKS_PER_BATCH || cursor.u16()? != 0 {
        return Err(ProtocolError::InvalidPayload("invalid result item count"));
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let coord = ChunkCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?);
        let status = cursor.u16()?;
        if cursor.u16()? != 0 {
            return Err(ProtocolError::InvalidPayload(
                "reserved result field is nonzero",
            ));
        }
        let len = cursor.u32()? as usize;
        let body = cursor.bytes(len)?;
        let result = if status == 0 {
            let snapshot = decode_chunk_snapshot(body, source_identity_hash)?;
            if snapshot.chunk.coord() != coord || snapshot.meshing_halo.coord() != coord {
                return Err(ProtocolError::InvalidPayload("chunk result key mismatch"));
            }
            Ok(snapshot)
        } else {
            if !body.is_empty() {
                return Err(ProtocolError::InvalidPayload("error result has a payload"));
            }
            Err(decode_world_source_error(status)?)
        };
        items.push(ChunkBatchItem { coord, result });
    }
    cursor.finish()?;
    Ok(ChunkBatchResult {
        request_id: frame.request_id,
        source_identity_hash,
        items,
    })
}

pub fn encode_surface_tile_batch(
    request: &SurfaceTileBatchRequest,
) -> Result<Vec<u8>, ProtocolError> {
    if request.request_id == 0
        || request.coords.is_empty()
        || request.coords.len() > MAX_SURFACE_TILES_PER_BATCH
    {
        return Err(ProtocolError::LimitExceeded("surface tile batch"));
    }
    let mut payload = Vec::with_capacity(4 + request.coords.len() * 12);
    payload.push(request.priority as u8);
    payload.push(0);
    push_u16(&mut payload, request.coords.len() as u16);
    for (index, coord) in request.coords.iter().enumerate() {
        if !coord.is_world_representable() {
            return Err(ProtocolError::InvalidPayload(
                "invalid surface tile coordinate",
            ));
        }
        if request.coords[..index].contains(coord) {
            return Err(ProtocolError::InvalidPayload(
                "duplicate surface tile coordinate",
            ));
        }
        payload.push(coord.level.index());
        payload.extend_from_slice(&[0; 3]);
        push_i32(&mut payload, coord.x);
        push_i32(&mut payload, coord.z);
    }
    Ok(encode_frame(
        KIND_SURFACE_TILE_BATCH,
        request.request_id,
        &payload,
    ))
}

pub fn decode_surface_tile_batch(bytes: &[u8]) -> Result<SurfaceTileBatchRequest, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_SURFACE_TILE_BATCH)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    let mut cursor = Cursor::new(frame.payload);
    let priority = decode_priority(cursor.u8()?)?;
    if cursor.u8()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved request byte is nonzero",
        ));
    }
    let count = usize::from(cursor.u16()?);
    if count == 0 || count > MAX_SURFACE_TILES_PER_BATCH {
        return Err(ProtocolError::LimitExceeded("surface tile batch"));
    }
    let mut coords = Vec::with_capacity(count);
    for _ in 0..count {
        let level = decode_surface_lod(cursor.u8()?)?;
        if cursor.bytes(3)? != [0; 3] {
            return Err(ProtocolError::InvalidPayload(
                "reserved surface coordinate bytes are nonzero",
            ));
        }
        let coord = SurfaceTileCoord::new(level, cursor.i32()?, cursor.i32()?);
        if !coord.is_world_representable() {
            return Err(ProtocolError::InvalidPayload(
                "invalid surface tile coordinate",
            ));
        }
        if coords.contains(&coord) {
            return Err(ProtocolError::InvalidPayload(
                "duplicate surface tile coordinate",
            ));
        }
        coords.push(coord);
    }
    cursor.finish()?;
    Ok(SurfaceTileBatchRequest {
        request_id: frame.request_id,
        priority,
        coords,
    })
}

pub fn encode_surface_tile_batch_result(
    result: &SurfaceTileBatchResult,
) -> Result<Vec<u8>, ProtocolError> {
    if result.request_id == 0
        || result.items.is_empty()
        || result.items.len() > MAX_SURFACE_TILES_PER_BATCH
    {
        return Err(ProtocolError::LimitExceeded("surface tile result batch"));
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(result.source_identity_hash.as_bytes());
    push_u16(&mut payload, result.items.len() as u16);
    push_u16(&mut payload, 0);
    for item in &result.items {
        encode_surface_coord(&mut payload, item.coord);
        match &item.result {
            Ok(snapshot) => {
                if snapshot.source_identity_hash != result.source_identity_hash
                    || snapshot.terrain.coord != item.coord
                    || snapshot.water.coord != item.coord
                {
                    return Err(ProtocolError::InvalidPayload(
                        "surface result key or identity mismatch",
                    ));
                }
                push_u16(&mut payload, 0);
                push_u16(&mut payload, 0);
                let encoded = encode_surface_snapshot(snapshot)?;
                push_u32(&mut payload, encoded.len() as u32);
                payload.extend_from_slice(&encoded);
            }
            Err(error) => {
                push_u16(&mut payload, encode_world_source_error(*error));
                push_u16(&mut payload, 0);
                push_u32(&mut payload, 0);
            }
        }
    }
    if payload.len() + FRAME_HEADER_BYTES > MAX_PROTOCOL_FRAME_BYTES {
        return Err(ProtocolError::LimitExceeded("frame bytes"));
    }
    Ok(encode_frame(
        KIND_SURFACE_TILE_BATCH_RESULT,
        result.request_id,
        &payload,
    ))
}

pub fn decode_surface_tile_batch_result(
    bytes: &[u8],
) -> Result<SurfaceTileBatchResult, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_SURFACE_TILE_BATCH_RESULT)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    let mut cursor = Cursor::new(frame.payload);
    let source_identity_hash = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    let count = usize::from(cursor.u16()?);
    if count == 0 || count > MAX_SURFACE_TILES_PER_BATCH || cursor.u16()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface result item count",
        ));
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let coord = decode_surface_coord(&mut cursor)?;
        let status = cursor.u16()?;
        if cursor.u16()? != 0 {
            return Err(ProtocolError::InvalidPayload(
                "reserved surface result field is nonzero",
            ));
        }
        let len = cursor.u32()? as usize;
        let body = cursor.bytes(len)?;
        let result = if status == 0 {
            let snapshot = decode_surface_snapshot(body, coord, source_identity_hash)?;
            Ok(snapshot)
        } else {
            if !body.is_empty() {
                return Err(ProtocolError::InvalidPayload(
                    "surface error result has a payload",
                ));
            }
            Err(decode_world_source_error(status)?)
        };
        items.push(SurfaceTileBatchItem { coord, result });
    }
    cursor.finish()?;
    Ok(SurfaceTileBatchResult {
        request_id: frame.request_id,
        source_identity_hash,
        items,
    })
}

pub fn encode_cancel(request_id: u64) -> Result<Vec<u8>, ProtocolError> {
    if request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    Ok(encode_frame(KIND_CANCEL, request_id, &[]))
}

pub fn decode_cancel(bytes: &[u8]) -> Result<u64, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_CANCEL)?;
    if frame.request_id == 0 || !frame.payload.is_empty() {
        return Err(ProtocolError::InvalidPayload("invalid Cancel body"));
    }
    Ok(frame.request_id)
}

pub fn encode_error(request_id: u64, message: &str) -> Vec<u8> {
    let bytes = message.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    let mut payload = Vec::with_capacity(len + 2);
    push_u16(&mut payload, len as u16);
    payload.extend_from_slice(&bytes[..len]);
    encode_frame(KIND_ERROR, request_id, &payload)
}

pub fn decode_error(bytes: &[u8]) -> Result<(u64, String), ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_ERROR)?;
    let mut cursor = Cursor::new(frame.payload);
    let len = usize::from(cursor.u16()?);
    let message = std::str::from_utf8(cursor.bytes(len)?)
        .map_err(|_| ProtocolError::InvalidUtf8)?
        .to_owned();
    cursor.finish()?;
    Ok((frame.request_id, message))
}

pub fn message_kind(bytes: &[u8]) -> Result<u16, ProtocolError> {
    Ok(decode_frame(bytes)?.kind)
}

pub const fn open_world_kind() -> u16 {
    KIND_OPEN_WORLD
}

pub const fn chunk_batch_kind() -> u16 {
    KIND_CHUNK_BATCH
}

pub const fn cancel_kind() -> u16 {
    KIND_CANCEL
}

pub const fn chunk_batch_result_kind() -> u16 {
    KIND_CHUNK_BATCH_RESULT
}

pub const fn world_opened_kind() -> u16 {
    KIND_WORLD_OPENED
}

pub const fn error_kind() -> u16 {
    KIND_ERROR
}

pub const fn surface_tile_batch_kind() -> u16 {
    KIND_SURFACE_TILE_BATCH
}

pub const fn surface_tile_batch_result_kind() -> u16 {
    KIND_SURFACE_TILE_BATCH_RESULT
}

fn validate_player_identity(identity: &PlayerIdentity) -> Result<(), ProtocolError> {
    identity.validate().map_err(ProtocolError::InvalidPayload)
}

fn valid_player_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PLAYER_NAME_BYTES
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn parse_uuid_bytes(value: &str) -> Option<[u8; 16]> {
    if value.len() != 36 {
        return None;
    }
    let bytes = value.as_bytes();
    if [8, 13, 18, 23]
        .into_iter()
        .any(|index| bytes[index] != b'-')
    {
        return None;
    }
    let mut parsed = [0_u8; 16];
    let mut output = 0;
    let mut index = 0;
    while index < bytes.len() {
        if matches!(index, 8 | 13 | 18 | 23) {
            index += 1;
            continue;
        }
        let high = hex_nibble(bytes[index])?;
        let low = hex_nibble(bytes[index + 1])?;
        parsed[output] = (high << 4) | low;
        output += 1;
        index += 2;
    }
    (output == parsed.len()).then_some(parsed)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_frame(kind: u16, request_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(PROTOCOL_MAGIC);
    push_u16(&mut frame, PROTOCOL_VERSION);
    push_u16(&mut frame, kind);
    push_u16(&mut frame, FLAG_NONE);
    push_u16(&mut frame, RESERVED);
    push_u64(&mut frame, request_id);
    push_u32(&mut frame, payload.len() as u32);
    frame.extend_from_slice(payload);
    frame
}

fn decode_frame(bytes: &[u8]) -> Result<Frame<'_>, ProtocolError> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return Err(ProtocolError::Truncated);
    }
    if &bytes[..4] != PROTOCOL_MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    let version = read_u16(bytes, 4)?;
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(version));
    }
    if read_u16(bytes, 8)? != FLAG_NONE || read_u16(bytes, 10)? != RESERVED {
        return Err(ProtocolError::InvalidHeader(
            "flags or reserved bits are nonzero",
        ));
    }
    let payload_len = read_u32(bytes, 20)? as usize;
    let expected = FRAME_HEADER_BYTES
        .checked_add(payload_len)
        .ok_or(ProtocolError::LimitExceeded("frame bytes"))?;
    if expected > MAX_PROTOCOL_FRAME_BYTES {
        return Err(ProtocolError::LimitExceeded("frame bytes"));
    }
    if bytes.len() < expected {
        return Err(ProtocolError::Truncated);
    }
    if bytes.len() != expected {
        return Err(ProtocolError::InvalidHeader("trailing bytes"));
    }
    Ok(Frame {
        kind: read_u16(bytes, 6)?,
        request_id: read_u64(bytes, 12)?,
        payload: &bytes[FRAME_HEADER_BYTES..],
    })
}

fn expect_kind(frame: &Frame<'_>, expected: u16) -> Result<(), ProtocolError> {
    if frame.kind != expected {
        return Err(ProtocolError::UnexpectedMessageKind(frame.kind));
    }
    Ok(())
}

fn encode_manifest(manifest: &WorldManifest) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(manifest.world_id.as_bytes());
    push_u64(&mut bytes, manifest.seed);
    push_u32(&mut bytes, manifest.world_schema_version);
    push_u16(&mut bytes, manifest.material_schema_version);
    let source = &manifest.source;
    bytes.push(source.source_kind as u8);
    bytes.push(source.device_requirement as u8);
    push_u32(&mut bytes, source.implementation_version);
    bytes.extend_from_slice(source.configuration_hash.as_bytes());
    match &source.model {
        Some(model) => {
            bytes.push(1);
            push_string(&mut bytes, &model.repository);
            push_string(&mut bytes, &model.immutable_revision);
            push_u16(&mut bytes, model.weight_hashes.len() as u16);
            for hash in &model.weight_hashes {
                bytes.extend_from_slice(hash.as_bytes());
            }
        }
        None => bytes.push(0),
    }
    push_u32(&mut bytes, source.sampler_version);
    push_u32(&mut bytes, source.scheduler_version);
    push_u32(&mut bytes, source.macro_field_schema_version);
    for origin in source.macro_coordinate_transform.origin_voxels {
        push_i64(&mut bytes, origin);
    }
    push_u32(
        &mut bytes,
        source
            .macro_coordinate_transform
            .horizontal_unit_millimetres,
    );
    bytes.push(source.macro_coordinate_transform.x_axis_sign as u8);
    bytes.push(source.macro_coordinate_transform.z_axis_sign as u8);
    push_u32(&mut bytes, source.voxel_composer_version);
    push_u32(&mut bytes, source.authored_content_version);
    bytes
}

fn decode_manifest(bytes: &[u8]) -> Result<WorldManifest, ProtocolError> {
    let mut cursor = Cursor::new(bytes);
    let world_id = WorldId::from_bytes(cursor.array()?);
    let seed = cursor.u64()?;
    let world_schema_version = cursor.u32()?;
    let material_schema_version = cursor.u16()?;
    let source_kind = decode_source_kind(cursor.u8()?)?;
    let device_requirement = decode_device_requirement(cursor.u8()?)?;
    let implementation_version = cursor.u32()?;
    let configuration_hash = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    let model = match cursor.u8()? {
        0 => None,
        1 => {
            let repository = cursor.string()?;
            let immutable_revision = cursor.string()?;
            let count = usize::from(cursor.u16()?);
            if count == 0 || count > 64 {
                return Err(ProtocolError::LimitExceeded("model hash count"));
            }
            let mut weight_hashes = Vec::with_capacity(count);
            for _ in 0..count {
                weight_hashes.push(WorldSourceIdentityHash::from_bytes(cursor.array()?));
            }
            Some(ModelIdentity {
                repository,
                immutable_revision,
                weight_hashes,
            })
        }
        value => {
            return Err(ProtocolError::UnknownEnum(
                "model presence",
                u64::from(value),
            ));
        }
    };
    let sampler_version = cursor.u32()?;
    let scheduler_version = cursor.u32()?;
    let macro_field_schema_version = cursor.u32()?;
    let origin_voxels = [cursor.i64()?, cursor.i64()?];
    let horizontal_unit_millimetres = cursor.u32()?;
    let x_axis_sign = cursor.u8()? as i8;
    let z_axis_sign = cursor.u8()? as i8;
    let voxel_composer_version = cursor.u32()?;
    let authored_content_version = cursor.u32()?;
    cursor.finish()?;
    let manifest = WorldManifest {
        world_id,
        seed,
        world_schema_version,
        material_schema_version,
        source: WorldSourceIdentity {
            source_kind,
            implementation_version,
            configuration_hash,
            model,
            sampler_version,
            scheduler_version,
            macro_field_schema_version,
            macro_coordinate_transform: crate::MacroCoordinateTransform {
                origin_voxels,
                horizontal_unit_millimetres,
                x_axis_sign,
                z_axis_sign,
            },
            voxel_composer_version,
            authored_content_version,
            device_requirement,
        },
    };
    manifest
        .validate()
        .map_err(|_| ProtocolError::InvalidPayload("manifest validation failed"))?;
    Ok(manifest)
}

fn encode_surface_coord(output: &mut Vec<u8>, coord: SurfaceTileCoord) {
    output.push(coord.level.index());
    output.extend_from_slice(&[0; 3]);
    push_i32(output, coord.x);
    push_i32(output, coord.z);
}

fn decode_surface_coord(cursor: &mut Cursor<'_>) -> Result<SurfaceTileCoord, ProtocolError> {
    let level = decode_surface_lod(cursor.u8()?)?;
    if cursor.bytes(3)? != [0; 3] {
        return Err(ProtocolError::InvalidPayload(
            "reserved surface coordinate bytes are nonzero",
        ));
    }
    let coord = SurfaceTileCoord::new(level, cursor.i32()?, cursor.i32()?);
    if !coord.is_world_representable() {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface tile coordinate",
        ));
    }
    Ok(coord)
}

fn encode_surface_snapshot(snapshot: &SurfaceTileSnapshot) -> Result<Vec<u8>, ProtocolError> {
    validate_surface_mesh(&snapshot.terrain)?;
    validate_water_mesh(&snapshot.water)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(SURFACE_SNAPSHOT_MAGIC);
    push_u16(&mut bytes, SURFACE_SNAPSHOT_VERSION);
    push_u16(&mut bytes, 0);
    encode_surface_mesh(&mut bytes, &snapshot.terrain);
    encode_water_mesh(&mut bytes, &snapshot.water);
    Ok(bytes)
}

fn decode_surface_snapshot(
    bytes: &[u8],
    coord: SurfaceTileCoord,
    source_identity_hash: WorldSourceIdentityHash,
) -> Result<SurfaceTileSnapshot, ProtocolError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.bytes(4)? != SURFACE_SNAPSHOT_MAGIC {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface snapshot magic",
        ));
    }
    let version = cursor.u16()?;
    if version != SURFACE_SNAPSHOT_VERSION {
        return Err(ProtocolError::UnsupportedVersion(version));
    }
    if cursor.u16()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved surface snapshot field is nonzero",
        ));
    }
    let terrain = decode_surface_mesh(&mut cursor, coord)?;
    let water = decode_water_mesh(&mut cursor, coord)?;
    cursor.finish()?;
    Ok(SurfaceTileSnapshot {
        source_identity_hash,
        terrain,
        water,
    })
}

fn encode_surface_mesh(output: &mut Vec<u8>, mesh: &SurfaceTileMesh) {
    push_u32(output, mesh.quads.len() as u32);
    push_u16(output, mesh.patches.len() as u16);
    push_u16(output, 0);
    for quad in &mesh.quads {
        encode_surface_quad(output, *quad);
    }
    for patch in &mesh.patches {
        output.extend_from_slice(&[
            patch.cell_bounds[0][0],
            patch.cell_bounds[0][1],
            patch.cell_bounds[1][0],
            patch.cell_bounds[1][1],
        ]);
        encode_range(output, &patch.quad_range);
        for range in &patch.skirt_ranges {
            encode_range(output, range);
        }
        encode_surface_bounds(output, patch.bounds);
    }
}

fn decode_surface_mesh(
    cursor: &mut Cursor<'_>,
    coord: SurfaceTileCoord,
) -> Result<SurfaceTileMesh, ProtocolError> {
    let quad_count = cursor.u32()? as usize;
    let patch_count = usize::from(cursor.u16()?);
    if quad_count > MAX_SURFACE_QUADS_PER_TILE
        || patch_count == 0
        || patch_count > MAX_SURFACE_PATCHES_PER_TILE
        || cursor.u16()? != 0
    {
        return Err(ProtocolError::LimitExceeded("surface mesh geometry"));
    }
    let mut quads = Vec::with_capacity(quad_count);
    for _ in 0..quad_count {
        quads.push(decode_surface_quad(cursor)?);
    }
    let mut patches = Vec::with_capacity(patch_count);
    for _ in 0..patch_count {
        let cell_bounds = [[cursor.u8()?, cursor.u8()?], [cursor.u8()?, cursor.u8()?]];
        let quad_range = decode_range(cursor)?;
        let skirt_ranges = [
            decode_range(cursor)?,
            decode_range(cursor)?,
            decode_range(cursor)?,
            decode_range(cursor)?,
        ];
        let bounds = decode_surface_bounds(cursor)?;
        patches.push(SurfacePatch {
            cell_bounds,
            quad_range,
            skirt_ranges,
            bounds,
        });
    }
    let mesh = SurfaceTileMesh {
        coord,
        quads,
        patches,
    };
    validate_surface_mesh(&mesh)?;
    Ok(mesh)
}

fn encode_water_mesh(output: &mut Vec<u8>, mesh: &WaterTileMesh) {
    push_u32(output, mesh.quads.len() as u32);
    push_u16(output, mesh.patches.len() as u16);
    push_u16(output, 0);
    for quad in &mesh.quads {
        encode_surface_quad(output, *quad);
    }
    for patch in &mesh.patches {
        output.extend_from_slice(&[
            patch.cell_bounds[0][0],
            patch.cell_bounds[0][1],
            patch.cell_bounds[1][0],
            patch.cell_bounds[1][1],
        ]);
        encode_range(output, &patch.quad_range);
        encode_surface_bounds(output, patch.bounds);
    }
}

fn decode_water_mesh(
    cursor: &mut Cursor<'_>,
    coord: SurfaceTileCoord,
) -> Result<WaterTileMesh, ProtocolError> {
    let quad_count = cursor.u32()? as usize;
    let patch_count = usize::from(cursor.u16()?);
    if quad_count > MAX_SURFACE_QUADS_PER_TILE
        || patch_count > MAX_SURFACE_PATCHES_PER_TILE
        || cursor.u16()? != 0
    {
        return Err(ProtocolError::LimitExceeded("water mesh geometry"));
    }
    let mut quads = Vec::with_capacity(quad_count);
    for _ in 0..quad_count {
        quads.push(decode_surface_quad(cursor)?);
    }
    let mut patches = Vec::with_capacity(patch_count);
    for _ in 0..patch_count {
        let cell_bounds = [[cursor.u8()?, cursor.u8()?], [cursor.u8()?, cursor.u8()?]];
        let quad_range = decode_range(cursor)?;
        let bounds = decode_surface_bounds(cursor)?;
        patches.push(WaterPatch {
            cell_bounds,
            quad_range,
            bounds,
        });
    }
    let mesh = WaterTileMesh {
        coord,
        quads,
        patches,
    };
    validate_water_mesh(&mesh)?;
    Ok(mesh)
}

fn encode_surface_quad(output: &mut Vec<u8>, quad: SurfaceQuad) {
    for value in quad.origin {
        push_i32(output, value);
    }
    output.push(quad.face);
    output.push(0);
    push_u16(output, quad.extent[0]);
    push_u16(output, quad.extent[1]);
    push_u16(output, quad.material.id());
}

fn decode_surface_quad(cursor: &mut Cursor<'_>) -> Result<SurfaceQuad, ProtocolError> {
    let origin = [cursor.i32()?, cursor.i32()?, cursor.i32()?];
    let face = cursor.u8()?;
    if face > 5 || cursor.u8()? != 0 {
        return Err(ProtocolError::InvalidPayload("invalid surface quad face"));
    }
    let extent = [cursor.u16()?, cursor.u16()?];
    if extent.contains(&0) {
        return Err(ProtocolError::InvalidPayload("empty surface quad"));
    }
    let material_id = cursor.u16()?;
    let material = Material::from_id(material_id).ok_or(ProtocolError::UnknownEnum(
        "material",
        u64::from(material_id),
    ))?;
    Ok(SurfaceQuad {
        origin,
        face,
        extent,
        material,
    })
}

fn encode_range(output: &mut Vec<u8>, range: &std::ops::Range<u32>) {
    push_u32(output, range.start);
    push_u32(output, range.end);
}

fn decode_range(cursor: &mut Cursor<'_>) -> Result<std::ops::Range<u32>, ProtocolError> {
    Ok(cursor.u32()?..cursor.u32()?)
}

fn encode_surface_bounds(output: &mut Vec<u8>, bounds: SurfaceBounds) {
    for value in bounds.min {
        push_i32(output, value);
    }
    for value in bounds.max {
        push_i32(output, value);
    }
}

fn decode_surface_bounds(cursor: &mut Cursor<'_>) -> Result<SurfaceBounds, ProtocolError> {
    let bounds = SurfaceBounds {
        min: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
        max: [cursor.i32()?, cursor.i32()?, cursor.i32()?],
    };
    if (0..3).any(|axis| bounds.min[axis] >= bounds.max[axis]) {
        return Err(ProtocolError::InvalidPayload("invalid surface bounds"));
    }
    Ok(bounds)
}

fn validate_cell_bounds(bounds: [[u8; 2]; 2]) -> Result<(), ProtocolError> {
    let edge = crate::SURFACE_TILE_EDGE_CELLS as u8;
    if bounds[0][0] >= bounds[1][0]
        || bounds[0][1] >= bounds[1][1]
        || bounds[1][0] > edge
        || bounds[1][1] > edge
    {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface patch cell bounds",
        ));
    }
    Ok(())
}

fn validate_range(range: &std::ops::Range<u32>, quad_count: usize) -> Result<(), ProtocolError> {
    if range.start > range.end || range.end as usize > quad_count {
        return Err(ProtocolError::InvalidPayload(
            "surface patch range is out of bounds",
        ));
    }
    Ok(())
}

fn validate_surface_mesh(mesh: &SurfaceTileMesh) -> Result<(), ProtocolError> {
    if mesh.quads.len() > MAX_SURFACE_QUADS_PER_TILE
        || mesh.patches.is_empty()
        || mesh.patches.len() > MAX_SURFACE_PATCHES_PER_TILE
    {
        return Err(ProtocolError::LimitExceeded("surface mesh geometry"));
    }
    for patch in &mesh.patches {
        validate_cell_bounds(patch.cell_bounds)?;
        validate_range(&patch.quad_range, mesh.quads.len())?;
        for range in &patch.skirt_ranges {
            validate_range(range, mesh.quads.len())?;
        }
        if (0..3).any(|axis| patch.bounds.min[axis] >= patch.bounds.max[axis]) {
            return Err(ProtocolError::InvalidPayload("invalid surface bounds"));
        }
    }
    Ok(())
}

fn validate_water_mesh(mesh: &WaterTileMesh) -> Result<(), ProtocolError> {
    if mesh.quads.len() > MAX_SURFACE_QUADS_PER_TILE
        || mesh.patches.len() > MAX_SURFACE_PATCHES_PER_TILE
    {
        return Err(ProtocolError::LimitExceeded("water mesh geometry"));
    }
    for patch in &mesh.patches {
        validate_cell_bounds(patch.cell_bounds)?;
        validate_range(&patch.quad_range, mesh.quads.len())?;
        if (0..3).any(|axis| patch.bounds.min[axis] >= patch.bounds.max[axis]) {
            return Err(ProtocolError::InvalidPayload("invalid surface bounds"));
        }
    }
    Ok(())
}

fn encode_chunk_snapshot(snapshot: &ChunkSnapshot) -> Vec<u8> {
    let core = codec::encode_chunk(&snapshot.chunk, snapshot.source_identity_hash);
    let halo = encode_halo(
        snapshot.meshing_halo.voxels(),
        snapshot.meshing_halo.coord(),
        snapshot.source_identity_hash,
    );
    let mut bytes = Vec::with_capacity(core.len() + halo.len() + 8);
    push_u32(&mut bytes, core.len() as u32);
    bytes.extend_from_slice(&core);
    push_u32(&mut bytes, halo.len() as u32);
    bytes.extend_from_slice(&halo);
    bytes
}

fn decode_chunk_snapshot(
    bytes: &[u8],
    source_identity_hash: WorldSourceIdentityHash,
) -> Result<ChunkSnapshot, ProtocolError> {
    let mut cursor = Cursor::new(bytes);
    let core_len = cursor.u32()? as usize;
    let chunk = codec::decode_chunk(cursor.bytes(core_len)?, source_identity_hash)?;
    let halo_len = cursor.u32()? as usize;
    let halo = decode_halo(cursor.bytes(halo_len)?, chunk.coord(), source_identity_hash)?;
    cursor.finish()?;
    Ok(ChunkSnapshot {
        source_identity_hash,
        chunk,
        meshing_halo: halo,
    })
}

fn encode_halo(
    materials: &[Material],
    coord: ChunkCoord,
    identity: WorldSourceIdentityHash,
) -> Vec<u8> {
    let mut present = [false; Material::ALL.len()];
    for material in materials {
        present[usize::from(material.id())] = true;
    }
    let palette = Material::ALL
        .into_iter()
        .filter(|material| present[usize::from(material.id())])
        .collect::<Vec<_>>();
    let bits = bits_for(palette.len());
    let mut indices = [0_u8; Material::ALL.len()];
    for (index, material) in palette.iter().enumerate() {
        indices[usize::from(material.id())] = index as u8;
    }
    let packed = pack_materials(materials, &indices, bits);
    let hash = halo_hash(materials, coord, identity);
    let mut bytes = Vec::with_capacity(44 + palette.len() * 2 + packed.len());
    push_u32(&mut bytes, materials.len() as u32);
    push_u16(&mut bytes, palette.len() as u16);
    bytes.push(bits);
    bytes.push(0);
    push_u32(&mut bytes, packed.len() as u32);
    bytes.extend_from_slice(hash.as_bytes());
    for material in palette {
        push_u16(&mut bytes, material.id());
    }
    bytes.extend_from_slice(&packed);
    bytes
}

fn decode_halo(
    bytes: &[u8],
    coord: ChunkCoord,
    identity: WorldSourceIdentityHash,
) -> Result<MeshingHalo, ProtocolError> {
    let mut cursor = Cursor::new(bytes);
    let count = cursor.u32()? as usize;
    if count != crate::MESHING_HALO_VOXELS {
        return Err(ProtocolError::InvalidPayload("wrong halo voxel count"));
    }
    let palette_len = usize::from(cursor.u16()?);
    if palette_len == 0 || palette_len > Material::ALL.len() {
        return Err(ProtocolError::InvalidPayload("invalid halo palette"));
    }
    let bits = cursor.u8()?;
    if bits != bits_for(palette_len) || cursor.u8()? != 0 {
        return Err(ProtocolError::InvalidPayload("invalid halo palette bits"));
    }
    let packed_len = cursor.u32()? as usize;
    let expected_hash: [u8; 32] = cursor.array()?;
    let mut palette = Vec::with_capacity(palette_len);
    for _ in 0..palette_len {
        let id = cursor.u16()?;
        palette.push(
            Material::from_id(id).ok_or(ProtocolError::UnknownEnum("material", u64::from(id)))?,
        );
    }
    let materials = unpack_materials(cursor.bytes(packed_len)?, &palette, bits, count)?;
    cursor.finish()?;
    if halo_hash(&materials, coord, identity).as_bytes() != &expected_hash {
        return Err(ProtocolError::InvalidPayload("halo content hash mismatch"));
    }
    MeshingHalo::from_voxels(coord, materials)
        .ok_or(ProtocolError::InvalidPayload("invalid halo body"))
}

fn bits_for(palette_len: usize) -> u8 {
    if palette_len <= 1 {
        0
    } else {
        usize::BITS.saturating_sub((palette_len - 1).leading_zeros()) as u8
    }
}

fn pack_materials(
    materials: &[Material],
    palette_indices: &[u8; Material::ALL.len()],
    bits: u8,
) -> Vec<u8> {
    if bits == 0 {
        return Vec::new();
    }
    let mut output = Vec::with_capacity((materials.len() * usize::from(bits)).div_ceil(8));
    let mut accumulator = 0_u64;
    let mut held = 0_u32;
    for material in materials {
        accumulator |= u64::from(palette_indices[usize::from(material.id())]) << held;
        held += u32::from(bits);
        while held >= 8 {
            output.push(accumulator as u8);
            accumulator >>= 8;
            held -= 8;
        }
    }
    if held > 0 {
        output.push(accumulator as u8);
    }
    output
}

fn unpack_materials(
    packed: &[u8],
    palette: &[Material],
    bits: u8,
    count: usize,
) -> Result<Vec<Material>, ProtocolError> {
    if bits == 0 {
        if palette.len() == 1 && packed.is_empty() {
            return Ok(vec![palette[0]; count]);
        }
        return Err(ProtocolError::InvalidPayload("invalid uniform halo"));
    }
    if packed.len() != (count * usize::from(bits)).div_ceil(8) {
        return Err(ProtocolError::InvalidPayload("invalid packed halo length"));
    }
    let mask = (1_u64 << bits) - 1;
    let mut accumulator = 0_u64;
    let mut held = 0_u32;
    let mut input = packed.iter().copied();
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        while held < u32::from(bits) {
            let byte = input.next().ok_or(ProtocolError::Truncated)?;
            accumulator |= u64::from(byte) << held;
            held += 8;
        }
        let index = (accumulator & mask) as usize;
        output.push(
            palette
                .get(index)
                .copied()
                .ok_or(ProtocolError::InvalidPayload(
                    "halo palette index out of range",
                ))?,
        );
        accumulator >>= bits;
        held -= u32::from(bits);
    }
    Ok(output)
}

fn halo_hash(
    materials: &[Material],
    coord: ChunkCoord,
    identity: WorldSourceIdentityHash,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HALO_HASH_DOMAIN);
    hasher.update(identity.as_bytes());
    hasher.update(&coord.x.to_le_bytes());
    hasher.update(&coord.y.to_le_bytes());
    hasher.update(&coord.z.to_le_bytes());
    for material in materials {
        hasher.update(&material.id().to_le_bytes());
    }
    hasher.finalize()
}

fn encode_world_source_error(error: WorldSourceError) -> u16 {
    match error {
        WorldSourceError::BatchTooLarge => 1,
        WorldSourceError::InvalidChunkCoordinate => 2,
        WorldSourceError::InvalidSurfaceTileCoordinate => 3,
        WorldSourceError::InvalidBlockCoordinate => 4,
        WorldSourceError::EmptyBlock => 5,
        WorldSourceError::BlockTooLarge => 6,
        WorldSourceError::InvalidSearchRadius => 7,
        WorldSourceError::SearchRadiusTooLarge => 8,
        WorldSourceError::EmptyMacroBlock => 9,
        WorldSourceError::MacroBlockTooLarge => 10,
        WorldSourceError::SourceCoverageUnavailable => 11,
        WorldSourceError::MalformedMacroBlock => 12,
    }
}

fn decode_world_source_error(value: u16) -> Result<WorldSourceError, ProtocolError> {
    Ok(match value {
        1 => WorldSourceError::BatchTooLarge,
        2 => WorldSourceError::InvalidChunkCoordinate,
        3 => WorldSourceError::InvalidSurfaceTileCoordinate,
        4 => WorldSourceError::InvalidBlockCoordinate,
        5 => WorldSourceError::EmptyBlock,
        6 => WorldSourceError::BlockTooLarge,
        7 => WorldSourceError::InvalidSearchRadius,
        8 => WorldSourceError::SearchRadiusTooLarge,
        9 => WorldSourceError::EmptyMacroBlock,
        10 => WorldSourceError::MacroBlockTooLarge,
        11 => WorldSourceError::SourceCoverageUnavailable,
        12 => WorldSourceError::MalformedMacroBlock,
        _ => {
            return Err(ProtocolError::UnknownEnum(
                "world source error",
                u64::from(value),
            ));
        }
    })
}

fn decode_priority(value: u8) -> Result<WorldProductPriority, ProtocolError> {
    Ok(match value {
        1 => WorldProductPriority::CollisionCritical,
        2 => WorldProductPriority::VisibleChunk,
        3 => WorldProductPriority::VisibleSurface,
        4 => WorldProductPriority::ReplacementSurface,
        5 => WorldProductPriority::Prefetch,
        _ => return Err(ProtocolError::UnknownEnum("priority", u64::from(value))),
    })
}

fn decode_surface_lod(value: u8) -> Result<SurfaceLodLevel, ProtocolError> {
    Ok(match value {
        0 => SurfaceLodLevel::Stride2,
        1 => SurfaceLodLevel::Stride4,
        2 => SurfaceLodLevel::Stride8,
        3 => SurfaceLodLevel::Stride16,
        _ => {
            return Err(ProtocolError::UnknownEnum(
                "surface LOD level",
                u64::from(value),
            ));
        }
    })
}

fn decode_source_kind(value: u8) -> Result<WorldSourceKind, ProtocolError> {
    Ok(match value {
        1 => WorldSourceKind::ProceduralV16,
        2 => WorldSourceKind::TerrainDiffusion30m,
        _ => return Err(ProtocolError::UnknownEnum("source kind", u64::from(value))),
    })
}

fn decode_device_requirement(value: u8) -> Result<SourceDeviceRequirement, ProtocolError> {
    Ok(match value {
        1 => SourceDeviceRequirement::PortableCpu,
        2 => SourceDeviceRequirement::AppleMetal,
        _ => {
            return Err(ProtocolError::UnknownEnum(
                "device requirement",
                u64::from(value),
            ));
        }
    })
}

fn surface_region_id(region: SurfaceRegion) -> u8 {
    match region {
        SurfaceRegion::VerdantForest => 1,
        SurfaceRegion::WindMoor => 2,
        SurfaceRegion::Alpine => 3,
        SurfaceRegion::RedBadlands => 4,
        SurfaceRegion::PaleDunes => 5,
        SurfaceRegion::Volcanic => 6,
    }
}

fn decode_surface_region(value: u8) -> Result<SurfaceRegion, ProtocolError> {
    Ok(match value {
        1 => SurfaceRegion::VerdantForest,
        2 => SurfaceRegion::WindMoor,
        3 => SurfaceRegion::Alpine,
        4 => SurfaceRegion::RedBadlands,
        5 => SurfaceRegion::PaleDunes,
        6 => SurfaceRegion::Volcanic,
        _ => {
            return Err(ProtocolError::UnknownEnum(
                "surface region",
                u64::from(value),
            ));
        }
    })
}

fn push_string(output: &mut Vec<u8>, value: &str) {
    push_u16(output, value.len() as u16);
    output.extend_from_slice(value.as_bytes());
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(output: &mut Vec<u8>, value: i64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(output: &mut Vec<u8>, value: f32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ProtocolError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or(ProtocolError::Truncated)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ProtocolError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or(ProtocolError::Truncated)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let bytes = bytes
        .get(offset..offset + 8)
        .ok_or(ProtocolError::Truncated)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(ProtocolError::Truncated)?;
        let output = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::Truncated)?;
        self.position = end;
        Ok(output)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64, ProtocolError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, ProtocolError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn string(&mut self) -> Result<String, ProtocolError> {
        let len = usize::from(self.u16()?);
        if len > 4096 {
            return Err(ProtocolError::LimitExceeded("string bytes"));
        }
        Ok(std::str::from_utf8(self.bytes(len)?)
            .map_err(|_| ProtocolError::InvalidUtf8)?
            .to_owned())
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.position != self.bytes.len() {
            return Err(ProtocolError::InvalidPayload("trailing payload bytes"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "protocol tests use direct assertions to keep malformed-wire fixtures legible"
)]
mod tests {
    use super::*;
    use crate::{
        ProceduralWorldSource, WorldProduct, WorldProductBatch, WorldProductRequest,
        WorldSourceEngine,
    };

    fn player_identity(seed: u8, player_name: &str) -> PlayerIdentity {
        PlayerIdentity {
            browser_user_id: BrowserUserId::from_bytes([seed; 16]),
            player_id: PlayerId::from_bytes([seed.wrapping_add(1); 16]),
            player_name: player_name.to_owned(),
        }
    }

    #[test]
    fn open_world_identity_round_trips_and_rejects_unusable_claims() {
        let open = OpenWorld {
            max_in_flight_batches: 4,
            identity: player_identity(1, "alice"),
        };
        assert_eq!(
            decode_open_world(&encode_open_world(&open).expect("encode")),
            Ok(open.clone())
        );
        assert_eq!(
            BrowserUserId::from_uuid_str("00112233-4455-6677-8899-aabbccddeeff")
                .map(|id| id.to_string()),
            Some("00112233-4455-6677-8899-aabbccddeeff".to_owned())
        );
        assert!(BrowserUserId::from_uuid_str("not-a-uuid").is_none());

        let mut invalid = open.clone();
        invalid.identity.player_id = PlayerId::from_bytes([0; 16]);
        assert!(matches!(
            encode_open_world(&invalid),
            Err(ProtocolError::InvalidPayload("player id is nil"))
        ));
        invalid.identity = player_identity(1, "Alice");
        assert!(matches!(
            encode_open_world(&invalid),
            Err(ProtocolError::InvalidPayload(_))
        ));
        invalid.identity = player_identity(1, "a-player_name-that-is-far-too-long");
        assert!(matches!(
            encode_open_world(&invalid),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    #[test]
    fn world_opened_round_trip_is_manifest_validated() {
        let opened = WorldOpened {
            manifest: WorldManifest::procedural_v16(WorldId::from_bytes([7; 16]), 42),
            capabilities: WorldCapabilities::CANONICAL_CHUNKS
                .union(WorldCapabilities::AUTHORED_ROUTES),
            recommended_in_flight_batches: 4,
            identity: player_identity(7, "default"),
            spawn: SpawnPoint {
                x: 0,
                z: 52,
                height: 18,
                water_level: None,
                material: Material::Grass,
                region: SurfaceRegion::VerdantForest,
                moisture: 0.7,
                temperature: 0.6,
                ridge: 0.2,
            },
        };
        assert_eq!(
            decode_world_opened(&encode_world_opened(&opened)),
            Ok(opened)
        );
    }

    #[test]
    fn chunk_snapshot_round_trip_preserves_exact_core_and_halo() {
        let source = ProceduralWorldSource::new(42);
        let coord = ChunkCoord::new(-2, 1, 3);
        let batch = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleChunk,
                requests: vec![WorldProductRequest::ChunkWithHalo(coord)],
            })
            .expect("batch");
        let snapshot = match batch.items.into_iter().next().expect("item").result {
            Ok(WorldProduct::Chunk(snapshot)) => snapshot,
            other => panic!("unexpected product: {other:?}"),
        };
        let response = ChunkBatchResult {
            request_id: 9,
            source_identity_hash: source.source_identity_hash(),
            items: vec![ChunkBatchItem {
                coord,
                result: Ok(snapshot),
            }],
        };
        let encoded = encode_chunk_batch_result(&response).expect("encode");
        assert!(
            encoded.len() < 40_000,
            "palette envelope should stay compact"
        );
        assert_eq!(decode_chunk_batch_result(&encoded), Ok(response));
    }

    #[test]
    fn surface_tile_round_trip_preserves_coarse_render_product() {
        let source = ProceduralWorldSource::new(42);
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, -1, 2);
        let batch = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleSurface,
                requests: vec![WorldProductRequest::SurfaceTile(coord)],
            })
            .expect("batch");
        let snapshot = match batch.items.into_iter().next().expect("item").result {
            Ok(WorldProduct::SurfaceTile(snapshot)) => snapshot,
            other => panic!("unexpected product: {other:?}"),
        };
        let request = SurfaceTileBatchRequest {
            request_id: 10,
            priority: WorldProductPriority::VisibleSurface,
            coords: vec![coord],
        };
        assert_eq!(
            decode_surface_tile_batch(&encode_surface_tile_batch(&request).expect("encode")),
            Ok(request)
        );
        let response = SurfaceTileBatchResult {
            request_id: 10,
            source_identity_hash: source.source_identity_hash(),
            items: vec![SurfaceTileBatchItem {
                coord,
                result: Ok(snapshot),
            }],
        };
        let encoded = encode_surface_tile_batch_result(&response).expect("encode");
        assert!(encoded.len() < MAX_PROTOCOL_FRAME_BYTES);
        assert_eq!(decode_surface_tile_batch_result(&encoded), Ok(response));
    }

    #[test]
    fn surface_snapshot_codec_rejects_wrong_version_and_unbounded_ranges() {
        let source = ProceduralWorldSource::new(42);
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride8, 0, 0);
        let batch = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::VisibleSurface,
                requests: vec![WorldProductRequest::SurfaceTile(coord)],
            })
            .expect("batch");
        let mut snapshot = match batch.items.into_iter().next().expect("item").result {
            Ok(WorldProduct::SurfaceTile(snapshot)) => snapshot,
            other => panic!("unexpected product: {other:?}"),
        };
        let mut encoded = encode_surface_snapshot(&snapshot).expect("encode");
        encoded[4..6].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            decode_surface_snapshot(&encoded, coord, source.source_identity_hash()),
            Err(ProtocolError::UnsupportedVersion(2))
        );

        snapshot.terrain.patches[0].quad_range.end = u32::MAX;
        assert!(matches!(
            encode_surface_snapshot(&snapshot),
            Err(ProtocolError::InvalidPayload(
                "surface patch range is out of bounds"
            ))
        ));
    }

    #[test]
    fn malformed_and_oversized_frames_fail_closed() {
        let request = OpenWorld {
            max_in_flight_batches: 4,
            identity: player_identity(1, "default"),
        };
        let mut open = encode_open_world(&request).expect("encode");
        open[0] = b'B';
        assert_eq!(decode_open_world(&open), Err(ProtocolError::InvalidMagic));

        let mut open = encode_open_world(&request).expect("encode");
        open.extend_from_slice(&[0]);
        assert_eq!(
            decode_open_world(&open),
            Err(ProtocolError::InvalidHeader("trailing bytes"))
        );

        assert!(matches!(
            encode_chunk_batch(&ChunkBatchRequest {
                request_id: 1,
                priority: WorldProductPriority::VisibleChunk,
                coords: vec![],
            }),
            Err(ProtocolError::LimitExceeded("chunk batch"))
        ));

        let duplicate = SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 0, 0);
        assert!(matches!(
            encode_surface_tile_batch(&SurfaceTileBatchRequest {
                request_id: 2,
                priority: WorldProductPriority::VisibleSurface,
                coords: vec![duplicate, duplicate],
            }),
            Err(ProtocolError::InvalidPayload(
                "duplicate surface tile coordinate"
            ))
        ));
    }

    #[test]
    fn request_keys_and_terminal_errors_round_trip() {
        let request = ChunkBatchRequest {
            request_id: 22,
            priority: WorldProductPriority::Prefetch,
            coords: vec![ChunkCoord::new(1, -2, 3), ChunkCoord::new(4, 5, 6)],
        };
        assert_eq!(
            decode_chunk_batch(&encode_chunk_batch(&request).expect("encode")),
            Ok(request)
        );

        let response = ChunkBatchResult {
            request_id: 22,
            source_identity_hash: WorldSourceIdentityHash::from_bytes([8; 32]),
            items: vec![ChunkBatchItem {
                coord: ChunkCoord::new(1, -2, 3),
                result: Err(WorldSourceError::SourceCoverageUnavailable),
            }],
        };
        assert_eq!(
            decode_chunk_batch_result(&encode_chunk_batch_result(&response).expect("encode")),
            Ok(response)
        );
    }
}
