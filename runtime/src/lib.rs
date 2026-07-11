//! Host-testable orchestration for bounded, deterministic voxel streaming.
//!
//! This crate owns no chunk payloads and performs no generation, meshing, or GPU work itself.
//! Instead, it issues versioned work tickets and advances chunks only after the host reports a
//! matching completion. That keeps scheduling deterministic while allowing native threads, web
//! workers, and render backends to execute the expensive stages differently.

use std::collections::{BTreeMap, BTreeSet};

use voxels_world::{CHUNK_EDGE, ChunkCoord, EditMap, VOXEL_SIZE_METRES, VoxelCoord};

/// Physical edge length of a full-resolution chunk.
pub const CHUNK_EDGE_METRES: f32 = CHUNK_EDGE as f32 * VOXEL_SIZE_METRES;

const MAX_LOAD_RADIUS_CHUNKS: i32 = 64;
const MAX_VERTICAL_RADIUS_CHUNKS: i32 = 32;

type CoordKey = (i32, i32, i32);

/// Limits for one scheduler instance. LOD tiers should use separate schedulers/configurations so
/// the full-resolution residency ceiling stays explicit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamConfig {
    pub load_radius_chunks: i32,
    pub vertical_radius_chunks: i32,
    pub retention_margin_chunks: i32,
    pub max_tracked_chunks: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            load_radius_chunks: 9,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 2,
            max_tracked_chunks: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    NegativeRadius,
    RadiusTooLarge,
    EmptyCapacity,
}

/// Maximum amount of new work that may enter each asynchronous stage during one frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameBudget {
    pub generation: usize,
    pub meshing: usize,
    pub upload: usize,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamDiagnostics {
    pub frame: u64,
    pub tracked: usize,
    pub desired: usize,
    pub resident: usize,
    pub generation: StageCounts,
    pub meshing: StageCounts,
    pub upload: StageCounts,
    pub accepted_completions: u64,
    pub stale_completions: u64,
    pub total_evictions: u64,
    pub started_this_frame: FrameBudget,
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
}

/// Deterministic metadata scheduler for the chunk generation -> meshing -> upload pipeline.
pub struct StreamScheduler {
    config: StreamConfig,
    focus: ChunkCoord,
    entries: BTreeMap<CoordKey, Entry>,
    evictions: Vec<EvictedChunk>,
    next_ticket_serial: u64,
    frame: u64,
    accepted_completions: u64,
    stale_completions: u64,
    total_evictions: u64,
    started_this_frame: FrameBudget,
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

        Ok(Self {
            config,
            focus: ChunkCoord::new(0, 0, 0),
            entries: BTreeMap::new(),
            evictions: Vec::new(),
            next_ticket_serial: 1,
            frame: 0,
            accepted_completions: 0,
            stale_completions: 0,
            total_evictions: 0,
            started_this_frame: FrameBudget::default(),
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
    pub fn update_focus(&mut self, focus: ChunkCoord) {
        self.focus = focus;
        let desired = self.desired_coordinates(focus);
        let desired_keys: BTreeSet<_> = desired.iter().copied().map(coord_key).collect();

        for (key, entry) in &mut self.entries {
            entry.desired = desired_keys.contains(key);
        }

        let outside_retention: Vec<_> = self
            .entries
            .values()
            .filter(|entry| !inside_retention(self.config, focus, entry.coord))
            .map(|entry| coord_key(entry.coord))
            .collect();
        for key in outside_retention {
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
                    },
                );
            }
        }
    }

    /// Starts at most the supplied number of jobs per stage. Work is stable-sorted by distance to
    /// the current focus and then by coordinate, so identical input histories issue identical
    /// tickets.
    pub fn schedule_frame(&mut self, budget: FrameBudget) -> FrameWork {
        self.frame = increment_nonzero(self.frame);
        self.started_this_frame = FrameBudget::default();

        let generation = self.start_stage(
            WorkStage::Generation,
            State::QueuedGeneration,
            budget.generation,
        );
        let meshing = self.start_stage(WorkStage::Meshing, State::QueuedMeshing, budget.meshing);
        let upload = self.start_stage(WorkStage::Upload, State::QueuedUpload, budget.upload);
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
            }
            self.accepted_completions = increment_nonzero(self.accepted_completions);
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
        let affected_chunks = EditMap::affected_chunks(voxel);
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
            ..StreamDiagnostics::default()
        };
        for entry in self.entries.values() {
            diagnostics.desired += usize::from(entry.desired);
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
                    candidates.push(ChunkCoord::new(x, y, z));
                }
            }
        }
        candidates.sort_by_key(|coord| priority(focus, *coord));
        candidates.truncate(self.config.max_tracked_chunks);
        candidates
    }

    fn start_stage(&mut self, stage: WorkStage, queued: State, budget: usize) -> Vec<WorkTicket> {
        let mut keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.desired && entry.state == queued)
            .map(|(key, _)| *key)
            .collect();
        keys.sort_by_key(|key| priority(self.focus, coord_from_key(*key)));
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
        dx * dx + dz * dz + dy * dy * 4,
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
    }

    #[test]
    fn canonical_scale_is_ten_centimetres_and_chunks_are_32_voxels() {
        assert_eq!(VOXEL_SIZE_METRES, 0.1);
        assert_eq!(CHUNK_EDGE, 32);
        assert!((CHUNK_EDGE_METRES - 3.2).abs() < f32::EPSILON);
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
    fn work_advances_through_all_pipeline_stages() {
        let focus = ChunkCoord::new(4, -2, 7);
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 1,
            max_tracked_chunks: 1,
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
    fn edit_invalidates_an_in_flight_ticket_and_bumps_revision() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 0,
            vertical_radius_chunks: 0,
            retention_margin_chunks: 0,
            max_tracked_chunks: 1,
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
    fn residency_never_exceeds_capacity_during_large_focus_jumps() {
        let mut scheduler = scheduler(StreamConfig {
            load_radius_chunks: 3,
            vertical_radius_chunks: 1,
            retention_margin_chunks: 2,
            max_tracked_chunks: 17,
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
}
