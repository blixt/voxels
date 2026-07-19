//! Versioned transport-neutral wire envelopes for authoritative world products.
//!
//! WebSocket messages use this framing today. The same complete byte envelopes can later ride a
//! reliable WebTransport stream without changing request identity, source validation, or bulk
//! product codecs. Rust enum layout and serde output are deliberately not part of the contract.

use crate::{
    ChunkCoord, ChunkSnapshot, Material, MeshingHalo, ModelIdentity, SURFACE_HORIZON_CELL_COUNT,
    SURFACE_PARENT_HORIZON_CELL_COUNT, SURFACE_PARENT_SHADING_EDGE_SAMPLES,
    SURFACE_SHADING_EDGE_SAMPLES, SourceDeviceRequirement, SurfaceBounds, SurfaceLodLevel,
    SurfacePatch, SurfaceQuad, SurfaceRegion, SurfaceShading, SurfaceTileCoord, SurfaceTileMesh,
    SurfaceTileSnapshot, VOXEL_SIZE_METRES, VoxelCoord, WaterPatch, WaterTileMesh, WorldId,
    WorldManifest, WorldProductPriority, WorldSourceError, WorldSourceIdentity,
    WorldSourceIdentityHash, WorldSourceKind, codec,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;

pub const PROTOCOL_MAGIC: &[u8; 4] = b"VXWP";
pub const PROTOCOL_VERSION: u16 = 23;
pub const FRAME_HEADER_BYTES: usize = 24;
pub const MAX_PROTOCOL_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNKS_PER_BATCH: usize = 256;
pub const MAX_SURFACE_TILES_PER_BATCH: usize = 32;
pub const MAX_PLAYERS_PER_PRESENCE_DELTA: usize = 1_024;
pub const MAX_PLAYER_NAME_BYTES: usize = 32;
pub const MAX_EDIT_MUTATIONS: usize = 125;
pub const MAX_EDIT_AFFECTED_CHUNKS: usize = 8;
pub const MAX_EDIT_AFFECTED_SURFACE_TILES: usize = 128;
pub const FRAME_FRAGMENT_OVERHEAD_BYTES: usize = FRAME_HEADER_BYTES + 8;
pub const MAX_FRAME_FRAGMENT_DATA_BYTES: usize =
    MAX_PROTOCOL_FRAME_BYTES - FRAME_FRAGMENT_OVERHEAD_BYTES;
pub const EDIT_SESSION_NOT_CURRENT: &str = "edit session is no longer current";
const MAX_SURFACE_QUADS_PER_TILE: usize = 65_535;
const MAX_SURFACE_PATCHES_PER_TILE: usize = 64;
const SURFACE_SNAPSHOT_MAGIC: &[u8; 4] = b"VXST";
const SURFACE_SNAPSHOT_VERSION: u16 = 6;

const KIND_OPEN_WORLD: u16 = 1;
const KIND_WORLD_OPENED: u16 = 2;
const KIND_CHUNK_BATCH: u16 = 3;
const KIND_CHUNK_BATCH_RESULT: u16 = 4;
const KIND_CANCEL: u16 = 5;
const KIND_ERROR: u16 = 6;
const KIND_SURFACE_TILE_BATCH: u16 = 7;
const KIND_SURFACE_TILE_BATCH_RESULT: u16 = 8;
const KIND_OPEN_PRESENCE: u16 = 9;
const KIND_PRESENCE_OPENED: u16 = 10;
const KIND_PLAYER_POSE: u16 = 11;
const KIND_PRESENCE_DELTA: u16 = 12;
const KIND_PRESENCE_PING: u16 = 13;
const KIND_PRESENCE_PONG: u16 = 14;
const KIND_EDIT_COMMAND: u16 = 15;
const KIND_EDIT_COMMIT: u16 = 16;
const KIND_RESYNC_REQUIRED: u16 = 17;
const KIND_FRAME_FRAGMENT: u16 = 18;
const FLAG_NONE: u16 = 0;
const RESERVED: u16 = 0;
const RESULT_CODEC_BROTLI: u8 = 1;
const RESULT_ENVELOPE_BYTES: usize = 8;
const BROTLI_BUFFER_BYTES: usize = 4 * 1024;
const BROTLI_QUALITY: u32 = 2;
const BROTLI_WINDOW_BITS: u32 = 20;
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
    pub const PLAYER_PRESENCE: Self = Self(1 << 8);
    /// The server permits bodyless, non-editing spectator cameras for this world connection.
    pub const SPECTATOR_MODE: Self = Self(1 << 9);
    /// The server permits ordinary players to deploy the deterministic airborne glider.
    pub const GLIDING: Self = Self(1 << 10);

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
opaque_uuid_id!(PresenceSessionId);
opaque_uuid_id!(EditSessionId);

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
    pub environment: WorldEnvironmentSnapshot,
    pub recommended_in_flight_batches: u16,
    pub identity: PlayerIdentity,
    pub connection_id: u64,
    pub presence_session_id: PresenceSessionId,
    /// Server-issued namespace for durable, idempotent edit operation IDs.
    pub edit_session_id: EditSessionId,
    pub spawn: SpawnPoint,
    /// Authoritative camera state. New players receive a state derived from `spawn`; returning
    /// players receive their last server-accepted state instead of choosing a client-side resume.
    pub player_resume: PlayerResume,
    pub inventory: MaterialInventory,
}

/// Server-authored environment state sampled against the same monotonic clock used by presence.
///
/// Clients extrapolate the day fraction and cloud offset from `sample_server_time_ms`, so all
/// players see one world clock without receiving per-frame environment messages. A future weather
/// authority can replace the snapshot whenever `weather_revision` changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldEnvironmentSnapshot {
    pub sample_server_time_ms: u64,
    pub world_day_number: i64,
    pub day_fraction: f32,
    /// Zero freezes the celestial clock at `day_fraction`.
    pub day_length_seconds: f32,
    pub days_per_year: f32,
    pub moon_sidereal_orbit_days: f32,
    pub moon_orbit_phase_at_world_epoch: f32,
    pub planet_circumference_metres: f32,
    pub axial_tilt_radians: f32,
    pub moon_orbit_inclination_radians: f32,
    pub celestial_seed: u64,
    pub celestial_revision: u64,
    pub weather_fraction: f32,
    /// Zero freezes the weather timeline at `weather_fraction`.
    pub weather_cycle_seconds: f32,
    pub cloud_offset_metres: [f32; 2],
    pub cloud_velocity_metres_per_second: [f32; 2],
    /// Minimum clear-sky coverage before the weather cycle adds overcast.
    pub cloud_coverage: f32,
    pub cloud_base_metres: f32,
    pub cloud_top_metres: f32,
    pub weather_seed: u64,
    pub weather_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerResume {
    pub revision: u64,
    pub eye_position_metres: [f32; 3],
    pub look_yaw_radians: f32,
    pub look_pitch_radians: f32,
}

pub const MATERIAL_INVENTORY_SLOTS: usize = Material::ALL.len();

/// Dense authoritative material counts in stable [`Material::id`] order.
///
/// Air is retained as slot zero so the wire shape is exactly pinned to the material schema, but
/// its count must remain zero because empty space is not an inventory item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterialInventory {
    pub revision: u64,
    pub counts: [u64; MATERIAL_INVENTORY_SLOTS],
}

impl MaterialInventory {
    pub const EMPTY: Self = Self {
        revision: 1,
        counts: [0; MATERIAL_INVENTORY_SLOTS],
    };

    pub const fn count(self, material: Material) -> u64 {
        self.counts[material.id() as usize]
    }
}

