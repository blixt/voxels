//! Host-testable orchestration for bounded, deterministic voxel streaming.
//!
//! This crate owns no chunk payloads and performs no generation, meshing, or GPU work itself.
//! Instead, it issues versioned work tickets and advances chunks only after the host reports a
//! matching completion. That keeps scheduling deterministic while allowing native threads, web
//! workers, and render backends to execute the expensive stages differently.

use std::collections::{BTreeMap, BTreeSet};

use voxels_world::{
    CHUNK_EDGE, ChunkCoord, EditMap, SurfaceTileCoord, VOXEL_SIZE_METRES, VoxelCoord,
};

mod surface;
pub use surface::{SurfaceFocusAction, SurfaceRevisionCache, SurfaceRevisionStatus};

/// Physical edge length of a full-resolution chunk.
pub const CHUNK_EDGE_METRES: f32 = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;

/// Hard ceiling for the canonical load radius, including its retention margin.
pub const MAX_LOAD_RADIUS_CHUNKS: i32 = 64;
/// Hard ceiling for canonical vertical interest.
pub const MAX_VERTICAL_RADIUS_CHUNKS: i32 = 32;
/// Compile-time safety ceiling for secondary-interest normalization. Runtime configuration may
/// choose a smaller active limit but cannot allocate beyond this bound.
pub const MAX_SECONDARY_INTEREST_CHUNKS: usize = 192;
const LATENCY_HISTOGRAM_BUCKETS: usize = 256;

/// Returns whether `candidate` is the requested revision or a later one in the wrapping sequence.
/// Revisions advance one step at a time and skip zero, so serial-number arithmetic keeps work that
/// straddles `u64::MAX` ordered without mistaking a pre-wrap completion for current geometry.
pub const fn revision_satisfies(candidate: u64, requested: u64) -> bool {
    candidate == requested || candidate.wrapping_sub(requested) < (1u64 << 63)
}

/// Client-side floors for server-authored edit products. Different world connections may finish
/// concurrent commands out of global revision order, so voxel values and product floors advance
/// independently and never regress when an older commit arrives later.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AuthoritativeEditRevisions {
    voxels: BTreeMap<VoxelCoord, u64>,
    chunks: BTreeMap<ChunkCoord, u64>,
    surfaces: BTreeMap<SurfaceTileCoord, u64>,
}

impl AuthoritativeEditRevisions {
    /// Records all invalidation floors and returns whether this commit owns the newest value for
    /// its edited voxel. Callers still invalidate products for `false`: an older commit can name a
    /// derived product that a newer same-voxel commit did not need to rebuild.
    pub fn observe_commit(
        &mut self,
        coord: VoxelCoord,
        revision: u64,
        affected_chunks: &[ChunkCoord],
        affected_surfaces: &[SurfaceTileCoord],
    ) -> bool {
        self.observe_commit_batch(&[coord], revision, affected_chunks, affected_surfaces)[0]
    }

    /// Records one atomic multi-voxel commit without repeatedly advancing identical product
    /// floors. The returned mask is parallel to `coords` and identifies values newer than the
    /// client's current value for that coordinate.
    pub fn observe_commit_batch(
        &mut self,
        coords: &[VoxelCoord],
        revision: u64,
        affected_chunks: &[ChunkCoord],
        affected_surfaces: &[SurfaceTileCoord],
    ) -> Vec<bool> {
        let apply_values = coords
            .iter()
            .map(|&coord| advance_revision(&mut self.voxels, coord, revision))
            .collect();
        for &chunk in affected_chunks {
            advance_revision(&mut self.chunks, chunk, revision);
        }
        for &surface in affected_surfaces {
            advance_revision(&mut self.surfaces, surface, revision);
        }
        apply_values
    }

    pub fn chunk_floor(&self, coord: ChunkCoord) -> u64 {
        self.chunks.get(&coord).copied().unwrap_or(1)
    }

    pub fn surface_floor(&self, coord: SurfaceTileCoord) -> u64 {
        self.surfaces.get(&coord).copied().unwrap_or(1)
    }

    pub fn clear(&mut self) {
        self.voxels.clear();
        self.chunks.clear();
        self.surfaces.clear();
    }
}

fn advance_revision<K: Ord>(revisions: &mut BTreeMap<K, u64>, key: K, candidate: u64) -> bool {
    match revisions.get_mut(&key) {
        Some(current) if candidate != *current && revision_satisfies(candidate, *current) => {
            *current = candidate;
            true
        }
        Some(_) => false,
        None => {
            revisions.insert(key, candidate);
            true
        }
    }
}

type CoordKey = (i32, i32, i32);

/// Limits for one scheduler instance. LOD tiers should use separate schedulers/configurations so
/// the full-resolution residency ceiling stays explicit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamConfig {
    pub load_radius_chunks: i32,
    pub vertical_radius_chunks: i32,
    pub retention_margin_chunks: i32,
    pub max_tracked_chunks: usize,
    pub max_secondary_interest_chunks: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            load_radius_chunks: 9,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 2,
            max_tracked_chunks: 1_024,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    NegativeRadius,
    RadiusTooLarge,
    EmptyCapacity,
    SecondaryInterestTooLarge,
}

/// Maximum amount of new work that may enter each asynchronous stage during one frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameBudget {
    pub generation: usize,
    pub meshing: usize,
    pub upload: usize,
}

/// Deterministic view and motion cue for ordering bounded streaming work.
///
/// Coordinates remain the canonical cache and protocol identity. This hint only changes which
/// already-desired item starts first: the immediate vicinity stays first, then the current view
/// cone, then the velocity-predicted focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectionalStreamPriority {
    forward: [i32; 2],
    predicted_offset: [i32; 2],
    cone_cosine: i32,
}

impl DirectionalStreamPriority {
    const FIXED_SCALE: f32 = 4_096.0;
    const MAX_PREDICTED_OFFSET: f32 = MAX_LOAD_RADIUS_CHUNKS as f32;

    pub const fn neutral() -> Self {
        Self {
            forward: [0, 0],
            predicted_offset: [0, 0],
            cone_cosine: 0,
        }
    }

    /// Builds a grid-independent priority cue. Velocity is expressed in cells per second so the
    /// same definition can rank canonical chunks and every surface-LOD tile size.
    pub fn from_motion(
        forward_xz: [f32; 2],
        velocity_cells_per_second: [f32; 2],
        lookahead_seconds: f32,
        cone_half_angle_degrees: f32,
    ) -> Self {
        if !forward_xz
            .into_iter()
            .chain(velocity_cells_per_second)
            .chain([lookahead_seconds, cone_half_angle_degrees])
            .all(f32::is_finite)
        {
            return Self::neutral();
        }
        let length = forward_xz[0].hypot(forward_xz[1]);
        let forward = if length > f32::EPSILON {
            [
                (forward_xz[0] / length * Self::FIXED_SCALE).round() as i32,
                (forward_xz[1] / length * Self::FIXED_SCALE).round() as i32,
            ]
        } else {
            [0, 0]
        };
        let predicted_offset = velocity_cells_per_second.map(|velocity| {
            (velocity * lookahead_seconds)
                .round()
                .clamp(-Self::MAX_PREDICTED_OFFSET, Self::MAX_PREDICTED_OFFSET) as i32
        });
        let cone_cosine = (cone_half_angle_degrees.clamp(0.0, 90.0).to_radians().cos()
            * Self::FIXED_SCALE)
            .round() as i32;
        Self {
            forward,
            predicted_offset,
            cone_cosine,
        }
    }

