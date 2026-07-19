//! Deterministic durability benchmark owned by the native world-service authority.
//!
//! The TypeScript automation layer orchestrates artifacts and comparisons, while this module
//! exercises the production edit planner, SQLite transaction boundary, idempotency journal, and
//! restart hydration without reimplementing their semantics in scripts.

use crate::EDIT_DATABASE_SCHEMA_VERSION;
use crate::edits::EditAuthority;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use voxels_world::protocol::{EditAction, EditCommand, EditSessionId, PlayerId, PlayerResume};
use voxels_world::{
    ProceduralWorldSource, SurfaceSampleBlockRequest, VoxelCoord, WorldId, WorldProduct,
    WorldProductBatch, WorldProductPriority, WorldProductRequest, WorldSourceEngine,
};

pub const STORAGE_BENCHMARK_SCHEMA_VERSION: u32 = 1;
const BENCHMARK_SEED: u64 = 0x5e6a_2d49_7b10_c3f1;
const EDIT_QUEUE_CAPACITY: u16 = 8;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageBenchmarkProfile {
    Clustered,
    Frontier,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageBenchmarkRequest {
    pub schema_version: u32,
    pub database_path: PathBuf,
    pub operations: u32,
    pub players: u16,
    pub profile: StorageBenchmarkProfile,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBenchmarkResponse {
    pub schema_version: u32,
    pub edit_database_schema_version: i64,
    pub sqlite_version: String,
    pub profile: &'static str,
    pub players: u16,
    pub operations: u32,
    pub changed_operations: u32,
    pub mutations: u64,
    pub operation_latency_micros: LatencySummary,
    pub player_initialization_ms: f64,
    pub checkpoint_ms: f64,
    pub restart_ms: f64,
    pub retry_verification_ms: f64,
    pub database_before_checkpoint: DatabaseFiles,
    pub database_after_checkpoint: DatabaseFiles,
    pub tables: BTreeMap<String, TableStats>,
    pub page_size_bytes: u64,
    pub page_count: u64,
    pub freelist_pages: u64,
    pub revision_before_restart: u64,
    pub revision_after_restart: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub minimum: u64,
    pub median: u64,
    pub p95: u64,
    pub p99: u64,
    pub maximum: u64,
    pub mean: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseFiles {
    pub main_bytes: u64,
    pub wal_bytes: u64,
    pub shm_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStats {
    pub rows: u64,
    pub pages: u64,
    pub payload_bytes: u64,
    pub unused_bytes: u64,
    pub storage_bytes: u64,
}

struct DatabaseInspection {
    tables: BTreeMap<String, TableStats>,
    page_size_bytes: u64,
    page_count: u64,
    freelist_pages: u64,
    sqlite_version: String,
}

struct PlayerBenchmarkState {
    id: PlayerId,
    session: EditSessionId,
}

pub fn run_storage_benchmark(
    request: StorageBenchmarkRequest,
) -> Result<StorageBenchmarkResponse, Box<dyn std::error::Error>> {
    validate_request(&request)?;
    remove_database_files(&request.database_path)?;

    let source = ProceduralWorldSource::new(BENCHMARK_SEED);
    let world_id = benchmark_world_id();
    let player_initialization_started = Instant::now();
    let authority = EditAuthority::open(
        &request.database_path,
        world_id,
        &source,
        EDIT_QUEUE_CAPACITY,
    )?;
    let players = initialize_players(&authority, request.players)?;
    let player_initialization_ms = elapsed_millis(player_initialization_started.elapsed());
    let hits = benchmark_hits(&source, request.profile, request.operations)?;

    let mut operation_latencies = Vec::with_capacity(request.operations as usize);
    let mut changed_operations = 0_u32;
    let mut mutations = 0_u64;
    let mut first_commands = vec![None; players.len()];
    for operation_index in 0..request.operations {
        let player_index = operation_index as usize % players.len();
        let player = &players[player_index];
        let operation_id = u64::from(operation_index / u32::from(request.players)) + 1;
        let command = EditCommand {
            operation_id,
            edit_session_id: player.session,
            action: EditAction::Dig {
                hit: hits[operation_index as usize],
            },
        };
        if first_commands[player_index].is_none() {
            first_commands[player_index] = Some(command);
        }
        let started = Instant::now();
        let applied = authority.apply(
            &source,
            player.id,
            u64::from(player_index as u32 + 1),
            command,
        )?;
        operation_latencies.push(elapsed_micros(started.elapsed()));
        if applied.changed {
            changed_operations += 1;
            mutations += applied.commit.mutations.len() as u64;
        }
    }
    if changed_operations != request.operations {
        return Err(format!(
            "benchmark corpus changed only {changed_operations} of {} operations",
            request.operations
        )
        .into());
    }
    let revision_before_restart = authority.revision();
    let database_before_checkpoint = database_files(&request.database_path)?;

    let checkpoint_started = Instant::now();
    checkpoint_database(&request.database_path)?;
    let checkpoint_ms = elapsed_millis(checkpoint_started.elapsed());
    let database_after_checkpoint = database_files(&request.database_path)?;
    let database = inspect_database(&request.database_path)?;

    drop(authority);
    let restart_started = Instant::now();
    let reopened = EditAuthority::open(
        &request.database_path,
        world_id,
        &source,
        EDIT_QUEUE_CAPACITY,
    )?;
    let restart_ms = elapsed_millis(restart_started.elapsed());
    let revision_after_restart = reopened.revision();
    if revision_after_restart != revision_before_restart {
        return Err(format!(
            "restart revision changed from {revision_before_restart} to {revision_after_restart}"
        )
        .into());
    }

    let retry_started = Instant::now();
    for (index, (player, command)) in players.iter().zip(first_commands).enumerate() {
        let command = command.ok_or("player received no benchmark operation")?;
        let retried = reopened.apply(&source, player.id, u64::from(index as u32 + 1), command)?;
        if retried.changed {
            return Err("durable operation retry changed the world after restart".into());
        }
    }
    let retry_verification_ms = elapsed_millis(retry_started.elapsed());

    Ok(StorageBenchmarkResponse {
        schema_version: STORAGE_BENCHMARK_SCHEMA_VERSION,
        edit_database_schema_version: EDIT_DATABASE_SCHEMA_VERSION,
        sqlite_version: database.sqlite_version,
        profile: match request.profile {
            StorageBenchmarkProfile::Clustered => "clustered",
            StorageBenchmarkProfile::Frontier => "frontier",
        },
        players: request.players,
        operations: request.operations,
        changed_operations,
        mutations,
        operation_latency_micros: summarize_latencies(&mut operation_latencies),
        player_initialization_ms,
        checkpoint_ms,
        restart_ms,
        retry_verification_ms,
        database_before_checkpoint,
        database_after_checkpoint,
        tables: database.tables,
        page_size_bytes: database.page_size_bytes,
        page_count: database.page_count,
        freelist_pages: database.freelist_pages,
        revision_before_restart,
        revision_after_restart,
    })
}

fn validate_request(request: &StorageBenchmarkRequest) -> Result<(), &'static str> {
    if request.schema_version != STORAGE_BENCHMARK_SCHEMA_VERSION {
        return Err("unsupported storage benchmark schema");
    }
    if request.operations == 0 || request.operations > 1_000_000 {
        return Err("operations must be in 1..=1000000");
    }
    if request.players == 0 || request.players > 1_024 {
        return Err("players must be in 1..=1024");
    }
    if u32::from(request.players) > request.operations {
        return Err("operations must be at least the player count");
    }
    Ok(())
}

fn initialize_players(
    authority: &EditAuthority,
    count: u16,
) -> Result<Vec<PlayerBenchmarkState>, Box<dyn std::error::Error>> {
    (0..count)
        .map(|index| {
            let id = PlayerId::from_bytes(benchmark_uuid_bytes(0x70, u64::from(index) + 1));
            authority.load_player(
                id,
                PlayerResume {
                    revision: 1,
                    eye_position_metres: [0.0, 0.0, 0.0],
                    look_yaw_radians: 0.0,
                    look_pitch_radians: 0.0,
                },
            )?;
            Ok(PlayerBenchmarkState {
                id,
                session: authority.begin_player_session(id)?,
            })
        })
        .collect()
}

fn benchmark_xz(profile: StorageBenchmarkProfile, index: u32) -> [i32; 2] {
    let index = i64::from(index);
    let (x, z) = match profile {
        StorageBenchmarkProfile::Clustered => {
            let x = (index % 64) * 7;
            let z = ((index / 64) % 64) * 7;
            let layer = index / (64 * 64);
            (x + layer * 65_536, z)
        }
        StorageBenchmarkProfile::Frontier => {
            let ring = index / 256;
            let cell = index % 256;
            let x = ring * 32_768 + (cell % 16) * 257;
            let z = ring * -24_576 + (cell / 16) * 257;
            (x, z)
        }
    };
    [x as i32, z as i32]
}

fn benchmark_hits(
    source: &dyn WorldSourceEngine,
    profile: StorageBenchmarkProfile,
    operations: u32,
) -> Result<Vec<VoxelCoord>, Box<dyn std::error::Error>> {
    (0..operations)
        .map(|index| {
            let [x, z] = benchmark_xz(profile, index);
            let request = SurfaceSampleBlockRequest {
                origin: [x, z],
                sample_shape: [1, 1],
            };
            let result = source.generate_batch(WorldProductBatch {
                priority: WorldProductPriority::CollisionCritical,
                requests: vec![WorldProductRequest::SurfaceSampleBlock(request)],
            })?;
            let item = result
                .items
                .into_iter()
                .next()
                .ok_or("source omitted benchmark surface sample")?;
            let snapshot = match item.result? {
                WorldProduct::SurfaceSampleBlock(snapshot) if snapshot.request == request => {
                    snapshot
                }
                _ => return Err("source returned mismatched benchmark surface sample".into()),
            };
            let sample = snapshot
                .sample(x, z)
                .ok_or("benchmark surface sample omitted its origin")?;
            Ok(VoxelCoord::new(x, sample.height, z))
        })
        .collect()
}

fn benchmark_world_id() -> WorldId {
    WorldId::from_bytes(benchmark_uuid_bytes(0x57, 1))
}

fn benchmark_uuid_bytes(prefix: u8, value: u64) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[0] = prefix;
    bytes[8..].copy_from_slice(&value.to_be_bytes());
    bytes
}

fn checkpoint_database(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

fn inspect_database(path: &Path) -> Result<DatabaseInspection, Box<dyn std::error::Error>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let sqlite_version = connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let page_size_bytes = pragma_u64(&connection, "page_size")?;
    let page_count = pragma_u64(&connection, "page_count")?;
    let freelist_pages = pragma_u64(&connection, "freelist_count")?;
    let mut tables = BTreeMap::new();
    for table in [
        "metadata",
        "voxel_edits",
        "players",
        "player_inventory",
        "edit_operations",
    ] {
        let rows = connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })?;
        let (pages, payload_bytes, unused_bytes, storage_bytes) = connection.query_row(
            "SELECT count(*), coalesce(sum(payload), 0), coalesce(sum(unused), 0),
                    coalesce(sum(pgsize), 0)
             FROM dbstat WHERE name=?1",
            [table],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        tables.insert(
            table.to_owned(),
            TableStats {
                rows: nonnegative(rows, "table row count")?,
                pages: nonnegative(pages, "table page count")?,
                payload_bytes: nonnegative(payload_bytes, "table payload bytes")?,
                unused_bytes: nonnegative(unused_bytes, "table unused bytes")?,
                storage_bytes: nonnegative(storage_bytes, "table storage bytes")?,
            },
        );
    }
    Ok(DatabaseInspection {
        tables,
        page_size_bytes,
        page_count,
        freelist_pages,
        sqlite_version,
    })
}

fn pragma_u64(connection: &Connection, name: &str) -> rusqlite::Result<u64> {
    connection.query_row(&format!("PRAGMA {name}"), [], |row| {
        let value = row.get::<_, i64>(0)?;
        u64::try_from(value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })
    })
}