pub const PLAYER_POSE_GROUNDED: u16 = 1 << 0;
pub const PLAYER_POSE_SWIMMING: u16 = 1 << 1;
pub const PLAYER_POSE_DISCONTINUITY: u16 = 1 << 2;
pub const PLAYER_POSE_SPECTATOR: u16 = 1 << 3;
pub const PLAYER_POSE_GLIDING: u16 = 1 << 4;
const PLAYER_POSE_FLAGS: u16 = PLAYER_POSE_GROUNDED
    | PLAYER_POSE_SWIMMING
    | PLAYER_POSE_DISCONTINUITY
    | PLAYER_POSE_SPECTATOR
    | PLAYER_POSE_GLIDING;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenPresence {
    pub session_id: PresenceSessionId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresenceOpened {
    pub connection_id: u64,
    pub server_time_ms: u64,
    pub broadcast_interval_ms: u16,
    pub max_players: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerPoseUpdate {
    pub sequence: u64,
    /// Sender sample time mapped into the server monotonic clock. Zero asks the server to stamp
    /// receipt time while the clock handshake is still converging.
    pub sample_server_time_ms: u64,
    pub eye_position_metres: [f32; 3],
    pub linear_velocity_metres_per_second: [f32; 3],
    pub look_yaw_radians: f32,
    pub look_pitch_radians: f32,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerPresenceState {
    pub player_id: PlayerId,
    pub connection_id: u64,
    pub color_index: u16,
    pub pose: PlayerPoseUpdate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerPresenceUpdate {
    pub connection_id: u64,
    pub pose: PlayerPoseUpdate,
}

/// One receiver-specific presence update. Entries establish identity, updates replace dynamic
/// state for already-entered connections, and leaves explicitly retire connections. Omission means
/// unchanged, which permits distance-cadenced and bandwidth-budgeted replication.
#[derive(Clone, Debug, PartialEq)]
pub struct PresenceDelta {
    pub stream_sequence: u64,
    pub server_time_ms: u64,
    pub visible_player_count: u16,
    pub enters: Vec<PlayerPresenceState>,
    pub updates: Vec<PlayerPresenceUpdate>,
    pub leaves: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresencePing {
    pub sequence: u32,
    /// The previous pong's receiver-observed RTT, or zero before the first pong arrives.
    pub observed_round_trip_ms: u32,
    pub client_send_time_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresencePong {
    pub sequence: u32,
    /// Current shared world/presence application pacing rate selected by the server.
    pub outbound_rate_bytes_per_second: u32,
    pub client_send_time_ms: u64,
    pub server_receive_time_ms: u64,
    pub server_send_time_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameFragment {
    pub transfer_id: u64,
    pub total_bytes: u32,
    pub offset: u32,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct FrameReassembler {
    transfers: BTreeMap<u64, PartialFrame>,
}

struct PartialFrame {
    total_bytes: usize,
    bytes: Vec<u8>,
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
    /// Authoritative edit revision captured with this product. Zero denotes the pristine world.
    pub edit_revision: u64,
    pub result: Result<ChunkSnapshot, WorldSourceError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkBatchResult {
    pub request_id: u64,
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<ChunkBatchItem>,
}

/// A validated chunk result item ready to be shared across response batches.
///
/// The bytes intentionally exclude the batch header and outer compression envelope. The world
/// service can therefore cache this product by coordinate and edit revision, then preserve the
/// existing VXWP batch wire format when assembling a client response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedChunkBatchItem {
    coord: ChunkCoord,
    edit_revision: u64,
    source_identity_hash: WorldSourceIdentityHash,
    bytes: Vec<u8>,
}

impl EncodedChunkBatchItem {
    pub fn coord(&self) -> ChunkCoord {
        self.coord
    }

    pub fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub fn encoded_len(&self) -> usize {
        self.bytes.len()
    }
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
    /// Authoritative edit revision captured with this derived product.
    pub edit_revision: u64,
    pub result: Result<SurfaceTileSnapshot, WorldSourceError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceTileBatchResult {
    pub request_id: u64,
    pub source_identity_hash: WorldSourceIdentityHash,
    pub items: Vec<SurfaceTileBatchItem>,
}

/// A validated surface result item ready to be shared across response batches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedSurfaceTileBatchItem {
    coord: SurfaceTileCoord,
    edit_revision: u64,
    source_identity_hash: WorldSourceIdentityHash,
    bytes: Vec<u8>,
}

impl EncodedSurfaceTileBatchItem {
    pub fn coord(&self) -> SurfaceTileCoord {
        self.coord
    }

    pub fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    pub fn encoded_len(&self) -> usize {
        self.bytes.len()
    }
}

/// One idempotent server-authoritative gameplay edit.
///
/// The operation ID is scoped to the server-issued edit session and is also carried in the VXWP
/// request-id header. Retrying an operation uses the same edit session and nonzero operation ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditCommand {
    pub operation_id: u64,
    pub edit_session_id: EditSessionId,
    pub action: EditAction,
}

impl EditCommand {
    /// Reissues an operation that the server explicitly proved was absent from a rotated session.
    /// A stored old-session operation is answered idempotently instead and must never call this.
    pub fn reissue_after_session_rotation(
        self,
        operation_id: u64,
        edit_session_id: EditSessionId,
    ) -> Option<Self> {
        if self.edit_session_id == edit_session_id || operation_id == 0 {
            return None;
        }
        Some(Self {
            operation_id,
            edit_session_id,
            action: self.action,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditAction {
    /// Digs the fixed spherical stencil centred on the pointed voxel. The client cannot choose a
    /// radius, mutation count, or arbitrary coordinate list.
    Dig { hit: VoxelCoord },
    Place {
        coord: VoxelCoord,
        material: Material,
    },
}

/// Diameter of the authoritative half-metre dig sphere's voxel-centre stencil.
pub const DIG_DIAMETER_VOXELS: i32 = 5;
pub const DIG_RADIUS_VOXELS: i32 = DIG_DIAMETER_VOXELS / 2;
/// Exact lattice points whose voxel centres lie inside a 2.5-voxel Euclidean radius.
pub const DIG_VOLUME_VOXELS: usize = dig_volume_voxel_count();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DigVolume {
    pub min: VoxelCoord,
    pub max: VoxelCoord,
}

impl DigVolume {
    pub fn for_hit(hit: VoxelCoord) -> Option<Self> {
        Some(Self {
            min: VoxelCoord::new(
                hit.x.checked_sub(DIG_RADIUS_VOXELS)?,
                hit.y.checked_sub(DIG_RADIUS_VOXELS)?,
                hit.z.checked_sub(DIG_RADIUS_VOXELS)?,
            ),
            max: VoxelCoord::new(
                hit.x.checked_add(DIG_RADIUS_VOXELS)?,
                hit.y.checked_add(DIG_RADIUS_VOXELS)?,
                hit.z.checked_add(DIG_RADIUS_VOXELS)?,
            ),
        })
    }

    pub const fn sample_shape(self) -> [u32; 3] {
        [DIG_DIAMETER_VOXELS as u32; 3]
    }

    pub const fn contains(self, coord: VoxelCoord) -> bool {
        if !(coord.x >= self.min.x
            && coord.x <= self.max.x
            && coord.y >= self.min.y
            && coord.y <= self.max.y
            && coord.z >= self.min.z
            && coord.z <= self.max.z)
        {
            return false;
        }
        let centre = self.centre();
        let dx = coord.x as i64 - centre.x as i64;
        let dy = coord.y as i64 - centre.y as i64;
        let dz = coord.z as i64 - centre.z as i64;
        4 * (dx * dx + dy * dy + dz * dz) <= (DIG_DIAMETER_VOXELS * DIG_DIAMETER_VOXELS) as i64
    }

    pub const fn centre(self) -> VoxelCoord {
        VoxelCoord::new(
            self.min.x + DIG_RADIUS_VOXELS,
            self.min.y + DIG_RADIUS_VOXELS,
            self.min.z + DIG_RADIUS_VOXELS,
        )
    }

    pub fn coordinates(self) -> impl Iterator<Item = VoxelCoord> {
        (self.min.x..=self.max.x)
            .flat_map(move |x| {
                (self.min.y..=self.max.y).flat_map(move |y| {
                    (self.min.z..=self.max.z).map(move |z| VoxelCoord::new(x, y, z))
                })
            })
            .filter(move |&coord| self.contains(coord))
    }
}

const fn dig_volume_voxel_count() -> usize {
    let mut count = 0;
    let mut x = -DIG_RADIUS_VOXELS;
    while x <= DIG_RADIUS_VOXELS {
        let mut y = -DIG_RADIUS_VOXELS;
        while y <= DIG_RADIUS_VOXELS {
            let mut z = -DIG_RADIUS_VOXELS;
            while z <= DIG_RADIUS_VOXELS {
                if 4 * (x * x + y * y + z * z) <= DIG_DIAMETER_VOXELS * DIG_DIAMETER_VOXELS {
                    count += 1;
                }
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
    count
}

impl EditAction {
    pub const fn target(self) -> VoxelCoord {
        match self {
            Self::Dig { hit, .. } => hit,
            Self::Place { coord, .. } => coord,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VoxelMutation {
    pub coord: VoxelCoord,
    /// Exact authoritative value after the operation. Digging is represented by `Material::Air`.
    pub material: Material,
}

/// One durably ordered atomic edit and every derived product key it invalidates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditCommit {
    pub operation_id: u64,
    pub edit_session_id: EditSessionId,
    /// Connection that submitted this operation. Operation IDs are scoped to `edit_session_id`.
    pub editor_connection_id: u64,
    pub revision: u64,
    /// Strictly coordinate-sorted, unique final voxel values committed at `revision`.
    pub mutations: Vec<VoxelMutation>,
    pub affected_chunks: Vec<ChunkCoord>,
    pub affected_surface_tiles: Vec<SurfaceTileCoord>,
    /// Present only on the editor's receipt. Observer commits intentionally omit private inventory.
    pub editor_inventory: Option<MaterialInventory>,
}

/// The receiver's bounded change queue lost incremental state and all retained products must be
/// reconciled against this authoritative revision before incremental commits resume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResyncRequired {
    pub revision: u64,
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
    Compression(&'static str),
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
            Self::Compression(reason) => write!(formatter, "VXWP compression: {reason}"),
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
    debug_assert!(opened.connection_id != 0);
    debug_assert!(!opened.presence_session_id.is_nil());
    debug_assert!(!opened.edit_session_id.is_nil());
    debug_assert!(validate_player_resume(&opened.player_resume).is_ok());
    debug_assert!(validate_inventory(&opened.inventory).is_ok());
    debug_assert!(validate_world_environment(&opened.environment).is_ok());
    let manifest = encode_manifest(&opened.manifest);
    let mut payload = Vec::with_capacity(manifest.len() + 320);
    push_u32(&mut payload, manifest.len() as u32);
    payload.extend_from_slice(&manifest);
    push_u64(&mut payload, opened.capabilities.bits());
    encode_world_environment(&mut payload, opened.environment);
    push_u16(&mut payload, opened.recommended_in_flight_batches);
    payload.extend_from_slice(opened.identity.browser_user_id.as_bytes());
    payload.extend_from_slice(opened.identity.player_id.as_bytes());
    push_string(&mut payload, &opened.identity.player_name);
    push_u64(&mut payload, opened.connection_id);
    payload.extend_from_slice(opened.presence_session_id.as_bytes());
    payload.extend_from_slice(opened.edit_session_id.as_bytes());
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
    encode_player_resume(&mut payload, opened.player_resume);
    encode_inventory(&mut payload, opened.inventory);
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
    let environment = decode_world_environment(&mut cursor)?;
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
    let connection_id = cursor.u64()?;
    if connection_id == 0 {
        return Err(ProtocolError::InvalidPayload("world connection id is zero"));
    }
    let presence_session_id = PresenceSessionId::from_bytes(cursor.array()?);
    if presence_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("presence session id is nil"));
    }
    let edit_session_id = EditSessionId::from_bytes(cursor.array()?);
    if edit_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("edit session id is nil"));
    }
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
    let player_resume = decode_player_resume(&mut cursor)?;
    let inventory = decode_inventory(&mut cursor)?;
    cursor.finish()?;
    Ok(WorldOpened {
        manifest,
        capabilities,
        environment,
        recommended_in_flight_batches,
        identity,
        connection_id,
        presence_session_id,
        edit_session_id,
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
        player_resume,
        inventory,
    })
}

fn encode_world_environment(payload: &mut Vec<u8>, environment: WorldEnvironmentSnapshot) {
    push_u64(payload, environment.sample_server_time_ms);
    push_i64(payload, environment.world_day_number);
    push_f32(payload, environment.day_fraction);
    push_f32(payload, environment.day_length_seconds);
    push_f32(payload, environment.days_per_year);
    push_f32(payload, environment.moon_sidereal_orbit_days);
    push_f32(payload, environment.moon_orbit_phase_at_world_epoch);
    push_f32(payload, environment.planet_circumference_metres);
    push_f32(payload, environment.axial_tilt_radians);
    push_f32(payload, environment.moon_orbit_inclination_radians);
    push_u64(payload, environment.celestial_seed);
    push_u64(payload, environment.celestial_revision);
    push_f32(payload, environment.weather_fraction);
    push_f32(payload, environment.weather_cycle_seconds);
    for value in environment.cloud_offset_metres {
        push_f32(payload, value);
    }
    for value in environment.cloud_velocity_metres_per_second {
        push_f32(payload, value);
    }
    push_f32(payload, environment.cloud_coverage);
    push_f32(payload, environment.cloud_base_metres);
    push_f32(payload, environment.cloud_top_metres);
    push_u64(payload, environment.weather_seed);
    push_u64(payload, environment.weather_revision);
}

fn decode_world_environment(
    cursor: &mut Cursor<'_>,
) -> Result<WorldEnvironmentSnapshot, ProtocolError> {
    let environment = WorldEnvironmentSnapshot {
        sample_server_time_ms: cursor.u64()?,
        world_day_number: cursor.i64()?,
        day_fraction: cursor.f32()?,
        day_length_seconds: cursor.f32()?,
        days_per_year: cursor.f32()?,
        moon_sidereal_orbit_days: cursor.f32()?,
        moon_orbit_phase_at_world_epoch: cursor.f32()?,
        planet_circumference_metres: cursor.f32()?,
        axial_tilt_radians: cursor.f32()?,
        moon_orbit_inclination_radians: cursor.f32()?,
        celestial_seed: cursor.u64()?,
        celestial_revision: cursor.u64()?,
        weather_fraction: cursor.f32()?,
        weather_cycle_seconds: cursor.f32()?,
        cloud_offset_metres: [cursor.f32()?, cursor.f32()?],
        cloud_velocity_metres_per_second: [cursor.f32()?, cursor.f32()?],
        cloud_coverage: cursor.f32()?,
        cloud_base_metres: cursor.f32()?,
        cloud_top_metres: cursor.f32()?,
        weather_seed: cursor.u64()?,
        weather_revision: cursor.u64()?,
    };
    validate_world_environment(&environment)?;
    Ok(environment)
}

fn validate_world_environment(environment: &WorldEnvironmentSnapshot) -> Result<(), ProtocolError> {
    if environment.sample_server_time_ms == 0
        || !environment.day_fraction.is_finite()
        || !(0.0..1.0).contains(&environment.day_fraction)
        || !environment.day_length_seconds.is_finite()
        || !(0.0..=86_400.0).contains(&environment.day_length_seconds)
        || environment.world_day_number.unsigned_abs() > 1_000_000_000
        || !environment.days_per_year.is_finite()
        || !(4.0..=4_096.0).contains(&environment.days_per_year)
        || !environment.moon_sidereal_orbit_days.is_finite()
        || !(0.25..=environment.days_per_year).contains(&environment.moon_sidereal_orbit_days)
        || !environment.moon_orbit_phase_at_world_epoch.is_finite()
        || !(0.0..1.0).contains(&environment.moon_orbit_phase_at_world_epoch)
        || !environment.planet_circumference_metres.is_finite()
        || !(100_000.0..=100_000_000.0).contains(&environment.planet_circumference_metres)
        || !environment.axial_tilt_radians.is_finite()
        || !(0.0..=std::f32::consts::FRAC_PI_4).contains(&environment.axial_tilt_radians)
        || !environment.moon_orbit_inclination_radians.is_finite()
        || !(0.0..=std::f32::consts::FRAC_PI_6)
            .contains(&environment.moon_orbit_inclination_radians)
        || environment.celestial_revision == 0
        || !environment.weather_fraction.is_finite()
        || !(0.0..1.0).contains(&environment.weather_fraction)
        || !environment.weather_cycle_seconds.is_finite()
        || environment.weather_cycle_seconds < 0.0
        || !environment
            .cloud_offset_metres
            .into_iter()
            .chain(environment.cloud_velocity_metres_per_second)
            .all(|value| value.is_finite())
        || !environment.cloud_coverage.is_finite()
        || !(0.0..=1.0).contains(&environment.cloud_coverage)
        || !environment.cloud_base_metres.is_finite()
        || environment.cloud_base_metres < 0.0
        || !environment.cloud_top_metres.is_finite()
        || environment.cloud_top_metres <= environment.cloud_base_metres
        || environment.weather_revision == 0
    {
        return Err(ProtocolError::InvalidPayload(
            "invalid world environment snapshot",
        ));
    }
    Ok(())
}

pub fn encode_chunk_batch(request: &ChunkBatchRequest) -> Result<Vec<u8>, ProtocolError> {
    if request.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
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
    let items = result
        .items
        .iter()
        .map(|item| encode_chunk_batch_item(result.source_identity_hash, item))
        .collect::<Result<Vec<_>, _>>()?;
    encode_chunk_batch_result_from_items(
        result.request_id,
        result.source_identity_hash,
        items.iter(),
    )
}

pub fn encode_chunk_batch_item(
    source_identity_hash: WorldSourceIdentityHash,
    item: &ChunkBatchItem,
) -> Result<EncodedChunkBatchItem, ProtocolError> {
    if !item.coord.is_world_representable() {
        return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
    }
    let mut bytes = Vec::new();
    push_i32(&mut bytes, item.coord.x);
    push_i32(&mut bytes, item.coord.y);
    push_i32(&mut bytes, item.coord.z);
    push_u64(&mut bytes, item.edit_revision);
    match &item.result {
        Ok(snapshot) => {
            if snapshot.chunk.coord() != item.coord
                || snapshot.meshing_halo.coord() != item.coord
                || snapshot.source_identity_hash != source_identity_hash
            {
                return Err(ProtocolError::InvalidPayload(
                    "chunk result key or identity mismatch",
                ));
            }
            push_u16(&mut bytes, 0);
            push_u16(&mut bytes, 0);
            let encoded = encode_chunk_snapshot(snapshot);
            push_u32(&mut bytes, encoded.len() as u32);
            bytes.extend_from_slice(&encoded);
        }
        Err(error) => {
            push_u16(&mut bytes, encode_world_source_error(*error));
            push_u16(&mut bytes, 0);
            push_u32(&mut bytes, 0);
        }
    }
    Ok(EncodedChunkBatchItem {
        coord: item.coord,
        edit_revision: item.edit_revision,
        source_identity_hash,
        bytes,
    })
}

pub fn encode_chunk_batch_result_from_items<'a>(
    request_id: u64,
    source_identity_hash: WorldSourceIdentityHash,
    items: impl IntoIterator<Item = &'a EncodedChunkBatchItem>,
) -> Result<Vec<u8>, ProtocolError> {
    let items = items.into_iter().collect::<Vec<_>>();
    if request_id == 0 || items.is_empty() || items.len() > MAX_CHUNKS_PER_BATCH {
        return Err(ProtocolError::LimitExceeded("chunk result batch"));
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(source_identity_hash.as_bytes());
    push_u16(&mut payload, items.len() as u16);
    push_u16(&mut payload, 0);
    for (index, item) in items.iter().enumerate() {
        if item.source_identity_hash != source_identity_hash {
            return Err(ProtocolError::InvalidPayload(
                "chunk result identity mismatch",
            ));
        }
        if items[..index].iter().any(|prior| prior.coord == item.coord) {
            return Err(ProtocolError::InvalidPayload(
                "duplicate chunk result coordinate",
            ));
        }
        payload.extend_from_slice(&item.bytes);
    }
    let payload = encode_result_payload(&payload)?;
    Ok(encode_frame(KIND_CHUNK_BATCH_RESULT, request_id, &payload))
}

pub fn decode_chunk_batch_result(bytes: &[u8]) -> Result<ChunkBatchResult, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_CHUNK_BATCH_RESULT)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload("request id must be nonzero"));
    }
    let payload = decode_result_payload(frame.payload)?;
    let mut cursor = Cursor::new(&payload);
    let source_identity_hash = WorldSourceIdentityHash::from_bytes(cursor.array()?);
    let count = usize::from(cursor.u16()?);
    if count == 0 || count > MAX_CHUNKS_PER_BATCH || cursor.u16()? != 0 {
        return Err(ProtocolError::InvalidPayload("invalid result item count"));
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let coord = ChunkCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?);
        if !coord.is_world_representable() {
            return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
        }
        if items
            .iter()
            .any(|item: &ChunkBatchItem| item.coord == coord)
        {
            return Err(ProtocolError::InvalidPayload(
                "duplicate chunk result coordinate",
            ));
        }
        let edit_revision = cursor.u64()?;
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
        items.push(ChunkBatchItem {
            coord,
            edit_revision,
            result,
        });
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
    let items = result
        .items
        .iter()
        .map(|item| encode_surface_tile_batch_item(result.source_identity_hash, item))
        .collect::<Result<Vec<_>, _>>()?;
    encode_surface_tile_batch_result_from_items(
        result.request_id,
        result.source_identity_hash,
        items.iter(),
    )
}

pub fn encode_surface_tile_batch_item(
    source_identity_hash: WorldSourceIdentityHash,
    item: &SurfaceTileBatchItem,
) -> Result<EncodedSurfaceTileBatchItem, ProtocolError> {
    if !item.coord.is_world_representable() {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface tile coordinate",
        ));
    }
    let mut bytes = Vec::new();
    encode_surface_coord(&mut bytes, item.coord);
    push_u64(&mut bytes, item.edit_revision);
    match &item.result {
        Ok(snapshot) => {
            if snapshot.source_identity_hash != source_identity_hash
                || snapshot.terrain.coord != item.coord
                || snapshot.water.coord != item.coord
            {
                return Err(ProtocolError::InvalidPayload(
                    "surface result key or identity mismatch",
                ));
            }
            push_u16(&mut bytes, 0);
            push_u16(&mut bytes, 0);
            let encoded = encode_surface_snapshot(snapshot)?;
            push_u32(&mut bytes, encoded.len() as u32);
            bytes.extend_from_slice(&encoded);
        }
        Err(error) => {
            push_u16(&mut bytes, encode_world_source_error(*error));
            push_u16(&mut bytes, 0);
            push_u32(&mut bytes, 0);
        }
    }
    Ok(EncodedSurfaceTileBatchItem {
        coord: item.coord,
        edit_revision: item.edit_revision,
        source_identity_hash,
        bytes,
    })
}

pub fn encode_surface_tile_batch_result_from_items<'a>(
    request_id: u64,
    source_identity_hash: WorldSourceIdentityHash,
    items: impl IntoIterator<Item = &'a EncodedSurfaceTileBatchItem>,
) -> Result<Vec<u8>, ProtocolError> {
    let items = items.into_iter().collect::<Vec<_>>();
    if request_id == 0 || items.is_empty() || items.len() > MAX_SURFACE_TILES_PER_BATCH {
        return Err(ProtocolError::LimitExceeded("surface tile result batch"));
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(source_identity_hash.as_bytes());
    push_u16(&mut payload, items.len() as u16);
    push_u16(&mut payload, 0);
    for (index, item) in items.iter().enumerate() {
        if item.source_identity_hash != source_identity_hash {
            return Err(ProtocolError::InvalidPayload(
                "surface result identity mismatch",
            ));
        }
        if items[..index].iter().any(|prior| prior.coord == item.coord) {
            return Err(ProtocolError::InvalidPayload(
                "duplicate surface result coordinate",
            ));
        }
        payload.extend_from_slice(&item.bytes);
    }
    let payload = encode_result_payload(&payload)?;
    Ok(encode_frame(
        KIND_SURFACE_TILE_BATCH_RESULT,
        request_id,
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
    let payload = decode_result_payload(frame.payload)?;
    let mut cursor = Cursor::new(&payload);
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
        if items
            .iter()
            .any(|item: &SurfaceTileBatchItem| item.coord == coord)
        {
            return Err(ProtocolError::InvalidPayload(
                "duplicate surface result coordinate",
            ));
        }
        let edit_revision = cursor.u64()?;
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
        items.push(SurfaceTileBatchItem {
            coord,
            edit_revision,
            result,
        });
    }
    cursor.finish()?;
    Ok(SurfaceTileBatchResult {
        request_id: frame.request_id,
        source_identity_hash,
        items,
    })
}

pub fn encode_edit_command(command: EditCommand) -> Result<Vec<u8>, ProtocolError> {
    if command.operation_id == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit operation id must be nonzero",
        ));
    }
    if command.edit_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("edit session id is nil"));
    }
    let (kind, argument, coord, material_id) = match command.action {
        EditAction::Dig { hit } => (1, 0, hit, 0),
        EditAction::Place { coord, material } => {
            if material == Material::Air {
                return Err(ProtocolError::InvalidPayload(
                    "cannot place air; use the dig action",
                ));
            }
            (2, 0, coord, material.id())
        }
    };
    validate_voxel_coord(coord)?;
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(command.edit_session_id.as_bytes());
    payload.push(kind);
    payload.push(argument);
    push_u16(&mut payload, material_id);
    encode_voxel_coord(&mut payload, coord);
    Ok(encode_frame(
        KIND_EDIT_COMMAND,
        command.operation_id,
        &payload,
    ))
}

pub fn decode_edit_command(bytes: &[u8]) -> Result<EditCommand, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_EDIT_COMMAND)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit operation id must be nonzero",
        ));
    }
    let mut cursor = Cursor::new(frame.payload);
    let edit_session_id = EditSessionId::from_bytes(cursor.array()?);
    if edit_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("edit session id is nil"));
    }
    let kind = cursor.u8()?;
    let argument = cursor.u8()?;
    let material_id = cursor.u16()?;
    let coord = decode_voxel_coord(&mut cursor)?;
    cursor.finish()?;
    let action = match kind {
        1 if argument == 0 && material_id == 0 => EditAction::Dig { hit: coord },
        1 => {
            return Err(ProtocolError::InvalidPayload(
                "dig action has a nonzero argument or material",
            ));
        }
        2 if argument == 0 => {
            let material = Material::from_id(material_id).ok_or(ProtocolError::UnknownEnum(
                "material",
                u64::from(material_id),
            ))?;
            if material == Material::Air {
                return Err(ProtocolError::InvalidPayload(
                    "cannot place air; use the dig action",
                ));
            }
            EditAction::Place { coord, material }
        }
        2 => {
            return Err(ProtocolError::InvalidPayload(
                "placement action argument is nonzero",
            ));
        }
        _ => return Err(ProtocolError::UnknownEnum("edit action", u64::from(kind))),
    };
    Ok(EditCommand {
        operation_id: frame.request_id,
        edit_session_id,
        action,
    })
}