    /// `(vicinity band, view class, predicted distance squared)`, ordered best-first.
    pub fn rank_offset(self, dx: i64, dz: i64) -> (u8, u8, i128) {
        let dx = i128::from(dx);
        let dz = i128::from(dz);
        let radius = dx.abs().max(dz.abs());
        let vicinity_band = radius.min(2) as u8;
        let predicted_dx = dx - i128::from(self.predicted_offset[0]);
        let predicted_dz = dz - i128::from(self.predicted_offset[1]);
        let predicted_distance = predicted_dx * predicted_dx + predicted_dz * predicted_dz;
        let distance_squared = dx * dx + dz * dz;
        let dot = dx * i128::from(self.forward[0]) + dz * i128::from(self.forward[1]);
        let view_class = if self.forward == [0, 0] || distance_squared == 0 {
            1
        } else if dot <= 0 {
            2
        } else if dot * dot
            >= distance_squared * i128::from(self.cone_cosine) * i128::from(self.cone_cosine)
        {
            0
        } else {
            1
        };
        (vicinity_band, view_class, predicted_distance)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkStage {
    Generation,
    Meshing,
    Upload,
}

/// A completion capability for one exact chunk revision and stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkTicket {
    pub coord: ChunkCoord,
    pub stage: WorkStage,
    pub revision: u64,
    pub serial: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FrameWork {
    pub generation: Vec<WorkTicket>,
    pub meshing: Vec<WorkTicket>,
    pub upload: Vec<WorkTicket>,
}

/// Collision-safe startup coverage around the current scheduler focus.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VicinityReadiness {
    pub resident: usize,
    pub required: usize,
}

impl VicinityReadiness {
    pub const fn is_ready(self) -> bool {
        self.required > 0 && self.resident == self.required
    }
}

impl FrameWork {
    pub fn len(&self) -> usize {
        self.generation.len() + self.meshing.len() + self.upload.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionStatus {
    Accepted,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChunkState {
    QueuedGeneration,
    Generating,
    QueuedMeshing,
    Meshing,
    QueuedUpload,
    Uploading,
    Resident,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkStatus {
    pub coord: ChunkCoord,
    pub state: ChunkState,
    pub revision: u64,
    pub desired: bool,
}

/// Notification that all host-owned payloads for a coordinate may be released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvictedChunk {
    pub coord: ChunkCoord,
    pub state: ChunkState,
    pub revision: u64,
}

/// Result of invalidating meshes after one canonical voxel changed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DirtyReport {
    pub affected_chunks: Vec<ChunkCoord>,
    pub invalidated_tickets: Vec<WorkTicket>,
    pub previously_resident: Vec<ChunkCoord>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StageCounts {
    pub queued: usize,
    pub in_flight: usize,
}

/// Lifetime latency summary measured in calls to [`StreamScheduler::schedule_frame`].
///
/// Frame counts make the metric deterministic and independent of host clock precision. The p95
/// value is exact for samples below 255 frames. If the p95 falls in the final overflow bucket it
/// conservatively reports the lifetime maximum instead.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameLatency {
    pub completed: u64,
    pub in_flight: usize,
    pub p95_frames: u64,
    pub max_frames: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamDiagnostics {
    pub frame: u64,
    pub tracked: usize,
    pub desired: usize,
    pub secondary_interest_requested: usize,
    pub secondary_interest_normalized: usize,
    pub secondary_interest_desired: usize,
    pub secondary_interest_truncated: usize,
    pub resident: usize,
    pub generation: StageCounts,
    pub meshing: StageCounts,
    pub upload: StageCounts,
    pub accepted_completions: u64,
    pub stale_completions: u64,
    pub total_evictions: u64,
    pub started_this_frame: FrameBudget,
    /// First tracking of a desired chunk through its first accepted resident upload.
    pub initial_residency_latency: FrameLatency,
    /// Invalidation of a previously resident chunk through its replacement resident upload.
    pub remesh_latency: FrameLatency,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LatencyHistogram {
    buckets: [u64; LATENCY_HISTOGRAM_BUCKETS],
    completed: u64,
    max_frames: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            buckets: [0; LATENCY_HISTOGRAM_BUCKETS],
            completed: 0,
            max_frames: 0,
        }
    }
}

impl LatencyHistogram {
    fn record(&mut self, frames: u64) {
        let last_bucket = LATENCY_HISTOGRAM_BUCKETS - 1;
        let bucket = usize::try_from(frames)
            .unwrap_or(last_bucket)
            .min(last_bucket);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.completed = self.completed.saturating_add(1);
        self.max_frames = self.max_frames.max(frames);
    }

    fn summary(&self, in_flight: usize) -> FrameLatency {
        FrameLatency {
            completed: self.completed,
            in_flight,
            p95_frames: self.percentile(95),
            max_frames: self.max_frames,
        }
    }

    fn percentile(&self, percentile: u64) -> u64 {
        if self.completed == 0 {
            return 0;
        }
        let rank = ((u128::from(self.completed) * u128::from(percentile)).div_ceil(100)) as u64;
        let mut cumulative = 0_u64;
        for (index, count) in self.buckets.iter().copied().enumerate() {
            cumulative = cumulative.saturating_add(count);
            if cumulative >= rank {
                return if index == LATENCY_HISTOGRAM_BUCKETS - 1 {
                    self.max_frames
                } else {
                    index as u64
                };
            }
        }
        self.max_frames
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    QueuedGeneration,
    Generating(WorkTicket),
    QueuedMeshing,
    Meshing(WorkTicket),
    QueuedUpload,
    Uploading(WorkTicket),
    Resident,
}

impl State {
    const fn public(self) -> ChunkState {
        match self {
            Self::QueuedGeneration => ChunkState::QueuedGeneration,
            Self::Generating(_) => ChunkState::Generating,
            Self::QueuedMeshing => ChunkState::QueuedMeshing,
            Self::Meshing(_) => ChunkState::Meshing,
            Self::QueuedUpload => ChunkState::QueuedUpload,
            Self::Uploading(_) => ChunkState::Uploading,
            Self::Resident => ChunkState::Resident,
        }
    }

    const fn ticket(self) -> Option<WorkTicket> {
        match self {
            Self::Generating(ticket) | Self::Meshing(ticket) | Self::Uploading(ticket) => {
                Some(ticket)
            }
            Self::QueuedGeneration | Self::QueuedMeshing | Self::QueuedUpload | Self::Resident => {
                None
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    coord: ChunkCoord,
    state: State,
    revision: u64,
    desired: bool,
    initial_residency_started_frame: Option<u64>,
    remesh_started_frame: Option<u64>,
    ever_resident: bool,
}

/// Deterministic metadata scheduler for the chunk generation -> meshing -> upload pipeline.
pub struct StreamScheduler {
    config: StreamConfig,
    focus: ChunkCoord,
    focus_initialized: bool,
    secondary_interest: Vec<ChunkCoord>,
    secondary_interest_requested: usize,
    secondary_interest_capacity_truncated: usize,
    entries: BTreeMap<CoordKey, Entry>,
    evictions: Vec<EvictedChunk>,
    next_ticket_serial: u64,
    frame: u64,
    accepted_completions: u64,
    stale_completions: u64,
    total_evictions: u64,
    started_this_frame: FrameBudget,
    initial_residency_latency: LatencyHistogram,
    remesh_latency: LatencyHistogram,
}

impl StreamScheduler {
    pub fn new(config: StreamConfig) -> Result<Self, ConfigError> {
        if config.load_radius_chunks < 0
            || config.vertical_radius_chunks < 0
            || config.retention_margin_chunks < 0
        {
            return Err(ConfigError::NegativeRadius);
        }
        if config.load_radius_chunks > MAX_LOAD_RADIUS_CHUNKS
            || config.vertical_radius_chunks > MAX_VERTICAL_RADIUS_CHUNKS
            || config
                .load_radius_chunks
                .saturating_add(config.retention_margin_chunks)
                > MAX_LOAD_RADIUS_CHUNKS
        {
            return Err(ConfigError::RadiusTooLarge);
        }
        if config.max_tracked_chunks == 0 {
            return Err(ConfigError::EmptyCapacity);
        }
        if config.max_secondary_interest_chunks > MAX_SECONDARY_INTEREST_CHUNKS {
            return Err(ConfigError::SecondaryInterestTooLarge);
        }

        Ok(Self {
            config,
            focus: ChunkCoord::new(0, 0, 0),
            focus_initialized: false,
            secondary_interest: Vec::new(),
            secondary_interest_requested: 0,
            secondary_interest_capacity_truncated: 0,
            entries: BTreeMap::new(),
            evictions: Vec::new(),
            next_ticket_serial: 1,
            frame: 0,
            accepted_completions: 0,
            stale_completions: 0,
            total_evictions: 0,
            started_this_frame: FrameBudget::default(),
            initial_residency_latency: LatencyHistogram::default(),
            remesh_latency: LatencyHistogram::default(),
        })
    }

    pub const fn config(&self) -> StreamConfig {
        self.config
    }

    pub const fn focus(&self) -> ChunkCoord {
        self.focus
    }

    /// Rebuilds the desired fine-chunk set around `focus`. Nearby chunks win when capacity is
    /// smaller than the configured cylinder. Previously tracked chunks survive inside the wider
    /// retention radius when spare capacity exists, preventing boundary thrash.
    pub fn update_focus(&mut self, focus: ChunkCoord) -> bool {
        self.update_focus_with_interest(focus, &[])
    }

    /// Rebuilds the primary cylinder plus a bounded, deterministic set of secondary chunks.
    /// Secondary interest is useful for semantic look-ahead such as connected cave cells. Primary
    /// coverage always wins, and both sets share the same hard `max_tracked_chunks` ceiling.
    /// Returns `true` when the desired set changed. Renderers use this to reconcile activation even
    /// when every newly desired chunk was already resident inside the retention window.
    pub fn update_focus_with_interest(
        &mut self,
        focus: ChunkCoord,
        interest: &[ChunkCoord],
    ) -> bool {
        let (secondary_interest, capacity_truncated) =
            normalized_interest(focus, interest, self.config.max_secondary_interest_chunks);
        if self.focus_initialized
            && self.focus == focus
            && self.secondary_interest == secondary_interest
            && self.secondary_interest_requested == interest.len()
            && self.secondary_interest_capacity_truncated == capacity_truncated
        {
            return false;
        }
        self.focus = focus;
        self.focus_initialized = true;
        self.secondary_interest = secondary_interest;
        self.secondary_interest_requested = interest.len();
        self.secondary_interest_capacity_truncated = capacity_truncated;
        let desired = self.desired_coordinates(focus);
        let desired_keys: BTreeSet<_> = desired.iter().copied().map(coord_key).collect();

        for (key, entry) in &mut self.entries {
            entry.desired = desired_keys.contains(key);
        }

        let outside_retention: Vec<_> = self
            .entries
            .values()
            .filter(|entry| !entry.desired && !inside_retention(self.config, focus, entry.coord))
            .map(|entry| coord_key(entry.coord))
            .collect();
        for key in outside_retention {
            self.evict_key(key);
        }

        // Retention prevents visible resident chunks from thrashing at a boundary. Incomplete work
        // has no visible value once it is undesired and is deliberately excluded from scheduling;
        // retaining it would leave permanent queued diagnostics and consume capacity forever.
        let incomplete_undesired: Vec<_> = self
            .entries
            .values()
            .filter(|entry| !entry.desired && entry.state != State::Resident)
            .map(|entry| coord_key(entry.coord))
            .collect();
        for key in incomplete_undesired {
            self.evict_key(key);
        }

        for coord in desired {
            let key = coord_key(coord);
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.desired = true;
                continue;
            }
            while self.entries.len() >= self.config.max_tracked_chunks {
                if !self.evict_farthest_undesired() {
                    break;
                }
            }
            if self.entries.len() < self.config.max_tracked_chunks {
                self.entries.insert(
                    key,
                    Entry {
                        coord,
                        state: State::QueuedGeneration,
                        revision: 1,
                        desired: true,
                        initial_residency_started_frame: Some(self.frame),
                        remesh_started_frame: None,
                        ever_resident: false,
                    },
                );
            }
        }
        true
    }

    /// Starts at most the supplied number of jobs per stage. Work is stable-sorted by distance to
    /// the current focus and then by coordinate, so identical input histories issue identical
    /// tickets.
    pub fn schedule_frame(&mut self, budget: FrameBudget) -> FrameWork {
        self.schedule_frame_prioritized(budget, DirectionalStreamPriority::neutral())
    }

    /// Starts bounded work using a deterministic view/velocity hint. The hint is deliberately not
    /// retained: the latest camera state can reorder queued work each frame without invalidating
    /// tickets that are already in flight.
    pub fn schedule_frame_prioritized(
        &mut self,
        budget: FrameBudget,
        priority_hint: DirectionalStreamPriority,
    ) -> FrameWork {
        self.schedule_frame_prioritized_with_urgency(budget, priority_hint, &[])
    }

    /// Starts bounded work while allowing a small collision/render corridor to preempt ordinary
    /// desired chunks at every asynchronous stage. Urgency changes ordering only: it cannot admit
    /// an undesired coordinate, cancel in-flight work, or expand any capacity bound.
    pub fn schedule_frame_prioritized_with_urgency(
        &mut self,
        budget: FrameBudget,
        priority_hint: DirectionalStreamPriority,
        urgent: &[ChunkCoord],
    ) -> FrameWork {
        self.frame = increment_nonzero(self.frame);
        self.started_this_frame = FrameBudget::default();
        let urgent = urgent
            .iter()
            .copied()
            .map(coord_key)
            .collect::<BTreeSet<_>>();

        let generation = self.start_stage(
            WorkStage::Generation,
            State::QueuedGeneration,
            budget.generation,
            priority_hint,
            &urgent,
        );
        let meshing = self.start_stage(
            WorkStage::Meshing,
            State::QueuedMeshing,
            budget.meshing,
            priority_hint,
            &urgent,
        );
        let upload = self.start_stage(
            WorkStage::Upload,
            State::QueuedUpload,
            budget.upload,
            priority_hint,
            &urgent,
        );
        self.started_this_frame = FrameBudget {
            generation: generation.len(),
            meshing: meshing.len(),
            upload: upload.len(),
        };
        FrameWork {
            generation,
            meshing,
            upload,
        }
    }

    /// Advances a chunk only when the ticket exactly matches its current stage and revision.
    pub fn complete(&mut self, ticket: WorkTicket) -> CompletionStatus {
        let key = coord_key(ticket.coord);
        let next = match self.entries.get(&key).map(|entry| entry.state) {
            Some(State::Generating(active)) if active == ticket => Some(State::QueuedMeshing),
            Some(State::Meshing(active)) if active == ticket => Some(State::QueuedUpload),
            Some(State::Uploading(active)) if active == ticket => Some(State::Resident),
            _ => None,
        };
        if let Some(next_state) = next {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.state = next_state;
                if next_state == State::Resident {
                    if let Some(started_frame) = entry.initial_residency_started_frame.take() {
                        self.initial_residency_latency
                            .record(elapsed_frames(started_frame, self.frame));
                    }
                    if let Some(started_frame) = entry.remesh_started_frame.take() {
                        self.remesh_latency
                            .record(elapsed_frames(started_frame, self.frame));
                    }
                    entry.ever_resident = true;
                }
            }
            self.accepted_completions = increment_nonzero(self.accepted_completions);
            CompletionStatus::Accepted
        } else {
            self.stale_completions = increment_nonzero(self.stale_completions);
            CompletionStatus::Stale
        }
    }

    /// Returns a matching in-flight job to its queue without advancing its revision. Hosts use
    /// this for transient resource pressure such as a failed GPU arena allocation.
    pub fn retry(&mut self, ticket: WorkTicket) -> CompletionStatus {
        let key = coord_key(ticket.coord);
        let queued = match self.entries.get(&key).map(|entry| entry.state) {
            Some(State::Generating(active)) if active == ticket => Some(State::QueuedGeneration),
            Some(State::Meshing(active)) if active == ticket => Some(State::QueuedMeshing),
            Some(State::Uploading(active)) if active == ticket => Some(State::QueuedUpload),
            _ => None,
        };
        if let Some(queued) = queued {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.state = queued;
            }
            CompletionStatus::Accepted
        } else {
            self.stale_completions = increment_nonzero(self.stale_completions);
            CompletionStatus::Stale
        }
    }

    /// Invalidates every mesh whose faces can be affected by an edit. The edited chunk and any
    /// boundary neighbors retain generated voxel data when available; generation already in flight
    /// is restarted under a new revision.
    pub fn mark_voxel_edited(&mut self, voxel: VoxelCoord) -> DirtyReport {
        self.mark_voxels_edited(&[voxel])
    }

    /// Invalidates the union of chunks affected by one atomic multi-voxel edit. A chunk revision
    /// advances once even when many adjacent voxels touch it, avoiding 125 redundant scheduler
    /// transitions for a half-metre dig.
    pub fn mark_voxels_edited(&mut self, voxels: &[VoxelCoord]) -> DirtyReport {
        let affected_chunks = voxels
            .iter()
            .flat_map(|&voxel| EditMap::affected_chunks(voxel))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut report = DirtyReport {
            affected_chunks: affected_chunks.clone(),
            ..DirtyReport::default()
        };
        for coord in affected_chunks {
            let key = coord_key(coord);
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if let Some(ticket) = entry.state.ticket() {
                report.invalidated_tickets.push(ticket);
            }
            if entry.state == State::Resident {
                report.previously_resident.push(coord);
            }
            if entry.ever_resident {
                entry.remesh_started_frame = Some(self.frame);
            }
            entry.revision = increment_nonzero(entry.revision);
            entry.state = match entry.state {
                State::QueuedGeneration | State::Generating(_) => State::QueuedGeneration,
                State::QueuedMeshing
                | State::Meshing(_)
                | State::QueuedUpload
                | State::Uploading(_)
                | State::Resident => State::QueuedMeshing,
            };
        }
        report
    }

    /// Invalidates every tracked canonical product after an incremental change-stream gap. Unlike
    /// a normal edit, the missing coordinates are unknown, so every product must return to source
    /// generation under a fresh ticket revision.
    pub fn invalidate_all_generation(&mut self) -> DirtyReport {
        let mut report = DirtyReport::default();
        for entry in self.entries.values_mut() {
            report.affected_chunks.push(entry.coord);
            if let Some(ticket) = entry.state.ticket() {
                report.invalidated_tickets.push(ticket);
            }
            if entry.state == State::Resident {
                report.previously_resident.push(entry.coord);
            }
            if entry.ever_resident {
                entry.remesh_started_frame = Some(self.frame);
            }
            entry.revision = increment_nonzero(entry.revision);
            entry.state = State::QueuedGeneration;
        }
        report
    }

    pub fn status(&self, coord: ChunkCoord) -> Option<ChunkStatus> {
        self.entries
            .get(&coord_key(coord))
            .map(|entry| ChunkStatus {
                coord: entry.coord,
                state: entry.state.public(),
                revision: entry.revision,
                desired: entry.desired,
            })
    }

    /// Reports uploaded canonical chunks inside a horizontal cylinder around the current focus.
    /// The configured vertical span is included so the renderer can activate every counted X/Z
    /// column atomically and collision never begins against missing data.
    pub fn vicinity_readiness(&self, radius_chunks: i32) -> VicinityReadiness {
        if !self.focus_initialized || radius_chunks < 0 {
            return VicinityReadiness::default();
        }
        let radius = radius_chunks.min(self.config.load_radius_chunks);
        let mut readiness = VicinityReadiness::default();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
                    > i64::from(radius) * i64::from(radius)
                {
                    continue;
                }
                for dy in -self.config.vertical_radius_chunks..=self.config.vertical_radius_chunks {
                    let (Some(x), Some(y), Some(z)) = (
                        self.focus.x.checked_add(dx),
                        self.focus.y.checked_add(dy),
                        self.focus.z.checked_add(dz),
                    ) else {
                        continue;
                    };
                    let coord = ChunkCoord::new(x, y, z);
                    if !coord.is_world_representable() {
                        continue;
                    }
                    readiness.required += 1;
                    readiness.resident += usize::from(self.status(coord).is_some_and(|status| {
                        status.desired && status.state == ChunkState::Resident
                    }));
                }
            }
        }
        readiness
    }

    pub fn drain_evictions(&mut self) -> Vec<EvictedChunk> {
        std::mem::take(&mut self.evictions)
    }

    pub fn diagnostics(&self) -> StreamDiagnostics {
        let mut diagnostics = StreamDiagnostics {
            frame: self.frame,
            tracked: self.entries.len(),
            accepted_completions: self.accepted_completions,
            stale_completions: self.stale_completions,
            total_evictions: self.total_evictions,
            started_this_frame: self.started_this_frame,
            initial_residency_latency: self.initial_residency_latency.summary(0),
            remesh_latency: self.remesh_latency.summary(0),
            ..StreamDiagnostics::default()
        };
        diagnostics.secondary_interest_requested = self.secondary_interest_requested;
        diagnostics.secondary_interest_normalized = self.secondary_interest.len();
        diagnostics.secondary_interest_desired = self
            .secondary_interest
            .iter()
            .filter(|coord| {
                self.entries
                    .get(&coord_key(**coord))
                    .is_some_and(|entry| entry.desired)
            })
            .count();
        diagnostics.secondary_interest_truncated =
            self.secondary_interest_capacity_truncated.saturating_add(
                diagnostics
                    .secondary_interest_normalized
                    .saturating_sub(diagnostics.secondary_interest_desired),
            );
        for entry in self.entries.values() {
            diagnostics.desired += usize::from(entry.desired);
            diagnostics.initial_residency_latency.in_flight +=
                usize::from(entry.initial_residency_started_frame.is_some());
            diagnostics.remesh_latency.in_flight +=
                usize::from(entry.remesh_started_frame.is_some());
            match entry.state {
                State::QueuedGeneration => diagnostics.generation.queued += 1,
                State::Generating(_) => diagnostics.generation.in_flight += 1,
                State::QueuedMeshing => diagnostics.meshing.queued += 1,
                State::Meshing(_) => diagnostics.meshing.in_flight += 1,
                State::QueuedUpload => diagnostics.upload.queued += 1,
                State::Uploading(_) => diagnostics.upload.in_flight += 1,
                State::Resident => diagnostics.resident += 1,
            }
        }
        diagnostics
    }

    fn desired_coordinates(&self, focus: ChunkCoord) -> Vec<ChunkCoord> {
        let radius = self.config.load_radius_chunks;
        let vertical = self.config.vertical_radius_chunks;
        let mut candidates = Vec::new();
        for dy in -vertical..=vertical {
            let Some(y) = focus.y.checked_add(dy) else {
                continue;
            };
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)
                        > i64::from(radius) * i64::from(radius)
                    {
                        continue;
                    }
                    let (Some(x), Some(z)) = (focus.x.checked_add(dx), focus.z.checked_add(dz))
                    else {
                        continue;
                    };
                    let coord = ChunkCoord::new(x, y, z);
                    if coord.is_world_representable() {
                        candidates.push(coord);
                    }
                }
            }
        }
        candidates.sort_by_key(|coord| priority(focus, *coord));
        candidates.truncate(self.config.max_tracked_chunks);
        for coord in &self.secondary_interest {
            if candidates.len() == self.config.max_tracked_chunks {
                break;
            }
            if !candidates.contains(coord) {
                candidates.push(*coord);
            }
        }
        candidates
    }

    /// True when every desired chunk in a rendered X/Z column has reached resident state.
    pub fn desired_column_ready(&self, x: i32, z: i32) -> bool {
        let mut found = false;
        for entry in self
            .entries
            .values()
            .filter(|entry| entry.desired && entry.coord.x == x && entry.coord.z == z)
        {
            found = true;
            if entry.state != State::Resident {
                return false;
            }
        }
        found
    }

    /// Whether a column still owns at least one uploaded mesh, including a stale mesh retained
    /// transactionally while a replacement is queued.
    pub fn column_has_renderable_chunk(&self, x: i32, z: i32) -> bool {
        self.entries
            .values()
            .any(|entry| entry.coord.x == x && entry.coord.z == z && entry.ever_resident)
    }

    fn start_stage(
        &mut self,
        stage: WorkStage,
        queued: State,
        budget: usize,
        priority_hint: DirectionalStreamPriority,
        urgent: &BTreeSet<CoordKey>,
    ) -> Vec<WorkTicket> {
        let mut keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.state == queued && (entry.desired || entry.ever_resident))
            .map(|(key, _)| *key)
            .collect();
        keys.sort_by_key(|key| {
            (
                !urgent.contains(key),
                directional_priority(self.focus, coord_from_key(*key), priority_hint),
            )
        });
        keys.truncate(budget);

        let mut tickets = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(entry) = self.entries.get(&key) else {
                continue;
            };
            let ticket = WorkTicket {
                coord: entry.coord,
                stage,
                revision: entry.revision,
                serial: self.take_ticket_serial(),
            };
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.state = match stage {
                    WorkStage::Generation => State::Generating(ticket),
                    WorkStage::Meshing => State::Meshing(ticket),
                    WorkStage::Upload => State::Uploading(ticket),
                };
                tickets.push(ticket);
            }
        }
        tickets
    }

    fn take_ticket_serial(&mut self) -> u64 {
        let serial = self.next_ticket_serial;
        self.next_ticket_serial = increment_nonzero(self.next_ticket_serial);
        serial
    }

    fn evict_farthest_undesired(&mut self) -> bool {
        let key = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.desired)
            .max_by_key(|(_, entry)| (priority(self.focus, entry.coord), coord_key(entry.coord)))
            .map(|(key, _)| *key);
        if let Some(key) = key {
            self.evict_key(key);
            true
        } else {
            false
        }
    }

    fn evict_key(&mut self, key: CoordKey) {
        if let Some(entry) = self.entries.remove(&key) {
            self.evictions.push(EvictedChunk {
                coord: entry.coord,
                state: entry.state.public(),
                revision: entry.revision,
            });
            self.total_evictions = increment_nonzero(self.total_evictions);
        }
    }
}

