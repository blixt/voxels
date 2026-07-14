//! Durable sparse edit authority shared by every world connection.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;
use voxels_world::protocol::{EditCommand, EditCommit, PlayerId};
use voxels_world::{
    ChunkCoord, EditMap, Material, SurfaceLodLevel, SurfaceTileCoord, VoxelCoord, WorldId,
    WorldSourceEngine, WorldSourceIdentityHash,
};

const EDIT_SCHEMA_VERSION: i64 = 1;
const INITIAL_REVISION: u64 = 1;

pub(crate) struct EditAuthority {
    inner: Mutex<EditState>,
    subscribers: Mutex<HashMap<u64, EditSubscriber>>,
    queue_capacity: usize,
}

struct EditState {
    connection: Connection,
    edits: EditMap,
    revision: u64,
    chunk_revisions: BTreeMap<ChunkCoord, u64>,
    surface_revisions: BTreeMap<SurfaceTileCoord, u64>,
}

struct EditSubscriber {
    sender: mpsc::Sender<EditCommit>,
    overflowed: Arc<AtomicBool>,
}

pub(crate) struct EditSubscription {
    pub(crate) receiver: mpsc::Receiver<EditCommit>,
    pub(crate) overflowed: Arc<AtomicBool>,
}

pub(crate) struct AppliedEdit {
    pub(crate) commit: EditCommit,
    pub(crate) changed: bool,
}

#[derive(Clone)]
pub(crate) struct ChunkEditSnapshot {
    pub(crate) edits: EditMap,
    pub(crate) revisions: Vec<u64>,
}

#[derive(Clone)]
pub(crate) struct SurfaceEditSnapshot {
    pub(crate) edits: EditMap,
    pub(crate) revisions: Vec<u64>,
}

#[derive(Debug)]
pub struct EditAuthorityError(String);

impl fmt::Display for EditAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EditAuthorityError {}