pub fn encode_edit_commit(commit: &EditCommit) -> Result<Vec<u8>, ProtocolError> {
    validate_edit_commit(commit)?;
    let mut payload = Vec::with_capacity(
        40 + commit.mutations.len() * 16
            + commit.affected_chunks.len() * 12
            + commit.affected_surface_tiles.len() * 12
            + commit.editor_inventory.is_some() as usize * (8 + MATERIAL_INVENTORY_SLOTS * 8),
    );
    payload.extend_from_slice(commit.edit_session_id.as_bytes());
    push_u64(&mut payload, commit.editor_connection_id);
    push_u64(&mut payload, commit.revision);
    push_u16(&mut payload, commit.mutations.len() as u16);
    push_u16(&mut payload, commit.affected_chunks.len() as u16);
    push_u16(&mut payload, commit.affected_surface_tiles.len() as u16);
    push_u16(&mut payload, u16::from(commit.editor_inventory.is_some()));
    for mutation in &commit.mutations {
        encode_voxel_coord(&mut payload, mutation.coord);
        push_u16(&mut payload, mutation.material.id());
        push_u16(&mut payload, 0);
    }
    for coord in &commit.affected_chunks {
        push_chunk_coord(&mut payload, *coord);
    }
    for coord in &commit.affected_surface_tiles {
        encode_surface_coord(&mut payload, *coord);
    }
    if let Some(inventory) = commit.editor_inventory {
        encode_inventory(&mut payload, inventory);
    }
    Ok(encode_frame(
        KIND_EDIT_COMMIT,
        commit.operation_id,
        &payload,
    ))
}

pub fn decode_edit_commit(bytes: &[u8]) -> Result<EditCommit, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_EDIT_COMMIT)?;
    if frame.request_id == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit operation id must be nonzero",
        ));
    }
    let mut cursor = Cursor::new(frame.payload);
    let edit_session_id = EditSessionId::from_bytes(cursor.array()?);
    if edit_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("edit session id is nil"));
    }
    let editor_connection_id = cursor.u64()?;
    let revision = cursor.u64()?;
    let mutation_count = usize::from(cursor.u16()?);
    let chunk_count = usize::from(cursor.u16()?);
    let surface_count = usize::from(cursor.u16()?);
    let inventory_present = cursor.u16()?;
    if mutation_count > MAX_EDIT_MUTATIONS {
        return Err(ProtocolError::LimitExceeded("edit mutations"));
    }
    if chunk_count > MAX_EDIT_AFFECTED_CHUNKS {
        return Err(ProtocolError::LimitExceeded("edit affected chunks"));
    }
    if surface_count > MAX_EDIT_AFFECTED_SURFACE_TILES {
        return Err(ProtocolError::LimitExceeded("edit affected surface tiles"));
    }
    if inventory_present > 1 {
        return Err(ProtocolError::InvalidPayload(
            "invalid edit inventory presence flag",
        ));
    }
    let mut mutations = Vec::with_capacity(mutation_count);
    for _ in 0..mutation_count {
        let coord = decode_voxel_coord(&mut cursor)?;
        let material_id = cursor.u16()?;
        let material = Material::from_id(material_id).ok_or(ProtocolError::UnknownEnum(
            "material",
            u64::from(material_id),
        ))?;
        if cursor.u16()? != 0 {
            return Err(ProtocolError::InvalidPayload(
                "reserved voxel mutation field is nonzero",
            ));
        }
        mutations.push(VoxelMutation { coord, material });
    }
    let mut affected_chunks = Vec::with_capacity(chunk_count);
    for _ in 0..chunk_count {
        affected_chunks.push(decode_chunk_coord(&mut cursor)?);
    }
    let mut affected_surface_tiles = Vec::with_capacity(surface_count);
    for _ in 0..surface_count {
        affected_surface_tiles.push(decode_surface_coord(&mut cursor)?);
    }
    let editor_inventory = (inventory_present == 1)
        .then(|| decode_inventory(&mut cursor))
        .transpose()?;
    cursor.finish()?;
    let commit = EditCommit {
        operation_id: frame.request_id,
        edit_session_id,
        editor_connection_id,
        revision,
        mutations,
        affected_chunks,
        affected_surface_tiles,
        editor_inventory,
    };
    validate_edit_commit(&commit)?;
    Ok(commit)
}