fn normalized_interest(
    focus: ChunkCoord,
    interest: &[ChunkCoord],
    limit: usize,
) -> (Vec<ChunkCoord>, usize) {
    let mut normalized = Vec::with_capacity(interest.len().min(limit));
    let mut truncated = 0usize;
    for coord in interest.iter().copied() {
        if !coord.is_world_representable() {
            continue;
        }
        if normalized.contains(&coord) {
            continue;
        }
        if normalized.len() < limit {
            normalized.push(coord);
            continue;
        }
        let farthest = normalized
            .iter()
            .enumerate()
            .max_by_key(|(_, candidate)| priority(focus, **candidate))
            .map(|(index, _)| index);
        if let Some(farthest) = farthest
            && priority(focus, coord) < priority(focus, normalized[farthest])
        {
            normalized[farthest] = coord;
        }
        truncated = truncated.saturating_add(1);
    }
    normalized.sort_unstable_by_key(|coord| priority(focus, *coord));
    (normalized, truncated)
}

const fn coord_key(coord: ChunkCoord) -> CoordKey {
    (coord.x, coord.y, coord.z)
}

const fn coord_from_key(key: CoordKey) -> ChunkCoord {
    ChunkCoord::new(key.0, key.1, key.2)
}

fn priority(focus: ChunkCoord, coord: ChunkCoord) -> (i128, i128, i128, i32, i32, i32) {
    let dx = i128::from(coord.x) - i128::from(focus.x);
    let dy = i128::from(coord.y) - i128::from(focus.y);
    let dz = i128::from(coord.z) - i128::from(focus.z);
    (
        // Complete nearby vertical columns before spreading across a horizontal slice. The
        // renderer exposes columns atomically, so this produces useful collision/render coverage
        // sooner without changing the final desired set.
        dx * dx + dz * dz,
        dy.abs(),
        dx.abs() + dz.abs(),
        coord.y,
        coord.z,
        coord.x,
    )
}