impl EditAuthority {
    pub(crate) fn open(
        path: &Path,
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
    ) -> Result<Arc<Self>, EditAuthorityError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                EditAuthorityError(format!(
                    "create edit database directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let connection = Connection::open(path).map_err(sql_error("open edit database"))?;
        Self::from_connection(connection, world_id, source, queue_capacity)
    }

    pub(crate) fn in_memory(
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
    ) -> Result<Arc<Self>, EditAuthorityError> {
        let connection = Connection::open_in_memory().map_err(sql_error("open edit database"))?;
        Self::from_connection(connection, world_id, source, queue_capacity)
    }

    fn from_connection(
        mut connection: Connection,
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
    ) -> Result<Arc<Self>, EditAuthorityError> {
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
            )
            .map_err(sql_error("configure edit database"))?;
        initialize_schema(&mut connection, world_id, source.identity().identity_hash())?;
        let (edits, rows, revision) = load_edits(&connection)?;
        let mut chunk_revisions = BTreeMap::new();
        let mut surface_revisions = BTreeMap::new();
        for (coord, row_revision) in rows {
            for chunk in EditMap::affected_chunks(coord) {
                bump_product_revision(&mut chunk_revisions, chunk, row_revision);
            }
            for level in SurfaceLodLevel::ALL {
                for tile in source.surface_tiles_affected_by_voxel(&edits, level, coord) {
                    bump_product_revision(&mut surface_revisions, tile, row_revision);
                }
            }
        }
        Ok(Arc::new(Self {
            inner: Mutex::new(EditState {
                connection,
                edits,
                revision,
                chunk_revisions,
                surface_revisions,
            }),
            subscribers: Mutex::new(HashMap::new()),
            queue_capacity: usize::from(queue_capacity),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, EditState> {
        match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn lock_subscribers(&self) -> MutexGuard<'_, HashMap<u64, EditSubscriber>> {
        match self.subscribers.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn subscribe(&self, connection_id: u64) -> EditSubscription {
        let (sender, receiver) = mpsc::channel(self.queue_capacity);
        let overflowed = Arc::new(AtomicBool::new(false));
        self.lock_subscribers().insert(
            connection_id,
            EditSubscriber {
                sender,
                overflowed: Arc::clone(&overflowed),
            },
        );
        EditSubscription {
            receiver,
            overflowed,
        }
    }

    pub(crate) fn unsubscribe(&self, connection_id: u64) {
        self.lock_subscribers().remove(&connection_id);
    }

    pub(crate) fn publish(&self, commit: &EditCommit, recipients: &BTreeSet<u64>) {
        let mut subscribers = self.lock_subscribers();
        subscribers.retain(|connection_id, subscriber| {
            if !recipients.contains(connection_id) {
                return !subscriber.sender.is_closed();
            }
            match subscriber.sender.try_send(commit.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    subscriber.overflowed.store(true, Ordering::Release);
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }

    pub(crate) fn revision(&self) -> u64 {
        self.lock().revision
    }

    pub(crate) fn snapshot_chunks(&self, coords: &[ChunkCoord]) -> ChunkEditSnapshot {
        let state = self.lock();
        ChunkEditSnapshot {
            edits: state.edits.snapshot_for_chunks(coords),
            revisions: coords
                .iter()
                .map(|coord| {
                    state
                        .chunk_revisions
                        .get(coord)
                        .copied()
                        .unwrap_or(INITIAL_REVISION)
                })
                .collect(),
        }
    }

    pub(crate) fn snapshot_surface(&self, coords: &[SurfaceTileCoord]) -> SurfaceEditSnapshot {
        let state = self.lock();
        SurfaceEditSnapshot {
            edits: state.edits.snapshot_for_surface_tiles(coords),
            revisions: coords
                .iter()
                .map(|coord| {
                    state
                        .surface_revisions
                        .get(coord)
                        .copied()
                        .unwrap_or(INITIAL_REVISION)
                })
                .collect(),
        }
    }

    pub(crate) fn apply(
        &self,
        source: &dyn WorldSourceEngine,
        player_id: PlayerId,
        command: EditCommand,
    ) -> Result<AppliedEdit, EditAuthorityError> {
        let mut state = self.lock();
        if let Some(stored) = load_operation(&state.connection, player_id, command.operation_id)? {
            if stored.coord != command.coord || stored.material != command.material {
                return Err(EditAuthorityError(
                    "edit operation id was reused with a different command".to_owned(),
                ));
            }
            let commit = current_commit(source, &state.edits, command, stored.revision);
            return Ok(AppliedEdit {
                commit,
                changed: false,
            });
        }

        let previous = state.edits.override_at(command.coord);
        if previous == command.material {
            let revision = state.revision;
            let transaction = state
                .connection
                .transaction()
                .map_err(sql_error("begin edit transaction"))?;
            persist_operation(&transaction, player_id, command, revision)?;
            transaction
                .commit()
                .map_err(sql_error("commit edit operation"))?;
            return Ok(AppliedEdit {
                commit: current_commit(source, &state.edits, command, revision),
                changed: false,
            });
        }

        let before_surface = affected_surface(source, &state.edits, command.coord);
        state
            .edits
            .replace_durable_override(command.coord, command.material);
        let after_surface = affected_surface(source, &state.edits, command.coord);
        let affected_surface_tiles = before_surface
            .into_iter()
            .chain(after_surface)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut affected_chunks = EditMap::affected_chunks(command.coord);
        affected_chunks.sort_unstable();
        let revision = state.revision.saturating_add(1);
        let persisted = persist_change(&mut state.connection, player_id, command, revision);
        if let Err(error) = persisted {
            state
                .edits
                .replace_durable_override(command.coord, previous);
            return Err(error);
        }
        state.revision = revision;
        for coord in &affected_chunks {
            state.chunk_revisions.insert(*coord, revision);
        }
        for coord in &affected_surface_tiles {
            state.surface_revisions.insert(*coord, revision);
        }
        Ok(AppliedEdit {
            commit: EditCommit {
                operation_id: command.operation_id,
                revision,
                coord: command.coord,
                material: command.material,
                affected_chunks,
                affected_surface_tiles,
            },
            changed: true,
        })
    }
}

struct StoredOperation {
    revision: u64,
    coord: VoxelCoord,
    material: Option<Material>,
}

fn initialize_schema(
    connection: &mut Connection,
    world_id: WorldId,
    source_hash: WorldSourceIdentityHash,
) -> Result<(), EditAuthorityError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql_error("read edit schema version"))?;
    if version == 0 {
        let transaction = connection
            .transaction()
            .map_err(sql_error("begin edit schema"))?;
        transaction
            .execute_batch(
                "CREATE TABLE metadata (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    world_id BLOB NOT NULL,
                    source_hash BLOB NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision >= 1)
                 );
                 CREATE TABLE voxel_edits (
                    x INTEGER NOT NULL,
                    y INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    material INTEGER NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision >= 1),
                    PRIMARY KEY (x, y, z)
                 ) WITHOUT ROWID;
                 CREATE TABLE edit_operations (
                    player_id BLOB NOT NULL,
                    operation_id BLOB NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision >= 1),
                    x INTEGER NOT NULL,
                    y INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    material INTEGER,
                    PRIMARY KEY (player_id, operation_id)
                 ) WITHOUT ROWID;
                 PRAGMA user_version = 1;",
            )
            .map_err(sql_error("create edit schema"))?;
        transaction
            .execute(
                "INSERT INTO metadata(singleton, world_id, source_hash, revision) VALUES(1, ?1, ?2, ?3)",
                params![
                    world_id.as_bytes().as_slice(),
                    source_hash.as_bytes().as_slice(),
                    INITIAL_REVISION as i64
                ],
            )
            .map_err(sql_error("initialize edit metadata"))?;
        transaction
            .commit()
            .map_err(sql_error("commit edit schema"))?;
        return Ok(());
    }
    if version != EDIT_SCHEMA_VERSION {
        return Err(EditAuthorityError(format!(
            "unsupported edit database schema {version}; expected {EDIT_SCHEMA_VERSION}"
        )));
    }
    let identity = connection
        .query_row(
            "SELECT world_id, source_hash FROM metadata WHERE singleton=1",
            [],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(sql_error("read edit metadata"))?;
    if identity.0 != world_id.as_bytes() || identity.1 != source_hash.as_bytes() {
        return Err(EditAuthorityError(
            "edit database belongs to a different world manifest".to_owned(),
        ));
    }
    Ok(())
}

type LoadedRows = (EditMap, Vec<(VoxelCoord, u64)>, u64);

fn load_edits(connection: &Connection) -> Result<LoadedRows, EditAuthorityError> {
    let revision = connection
        .query_row(
            "SELECT revision FROM metadata WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql_error("read edit revision"))?;
    let mut statement = connection
        .prepare("SELECT x, y, z, material, revision FROM voxel_edits ORDER BY x, y, z")
        .map_err(sql_error("prepare edit load"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                VoxelCoord::new(row.get(0)?, row.get(1)?, row.get(2)?),
                row.get::<_, u16>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(sql_error("load edits"))?;
    let mut edits = EditMap::default();
    let mut revisions = Vec::new();
    for row in rows {
        let (coord, material_id, row_revision) = row.map_err(sql_error("decode edit row"))?;
        let material = Material::from_id(material_id).ok_or_else(|| {
            EditAuthorityError(format!("unknown durable material id {material_id}"))
        })?;
        edits.insert_override(coord, material);
        revisions.push((coord, row_revision as u64));
    }
    Ok((edits, revisions, (revision as u64).max(INITIAL_REVISION)))
}

fn load_operation(
    connection: &Connection,
    player_id: PlayerId,
    operation_id: u64,
) -> Result<Option<StoredOperation>, EditAuthorityError> {
    let stored = connection
        .query_row(
            "SELECT revision, x, y, z, material FROM edit_operations WHERE player_id=?1 AND operation_id=?2",
            params![player_id.as_bytes().as_slice(), operation_id.to_le_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    VoxelCoord::new(row.get(1)?, row.get(2)?, row.get(3)?),
                    row.get::<_, Option<u16>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error("load edit operation"))?;
    stored
        .map(|(revision, coord, material_id)| {
            let material = material_id
                .map(|material_id| {
                    Material::from_id(material_id).ok_or_else(|| {
                        EditAuthorityError(format!(
                            "unknown durable operation material id {material_id}"
                        ))
                    })
                })
                .transpose()?;
            Ok(StoredOperation {
                revision: revision as u64,
                coord,
                material,
            })
        })
        .transpose()
}

fn persist_change(
    connection: &mut Connection,
    player_id: PlayerId,
    command: EditCommand,
    revision: u64,
) -> Result<(), EditAuthorityError> {
    let revision = i64::try_from(revision)
        .map_err(|_| EditAuthorityError("edit revision exceeded SQLite INTEGER".to_owned()))?;
    let transaction = connection
        .transaction()
        .map_err(sql_error("begin edit transaction"))?;
    if let Some(material) = command.material {
        transaction
            .execute(
                "INSERT INTO voxel_edits(x,y,z,material,revision) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(x,y,z) DO UPDATE SET material=excluded.material, revision=excluded.revision",
                params![command.coord.x, command.coord.y, command.coord.z, material.id(), revision],
            )
            .map_err(sql_error("persist voxel edit"))?;
    } else {
        transaction
            .execute(
                "DELETE FROM voxel_edits WHERE x=?1 AND y=?2 AND z=?3",
                params![command.coord.x, command.coord.y, command.coord.z],
            )
            .map_err(sql_error("delete voxel edit"))?;
    }
    transaction
        .execute(
            "UPDATE metadata SET revision=?1 WHERE singleton=1",
            [revision],
        )
        .map_err(sql_error("persist edit revision"))?;
    persist_operation(&transaction, player_id, command, revision as u64)?;
    transaction.commit().map_err(sql_error("commit voxel edit"))
}

fn persist_operation(
    transaction: &Transaction<'_>,
    player_id: PlayerId,
    command: EditCommand,
    revision: u64,
) -> Result<(), EditAuthorityError> {
    let revision = i64::try_from(revision)
        .map_err(|_| EditAuthorityError("edit revision exceeded SQLite INTEGER".to_owned()))?;
    transaction
        .execute(
            "INSERT INTO edit_operations(player_id,operation_id,revision,x,y,z,material)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                player_id.as_bytes().as_slice(),
                command.operation_id.to_le_bytes().as_slice(),
                revision,
                command.coord.x,
                command.coord.y,
                command.coord.z,
                command.material.map(Material::id)
            ],
        )
        .map(|_| ())
        .map_err(sql_error("persist edit operation"))
}

fn current_commit(
    source: &dyn WorldSourceEngine,
    edits: &EditMap,
    command: EditCommand,
    revision: u64,
) -> EditCommit {
    let mut affected_chunks = EditMap::affected_chunks(command.coord);
    affected_chunks.sort_unstable();
    EditCommit {
        operation_id: command.operation_id,
        revision,
        coord: command.coord,
        material: command.material,
        affected_chunks,
        affected_surface_tiles: affected_surface(source, edits, command.coord),
    }
}

fn affected_surface(
    source: &dyn WorldSourceEngine,
    edits: &EditMap,
    coord: VoxelCoord,
) -> Vec<SurfaceTileCoord> {
    let mut affected = BTreeSet::new();
    for level in SurfaceLodLevel::ALL {
        affected.extend(source.surface_tiles_affected_by_voxel(edits, level, coord));
    }
    affected.into_iter().collect()
}

fn bump_product_revision<K: Ord>(revisions: &mut BTreeMap<K, u64>, key: K, revision: u64) {
    revisions
        .entry(key)
        .and_modify(|current| *current = (*current).max(revision))
        .or_insert(revision);
}

fn sql_error(context: &'static str) -> impl FnOnce(rusqlite::Error) -> EditAuthorityError {
    move |error| EditAuthorityError(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use voxels_world::ProceduralWorldSource;

    fn world_id(seed: u8) -> WorldId {
        WorldId::from_bytes([seed; 16])
    }

    fn player_id(seed: u8) -> PlayerId {
        PlayerId::from_bytes([seed; 16])
    }

    fn command(operation_id: u64, coord: VoxelCoord, material: Option<Material>) -> EditCommand {
        EditCommand {
            operation_id,
            coord,
            material,
        }
    }

    #[test]
    fn idempotent_operations_advance_only_local_product_revisions() {
        let source = ProceduralWorldSource::new(42);
        let authority = EditAuthority::in_memory(world_id(1), &source, 4).unwrap();
        let coord = VoxelCoord::new(13, 200, -21);
        let owner = coord.chunk();
        let unrelated = ChunkCoord::new(owner.x + 100, owner.y, owner.z);
        let edit = command(7, coord, Some(Material::Wood));

        let applied = authority.apply(&source, player_id(2), edit).unwrap();
        assert!(applied.changed);
        assert_eq!(applied.commit.revision, 2);
        assert!(applied.commit.affected_chunks.contains(&owner));
        assert!(!applied.commit.affected_surface_tiles.is_empty());

        let chunks = authority.snapshot_chunks(&[owner, unrelated]);
        assert_eq!(chunks.edits.override_at(coord), Some(Material::Wood));
        assert_eq!(chunks.revisions, vec![2, INITIAL_REVISION]);
        let surface = authority.snapshot_surface(&applied.commit.affected_surface_tiles);
        assert!(surface.revisions.iter().all(|revision| *revision == 2));

        let retried = authority.apply(&source, player_id(2), edit).unwrap();
        assert!(!retried.changed);
        assert_eq!(retried.commit.revision, 2);
        assert_eq!(authority.revision(), 2);

        let reused = authority.apply(
            &source,
            player_id(2),
            command(7, coord, Some(Material::Stone)),
        );
        assert!(
            reused
                .err()
                .unwrap()
                .to_string()
                .contains("reused with a different command")
        );
    }

    #[test]
    fn sqlite_restart_preserves_edits_and_rejects_a_different_world() {
        let source = ProceduralWorldSource::new(91);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "voxels-edit-authority-{}-{unique}.sqlite3",
            std::process::id()
        ));
        let coord = VoxelCoord::new(-57, 180, 89);
        let owner = coord.chunk();

        {
            let authority = EditAuthority::open(&path, world_id(3), &source, 4).unwrap();
            let applied = authority
                .apply(
                    &source,
                    player_id(4),
                    command(11, coord, Some(Material::GlowCrystal)),
                )
                .unwrap();
            assert_eq!(applied.commit.revision, 2);
        }
        {
            let reopened = EditAuthority::open(&path, world_id(3), &source, 4).unwrap();
            let snapshot = reopened.snapshot_chunks(&[owner]);
            assert_eq!(
                snapshot.edits.override_at(coord),
                Some(Material::GlowCrystal)
            );
            assert_eq!(snapshot.revisions, vec![2]);
            assert_eq!(reopened.revision(), 2);
        }
        let mismatch = EditAuthority::open(&path, world_id(5), &source, 4);
        assert!(
            mismatch
                .err()
                .unwrap()
                .to_string()
                .contains("different world manifest")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    }

    #[test]
    fn corrupted_operation_material_is_not_reinterpreted_as_a_removal() {
        let source = ProceduralWorldSource::new(17);
        let authority = EditAuthority::in_memory(world_id(6), &source, 4).unwrap();
        let coord = VoxelCoord::new(1, 200, 1);
        let edit = command(15, coord, Some(Material::Wood));
        authority.apply(&source, player_id(7), edit).unwrap();
        authority
            .lock()
            .connection
            .execute("UPDATE edit_operations SET material=65535", [])
            .unwrap();

        let error = authority.apply(&source, player_id(7), edit).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("unknown durable operation material")
        );
    }
}