pub fn encode_resync_required(resync: ResyncRequired) -> Result<Vec<u8>, ProtocolError> {
    if resync.revision == 0 {
        return Err(ProtocolError::InvalidPayload(
            "resync revision must be nonzero",
        ));
    }
    let mut payload = Vec::with_capacity(8);
    push_u64(&mut payload, resync.revision);
    Ok(encode_frame(KIND_RESYNC_REQUIRED, 0, &payload))
}

pub fn decode_resync_required(bytes: &[u8]) -> Result<ResyncRequired, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_RESYNC_REQUIRED)?;
    if frame.request_id != 0 {
        return Err(ProtocolError::InvalidPayload(
            "resync request id must be zero",
        ));
    }
    let mut cursor = Cursor::new(frame.payload);
    let revision = cursor.u64()?;
    cursor.finish()?;
    if revision == 0 {
        return Err(ProtocolError::InvalidPayload(
            "resync revision must be nonzero",
        ));
    }
    Ok(ResyncRequired { revision })
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

pub fn encode_open_presence(open: OpenPresence) -> Result<Vec<u8>, ProtocolError> {
    if open.session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("presence session id is nil"));
    }
    Ok(encode_frame(
        KIND_OPEN_PRESENCE,
        0,
        open.session_id.as_bytes(),
    ))
}

pub fn decode_open_presence(bytes: &[u8]) -> Result<OpenPresence, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_OPEN_PRESENCE)?;
    let mut cursor = Cursor::new(frame.payload);
    let session_id = PresenceSessionId::from_bytes(cursor.array()?);
    cursor.finish()?;
    if session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("presence session id is nil"));
    }
    Ok(OpenPresence { session_id })
}

pub fn encode_presence_opened(opened: PresenceOpened) -> Result<Vec<u8>, ProtocolError> {
    if opened.connection_id == 0
        || opened.server_time_ms == 0
        || opened.broadcast_interval_ms == 0
        || opened.max_players == 0
        || usize::from(opened.max_players) > MAX_PLAYERS_PER_PRESENCE_DELTA
    {
        return Err(ProtocolError::InvalidPayload("invalid PresenceOpened body"));
    }
    let mut payload = Vec::with_capacity(24);
    push_u64(&mut payload, opened.connection_id);
    push_u64(&mut payload, opened.server_time_ms);
    push_u16(&mut payload, opened.broadcast_interval_ms);
    push_u16(&mut payload, opened.max_players);
    push_u32(&mut payload, 0);
    Ok(encode_frame(KIND_PRESENCE_OPENED, 0, &payload))
}

pub fn decode_presence_opened(bytes: &[u8]) -> Result<PresenceOpened, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_PRESENCE_OPENED)?;
    let mut cursor = Cursor::new(frame.payload);
    let opened = PresenceOpened {
        connection_id: cursor.u64()?,
        server_time_ms: cursor.u64()?,
        broadcast_interval_ms: cursor.u16()?,
        max_players: cursor.u16()?,
    };
    if cursor.u32()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved PresenceOpened field is nonzero",
        ));
    }
    cursor.finish()?;
    if opened.connection_id == 0
        || opened.server_time_ms == 0
        || opened.broadcast_interval_ms == 0
        || opened.max_players == 0
        || usize::from(opened.max_players) > MAX_PLAYERS_PER_PRESENCE_DELTA
    {
        return Err(ProtocolError::InvalidPayload("invalid PresenceOpened body"));
    }
    Ok(opened)
}

pub fn encode_player_pose(pose: PlayerPoseUpdate) -> Result<Vec<u8>, ProtocolError> {
    validate_player_pose(&pose, false)?;
    let mut payload = Vec::with_capacity(56);
    encode_player_pose_body(&mut payload, pose);
    Ok(encode_frame(KIND_PLAYER_POSE, 0, &payload))
}

pub fn decode_player_pose(bytes: &[u8]) -> Result<PlayerPoseUpdate, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_PLAYER_POSE)?;
    let mut cursor = Cursor::new(frame.payload);
    let pose = decode_player_pose_body(&mut cursor)?;
    cursor.finish()?;
    validate_player_pose(&pose, false)?;
    Ok(pose)
}

pub fn encode_presence_delta(delta: &PresenceDelta) -> Result<Vec<u8>, ProtocolError> {
    if delta.stream_sequence == 0 || delta.server_time_ms == 0 {
        return Err(ProtocolError::InvalidPayload(
            "presence delta sequence or time is zero",
        ));
    }
    if usize::from(delta.visible_player_count) > MAX_PLAYERS_PER_PRESENCE_DELTA
        || delta.enters.len() > MAX_PLAYERS_PER_PRESENCE_DELTA
        || delta.updates.len() > MAX_PLAYERS_PER_PRESENCE_DELTA
        || delta.leaves.len() > MAX_PLAYERS_PER_PRESENCE_DELTA
    {
        return Err(ProtocolError::LimitExceeded("presence delta players"));
    }
    validate_presence_delta(delta)?;
    let mut payload = Vec::with_capacity(
        24 + delta.enters.len() * 80 + delta.updates.len() * 60 + delta.leaves.len() * 8,
    );
    push_u64(&mut payload, delta.stream_sequence);
    push_u64(&mut payload, delta.server_time_ms);
    push_u16(&mut payload, delta.visible_player_count);
    push_u16(&mut payload, delta.enters.len() as u16);
    push_u16(&mut payload, delta.updates.len() as u16);
    push_u16(&mut payload, delta.leaves.len() as u16);
    for player in &delta.enters {
        payload.extend_from_slice(player.player_id.as_bytes());
        push_u64(&mut payload, player.connection_id);
        push_u16(&mut payload, player.color_index);
        push_u16(&mut payload, 0);
        encode_player_pose_body(&mut payload, player.pose);
    }
    for update in &delta.updates {
        push_u64(&mut payload, update.connection_id);
        encode_player_pose_body(&mut payload, update.pose);
    }
    for connection_id in &delta.leaves {
        push_u64(&mut payload, *connection_id);
    }
    Ok(encode_frame(KIND_PRESENCE_DELTA, 0, &payload))
}

pub fn decode_presence_delta(bytes: &[u8]) -> Result<PresenceDelta, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_PRESENCE_DELTA)?;
    let mut cursor = Cursor::new(frame.payload);
    let stream_sequence = cursor.u64()?;
    let server_time_ms = cursor.u64()?;
    let visible_player_count = cursor.u16()?;
    let enter_count = usize::from(cursor.u16()?);
    let update_count = usize::from(cursor.u16()?);
    let leave_count = usize::from(cursor.u16()?);
    if stream_sequence == 0 || server_time_ms == 0 {
        return Err(ProtocolError::InvalidPayload(
            "presence delta sequence or time is zero",
        ));
    }
    if usize::from(visible_player_count) > MAX_PLAYERS_PER_PRESENCE_DELTA
        || enter_count > MAX_PLAYERS_PER_PRESENCE_DELTA
        || update_count > MAX_PLAYERS_PER_PRESENCE_DELTA
        || leave_count > MAX_PLAYERS_PER_PRESENCE_DELTA
    {
        return Err(ProtocolError::LimitExceeded("presence delta players"));
    }
    let mut enters = Vec::with_capacity(enter_count);
    for _ in 0..enter_count {
        let player_id = PlayerId::from_bytes(cursor.array()?);
        let connection_id = cursor.u64()?;
        let color_index = cursor.u16()?;
        if cursor.u16()? != 0 {
            return Err(ProtocolError::InvalidPayload(
                "reserved player presence field is nonzero",
            ));
        }
        let pose = decode_player_pose_body(&mut cursor)?;
        enters.push(PlayerPresenceState {
            player_id,
            connection_id,
            color_index,
            pose,
        });
    }
    let mut updates = Vec::with_capacity(update_count);
    for _ in 0..update_count {
        updates.push(PlayerPresenceUpdate {
            connection_id: cursor.u64()?,
            pose: decode_player_pose_body(&mut cursor)?,
        });
    }
    let mut leaves = Vec::with_capacity(leave_count);
    for _ in 0..leave_count {
        leaves.push(cursor.u64()?);
    }
    cursor.finish()?;
    let delta = PresenceDelta {
        stream_sequence,
        server_time_ms,
        visible_player_count,
        enters,
        updates,
        leaves,
    };
    validate_presence_delta(&delta)?;
    Ok(delta)
}

pub fn encode_presence_ping(ping: PresencePing) -> Result<Vec<u8>, ProtocolError> {
    if ping.sequence == 0 || ping.client_send_time_ms == 0 {
        return Err(ProtocolError::InvalidPayload("invalid presence ping"));
    }
    let mut payload = Vec::with_capacity(16);
    push_u32(&mut payload, ping.sequence);
    push_u32(&mut payload, ping.observed_round_trip_ms);
    push_u64(&mut payload, ping.client_send_time_ms);
    Ok(encode_frame(KIND_PRESENCE_PING, 0, &payload))
}

pub fn decode_presence_ping(bytes: &[u8]) -> Result<PresencePing, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_PRESENCE_PING)?;
    let mut cursor = Cursor::new(frame.payload);
    let sequence = cursor.u32()?;
    let observed_round_trip_ms = cursor.u32()?;
    let client_send_time_ms = cursor.u64()?;
    cursor.finish()?;
    if sequence == 0 || client_send_time_ms == 0 {
        return Err(ProtocolError::InvalidPayload("invalid presence ping"));
    }
    Ok(PresencePing {
        sequence,
        observed_round_trip_ms,
        client_send_time_ms,
    })
}

pub fn encode_presence_pong(pong: PresencePong) -> Result<Vec<u8>, ProtocolError> {
    if pong.sequence == 0
        || pong.outbound_rate_bytes_per_second == 0
        || pong.client_send_time_ms == 0
        || pong.server_receive_time_ms == 0
        || pong.server_send_time_ms < pong.server_receive_time_ms
    {
        return Err(ProtocolError::InvalidPayload("invalid presence pong"));
    }
    let mut payload = Vec::with_capacity(32);
    push_u32(&mut payload, pong.sequence);
    push_u32(&mut payload, pong.outbound_rate_bytes_per_second);
    push_u64(&mut payload, pong.client_send_time_ms);
    push_u64(&mut payload, pong.server_receive_time_ms);
    push_u64(&mut payload, pong.server_send_time_ms);
    Ok(encode_frame(KIND_PRESENCE_PONG, 0, &payload))
}

pub fn decode_presence_pong(bytes: &[u8]) -> Result<PresencePong, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_zero_request_id(&frame)?;
    expect_kind(&frame, KIND_PRESENCE_PONG)?;
    let mut cursor = Cursor::new(frame.payload);
    let sequence = cursor.u32()?;
    let outbound_rate_bytes_per_second = cursor.u32()?;
    let pong = PresencePong {
        sequence,
        outbound_rate_bytes_per_second,
        client_send_time_ms: cursor.u64()?,
        server_receive_time_ms: cursor.u64()?,
        server_send_time_ms: cursor.u64()?,
    };
    cursor.finish()?;
    if pong.sequence == 0
        || pong.outbound_rate_bytes_per_second == 0
        || pong.client_send_time_ms == 0
        || pong.server_receive_time_ms == 0
        || pong.server_send_time_ms < pong.server_receive_time_ms
    {
        return Err(ProtocolError::InvalidPayload("invalid presence pong"));
    }
    Ok(pong)
}

pub fn encode_frame_fragment(
    transfer_id: u64,
    total_bytes: usize,
    offset: usize,
    bytes: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let total_bytes =
        u32::try_from(total_bytes).map_err(|_| ProtocolError::LimitExceeded("fragmented frame"))?;
    let offset =
        u32::try_from(offset).map_err(|_| ProtocolError::LimitExceeded("fragment offset"))?;
    validate_frame_fragment_fields(transfer_id, total_bytes, offset, bytes.len())?;
    let mut payload = Vec::with_capacity(8 + bytes.len());
    push_u32(&mut payload, total_bytes);
    push_u32(&mut payload, offset);
    payload.extend_from_slice(bytes);
    Ok(encode_frame(KIND_FRAME_FRAGMENT, transfer_id, &payload))
}