fn directional_priority(
    focus: ChunkCoord,
    coord: ChunkCoord,
    hint: DirectionalStreamPriority,
) -> (u8, u8, i128, i128, i128, i128, i32, i32, i32) {
    let dx_i64 = i64::from(coord.x) - i64::from(focus.x);
    let dz_i64 = i64::from(coord.z) - i64::from(focus.z);
    let dx = i128::from(coord.x) - i128::from(focus.x);
    let dy = i128::from(coord.y) - i128::from(focus.y);
    let dz = i128::from(coord.z) - i128::from(focus.z);
    let (vicinity_band, view_class, predicted_distance) = hint.rank_offset(dx_i64, dz_i64);
    (
        vicinity_band,
        view_class,
        predicted_distance,
        dx * dx + dz * dz,
        dy.abs(),
        dx.abs() + dz.abs(),
        coord.y,
        coord.z,
        coord.x,
    )
}

fn inside_retention(config: StreamConfig, focus: ChunkCoord, coord: ChunkCoord) -> bool {
    let radius = i128::from(config.load_radius_chunks + config.retention_margin_chunks);
    let vertical = i128::from(config.vertical_radius_chunks + config.retention_margin_chunks);
    let dx = i128::from(coord.x) - i128::from(focus.x);
    let dy = i128::from(coord.y) - i128::from(focus.y);
    let dz = i128::from(coord.z) - i128::from(focus.z);
    dx * dx + dz * dz <= radius * radius && dy.abs() <= vertical
}