fn nonnegative(value: i64, label: &str) -> Result<u64, Box<dyn std::error::Error>> {
    u64::try_from(value).map_err(|_| format!("{label} is negative").into())
}

fn database_files(path: &Path) -> std::io::Result<DatabaseFiles> {
    let main_bytes = file_len(path)?;
    let wal_bytes = file_len(&PathBuf::from(format!("{}-wal", path.display())))?;
    let shm_bytes = file_len(&PathBuf::from(format!("{}-shm", path.display())))?;
    Ok(DatabaseFiles {
        main_bytes,
        wal_bytes,
        shm_bytes,
        total_bytes: main_bytes + wal_bytes + shm_bytes,
    })
}

fn file_len(path: &Path) -> std::io::Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

fn remove_database_files(path: &Path) -> std::io::Result<()> {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn summarize_latencies(values: &mut [u64]) -> LatencySummary {
    values.sort_unstable();
    let total = values.iter().copied().map(u128::from).sum::<u128>();
    LatencySummary {
        minimum: values[0],
        median: percentile(values, 50),
        p95: percentile(values, 95),
        p99: percentile(values, 99),
        maximum: values[values.len() - 1],
        mean: total as f64 / values.len() as f64,
    }
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    let index = (values.len() - 1) * percentile / 100;
    values[index]
}

fn elapsed_micros(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

fn elapsed_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_coordinates_do_not_overlap_dig_spheres() {
        for profile in [
            StorageBenchmarkProfile::Clustered,
            StorageBenchmarkProfile::Frontier,
        ] {
            let coordinates = (0..10_000)
                .map(|index| benchmark_xz(profile, index))
                .collect::<Vec<_>>();
            let mut sorted = coordinates.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), coordinates.len());
        }
    }

    #[test]
    fn latency_summary_uses_stable_nearest_rank_indices() {
        let mut values = (1..=100).collect::<Vec<_>>();
        let summary = summarize_latencies(&mut values);
        assert_eq!(summary.minimum, 1);
        assert_eq!(summary.median, 50);
        assert_eq!(summary.p95, 95);
        assert_eq!(summary.p99, 99);
        assert_eq!(summary.maximum, 100);
        assert_eq!(summary.mean, 50.5);
    }
}