pub fn decode_frame_fragment(bytes: &[u8]) -> Result<FrameFragment, ProtocolError> {
    let frame = decode_frame(bytes)?;
    expect_kind(&frame, KIND_FRAME_FRAGMENT)?;
    let mut cursor = Cursor::new(frame.payload);
    let fragment = FrameFragment {
        transfer_id: frame.request_id,
        total_bytes: cursor.u32()?,
        offset: cursor.u32()?,
        bytes: cursor.bytes(cursor.remaining())?.to_vec(),
    };
    cursor.finish()?;
    validate_frame_fragment(&fragment)?;
    Ok(fragment)
}

fn validate_frame_fragment(fragment: &FrameFragment) -> Result<(), ProtocolError> {
    validate_frame_fragment_fields(
        fragment.transfer_id,
        fragment.total_bytes,
        fragment.offset,
        fragment.bytes.len(),
    )
}

fn validate_frame_fragment_fields(
    transfer_id: u64,
    total_bytes: u32,
    offset: u32,
    fragment_bytes: usize,
) -> Result<(), ProtocolError> {
    let total_bytes = total_bytes as usize;
    let offset = offset as usize;
    if transfer_id == 0
        || !(FRAME_HEADER_BYTES..=MAX_PROTOCOL_FRAME_BYTES).contains(&total_bytes)
        || fragment_bytes == 0
        || fragment_bytes > MAX_FRAME_FRAGMENT_DATA_BYTES
        || offset >= total_bytes
        || offset
            .checked_add(fragment_bytes)
            .is_none_or(|end| end > total_bytes)
    {
        return Err(ProtocolError::InvalidPayload("invalid frame fragment"));
    }
    Ok(())
}

impl FrameReassembler {
    pub fn accept(&mut self, bytes: &[u8]) -> Result<Option<Vec<u8>>, ProtocolError> {
        const MAX_ACTIVE_TRANSFERS: usize = 32;

        let fragment = decode_frame_fragment(bytes)?;
        let transfer_id = fragment.transfer_id;
        let total_bytes = fragment.total_bytes as usize;
        let offset = fragment.offset as usize;
        if offset == 0 {
            if self.transfers.contains_key(&transfer_id) {
                return Err(ProtocolError::InvalidPayload(
                    "duplicate frame fragment start",
                ));
            }
            if self.transfers.len() >= MAX_ACTIVE_TRANSFERS {
                return Err(ProtocolError::LimitExceeded("concurrent fragmented frames"));
            }
            self.transfers.insert(
                transfer_id,
                PartialFrame {
                    total_bytes,
                    bytes: Vec::with_capacity(fragment.bytes.len()),
                },
            );
        }
        let Some(partial) = self.transfers.get_mut(&transfer_id) else {
            return Err(ProtocolError::InvalidPayload("frame fragment has no start"));
        };
        if partial.total_bytes != total_bytes || partial.bytes.len() != offset {
            return Err(ProtocolError::InvalidPayload(
                "frame fragments are not contiguous",
            ));
        }
        partial.bytes.extend_from_slice(&fragment.bytes);
        if partial.bytes.len() < partial.total_bytes {
            return Ok(None);
        }
        let completed = self
            .transfers
            .remove(&transfer_id)
            .ok_or(ProtocolError::InvalidPayload(
                "frame fragment has no registered transfer",
            ))?
            .bytes;
        let frame = decode_frame(&completed)?;
        if frame.kind == KIND_FRAME_FRAGMENT || frame.request_id != transfer_id {
            return Err(ProtocolError::InvalidPayload(
                "fragmented frame identity mismatch",
            ));
        }
        Ok(Some(completed))
    }

    pub fn clear(&mut self) {
        self.transfers.clear();
    }
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

pub fn message_request_id(bytes: &[u8]) -> Result<u64, ProtocolError> {
    Ok(decode_frame(bytes)?.request_id)
}

/// Clones one validated frame while replacing only its request identifier.
///
/// Result batches use this to share an immutable compressed payload across clients without
/// decompressing or recompressing it. The payload length and every payload byte remain unchanged.
pub fn clone_message_with_request_id(
    bytes: &[u8],
    request_id: u64,
) -> Result<Vec<u8>, ProtocolError> {
    decode_frame(bytes)?;
    let mut cloned = bytes.to_vec();
    cloned[12..20].copy_from_slice(&request_id.to_le_bytes());
    Ok(cloned)
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

pub const fn open_presence_kind() -> u16 {
    KIND_OPEN_PRESENCE
}

pub const fn presence_opened_kind() -> u16 {
    KIND_PRESENCE_OPENED
}

pub const fn player_pose_kind() -> u16 {
    KIND_PLAYER_POSE
}

pub const fn presence_delta_kind() -> u16 {
    KIND_PRESENCE_DELTA
}

pub const fn presence_ping_kind() -> u16 {
    KIND_PRESENCE_PING
}

pub const fn presence_pong_kind() -> u16 {
    KIND_PRESENCE_PONG
}

pub const fn edit_command_kind() -> u16 {
    KIND_EDIT_COMMAND
}

pub const fn edit_commit_kind() -> u16 {
    KIND_EDIT_COMMIT
}

pub const fn resync_required_kind() -> u16 {
    KIND_RESYNC_REQUIRED
}

pub const fn frame_fragment_kind() -> u16 {
    KIND_FRAME_FRAGMENT
}

fn encode_player_pose_body(output: &mut Vec<u8>, pose: PlayerPoseUpdate) {
    push_u64(output, pose.sequence);
    push_u64(output, pose.sample_server_time_ms);
    for value in pose.eye_position_metres {
        push_f32(output, value);
    }
    for value in pose.linear_velocity_metres_per_second {
        push_f32(output, value);
    }
    push_f32(output, pose.look_yaw_radians);
    push_f32(output, pose.look_pitch_radians);
    push_u16(output, pose.flags);
    push_u16(output, 0);
}

fn decode_player_pose_body(cursor: &mut Cursor<'_>) -> Result<PlayerPoseUpdate, ProtocolError> {
    let pose = PlayerPoseUpdate {
        sequence: cursor.u64()?,
        sample_server_time_ms: cursor.u64()?,
        eye_position_metres: [cursor.f32()?, cursor.f32()?, cursor.f32()?],
        linear_velocity_metres_per_second: [cursor.f32()?, cursor.f32()?, cursor.f32()?],
        look_yaw_radians: cursor.f32()?,
        look_pitch_radians: cursor.f32()?,
        flags: cursor.u16()?,
    };
    if cursor.u16()? != 0 {
        return Err(ProtocolError::InvalidPayload(
            "reserved player pose field is nonzero",
        ));
    }
    Ok(pose)
}

fn encode_player_resume(output: &mut Vec<u8>, resume: PlayerResume) {
    push_u64(output, resume.revision);
    for value in resume.eye_position_metres {
        push_f32(output, value);
    }
    push_f32(output, resume.look_yaw_radians);
    push_f32(output, resume.look_pitch_radians);
}

fn decode_player_resume(cursor: &mut Cursor<'_>) -> Result<PlayerResume, ProtocolError> {
    let resume = PlayerResume {
        revision: cursor.u64()?,
        eye_position_metres: [cursor.f32()?, cursor.f32()?, cursor.f32()?],
        look_yaw_radians: cursor.f32()?,
        look_pitch_radians: cursor.f32()?,
    };
    validate_player_resume(&resume)?;
    Ok(resume)
}

fn validate_player_resume(resume: &PlayerResume) -> Result<(), ProtocolError> {
    if resume.revision == 0 {
        return Err(ProtocolError::InvalidPayload(
            "player resume revision is zero",
        ));
    }
    let position_limit = (i32::MAX as f32 - 64.0) * VOXEL_SIZE_METRES;
    if !resume
        .eye_position_metres
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= position_limit)
    {
        return Err(ProtocolError::InvalidPayload(
            "player resume position is nonfinite or outside world bounds",
        ));
    }
    if !resume.look_yaw_radians.is_finite()
        || !(-std::f32::consts::PI..=std::f32::consts::PI).contains(&resume.look_yaw_radians)
        || !resume.look_pitch_radians.is_finite()
        || !(-1.5..=1.5).contains(&resume.look_pitch_radians)
    {
        return Err(ProtocolError::InvalidPayload(
            "player resume look angles are invalid",
        ));
    }
    Ok(())
}

fn encode_inventory(output: &mut Vec<u8>, inventory: MaterialInventory) {
    push_u64(output, inventory.revision);
    for count in inventory.counts {
        push_u64(output, count);
    }
}

fn decode_inventory(cursor: &mut Cursor<'_>) -> Result<MaterialInventory, ProtocolError> {
    let revision = cursor.u64()?;
    let mut counts = [0; MATERIAL_INVENTORY_SLOTS];
    for count in &mut counts {
        *count = cursor.u64()?;
    }
    let inventory = MaterialInventory { revision, counts };
    validate_inventory(&inventory)?;
    Ok(inventory)
}

fn validate_inventory(inventory: &MaterialInventory) -> Result<(), ProtocolError> {
    if inventory.revision == 0 {
        return Err(ProtocolError::InvalidPayload("inventory revision is zero"));
    }
    if inventory.counts[Material::Air.id() as usize] != 0 {
        return Err(ProtocolError::InvalidPayload(
            "air inventory count is nonzero",
        ));
    }
    Ok(())
}

fn validate_player_pose(
    pose: &PlayerPoseUpdate,
    require_sample_time: bool,
) -> Result<(), ProtocolError> {
    if pose.sequence == 0 || (require_sample_time && pose.sample_server_time_ms == 0) {
        return Err(ProtocolError::InvalidPayload(
            "player pose sequence or sample time is zero",
        ));
    }
    let position_limit = (i32::MAX as f32 - 64.0) * VOXEL_SIZE_METRES;
    if !pose
        .eye_position_metres
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= position_limit)
    {
        return Err(ProtocolError::InvalidPayload(
            "player position is nonfinite or outside world bounds",
        ));
    }
    let velocity_squared = pose
        .linear_velocity_metres_per_second
        .into_iter()
        .map(|value| value * value)
        .sum::<f32>();
    if !velocity_squared.is_finite() || velocity_squared > 64.0 * 64.0 {
        return Err(ProtocolError::InvalidPayload(
            "player velocity is nonfinite or too large",
        ));
    }
    if !pose.look_yaw_radians.is_finite()
        || !(-std::f32::consts::PI..=std::f32::consts::PI).contains(&pose.look_yaw_radians)
        || !pose.look_pitch_radians.is_finite()
        || !(-1.5..=1.5).contains(&pose.look_pitch_radians)
    {
        return Err(ProtocolError::InvalidPayload(
            "player look angles are invalid",
        ));
    }
    if pose.flags & !PLAYER_POSE_FLAGS != 0 {
        return Err(ProtocolError::InvalidPayload(
            "player pose has unknown flags",
        ));
    }
    if pose.flags & PLAYER_POSE_GLIDING != 0
        && pose.flags & (PLAYER_POSE_GROUNDED | PLAYER_POSE_SWIMMING | PLAYER_POSE_SPECTATOR) != 0
    {
        return Err(ProtocolError::InvalidPayload(
            "gliding conflicts with another locomotion flag",
        ));
    }
    if pose.flags & PLAYER_POSE_SPECTATOR != 0
        && pose.flags & (PLAYER_POSE_GROUNDED | PLAYER_POSE_SWIMMING | PLAYER_POSE_GLIDING) != 0
    {
        return Err(ProtocolError::InvalidPayload(
            "spectator conflicts with a player locomotion flag",
        ));
    }
    Ok(())
}

fn validate_presence_players(players: &[PlayerPresenceState]) -> Result<(), ProtocolError> {
    let mut prior_player = None;
    let mut connections = BTreeSet::new();
    let mut colors = BTreeSet::new();
    for player in players {
        if player.player_id.is_nil() || player.connection_id == 0 {
            return Err(ProtocolError::InvalidPayload(
                "presence player id or connection id is invalid",
            ));
        }
        if prior_player.is_some_and(|prior| prior >= player.player_id) {
            return Err(ProtocolError::InvalidPayload(
                "presence players are not strictly sorted",
            ));
        }
        if !connections.insert(player.connection_id)
            || usize::from(player.color_index) >= MAX_PLAYERS_PER_PRESENCE_DELTA
            || !colors.insert(player.color_index)
        {
            return Err(ProtocolError::InvalidPayload(
                "presence player colors are invalid or duplicated",
            ));
        }
        validate_player_pose(&player.pose, true)?;
        if player.pose.flags & PLAYER_POSE_SPECTATOR != 0 {
            return Err(ProtocolError::InvalidPayload(
                "spectators cannot be replicated as player presence",
            ));
        }
        prior_player = Some(player.player_id);
    }
    Ok(())
}