const fn increment_nonzero(value: u64) -> u64 {
    let incremented = value.wrapping_add(1);
    if incremented == 0 { 1 } else { incremented }
}

const fn elapsed_frames(start: u64, end: u64) -> u64 {
    if end >= start {
        end - start
    } else {
        // The scheduler frame counter skips zero when it wraps.
        u64::MAX - start + end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler(config: StreamConfig) -> StreamScheduler {
        match StreamScheduler::new(config) {
            Ok(scheduler) => scheduler,
            Err(error) => unreachable!("test configuration must be valid: {error:?}"),
        }
    }

    fn compact_config(capacity: usize) -> StreamConfig {
        StreamConfig {
            load_radius_chunks: 1,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: capacity,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        }
    }

    fn advance_to_resident(scheduler: &mut StreamScheduler, coord: ChunkCoord) {
        for stage in [WorkStage::Generation, WorkStage::Meshing, WorkStage::Upload] {
            let work = scheduler.schedule_frame(FrameBudget {
                generation: usize::from(stage == WorkStage::Generation),
                meshing: usize::from(stage == WorkStage::Meshing),
                upload: usize::from(stage == WorkStage::Upload),
            });
            let ticket = match stage {
                WorkStage::Generation => work.generation.first(),
                WorkStage::Meshing => work.meshing.first(),
                WorkStage::Upload => work.upload.first(),
            };
            assert_eq!(ticket.map(|ticket| ticket.coord), Some(coord));
            if let Some(ticket) = ticket {
                assert_eq!(scheduler.complete(*ticket), CompletionStatus::Accepted);
            }
        }
    }

    #[test]
    fn startup_vicinity_requires_complete_gpu_resident_columns() {
        let focus = ChunkCoord::new(4, 7, -3);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 2,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 1,
            max_tracked_chunks: 39,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        assert_eq!(
            scheduler.vicinity_readiness(1),
            VicinityReadiness::default()
        );

        scheduler.update_focus(focus);
        assert_eq!(
            scheduler.vicinity_readiness(1),
            VicinityReadiness {
                resident: 0,
                required: 15,
            }
        );

        let generation = scheduler.schedule_frame(FrameBudget {
            generation: 3,
            ..FrameBudget::default()
        });
        assert_eq!(generation.generation.len(), 3);
        assert!(
            generation
                .generation
                .iter()
                .all(|ticket| ticket.coord.x == focus.x && ticket.coord.z == focus.z),
            "the scheduler should finish the central column before spreading horizontally"
        );
        for ticket in generation.generation {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        let meshing = scheduler.schedule_frame(FrameBudget {
            meshing: 3,
            ..FrameBudget::default()
        });
        for ticket in meshing.meshing {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        let upload = scheduler.schedule_frame(FrameBudget {
            upload: 3,
            ..FrameBudget::default()
        });
        for ticket in upload.upload {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        assert_eq!(
            scheduler.vicinity_readiness(1),
            VicinityReadiness {
                resident: 3,
                required: 15,
            }
        );
        assert!(!scheduler.vicinity_readiness(1).is_ready());

        let generation = scheduler.schedule_frame(FrameBudget {
            generation: usize::MAX,
            ..FrameBudget::default()
        });
        for ticket in generation.generation {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        let meshing = scheduler.schedule_frame(FrameBudget {
            meshing: usize::MAX,
            ..FrameBudget::default()
        });
        for ticket in meshing.meshing {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        let upload = scheduler.schedule_frame(FrameBudget {
            upload: usize::MAX,
            ..FrameBudget::default()
        });
        for ticket in upload.upload {
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        assert_eq!(
            scheduler.vicinity_readiness(1),
            VicinityReadiness {
                resident: 15,
                required: 15,
            }
        );
        assert!(scheduler.vicinity_readiness(1).is_ready());
    }

    #[test]
    fn rejects_unbounded_or_invalid_configurations() {
        assert_eq!(
            StreamScheduler::new(StreamConfig {
                load_radius_chunks: -1,
                ..StreamConfig::default()
            })
            .err(),
            Some(ConfigError::NegativeRadius)
        );
        assert_eq!(
            StreamScheduler::new(StreamConfig {
                max_tracked_chunks: 0,
                ..StreamConfig::default()
            })
            .err(),
            Some(ConfigError::EmptyCapacity)
        );
        assert_eq!(
            StreamScheduler::new(StreamConfig {
                max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS + 1,
                ..StreamConfig::default()
            })
            .err(),
            Some(ConfigError::SecondaryInterestTooLarge)
        );
    }

    #[test]
    fn canonical_scale_is_ten_centimetres_and_chunks_are_32_voxels() {
        assert_eq!(VOXEL_SIZE_METRES, 0.1);
        assert_eq!(CHUNK_EDGE, 32);
        assert!((CHUNK_EDGE_METRES - 3.2).abs() < f32::EPSILON);
    }

    #[test]
    fn world_edge_focus_never_schedules_unrepresentable_chunks() {
        let focus = VoxelCoord::new(i32::MAX, i32::MAX, i32::MAX).chunk();
        let invalid = ChunkCoord::new(focus.x + 1, focus.y, focus.z);
        let mut scheduler = scheduler(compact_config(32));
        scheduler.update_focus_with_interest(focus, &[invalid]);

        assert!(
            scheduler
                .entries
                .values()
                .all(|entry| entry.coord.is_world_representable())
        );
        assert_eq!(scheduler.diagnostics().secondary_interest_normalized, 0);
    }

    #[test]
    fn budgets_and_distance_order_are_deterministic() {
        let mut first = scheduler(compact_config(5));
        let mut second = scheduler(compact_config(5));
        first.update_focus(ChunkCoord::new(0, 0, 0));
        second.update_focus(ChunkCoord::new(0, 0, 0));

        let budget = FrameBudget {
            generation: 2,
            meshing: 9,
            upload: 9,
        };
        let first_work = first.schedule_frame(budget);
        let second_work = second.schedule_frame(budget);
        assert_eq!(first_work, second_work);
        assert_eq!(first_work.generation.len(), 2);
        assert_eq!(first_work.generation[0].coord, ChunkCoord::new(0, 0, 0));
        assert!(first_work.meshing.is_empty());
        assert!(first_work.upload.is_empty());
        assert_eq!(first.diagnostics().generation.queued, 3);
        assert_eq!(first.diagnostics().generation.in_flight, 2);
    }

    #[test]
    fn directional_priority_keeps_the_vicinity_then_leads_view_and_velocity() {
        let config = StreamConfig {
            load_radius_chunks: 3,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: 64,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        };
        let focus = ChunkCoord::new(0, 0, 0);
        let mut east = scheduler(config);
        let mut west = scheduler(config);
        east.update_focus(focus);
        west.update_focus(focus);
        let budget = FrameBudget {
            generation: 10,
            ..FrameBudget::default()
        };
        let east = east.schedule_frame_prioritized(
            budget,
            DirectionalStreamPriority::from_motion([1.0, 0.0], [2.0, 0.0], 1.0, 45.0),
        );
        let west = west.schedule_frame_prioritized(
            budget,
            DirectionalStreamPriority::from_motion([-1.0, 0.0], [-2.0, 0.0], 1.0, 45.0),
        );
        for work in [&east, &west] {
            assert_eq!(work.generation[0].coord, focus);
            assert!(
                work.generation[..9].iter().all(|ticket| {
                    let dx = i64::from(ticket.coord.x) - i64::from(focus.x);
                    let dz = i64::from(ticket.coord.z) - i64::from(focus.z);
                    dx.abs().max(dz.abs()) <= 1
                }),
                "the complete immediate vicinity must remain ahead of prediction"
            );
        }
        assert_eq!(east.generation[9].coord, ChunkCoord::new(2, 0, 0));
        assert_eq!(west.generation[9].coord, ChunkCoord::new(-2, 0, 0));
    }

    #[test]
    fn directional_priority_has_distinct_cone_and_prediction_terms() {
        let priority = DirectionalStreamPriority::from_motion([1.0, 0.0], [2.0, 0.0], 1.0, 45.0);
        assert_eq!(priority.rank_offset(2, 0), (2, 0, 0));
        assert_eq!(priority.rank_offset(-2, 0).1, 2);
        assert_eq!(priority.rank_offset(1, 2).1, 1);
        assert!(
            priority.rank_offset(3, 0).2 < priority.rank_offset(5, 0).2,
            "velocity look-ahead must lead work toward the predicted focus"
        );
    }

    #[test]
    fn urgent_corridor_preempts_ordinary_work_through_every_pipeline_stage() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 3,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: 64,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let focus = ChunkCoord::new(0, 0, 0);
        let urgent = ChunkCoord::new(3, 0, 0);
        scheduler.update_focus(focus);

        for stage in [WorkStage::Generation, WorkStage::Meshing, WorkStage::Upload] {
            let work = scheduler.schedule_frame_prioritized_with_urgency(
                FrameBudget {
                    generation: usize::from(stage == WorkStage::Generation),
                    meshing: usize::from(stage == WorkStage::Meshing),
                    upload: usize::from(stage == WorkStage::Upload),
                },
                DirectionalStreamPriority::neutral(),
                &[urgent],
            );
            let ticket = match stage {
                WorkStage::Generation => work.generation.first(),
                WorkStage::Meshing => work.meshing.first(),
                WorkStage::Upload => work.upload.first(),
            }
            .copied()
            .expect("urgent stage ticket");
            assert_eq!(ticket.coord, urgent);
            assert_eq!(scheduler.complete(ticket), CompletionStatus::Accepted);
        }
        assert_eq!(
            scheduler.status(urgent).map(|status| status.state),
            Some(ChunkState::Resident)
        );
        assert_eq!(
            scheduler.status(focus).map(|status| status.state),
            Some(ChunkState::QueuedGeneration),
            "ordinary nearer work must wait behind the urgent path"
        );
    }

    #[test]
    fn repeated_focus_updates_are_noops_after_initial_population() {
        let focus = ChunkCoord::new(0, 0, 0);
        let mut scheduler = scheduler(compact_config(5));

        scheduler.update_focus(focus);
        let populated = scheduler.diagnostics();
        assert_eq!(populated.tracked, 5);

        scheduler.update_focus(focus);
        assert_eq!(scheduler.diagnostics(), populated);
        assert!(scheduler.drain_evictions().is_empty());
    }

    #[test]
    fn secondary_interest_uses_spare_capacity_and_stays_hard_bounded() {
        let focus = ChunkCoord::new(0, 0, 0);
        let nearest = ChunkCoord::new(8, 0, 0);
        let farther = ChunkCoord::new(12, 0, 0);
        let overflow = ChunkCoord::new(16, 0, 0);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 3,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus_with_interest(focus, &[overflow, farther, nearest, nearest]);

        let diagnostics = scheduler.diagnostics();
        assert_eq!(diagnostics.desired, 3);
        assert_eq!(diagnostics.secondary_interest_requested, 4);
        assert_eq!(diagnostics.secondary_interest_normalized, 3);
        assert_eq!(diagnostics.secondary_interest_desired, 2);
        assert_eq!(diagnostics.secondary_interest_truncated, 1);
        assert!(
            scheduler
                .status(nearest)
                .is_some_and(|status| status.desired)
        );
        assert!(
            scheduler
                .status(farther)
                .is_some_and(|status| status.desired)
        );
        assert!(scheduler.status(overflow).is_none());
    }

    #[test]
    fn secondary_interest_order_and_duplicates_do_not_change_ticket_order() {
        let focus = ChunkCoord::new(0, 0, 0);
        let a = ChunkCoord::new(8, 0, 0);
        let b = ChunkCoord::new(-8, 0, 0);
        let c = ChunkCoord::new(0, -2, 8);
        let config = StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 4,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        };
        let mut first = scheduler(config);
        let mut second = scheduler(config);
        first.update_focus_with_interest(focus, &[c, a, b, a]);
        second.update_focus_with_interest(focus, &[b, c, a]);
        let budget = FrameBudget {
            generation: 4,
            ..FrameBudget::default()
        };
        assert_eq!(first.schedule_frame(budget), second.schedule_frame(budget));
    }

    #[test]
    fn secondary_interest_capacity_truncation_is_observable() {
        let focus = ChunkCoord::new(0, 0, 0);
        let configured_limit = 4;
        let interest: Vec<_> = (1..=configured_limit + 8)
            .map(|x| ChunkCoord::new(x as i32, 0, 0))
            .collect();
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: configured_limit + 1,
            max_secondary_interest_chunks: configured_limit,
        });
        scheduler.update_focus_with_interest(focus, &interest);
        let diagnostics = scheduler.diagnostics();
        assert_eq!(diagnostics.secondary_interest_requested, interest.len());
        assert_eq!(diagnostics.secondary_interest_normalized, configured_limit);
        assert_eq!(diagnostics.secondary_interest_desired, configured_limit);
        assert_eq!(diagnostics.secondary_interest_truncated, 8);
    }

    #[test]
    fn unchanged_interest_still_refreshes_capacity_truncation() {
        let focus = ChunkCoord::new(0, 0, 0);
        let interest: Vec<_> = (1..=MAX_SECONDARY_INTEREST_CHUNKS)
            .map(|x| ChunkCoord::new(x as i32, 0, 0))
            .collect();
        let mut with_duplicate = interest.clone();
        with_duplicate.push(interest[0]);
        let mut with_rejected = interest;
        with_rejected.push(ChunkCoord::new(10_000, 0, 0));
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: MAX_SECONDARY_INTEREST_CHUNKS + 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });

        scheduler.update_focus_with_interest(focus, &with_duplicate);
        assert_eq!(scheduler.diagnostics().secondary_interest_truncated, 0);
        scheduler.update_focus_with_interest(focus, &with_rejected);
        assert_eq!(scheduler.diagnostics().secondary_interest_truncated, 1);
    }

    #[test]
    fn changed_secondary_interest_evicts_old_lookahead_outside_retention() {
        let focus = ChunkCoord::new(0, 0, 0);
        let old = ChunkCoord::new(8, 0, 0);
        let new = ChunkCoord::new(-8, 0, 0);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 2,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus_with_interest(focus, &[old]);
        scheduler.update_focus_with_interest(focus, &[new]);

        assert!(scheduler.status(old).is_none());
        assert!(scheduler.status(new).is_some_and(|status| status.desired));
        assert!(
            scheduler
                .drain_evictions()
                .iter()
                .any(|eviction| eviction.coord == old)
        );
    }

    #[test]
    fn focus_update_reports_desired_set_changes_even_when_chunks_are_retained() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 2,
            max_tracked_chunks: 3,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let origin = ChunkCoord::new(0, 0, 0);
        let neighbor = ChunkCoord::new(1, 0, 0);

        assert!(scheduler.update_focus(origin));
        advance_to_resident(&mut scheduler, origin);
        assert!(!scheduler.update_focus(origin));

        assert!(scheduler.update_focus(neighbor));
        advance_to_resident(&mut scheduler, neighbor);
        assert!(scheduler.update_focus(origin));
        assert!(scheduler.drain_evictions().is_empty());
        assert!(
            scheduler
                .status(origin)
                .is_some_and(|status| { status.desired && status.state == ChunkState::Resident })
        );
    }

    #[test]
    fn desired_column_readiness_includes_secondary_vertical_chunks() {
        let focus = ChunkCoord::new(0, 0, 0);
        let lookahead = ChunkCoord::new(8, -2, 0);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 2,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus_with_interest(focus, &[lookahead]);
        assert!(!scheduler.desired_column_ready(8, 0));

        for _ in 0..3 {
            let work = scheduler.schedule_frame(FrameBudget {
                generation: 2,
                meshing: 2,
                upload: 2,
            });
            for ticket in work
                .generation
                .iter()
                .chain(&work.meshing)
                .chain(&work.upload)
            {
                let _ = scheduler.complete(*ticket);
            }
        }
        assert!(scheduler.desired_column_ready(8, 0));
        assert!(!scheduler.desired_column_ready(7, 0));
    }

    #[test]
    fn edited_retained_resident_chunk_cannot_strand_remesh_work() {
        let focus = ChunkCoord::new(0, 0, 0);
        let retained = ChunkCoord::new(1, 0, 0);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 2,
            max_tracked_chunks: 2,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus_with_interest(focus, &[retained]);
        advance_to_resident(&mut scheduler, focus);
        advance_to_resident(&mut scheduler, retained);
        scheduler.update_focus(focus);
        assert!(
            scheduler
                .status(retained)
                .is_some_and(|status| { !status.desired && status.state == ChunkState::Resident })
        );

        scheduler.mark_voxel_edited(VoxelCoord::new(CHUNK_EDGE as i32 + 8, 4, 4));
        let meshing = scheduler.schedule_frame(FrameBudget {
            meshing: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            meshing.meshing.first().map(|ticket| ticket.coord),
            Some(retained)
        );
        assert_eq!(
            scheduler.complete(meshing.meshing[0]),
            CompletionStatus::Accepted
        );
        let upload = scheduler.schedule_frame(FrameBudget {
            upload: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            upload.upload.first().map(|ticket| ticket.coord),
            Some(retained)
        );
        assert_eq!(
            scheduler.complete(upload.upload[0]),
            CompletionStatus::Accepted
        );
        assert!(
            scheduler
                .status(retained)
                .is_some_and(|status| { !status.desired && status.state == ChunkState::Resident })
        );
        assert_eq!(scheduler.diagnostics().meshing.queued, 0);
        assert_eq!(scheduler.diagnostics().upload.queued, 0);
    }

    #[test]
    fn work_advances_through_all_pipeline_stages() {
        let focus = ChunkCoord::new(4, -2, 7);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus(focus);
        advance_to_resident(&mut scheduler, focus);
        assert_eq!(
            scheduler.status(focus).map(|status| status.state),
            Some(ChunkState::Resident)
        );
        let diagnostics = scheduler.diagnostics();
        assert_eq!(diagnostics.resident, 1);
        assert_eq!(diagnostics.accepted_completions, 3);
    }

    #[test]
    fn initial_residency_latency_counts_scheduler_frames() {
        let focus = ChunkCoord::new(4, -2, 7);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus(focus);

        assert_eq!(
            scheduler.diagnostics().initial_residency_latency,
            FrameLatency {
                completed: 0,
                in_flight: 1,
                p95_frames: 0,
                max_frames: 0,
            }
        );

        advance_to_resident(&mut scheduler, focus);
        assert_eq!(
            scheduler.diagnostics().initial_residency_latency,
            FrameLatency {
                completed: 1,
                in_flight: 0,
                p95_frames: 3,
                max_frames: 3,
            }
        );
    }

    #[test]
    fn remesh_latency_starts_at_edit_and_ends_at_replacement_upload() {
        let coord = ChunkCoord::new(0, 0, 0);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus(coord);
        advance_to_resident(&mut scheduler, coord);

        let report = scheduler.mark_voxel_edited(VoxelCoord::new(2, 2, 2));
        assert_eq!(report.previously_resident, vec![coord]);
        assert_eq!(scheduler.diagnostics().remesh_latency.in_flight, 1);

        let mesh = scheduler.schedule_frame(FrameBudget {
            meshing: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            scheduler.complete(mesh.meshing[0]),
            CompletionStatus::Accepted
        );
        assert_eq!(scheduler.diagnostics().remesh_latency.completed, 0);

        let upload = scheduler.schedule_frame(FrameBudget {
            upload: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            scheduler.complete(upload.upload[0]),
            CompletionStatus::Accepted
        );
        assert_eq!(
            scheduler.diagnostics().remesh_latency,
            FrameLatency {
                completed: 1,
                in_flight: 0,
                p95_frames: 2,
                max_frames: 2,
            }
        );
        assert_eq!(
            scheduler.diagnostics().initial_residency_latency.completed,
            1
        );
    }

    #[test]
    fn superseded_remesh_only_records_the_replacement_revision() {
        let coord = ChunkCoord::new(0, 0, 0);
        let voxel = VoxelCoord::new(2, 2, 2);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus(coord);
        advance_to_resident(&mut scheduler, coord);
        scheduler.mark_voxel_edited(voxel);

        let first_mesh = scheduler.schedule_frame(FrameBudget {
            meshing: 1,
            ..FrameBudget::default()
        });
        let stale_ticket = first_mesh.meshing[0];
        scheduler.mark_voxel_edited(voxel);
        assert_eq!(scheduler.complete(stale_ticket), CompletionStatus::Stale);
        assert_eq!(scheduler.diagnostics().remesh_latency.completed, 0);

        let replacement_mesh = scheduler.schedule_frame(FrameBudget {
            meshing: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            scheduler.complete(replacement_mesh.meshing[0]),
            CompletionStatus::Accepted
        );
        let replacement_upload = scheduler.schedule_frame(FrameBudget {
            upload: 1,
            ..FrameBudget::default()
        });
        assert_eq!(
            scheduler.complete(replacement_upload.upload[0]),
            CompletionStatus::Accepted
        );
        assert_eq!(scheduler.diagnostics().remesh_latency.completed, 1);
        assert_eq!(scheduler.diagnostics().remesh_latency.p95_frames, 2);
    }

    #[test]
    fn latency_histogram_reports_exact_p95_and_conservative_overflow() {
        let mut histogram = LatencyHistogram::default();
        for frames in 1..=100 {
            histogram.record(frames);
        }
        assert_eq!(
            histogram.summary(7),
            FrameLatency {
                completed: 100,
                in_flight: 7,
                p95_frames: 95,
                max_frames: 100,
            }
        );

        let mut overflow = LatencyHistogram::default();
        for frames in 300..=399 {
            overflow.record(frames);
        }
        assert_eq!(overflow.summary(0).p95_frames, 399);
        assert_eq!(overflow.summary(0).max_frames, 399);
    }

    #[test]
    fn elapsed_frame_count_handles_the_nonzero_counter_wrap() {
        assert_eq!(elapsed_frames(12, 19), 7);
        assert_eq!(elapsed_frames(u64::MAX, 1), 1);
        assert_eq!(elapsed_frames(u64::MAX - 2, 2), 4);
    }

    #[test]
    fn revision_order_handles_the_nonzero_counter_wrap() {
        assert!(revision_satisfies(7, 7));
        assert!(revision_satisfies(8, 7));
        assert!(!revision_satisfies(7, 8));
        assert!(revision_satisfies(1, u64::MAX));
        assert!(!revision_satisfies(u64::MAX, 1));
    }

    #[test]
    fn authoritative_edit_floors_do_not_regress_on_out_of_order_commits() {
        let voxel = VoxelCoord::new(7, 8, 9);
        let chunk = voxel.chunk();
        let surface =
            SurfaceTileCoord::containing(voxels_world::SurfaceLodLevel::Stride16, voxel.x, voxel.z);
        let mut revisions = AuthoritativeEditRevisions::default();

        assert!(revisions.observe_commit(voxel, 12, &[chunk], &[surface]));
        assert_eq!(revisions.chunk_floor(chunk), 12);
        assert_eq!(revisions.surface_floor(surface), 12);

        assert!(!revisions.observe_commit(voxel, 11, &[chunk], &[surface]));
        assert_eq!(revisions.chunk_floor(chunk), 12);
        assert_eq!(revisions.surface_floor(surface), 12);
        assert!(!revisions.observe_commit(voxel, 12, &[chunk], &[surface]));

        assert!(revisions.observe_commit(voxel, 13, &[chunk], &[surface]));
        assert_eq!(revisions.chunk_floor(chunk), 13);
        assert_eq!(revisions.surface_floor(surface), 13);
        revisions.clear();
        assert_eq!(revisions.chunk_floor(chunk), 1);
        assert_eq!(revisions.surface_floor(surface), 1);
    }

    #[test]
    fn atomic_edit_batch_advances_each_value_and_shared_product_floor_once() {
        let voxels = [VoxelCoord::new(31, 8, 7), VoxelCoord::new(32, 8, 7)];
        let chunks = [ChunkCoord::new(0, 0, 0), ChunkCoord::new(1, 0, 0)];
        let surface = SurfaceTileCoord::containing(
            voxels_world::SurfaceLodLevel::Stride16,
            voxels[0].x,
            voxels[0].z,
        );
        let mut revisions = AuthoritativeEditRevisions::default();

        assert_eq!(
            revisions.observe_commit_batch(&voxels, 9, &chunks, &[surface]),
            vec![true, true]
        );
        assert_eq!(
            revisions.observe_commit_batch(&voxels, 9, &chunks, &[surface]),
            vec![false, false]
        );
        assert_eq!(revisions.chunk_floor(chunks[0]), 9);
        assert_eq!(revisions.chunk_floor(chunks[1]), 9);
        assert_eq!(revisions.surface_floor(surface), 9);
    }

    #[test]
    fn atomic_adjacent_edits_deduplicate_scheduler_chunk_invalidations() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 1,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 9,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        scheduler.update_focus(ChunkCoord::new(0, 0, 0));
        let report = scheduler.mark_voxels_edited(&[
            VoxelCoord::new(30, 4, 4),
            VoxelCoord::new(31, 4, 4),
            VoxelCoord::new(32, 4, 4),
        ]);
        assert_eq!(
            report.affected_chunks,
            vec![ChunkCoord::new(0, 0, 0), ChunkCoord::new(1, 0, 0)]
        );
    }

    #[test]
    fn edit_invalidates_an_in_flight_ticket_and_bumps_revision() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let coord = ChunkCoord::new(0, 0, 0);
        scheduler.update_focus(coord);
        let old = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        let old_ticket = old.generation[0];
        let dirty = scheduler.mark_voxel_edited(VoxelCoord::new(2, 2, 2));
        assert_eq!(dirty.invalidated_tickets, vec![old_ticket]);
        assert_eq!(scheduler.complete(old_ticket), CompletionStatus::Stale);

        let replacement = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        assert_eq!(replacement.generation.len(), 1);
        assert!(replacement.generation[0].revision > old_ticket.revision);
        assert_ne!(replacement.generation[0].serial, old_ticket.serial);
        assert_eq!(scheduler.diagnostics().stale_completions, 1);
    }

    #[test]
    fn boundary_edits_dirty_neighbor_meshes() {
        let mut scheduler = scheduler(compact_config(5));
        let center = ChunkCoord::new(0, 0, 0);
        let neighbor = ChunkCoord::new(1, 0, 0);
        scheduler.update_focus(center);
        advance_to_resident(&mut scheduler, center);

        // Give the neighbor generated data without depending on its tie-break position.
        loop {
            let work = scheduler.schedule_frame(FrameBudget {
                generation: 5,
                meshing: 5,
                upload: 5,
            });
            for ticket in work
                .generation
                .iter()
                .chain(&work.meshing)
                .chain(&work.upload)
            {
                let _ = scheduler.complete(*ticket);
            }
            if scheduler.status(neighbor).map(|status| status.state) == Some(ChunkState::Resident) {
                break;
            }
        }

        let report = scheduler.mark_voxel_edited(VoxelCoord::new(31, 4, 4));
        assert_eq!(report.affected_chunks, vec![center, neighbor]);
        assert!(report.previously_resident.contains(&center));
        assert!(report.previously_resident.contains(&neighbor));
        assert_eq!(
            scheduler.status(center).map(|status| status.state),
            Some(ChunkState::QueuedMeshing)
        );
        assert_eq!(
            scheduler.status(neighbor).map(|status| status.state),
            Some(ChunkState::QueuedMeshing)
        );
    }

    #[test]
    fn retention_hysteresis_avoids_immediate_eviction() {
        let mut scheduler = scheduler(compact_config(9));
        scheduler.update_focus(ChunkCoord::new(0, 0, 0));
        assert_eq!(scheduler.diagnostics().tracked, 5);
        while scheduler.diagnostics().resident < 5 {
            let work = scheduler.schedule_frame(FrameBudget {
                generation: 5,
                meshing: 5,
                upload: 5,
            });
            for ticket in work
                .generation
                .iter()
                .chain(&work.meshing)
                .chain(&work.upload)
            {
                assert_eq!(scheduler.complete(*ticket), CompletionStatus::Accepted);
            }
        }

        scheduler.update_focus(ChunkCoord::new(1, 0, 0));
        let diagnostics = scheduler.diagnostics();
        assert_eq!(diagnostics.desired, 5);
        assert_eq!(diagnostics.tracked, 8);
        assert!(scheduler.status(ChunkCoord::new(-1, 0, 0)).is_some());
        assert!(scheduler.drain_evictions().is_empty());

        scheduler.update_focus(ChunkCoord::new(10, 0, 0));
        assert_eq!(scheduler.diagnostics().tracked, 5);
        assert!(scheduler.status(ChunkCoord::new(-1, 0, 0)).is_none());
        assert_eq!(scheduler.drain_evictions().len(), 8);
    }

    #[test]
    fn incomplete_undesired_work_is_evicted_inside_retention_margin() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: 2,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let origin = ChunkCoord::new(0, 0, 0);
        scheduler.update_focus(origin);
        let work = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        let ticket = work.generation[0];

        scheduler.update_focus(ChunkCoord::new(1, 0, 0));
        assert!(scheduler.status(origin).is_none());
        assert_eq!(scheduler.complete(ticket), CompletionStatus::Stale);
        assert_eq!(scheduler.drain_evictions().len(), 1);
        assert_eq!(scheduler.diagnostics().generation.queued, 1);
    }

    #[test]
    fn residency_never_exceeds_capacity_during_large_focus_jumps() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 3,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 2,
            max_tracked_chunks: 17,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        for x in [0, 1, 20, -20, i32::MAX, i32::MIN] {
            scheduler.update_focus(ChunkCoord::new(x, 0, -x.saturating_abs()));
            let diagnostics = scheduler.diagnostics();
            assert!(diagnostics.tracked <= 17);
            assert!(diagnostics.desired <= 17);
        }
    }

    #[test]
    fn evicted_work_cannot_complete_into_a_reused_coordinate() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let origin = ChunkCoord::new(0, 0, 0);
        scheduler.update_focus(origin);
        let old = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        let old_ticket = old.generation[0];
        scheduler.update_focus(ChunkCoord::new(10, 0, 0));
        scheduler.update_focus(origin);
        assert_eq!(scheduler.complete(old_ticket), CompletionStatus::Stale);
        assert_eq!(
            scheduler.status(origin).map(|status| status.state),
            Some(ChunkState::QueuedGeneration)
        );
    }

    #[test]
    fn transient_failure_requeues_the_same_revision_with_a_new_ticket() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
            max_secondary_interest_chunks: MAX_SECONDARY_INTEREST_CHUNKS,
        });
        let coord = ChunkCoord::new(0, 0, 0);
        scheduler.update_focus(coord);
        let first = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        let ticket = first.generation[0];
        assert_eq!(scheduler.retry(ticket), CompletionStatus::Accepted);
        let second = scheduler.schedule_frame(FrameBudget {
            generation: 1,
            ..FrameBudget::default()
        });
        assert_eq!(second.generation[0].revision, ticket.revision);
        assert_ne!(second.generation[0].serial, ticket.serial);
    }
}