fn validate_presence_delta(delta: &PresenceDelta) -> Result<(), ProtocolError> {
    validate_presence_players(&delta.enters)?;
    let entered = delta
        .enters
        .iter()
        .map(|player| player.connection_id)
        .collect::<BTreeSet<_>>();
    let mut prior_update = None;
    for update in &delta.updates {
        if update.connection_id == 0
            || entered.contains(&update.connection_id)
            || prior_update.is_some_and(|prior| prior >= update.connection_id)
        {
            return Err(ProtocolError::InvalidPayload(
                "presence updates are invalid, duplicated, or not strictly sorted",
            ));
        }
        validate_player_pose(&update.pose, true)?;
        if update.pose.flags & PLAYER_POSE_SPECTATOR != 0 {
            return Err(ProtocolError::InvalidPayload(
                "spectators cannot be replicated as player presence",
            ));
        }
        prior_update = Some(update.connection_id);
    }
    let updated = delta
        .updates
        .iter()
        .map(|update| update.connection_id)
        .collect::<BTreeSet<_>>();
    let mut prior_leave = None;
    for connection_id in &delta.leaves {
        if *connection_id == 0
            || entered.contains(connection_id)
            || updated.contains(connection_id)
            || prior_leave.is_some_and(|prior| prior >= *connection_id)
        {
            return Err(ProtocolError::InvalidPayload(
                "presence leaves are invalid, duplicated, or not strictly sorted",
            ));
        }
        prior_leave = Some(*connection_id);
    }
    Ok(())
}

fn validate_edit_commit(commit: &EditCommit) -> Result<(), ProtocolError> {
    if commit.operation_id == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit operation id must be nonzero",
        ));
    }
    if commit.editor_connection_id == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit connection id must be nonzero",
        ));
    }
    if commit.revision == 0 {
        return Err(ProtocolError::InvalidPayload(
            "edit revision must be nonzero",
        ));
    }
    if commit.edit_session_id.is_nil() {
        return Err(ProtocolError::InvalidPayload("edit session id is nil"));
    }
    if commit.mutations.len() > MAX_EDIT_MUTATIONS {
        return Err(ProtocolError::LimitExceeded("edit mutations"));
    }
    if !commit
        .mutations
        .windows(2)
        .all(|pair| pair[0].coord < pair[1].coord)
    {
        return Err(ProtocolError::InvalidPayload(
            "edit mutations are not strictly coordinate-sorted",
        ));
    }
    if commit
        .mutations
        .iter()
        .any(|mutation| validate_voxel_coord(mutation.coord).is_err())
    {
        return Err(ProtocolError::InvalidPayload("invalid voxel coordinate"));
    }
    if commit.affected_chunks.len() > MAX_EDIT_AFFECTED_CHUNKS {
        return Err(ProtocolError::LimitExceeded("edit affected chunks"));
    }
    if (commit.mutations.is_empty()
        && (!commit.affected_chunks.is_empty() || !commit.affected_surface_tiles.is_empty()))
        || (!commit.mutations.is_empty() && commit.affected_chunks.is_empty())
    {
        return Err(ProtocolError::InvalidPayload(
            "no-op edit has affected products or changed edit omits them",
        ));
    }
    if commit.affected_surface_tiles.len() > MAX_EDIT_AFFECTED_SURFACE_TILES {
        return Err(ProtocolError::LimitExceeded("edit affected surface tiles"));
    }
    if !strictly_sorted(&commit.affected_chunks) {
        return Err(ProtocolError::InvalidPayload(
            "edit affected chunks are not strictly sorted",
        ));
    }
    if commit
        .mutations
        .iter()
        .any(|mutation| !commit.affected_chunks.contains(&mutation.coord.chunk()))
    {
        return Err(ProtocolError::InvalidPayload(
            "edit affected chunks omit a mutated chunk",
        ));
    }
    if commit
        .mutations
        .iter()
        .flat_map(|mutation| crate::EditMap::affected_chunks(mutation.coord))
        .any(|coord| !commit.affected_chunks.contains(&coord))
    {
        return Err(ProtocolError::InvalidPayload(
            "edit affected chunks omit a mutation halo",
        ));
    }
    if commit
        .affected_chunks
        .iter()
        .any(|coord| !coord.is_world_representable())
    {
        return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
    }
    if !strictly_sorted(&commit.affected_surface_tiles) {
        return Err(ProtocolError::InvalidPayload(
            "edit affected surface tiles are not strictly sorted",
        ));
    }
    if commit
        .affected_surface_tiles
        .iter()
        .any(|coord| !coord.is_world_representable())
    {
        return Err(ProtocolError::InvalidPayload(
            "invalid surface tile coordinate",
        ));
    }
    if let Some(inventory) = &commit.editor_inventory {
        validate_inventory(inventory)?;
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
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

fn encode_result_payload(uncompressed: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if uncompressed.is_empty() || uncompressed.len() + FRAME_HEADER_BYTES > MAX_PROTOCOL_FRAME_BYTES
    {
        return Err(ProtocolError::LimitExceeded("uncompressed result bytes"));
    }
    let uncompressed_len = u32::try_from(uncompressed.len())
        .map_err(|_| ProtocolError::LimitExceeded("uncompressed result bytes"))?;
    let params = brotli::enc::BrotliEncoderParams {
        quality: BROTLI_QUALITY as i32,
        lgwin: BROTLI_WINDOW_BITS as i32,
        ..Default::default()
    };
    let mut input = uncompressed;
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut input, &mut compressed, &params)
        .map_err(|_| ProtocolError::Compression("could not encode result payload"))?;
    let mut payload = Vec::with_capacity(RESULT_ENVELOPE_BYTES + compressed.len());
    payload.push(RESULT_CODEC_BROTLI);
    payload.extend_from_slice(&[0; 3]);
    push_u32(&mut payload, uncompressed_len);
    payload.extend_from_slice(&compressed);
    if payload.len() + FRAME_HEADER_BYTES > MAX_PROTOCOL_FRAME_BYTES {
        return Err(ProtocolError::LimitExceeded("compressed result bytes"));
    }
    Ok(payload)
}

fn decode_result_payload(payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if payload.len() <= RESULT_ENVELOPE_BYTES {
        return Err(ProtocolError::Truncated);
    }
    if payload[0] != RESULT_CODEC_BROTLI {
        return Err(ProtocolError::UnknownEnum(
            "result compression codec",
            u64::from(payload[0]),
        ));
    }
    if payload[1..4] != [0; 3] {
        return Err(ProtocolError::InvalidPayload(
            "reserved result compression bytes are nonzero",
        ));
    }
    let uncompressed_len = read_u32(payload, 4)? as usize;
    if uncompressed_len == 0 || uncompressed_len + FRAME_HEADER_BYTES > MAX_PROTOCOL_FRAME_BYTES {
        return Err(ProtocolError::LimitExceeded("uncompressed result bytes"));
    }
    let mut decompressed = Vec::with_capacity(uncompressed_len);
    let decoder = brotli::Decompressor::new(&payload[RESULT_ENVELOPE_BYTES..], BROTLI_BUFFER_BYTES);
    decoder
        .take((uncompressed_len + 1) as u64)
        .read_to_end(&mut decompressed)
        .map_err(|_| ProtocolError::Compression("could not decode result payload"))?;
    if decompressed.len() != uncompressed_len {
        return Err(ProtocolError::Compression(
            "decoded result length differs from envelope",
        ));
    }
    Ok(decompressed)
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

fn expect_zero_request_id(frame: &Frame<'_>) -> Result<(), ProtocolError> {
    if frame.request_id != 0 {
        return Err(ProtocolError::InvalidPayload(
            "presence request id must be zero",
        ));
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

fn push_chunk_coord(output: &mut Vec<u8>, coord: ChunkCoord) {
    push_i32(output, coord.x);
    push_i32(output, coord.y);
    push_i32(output, coord.z);
}

fn decode_chunk_coord(cursor: &mut Cursor<'_>) -> Result<ChunkCoord, ProtocolError> {
    let coord = ChunkCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?);
    if !coord.is_world_representable() {
        return Err(ProtocolError::InvalidPayload("invalid chunk coordinate"));
    }
    Ok(coord)
}

fn encode_voxel_coord(output: &mut Vec<u8>, coord: VoxelCoord) {
    push_i32(output, coord.x);
    push_i32(output, coord.y);
    push_i32(output, coord.z);
}

fn decode_voxel_coord(cursor: &mut Cursor<'_>) -> Result<VoxelCoord, ProtocolError> {
    let coord = VoxelCoord::new(cursor.i32()?, cursor.i32()?, cursor.i32()?);
    validate_voxel_coord(coord)?;
    Ok(coord)
}

fn validate_voxel_coord(coord: VoxelCoord) -> Result<(), ProtocolError> {
    if !coord.chunk().is_world_representable() {
        return Err(ProtocolError::InvalidPayload("invalid voxel coordinate"));
    }
    Ok(())
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
        for range in &patch.edge_ranges {
            encode_range(output, range);
        }
        encode_surface_bounds(output, patch.bounds);
    }
    for height in &mesh.shading.heights {
        push_i32(output, *height);
    }
    for height in &mesh.shading.parent_heights {
        push_i32(output, *height);
    }
    output.extend_from_slice(&mesh.shading.horizons);
    output.extend_from_slice(&mesh.shading.parent_horizons);
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
        let edge_ranges = [
            decode_range(cursor)?,
            decode_range(cursor)?,
            decode_range(cursor)?,
            decode_range(cursor)?,
        ];
        let bounds = decode_surface_bounds(cursor)?;
        patches.push(SurfacePatch {
            cell_bounds,
            quad_range,
            edge_ranges,
            bounds,
        });
    }
    let heights = (0..SURFACE_SHADING_EDGE_SAMPLES.pow(2))
        .map(|_| cursor.i32())
        .collect::<Result<Vec<_>, _>>()?;
    let parent_heights = if coord.level.next_coarser().is_some() {
        (0..SURFACE_PARENT_SHADING_EDGE_SAMPLES.pow(2))
            .map(|_| cursor.i32())
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let horizons = cursor.bytes(SURFACE_HORIZON_CELL_COUNT)?.to_vec();
    let parent_horizons = if coord.level.next_coarser().is_some() {
        cursor.bytes(SURFACE_PARENT_HORIZON_CELL_COUNT)?.to_vec()
    } else {
        Vec::new()
    };
    let mesh = SurfaceTileMesh {
        coord,
        quads,
        patches,
        shading: SurfaceShading {
            heights,
            parent_heights,
            horizons,
            parent_horizons,
        },
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
    let patches_per_edge = crate::SURFACE_PATCHES_PER_TILE_EDGE as usize;
    let expected_patch_count = patches_per_edge * patches_per_edge;
    if mesh.quads.len() > MAX_SURFACE_QUADS_PER_TILE {
        return Err(ProtocolError::LimitExceeded("surface mesh geometry"));
    }
    let expected_parent_heights = if mesh.coord.level.next_coarser().is_some() {
        SURFACE_PARENT_SHADING_EDGE_SAMPLES.pow(2)
    } else {
        0
    };
    let expected_parent_horizons = if mesh.coord.level.next_coarser().is_some() {
        SURFACE_PARENT_HORIZON_CELL_COUNT
    } else {
        0
    };
    if mesh.shading.heights.len() != SURFACE_SHADING_EDGE_SAMPLES.pow(2)
        || mesh.shading.parent_heights.len() != expected_parent_heights
        || mesh.shading.horizons.len() != SURFACE_HORIZON_CELL_COUNT
        || mesh.shading.parent_horizons.len() != expected_parent_horizons
    {
        return Err(ProtocolError::InvalidPayload(
            "surface shading grid has an invalid shape",
        ));
    }
    if mesh.patches.len() != expected_patch_count {
        return Err(ProtocolError::InvalidPayload(
            "surface terrain must contain a complete patch grid",
        ));
    }
    for (index, patch) in mesh.patches.iter().enumerate() {
        validate_cell_bounds(patch.cell_bounds)?;
        let patch_x = index % patches_per_edge;
        let patch_z = index / patches_per_edge;
        let min_x = (patch_x as i32 * crate::SURFACE_PATCH_EDGE_CELLS) as u8;
        let min_z = (patch_z as i32 * crate::SURFACE_PATCH_EDGE_CELLS) as u8;
        let max_x = min_x + crate::SURFACE_PATCH_EDGE_CELLS as u8;
        let max_z = min_z + crate::SURFACE_PATCH_EDGE_CELLS as u8;
        if patch.cell_bounds != [[min_x, min_z], [max_x, max_z]] {
            return Err(ProtocolError::InvalidPayload(
                "surface terrain patch grid is incomplete or out of order",
            ));
        }
        validate_range(&patch.quad_range, mesh.quads.len())?;
        for range in &patch.edge_ranges {
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
        let material =
            Material::from_id(id).ok_or(ProtocolError::UnknownEnum("material", u64::from(id)))?;
        if palette
            .last()
            .is_some_and(|prior: &Material| prior.id() >= id)
        {
            return Err(ProtocolError::InvalidPayload(
                "halo palette material ids must be strictly increasing",
            ));
        }
        palette.push(material);
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
        4 => SurfaceLodLevel::Stride32,
        5 => SurfaceLodLevel::Stride64,
        6 => SurfaceLodLevel::Stride128,
        7 => SurfaceLodLevel::Stride256,
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

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
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
    fn dig_volume_is_an_exact_symmetric_half_metre_sphere() {
        let hit = VoxelCoord::new(10, -20, 30);
        let volume = DigVolume::for_hit(hit).expect("bounded dig volume");
        assert_eq!(volume.sample_shape(), [5, 5, 5]);
        assert_eq!(volume.min, VoxelCoord::new(8, -22, 28));
        assert_eq!(volume.max, VoxelCoord::new(12, -18, 32));
        assert_eq!(volume.centre(), hit);
        assert!(volume.contains(hit));
        assert!(volume.contains(VoxelCoord::new(12, -19, 31)));
        assert!(!volume.contains(VoxelCoord::new(12, -18, 30)));
        assert!(!volume.contains(VoxelCoord::new(12, -18, 32)));

        let coordinates = volume.coordinates().collect::<BTreeSet<_>>();
        assert_eq!(coordinates.len(), DIG_VOLUME_VOXELS);
        for coord in &coordinates {
            let opposite = VoxelCoord::new(
                hit.x * 2 - coord.x,
                hit.y * 2 - coord.y,
                hit.z * 2 - coord.z,
            );
            assert!(coordinates.contains(&opposite));
        }
        assert_eq!(DIG_VOLUME_VOXELS, 81);
    }

    #[test]
    fn dig_volume_rejects_coordinate_overflow_atomically() {
        assert!(DigVolume::for_hit(VoxelCoord::new(i32::MAX, 0, 0)).is_none());
        assert!(DigVolume::for_hit(VoxelCoord::new(i32::MIN, 0, 0)).is_none());
        assert!(DigVolume::for_hit(VoxelCoord::new(0, i32::MAX, 0)).is_none());
        assert!(DigVolume::for_hit(VoxelCoord::new(0, 0, i32::MIN)).is_none());
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
                .union(WorldCapabilities::AUTHORED_ROUTES)
                .union(WorldCapabilities::GLIDING)
                .union(WorldCapabilities::SPECTATOR_MODE),
            environment: WorldEnvironmentSnapshot {
                sample_server_time_ms: 12_345,
                world_day_number: 82,
                day_fraction: 0.625,
                day_length_seconds: 1_200.0,
                days_per_year: 365.242_2,
                moon_sidereal_orbit_days: 27.321_661,
                moon_orbit_phase_at_world_epoch: 0.17,
                planet_circumference_metres: 40_075_016.0,
                axial_tilt_radians: 23.439_3_f32.to_radians(),
                moon_orbit_inclination_radians: 5.145_f32.to_radians(),
                celestial_seed: 0x57a2_5eed,
                celestial_revision: 2,
                weather_fraction: 0.68,
                weather_cycle_seconds: 900.0,
                cloud_offset_metres: [412.5, 91.25],
                cloud_velocity_metres_per_second: [5.5, 1.6],
                cloud_coverage: 0.24,
                cloud_base_metres: 420.0,
                cloud_top_metres: 780.0,
                weather_seed: 77,
                weather_revision: 3,
            },
            recommended_in_flight_batches: 4,
            identity: player_identity(7, "default"),
            connection_id: 12,
            presence_session_id: PresenceSessionId::from_bytes([9; 16]),
            edit_session_id: EditSessionId::from_bytes([10; 16]),
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
            player_resume: PlayerResume {
                revision: 3,
                eye_position_metres: [0.05, 1.95, 5.25],
                look_yaw_radians: 0.7,
                look_pitch_radians: -0.1,
            },
            inventory: MaterialInventory {
                revision: 4,
                counts: {
                    let mut counts = [0; MATERIAL_INVENTORY_SLOTS];
                    counts[Material::Stone.id() as usize] = 27;
                    counts[Material::Wood.id() as usize] = 8;
                    counts
                },
            },
        };
        assert_eq!(
            decode_world_opened(&encode_world_opened(&opened)),
            Ok(opened)
        );
    }

    #[test]
    fn presence_frames_round_trip_and_reject_invalid_state() {
        let session_id = PresenceSessionId::from_bytes([9; 16]);
        let open = OpenPresence { session_id };
        assert_eq!(
            decode_open_presence(&encode_open_presence(open).expect("encode open presence")),
            Ok(open)
        );

        let opened = PresenceOpened {
            connection_id: 7,
            server_time_ms: 1_000,
            broadcast_interval_ms: 33,
            max_players: 32,
        };
        assert_eq!(
            decode_presence_opened(
                &encode_presence_opened(opened).expect("encode presence opened")
            ),
            Ok(opened)
        );

        let pose = PlayerPoseUpdate {
            sequence: 4,
            sample_server_time_ms: 950,
            eye_position_metres: [1.0, 2.0, 3.0],
            linear_velocity_metres_per_second: [2.0, 0.0, -1.0],
            look_yaw_radians: 0.7,
            look_pitch_radians: -0.2,
            flags: PLAYER_POSE_SPECTATOR,
        };
        assert_eq!(
            decode_player_pose(&encode_player_pose(pose).expect("encode pose")),
            Ok(pose)
        );

        let visible_pose = PlayerPoseUpdate {
            flags: PLAYER_POSE_GROUNDED,
            ..pose
        };
        let delta = PresenceDelta {
            stream_sequence: 3,
            server_time_ms: 1_010,
            visible_player_count: 2,
            enters: vec![
                PlayerPresenceState {
                    player_id: PlayerId::from_bytes([1; 16]),
                    connection_id: 7,
                    color_index: 2,
                    pose: visible_pose,
                },
                PlayerPresenceState {
                    player_id: PlayerId::from_bytes([2; 16]),
                    connection_id: 8,
                    color_index: 5,
                    pose: PlayerPoseUpdate {
                        sequence: 8,
                        sample_server_time_ms: 1_000,
                        eye_position_metres: [-3.0, 1.7, 4.0],
                        linear_velocity_metres_per_second: [0.0; 3],
                        look_yaw_radians: -1.2,
                        look_pitch_radians: 0.1,
                        flags: PLAYER_POSE_GROUNDED | PLAYER_POSE_DISCONTINUITY,
                    },
                },
            ],
            updates: Vec::new(),
            leaves: Vec::new(),
        };
        assert_eq!(
            decode_presence_delta(&encode_presence_delta(&delta).expect("encode presence delta")),
            Ok(delta.clone())
        );

        let update_delta = PresenceDelta {
            stream_sequence: 4,
            server_time_ms: 1_020,
            visible_player_count: 1,
            enters: Vec::new(),
            updates: vec![PlayerPresenceUpdate {
                connection_id: 7,
                pose: PlayerPoseUpdate {
                    sequence: 5,
                    ..visible_pose
                },
            }],
            leaves: vec![8],
        };
        assert_eq!(
            decode_presence_delta(
                &encode_presence_delta(&update_delta).expect("encode update delta")
            ),
            Ok(update_delta)
        );

        let ping = PresencePing {
            sequence: 10,
            observed_round_trip_ms: 43,
            client_send_time_ms: 500,
        };
        assert_eq!(
            decode_presence_ping(&encode_presence_ping(ping).expect("encode ping")),
            Ok(ping)
        );
        let pong = PresencePong {
            sequence: ping.sequence,
            outbound_rate_bytes_per_second: 786_432,
            client_send_time_ms: ping.client_send_time_ms,
            server_receive_time_ms: 1_000,
            server_send_time_ms: 1_001,
        };
        assert_eq!(
            decode_presence_pong(&encode_presence_pong(pong).expect("encode pong")),
            Ok(pong)
        );

        let mut invalid_pose = pose;
        invalid_pose.eye_position_metres[0] = f32::NAN;
        assert!(matches!(
            encode_player_pose(invalid_pose),
            Err(ProtocolError::InvalidPayload(_))
        ));
        let mut conflicting_pose = pose;
        conflicting_pose.flags = PLAYER_POSE_GLIDING | PLAYER_POSE_GROUNDED;
        assert_eq!(
            encode_player_pose(conflicting_pose),
            Err(ProtocolError::InvalidPayload(
                "gliding conflicts with another locomotion flag"
            ))
        );

        let mut duplicate_color = delta;
        duplicate_color.enters[1].color_index = duplicate_color.enters[0].color_index;
        assert!(matches!(
            encode_presence_delta(&duplicate_color),
            Err(ProtocolError::InvalidPayload(_))
        ));

        let mut prior_version = encode_player_pose(pose).expect("encode pose version fixture");
        prior_version[4..6].copy_from_slice(&19_u16.to_le_bytes());
        assert_eq!(
            decode_player_pose(&prior_version),
            Err(ProtocolError::UnsupportedVersion(19))
        );
    }

    #[test]
    fn fragmented_frames_reassemble_strictly_across_interleaved_transfers() {
        let first = encode_frame(KIND_ERROR, 71, &vec![0x71; 9_000]);
        let second = encode_frame(KIND_ERROR, 72, &vec![0x72; 5_000]);
        let first_a = encode_frame_fragment(71, first.len(), 0, &first[..3_000]).unwrap();
        let first_b = encode_frame_fragment(71, first.len(), 3_000, &first[3_000..]).unwrap();
        let second_a = encode_frame_fragment(72, second.len(), 0, &second[..2_000]).unwrap();
        let second_b = encode_frame_fragment(72, second.len(), 2_000, &second[2_000..]).unwrap();
        let mut reassembler = FrameReassembler::default();
        assert_eq!(reassembler.accept(&first_a), Ok(None));
        assert_eq!(reassembler.accept(&second_a), Ok(None));
        assert_eq!(reassembler.accept(&second_b), Ok(Some(second)));
        assert_eq!(reassembler.accept(&first_b), Ok(Some(first)));

        let orphan = encode_frame_fragment(73, 100, 10, &[1; 10]).unwrap();
        assert!(matches!(
            FrameReassembler::default().accept(&orphan),
            Err(ProtocolError::InvalidPayload("frame fragment has no start"))
        ));
    }

    #[test]
    fn fragmented_frame_start_allocates_for_received_bytes_not_declared_total() {
        let start = encode_frame_fragment(73, MAX_PROTOCOL_FRAME_BYTES, 0, &[1]).unwrap();
        let mut reassembler = FrameReassembler::default();
        assert_eq!(reassembler.accept(&start), Ok(None));

        let partial = reassembler.transfers.get(&73).expect("active transfer");
        assert_eq!(partial.total_bytes, MAX_PROTOCOL_FRAME_BYTES);
        assert_eq!(partial.bytes.len(), 1);
        assert!(
            partial.bytes.capacity() <= MAX_FRAME_FRAGMENT_DATA_BYTES,
            "one received byte reserved {} bytes",
            partial.bytes.capacity()
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
                edit_revision: 7,
                result: Ok(snapshot),
            }],
        };
        let encoded = encode_chunk_batch_result(&response).expect("encode");
        let item = encode_chunk_batch_item(response.source_identity_hash, &response.items[0])
            .expect("encode item");
        assert_eq!(item.coord(), coord);
        assert_eq!(item.edit_revision(), 7);
        assert_eq!(
            encode_chunk_batch_result_from_items(
                response.request_id,
                response.source_identity_hash,
                [&item],
            ),
            Ok(encoded.clone())
        );
        assert!(
            encoded.len() < 5_000,
            "compressed result should stay compact"
        );
        assert_eq!(decode_chunk_batch_result(&encoded), Ok(response.clone()));
        let reassigned =
            clone_message_with_request_id(&encoded, 99).expect("clone with new request id");
        let mut expected = response.clone();
        expected.request_id = 99;
        assert_eq!(decode_chunk_batch_result(&reassigned), Ok(expected));
        assert_eq!(&reassigned[..12], &encoded[..12]);
        assert_eq!(&reassigned[20..], &encoded[20..]);

        let mut unknown_codec = encoded.clone();
        unknown_codec[FRAME_HEADER_BYTES] = 0xff;
        assert_eq!(
            decode_chunk_batch_result(&unknown_codec),
            Err(ProtocolError::UnknownEnum("result compression codec", 0xff))
        );

        let mut oversized = encoded;
        oversized[FRAME_HEADER_BYTES + 4..FRAME_HEADER_BYTES + 8]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode_chunk_batch_result(&oversized),
            Err(ProtocolError::LimitExceeded("uncompressed result bytes"))
        );

        let mut truncated = encode_chunk_batch_result(&response).expect("encode truncated");
        truncated.pop();
        let truncated_payload_len =
            u32::try_from(truncated.len() - FRAME_HEADER_BYTES).expect("test frame stays bounded");
        truncated[20..24].copy_from_slice(&truncated_payload_len.to_le_bytes());
        assert!(decode_chunk_batch_result(&truncated).is_err());
    }

    #[test]
    fn reordered_halo_palette_is_rejected_even_when_indices_preserve_voxels() {
        let coord = ChunkCoord::new(0, 0, 0);
        let identity = WorldSourceIdentity::procedural_v16(42).identity_hash();
        let mut materials = vec![Material::Air; crate::MESHING_HALO_VOXELS];
        materials[0] = Material::Stone;
        let mut encoded = encode_halo(&materials, coord, identity);
        assert_eq!(read_u16(&encoded, 4), Ok(2));
        assert_eq!(encoded[6], 1);

        encoded[44..48].rotate_left(2);
        for byte in &mut encoded[48..] {
            *byte ^= u8::MAX;
        }
        assert_eq!(
            decode_halo(&encoded, coord, identity),
            Err(ProtocolError::InvalidPayload(
                "halo palette material ids must be strictly increasing"
            ))
        );
    }

    #[test]
    fn surface_tile_round_trip_preserves_coarse_render_product() {
        let source = ProceduralWorldSource::new(42);
        let coord = SurfaceTileCoord::new(SurfaceLodLevel::Stride64, -1, 2);
        let batch = source
            .generate_batch(WorldProductBatch {
                priority: WorldProductPriority::Prefetch,
                requests: vec![WorldProductRequest::SurfaceTile(coord)],
            })
            .expect("batch");
        let snapshot = match batch.items.into_iter().next().expect("item").result {
            Ok(WorldProduct::SurfaceTile(snapshot)) => snapshot,
            other => panic!("unexpected product: {other:?}"),
        };
        let request = SurfaceTileBatchRequest {
            request_id: 10,
            priority: WorldProductPriority::Prefetch,
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
                edit_revision: 11,
                result: Ok(snapshot),
            }],
        };
        let encoded = encode_surface_tile_batch_result(&response).expect("encode");
        let item =
            encode_surface_tile_batch_item(response.source_identity_hash, &response.items[0])
                .expect("encode item");
        assert_eq!(item.coord(), coord);
        assert_eq!(item.edit_revision(), 11);
        assert_eq!(
            encode_surface_tile_batch_result_from_items(
                response.request_id,
                response.source_identity_hash,
                [&item],
            ),
            Ok(encoded.clone())
        );
        assert!(
            encoded.len() < 50_000,
            "compressed surface result should stay compact"
        );
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
        let unsupported = SURFACE_SNAPSHOT_VERSION + 1;
        encoded[4..6].copy_from_slice(&unsupported.to_le_bytes());
        assert_eq!(
            decode_surface_snapshot(&encoded, coord, source.source_identity_hash()),
            Err(ProtocolError::UnsupportedVersion(unsupported))
        );

        snapshot.terrain.patches[0].quad_range.end = u32::MAX;
        assert!(matches!(
            encode_surface_snapshot(&snapshot),
            Err(ProtocolError::InvalidPayload(
                "surface patch range is out of bounds"
            ))
        ));

        let mut missing_patch = snapshot.clone();
        missing_patch.terrain.patches.pop();
        assert_eq!(
            encode_surface_snapshot(&missing_patch),
            Err(ProtocolError::InvalidPayload(
                "surface terrain must contain a complete patch grid"
            ))
        );

        let mut out_of_order = snapshot;
        out_of_order.terrain.patches.swap(0, 1);
        assert_eq!(
            encode_surface_snapshot(&out_of_order),
            Err(ProtocolError::InvalidPayload(
                "surface terrain patch grid is incomplete or out of order"
            ))
        );
    }

    #[test]
    fn server_edit_frames_round_trip_and_reject_ambiguous_state() {
        let coord = VoxelCoord::new(31, 64, -33);
        let edit_session_id = EditSessionId::from_bytes([17; 16]);
        let command = EditCommand {
            operation_id: 41,
            edit_session_id,
            action: EditAction::Place {
                coord,
                material: Material::Basalt,
            },
        };
        let encoded_command = encode_edit_command(command).expect("encode edit command");
        assert_eq!(message_kind(&encoded_command), Ok(edit_command_kind()));
        assert_eq!(decode_edit_command(&encoded_command), Ok(command));

        let next_session = EditSessionId::from_bytes([18; 16]);
        assert_eq!(
            command.reissue_after_session_rotation(43, next_session),
            Some(EditCommand {
                operation_id: 43,
                edit_session_id: next_session,
                action: command.action,
            })
        );
        assert_eq!(
            command.reissue_after_session_rotation(43, edit_session_id),
            None
        );
        assert_eq!(
            command.reissue_after_session_rotation(0, next_session),
            None
        );

        let dig = EditCommand {
            operation_id: 42,
            edit_session_id,
            action: EditAction::Dig { hit: coord },
        };
        assert_eq!(
            decode_edit_command(&encode_edit_command(dig).expect("encode edit dig")),
            Ok(dig)
        );

        let mut counts = [0; MATERIAL_INVENTORY_SLOTS];
        counts[Material::Basalt.id() as usize] = 11;
        let mutations = vec![
            VoxelMutation {
                coord,
                material: Material::Basalt,
            },
            VoxelMutation {
                coord: VoxelCoord::new(32, 64, -33),
                material: Material::Basalt,
            },
        ];
        let affected_chunks = mutations
            .iter()
            .flat_map(|mutation| crate::EditMap::affected_chunks(mutation.coord))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let commit = EditCommit {
            operation_id: command.operation_id,
            edit_session_id,
            editor_connection_id: 77,
            revision: 9,
            mutations,
            affected_chunks,
            affected_surface_tiles: vec![
                SurfaceTileCoord::new(SurfaceLodLevel::Stride2, 0, -1),
                SurfaceTileCoord::new(SurfaceLodLevel::Stride16, 0, -1),
            ],
            editor_inventory: Some(MaterialInventory {
                revision: 8,
                counts,
            }),
        };
        let encoded_commit = encode_edit_commit(&commit).expect("encode edit commit");
        assert_eq!(message_kind(&encoded_commit), Ok(edit_commit_kind()));
        assert_eq!(decode_edit_commit(&encoded_commit), Ok(commit.clone()));

        let resync = ResyncRequired { revision: 12 };
        let encoded_resync = encode_resync_required(resync).expect("encode resync");
        assert_eq!(message_kind(&encoded_resync), Ok(resync_required_kind()));
        assert_eq!(decode_resync_required(&encoded_resync), Ok(resync));

        let mut zero_operation = command;
        zero_operation.operation_id = 0;
        assert_eq!(
            encode_edit_command(zero_operation),
            Err(ProtocolError::InvalidPayload(
                "edit operation id must be nonzero"
            ))
        );

        let mut zero_revision = commit.clone();
        zero_revision.revision = 0;
        assert_eq!(
            encode_edit_commit(&zero_revision),
            Err(ProtocolError::InvalidPayload(
                "edit revision must be nonzero"
            ))
        );

        let mut zero_connection = commit.clone();
        zero_connection.editor_connection_id = 0;
        assert_eq!(
            encode_edit_commit(&zero_connection),
            Err(ProtocolError::InvalidPayload(
                "edit connection id must be nonzero"
            ))
        );

        let mut nil_session = command;
        nil_session.edit_session_id = EditSessionId::from_bytes([0; 16]);
        assert_eq!(
            encode_edit_command(nil_session),
            Err(ProtocolError::InvalidPayload("edit session id is nil"))
        );

        let mut duplicate_chunk = commit.clone();
        duplicate_chunk.affected_chunks = vec![coord.chunk(), coord.chunk()];
        assert_eq!(
            encode_edit_commit(&duplicate_chunk),
            Err(ProtocolError::InvalidPayload(
                "edit affected chunks are not strictly sorted"
            ))
        );

        let mut duplicate_mutation = commit.clone();
        duplicate_mutation.mutations[1].coord = coord;
        assert_eq!(
            encode_edit_commit(&duplicate_mutation),
            Err(ProtocolError::InvalidPayload(
                "edit mutations are not strictly coordinate-sorted"
            ))
        );

        let mut omitted_owner = commit.clone();
        omitted_owner.affected_chunks = vec![ChunkCoord::new(-4, 0, 7)];
        assert_eq!(
            encode_edit_commit(&omitted_owner),
            Err(ProtocolError::InvalidPayload(
                "edit affected chunks omit a mutated chunk"
            ))
        );

        let owners = commit
            .mutations
            .iter()
            .map(|mutation| mutation.coord.chunk())
            .collect::<BTreeSet<_>>();
        let omitted_halo = commit
            .affected_chunks
            .iter()
            .copied()
            .find(|coord| !owners.contains(coord))
            .expect("boundary mutation should require a non-owner halo chunk");
        let mut incomplete_halo = commit.clone();
        incomplete_halo
            .affected_chunks
            .retain(|coord| *coord != omitted_halo);
        assert_eq!(
            encode_edit_commit(&incomplete_halo),
            Err(ProtocolError::InvalidPayload(
                "edit affected chunks omit a mutation halo"
            ))
        );

        let mut malformed_dig = encode_edit_command(dig).expect("encode malformed dig");
        malformed_dig[FRAME_HEADER_BYTES + 18..FRAME_HEADER_BYTES + 20]
            .copy_from_slice(&Material::Stone.id().to_le_bytes());
        assert_eq!(
            decode_edit_command(&malformed_dig),
            Err(ProtocolError::InvalidPayload(
                "dig action has a nonzero argument or material"
            ))
        );

        let mut malformed_argument =
            encode_edit_command(dig).expect("encode malformed dig argument");
        malformed_argument[FRAME_HEADER_BYTES + 17] = 1;
        assert_eq!(
            decode_edit_command(&malformed_argument),
            Err(ProtocolError::InvalidPayload(
                "dig action has a nonzero argument or material"
            ))
        );

        let mut air_inventory = commit;
        air_inventory
            .editor_inventory
            .as_mut()
            .expect("inventory")
            .counts[Material::Air.id() as usize] = 1;
        assert_eq!(
            encode_edit_commit(&air_inventory),
            Err(ProtocolError::InvalidPayload(
                "air inventory count is nonzero"
            ))
        );

        assert_eq!(
            encode_resync_required(ResyncRequired { revision: 0 }),
            Err(ProtocolError::InvalidPayload(
                "resync revision must be nonzero"
            ))
        );
    }

    #[test]
    fn maximum_atomic_commit_round_trips_at_protocol_limits() {
        let mut mutations = Vec::with_capacity(MAX_EDIT_MUTATIONS);
        for x in 30..35 {
            for y in 62..67 {
                for z in -34..-29 {
                    mutations.push(VoxelMutation {
                        coord: VoxelCoord::new(x, y, z),
                        material: Material::Air,
                    });
                }
            }
        }
        assert_eq!(mutations.len(), MAX_EDIT_MUTATIONS);
        let affected_chunks = mutations
            .iter()
            .flat_map(|mutation| crate::EditMap::affected_chunks(mutation.coord))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(affected_chunks.len(), MAX_EDIT_AFFECTED_CHUNKS);
        let commit = EditCommit {
            operation_id: 99,
            edit_session_id: EditSessionId::from_bytes([21; 16]),
            editor_connection_id: 7,
            revision: 12,
            mutations,
            affected_chunks,
            affected_surface_tiles: Vec::new(),
            editor_inventory: None,
        };
        let encoded = encode_edit_commit(&commit).expect("encode maximum commit");
        assert_eq!(decode_edit_commit(&encoded), Ok(commit.clone()));

        let mut oversized = commit;
        oversized.mutations.push(VoxelMutation {
            coord: VoxelCoord::new(35, 62, -34),
            material: Material::Air,
        });
        assert_eq!(
            encode_edit_commit(&oversized),
            Err(ProtocolError::LimitExceeded("edit mutations"))
        );
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
        assert_eq!(
            encode_chunk_batch(&ChunkBatchRequest {
                request_id: 0,
                priority: WorldProductPriority::VisibleChunk,
                coords: vec![ChunkCoord::new(0, 0, 0)],
            }),
            Err(ProtocolError::InvalidPayload("request id must be nonzero"))
        );

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
                edit_revision: 13,
                result: Err(WorldSourceError::SourceCoverageUnavailable),
            }],
        };
        assert_eq!(
            decode_chunk_batch_result(&encode_chunk_batch_result(&response).expect("encode")),
            Ok(response.clone())
        );

        let mut duplicate_result = response;
        duplicate_result
            .items
            .push(duplicate_result.items[0].clone());
        assert_eq!(
            encode_chunk_batch_result(&duplicate_result),
            Err(ProtocolError::InvalidPayload(
                "duplicate chunk result coordinate"
            ))
        );
    }
}
