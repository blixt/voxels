//! Durable world-edit, player-inventory, and resume-state authority.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;
use uuid::Uuid;
use voxels_world::protocol::{
    EDIT_SESSION_NOT_CURRENT, EditAction, EditCommand, EditCommit, EditSessionId,
    MATERIAL_INVENTORY_SLOTS, MAX_EDIT_AFFECTED_CHUNKS, MAX_EDIT_AFFECTED_SURFACE_TILES,
    MAX_EDIT_MUTATIONS, MaterialInventory, PlayerId, PlayerResume, VoxelFace, VoxelMutation,
};
use voxels_world::{
    ChunkCoord, EditMap, Material, SurfaceLodLevel, SurfaceTileCoord, VOXEL_SIZE_METRES,
    VoxelBlockRequest, VoxelCoord, WorldId, WorldProduct, WorldProductBatch, WorldProductPriority,
    WorldProductRequest, WorldSourceEngine, WorldSourceIdentityHash,
};

use crate::EDIT_DATABASE_SCHEMA_VERSION;

const INITIAL_REVISION: u64 = 1;
const DIG_EDGE_VOXELS: i32 = 5;
const DIG_RADIUS_VOXELS: i32 = DIG_EDGE_VOXELS / 2;
const DIG_SAMPLE_SHAPE: [u32; 3] = [DIG_EDGE_VOXELS as u32; 3];
const DIG_MAX_MUTATIONS: usize = DIG_EDGE_VOXELS.pow(3) as usize;

pub(crate) struct EditAuthority {
    inner: Mutex<EditState>,
    subscribers: Mutex<HashMap<u64, EditSubscriber>>,
    queue_capacity: usize,
    #[cfg(test)]
    test_starting_units_per_material: Option<u32>,
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

impl EditSubscription {
    /// Converts a bounded-queue gap into one clean resync boundary. The caller must suppress the
    /// commit it already popped when this returns `true`; every older queued commit is discarded so
    /// none can be delivered after the resync and become authoritative again.
    pub(crate) fn discard_stale_after_overflow(&mut self) -> bool {
        if !self.overflowed.swap(false, Ordering::AcqRel) {
            return false;
        }
        loop {
            while self.receiver.try_recv().is_ok() {}
            if !self.overflowed.swap(false, Ordering::AcqRel) {
                return true;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct AppliedEdit {
    pub(crate) commit: EditCommit,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LoadedPlayer {
    pub(crate) inventory: MaterialInventory,
    pub(crate) resume: PlayerResume,
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
        Self::from_connection(
            connection,
            world_id,
            source,
            queue_capacity,
            #[cfg(test)]
            None,
        )
    }

    pub(crate) fn in_memory(
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
    ) -> Result<Arc<Self>, EditAuthorityError> {
        let connection = Connection::open_in_memory().map_err(sql_error("open edit database"))?;
        Self::from_connection(
            connection,
            world_id,
            source,
            queue_capacity,
            #[cfg(test)]
            None,
        )
    }

    #[cfg(test)]
    fn in_memory_with_inventory(
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
        units_per_material: u32,
    ) -> Result<Arc<Self>, EditAuthorityError> {
        let connection = Connection::open_in_memory().map_err(sql_error("open edit database"))?;
        Self::from_connection(
            connection,
            world_id,
            source,
            queue_capacity,
            Some(units_per_material),
        )
    }

    #[cfg(test)]
    fn open_with_inventory(
        path: &Path,
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
        units_per_material: u32,
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
        Self::from_connection(
            connection,
            world_id,
            source,
            queue_capacity,
            Some(units_per_material),
        )
    }

    fn from_connection(
        mut connection: Connection,
        world_id: WorldId,
        source: &dyn WorldSourceEngine,
        queue_capacity: u16,
        #[cfg(test)] test_starting_units_per_material: Option<u32>,
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
            #[cfg(test)]
            test_starting_units_per_material,
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

    /// Loads or initializes one durable player without changing its active edit session. Presence
    /// admission must succeed before [`Self::begin_player_session`] rotates operation identity.
    pub(crate) fn load_player(
        &self,
        player_id: PlayerId,
        default_resume: PlayerResume,
    ) -> Result<LoadedPlayer, EditAuthorityError> {
        validate_resume(default_resume)?;
        let mut state = self.lock();
        if let Some(player) = load_durable_player(&state.connection, player_id)? {
            return Ok(LoadedPlayer {
                inventory: player.inventory,
                resume: player.resume,
            });
        }
        let transaction = state
            .connection
            .transaction()
            .map_err(sql_error("begin player initialization transaction"))?;
        transaction
            .execute(
                "INSERT INTO players(
                    player_id,current_edit_session,inventory_revision,resume_revision,
                    eye_x,eye_y,eye_z,look_yaw,look_pitch
                 ) VALUES(?1,NULL,1,?2,?3,?4,?5,?6,?7)",
                params![
                    player_id.as_bytes().as_slice(),
                    to_sql_i64(default_resume.revision, "player resume revision")?,
                    default_resume.eye_position_metres[0],
                    default_resume.eye_position_metres[1],
                    default_resume.eye_position_metres[2],
                    default_resume.look_yaw_radians,
                    default_resume.look_pitch_radians,
                ],
            )
            .map_err(sql_error("insert player"))?;
        #[cfg(test)]
        let inventory = self
            .test_starting_units_per_material
            .map_or(MaterialInventory::EMPTY, starting_inventory);
        #[cfg(not(test))]
        let inventory = MaterialInventory::EMPTY;
        persist_inventory(&transaction, player_id, inventory)?;
        transaction
            .commit()
            .map_err(sql_error("commit player initialization transaction"))?;
        Ok(LoadedPlayer {
            inventory,
            resume: default_resume,
        })
    }

    /// Rotates the namespace for newly admitted edit operations. Stored operations from earlier
    /// sessions remain queryable for exact retries, but cannot be extended with new IDs.
    pub(crate) fn begin_player_session(
        &self,
        player_id: PlayerId,
    ) -> Result<EditSessionId, EditAuthorityError> {
        let edit_session_id = new_edit_session_id();
        let state = self.lock();
        let updated = state
            .connection
            .execute(
                "UPDATE players SET current_edit_session=?1 WHERE player_id=?2",
                params![
                    edit_session_id.as_bytes().as_slice(),
                    player_id.as_bytes().as_slice()
                ],
            )
            .map_err(sql_error("begin player edit session"))?;
        if updated != 1 {
            return Err(EditAuthorityError(
                "cannot begin an edit session for an unloaded player".to_owned(),
            ));
        }
        Ok(edit_session_id)
    }

    /// Persists only monotonic server-accepted resume state. A late closing connection cannot
    /// overwrite a newer session's camera position.
    pub(crate) fn save_player_resume(
        &self,
        player_id: PlayerId,
        resume: PlayerResume,
    ) -> Result<(), EditAuthorityError> {
        validate_resume(resume)?;
        let state = self.lock();
        let updated = state
            .connection
            .execute(
                "UPDATE players SET
                    resume_revision=?1,eye_x=?2,eye_y=?3,eye_z=?4,look_yaw=?5,look_pitch=?6
                 WHERE player_id=?7 AND resume_revision<?1",
                params![
                    to_sql_i64(resume.revision, "player resume revision")?,
                    resume.eye_position_metres[0],
                    resume.eye_position_metres[1],
                    resume.eye_position_metres[2],
                    resume.look_yaw_radians,
                    resume.look_pitch_radians,
                    player_id.as_bytes().as_slice(),
                ],
            )
            .map_err(sql_error("save player resume"))?;
        if updated == 0 && load_durable_player(&state.connection, player_id)?.is_none() {
            return Err(EditAuthorityError(
                "cannot save resume for an unopened player".to_owned(),
            ));
        }
        Ok(())
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
            let mut delivery = commit.clone();
            if *connection_id != commit.editor_connection_id {
                delivery.editor_inventory = None;
            }
            match subscriber.sender.try_send(delivery) {
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
        editor_connection_id: u64,
        command: EditCommand,
    ) -> Result<AppliedEdit, EditAuthorityError> {
        let mut state = self.lock();
        if let Some(stored) = load_operation(
            &state.connection,
            player_id,
            command.edit_session_id,
            command.operation_id,
        )? {
            if stored.action != command.action {
                return Err(EditAuthorityError(
                    "edit operation id was reused with a different command".to_owned(),
                ));
            }
            let player = load_durable_player(&state.connection, player_id)?
                .ok_or_else(|| EditAuthorityError("edit player does not exist".to_owned()))?;
            return Ok(AppliedEdit {
                commit: stored.commit(command, editor_connection_id, Some(player.inventory)),
                changed: false,
            });
        }

        let player = load_durable_player(&state.connection, player_id)?
            .ok_or_else(|| EditAuthorityError("edit player does not exist".to_owned()))?;
        if player.current_edit_session != Some(command.edit_session_id) {
            return Err(EditAuthorityError(EDIT_SESSION_NOT_CURRENT.to_owned()));
        }

        let (mutations, inventory) =
            plan_action(source, &state.edits, player.inventory, command.action)?;
        let changed = !mutations.is_empty();
        let revision = if changed {
            state
                .revision
                .checked_add(1)
                .ok_or_else(|| EditAuthorityError("edit revision overflowed".to_owned()))?
        } else {
            state.revision
        };
        let inventory = if changed {
            MaterialInventory {
                revision: player.inventory.revision.checked_add(1).ok_or_else(|| {
                    EditAuthorityError("inventory revision overflowed".to_owned())
                })?,
                ..inventory
            }
        } else {
            player.inventory
        };

        let mut next_edits = state.edits.clone();
        for mutation in &mutations {
            next_edits.replace_durable_override(mutation.coord, Some(mutation.material));
        }
        let (affected_chunks, affected_surface_tiles) = if changed {
            affected_products(source, &state.edits, &next_edits, &mutations)?
        } else {
            (Vec::new(), Vec::new())
        };
        let commit = EditCommit {
            operation_id: command.operation_id,
            edit_session_id: command.edit_session_id,
            editor_connection_id,
            revision,
            mutations: mutations.clone(),
            affected_chunks: affected_chunks.clone(),
            affected_surface_tiles: affected_surface_tiles.clone(),
            editor_inventory: Some(inventory),
        };

        persist_action(
            &mut state.connection,
            player_id,
            command,
            revision,
            inventory,
            &mutations,
            &affected_chunks,
            &affected_surface_tiles,
        )?;
        if changed {
            state.edits = next_edits;
            state.revision = revision;
            for coord in &affected_chunks {
                state.chunk_revisions.insert(*coord, revision);
            }
            for coord in &affected_surface_tiles {
                state.surface_revisions.insert(*coord, revision);
            }
        }
        Ok(AppliedEdit { commit, changed })
    }
}

struct DurablePlayer {
    current_edit_session: Option<EditSessionId>,
    inventory: MaterialInventory,
    resume: PlayerResume,
}

struct StoredOperation {
    action: EditAction,
    revision: u64,
    mutations: Vec<VoxelMutation>,
    affected_chunks: Vec<ChunkCoord>,
    affected_surface_tiles: Vec<SurfaceTileCoord>,
}

impl StoredOperation {
    fn commit(
        self,
        command: EditCommand,
        editor_connection_id: u64,
        editor_inventory: Option<MaterialInventory>,
    ) -> EditCommit {
        EditCommit {
            operation_id: command.operation_id,
            edit_session_id: command.edit_session_id,
            editor_connection_id,
            revision: self.revision,
            mutations: self.mutations,
            affected_chunks: self.affected_chunks,
            affected_surface_tiles: self.affected_surface_tiles,
            editor_inventory,
        }
    }
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
                    world_id BLOB NOT NULL CHECK(length(world_id) = 16),
                    source_hash BLOB NOT NULL CHECK(length(source_hash) = 32),
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
                 CREATE TABLE players (
                    player_id BLOB PRIMARY KEY CHECK(length(player_id) = 16),
                    current_edit_session BLOB CHECK(
                        current_edit_session IS NULL OR length(current_edit_session) = 16
                    ),
                    inventory_revision INTEGER NOT NULL CHECK(inventory_revision >= 1),
                    resume_revision INTEGER NOT NULL CHECK(resume_revision >= 1),
                    eye_x REAL NOT NULL,
                    eye_y REAL NOT NULL,
                    eye_z REAL NOT NULL,
                    look_yaw REAL NOT NULL,
                    look_pitch REAL NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE player_inventory (
                    player_id BLOB NOT NULL,
                    material INTEGER NOT NULL,
                    units INTEGER NOT NULL CHECK(units >= 0),
                    PRIMARY KEY (player_id, material),
                    FOREIGN KEY (player_id) REFERENCES players(player_id) ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE TABLE edit_operations (
                    player_id BLOB NOT NULL,
                    edit_session_id BLOB NOT NULL CHECK(length(edit_session_id) = 16),
                    operation_id BLOB NOT NULL CHECK(length(operation_id) = 8),
                    action INTEGER NOT NULL,
                    x INTEGER NOT NULL,
                    y INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    argument INTEGER NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision >= 1),
                    inventory_revision INTEGER NOT NULL CHECK(inventory_revision >= 1),
                    PRIMARY KEY (player_id, edit_session_id, operation_id),
                    FOREIGN KEY (player_id) REFERENCES players(player_id) ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE TABLE edit_operation_mutations (
                    player_id BLOB NOT NULL,
                    edit_session_id BLOB NOT NULL,
                    operation_id BLOB NOT NULL,
                    ordinal INTEGER NOT NULL,
                    x INTEGER NOT NULL,
                    y INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    material INTEGER NOT NULL,
                    PRIMARY KEY (player_id, edit_session_id, operation_id, ordinal),
                    FOREIGN KEY (player_id, edit_session_id, operation_id)
                        REFERENCES edit_operations(player_id, edit_session_id, operation_id)
                        ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE TABLE edit_operation_chunks (
                    player_id BLOB NOT NULL,
                    edit_session_id BLOB NOT NULL,
                    operation_id BLOB NOT NULL,
                    ordinal INTEGER NOT NULL,
                    x INTEGER NOT NULL,
                    y INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    PRIMARY KEY (player_id, edit_session_id, operation_id, ordinal),
                    FOREIGN KEY (player_id, edit_session_id, operation_id)
                        REFERENCES edit_operations(player_id, edit_session_id, operation_id)
                        ON DELETE CASCADE
                 ) WITHOUT ROWID;
                 CREATE TABLE edit_operation_surfaces (
                    player_id BLOB NOT NULL,
                    edit_session_id BLOB NOT NULL,
                    operation_id BLOB NOT NULL,
                    ordinal INTEGER NOT NULL,
                    stride INTEGER NOT NULL,
                    x INTEGER NOT NULL,
                    z INTEGER NOT NULL,
                    PRIMARY KEY (player_id, edit_session_id, operation_id, ordinal),
                    FOREIGN KEY (player_id, edit_session_id, operation_id)
                        REFERENCES edit_operations(player_id, edit_session_id, operation_id)
                        ON DELETE CASCADE
                 ) WITHOUT ROWID;",
            )
            .map_err(sql_error("create edit schema"))?;
        transaction
            .pragma_update(None, "user_version", EDIT_DATABASE_SCHEMA_VERSION)
            .map_err(sql_error("write edit schema version"))?;
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
    if version != EDIT_DATABASE_SCHEMA_VERSION {
        return Err(EditAuthorityError(format!(
            "unsupported edit database schema {version}; expected {EDIT_DATABASE_SCHEMA_VERSION}; migrations are intentionally unsupported"
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
        let material = decode_material(material_id, "durable edit")?;
        edits.insert_override(coord, material);
        revisions.push((coord, row_revision as u64));
    }
    Ok((edits, revisions, (revision as u64).max(INITIAL_REVISION)))
}

fn load_durable_player(
    connection: &Connection,
    player_id: PlayerId,
) -> Result<Option<DurablePlayer>, EditAuthorityError> {
    let row = connection
        .query_row(
            "SELECT current_edit_session,inventory_revision,resume_revision,
                    eye_x,eye_y,eye_z,look_yaw,look_pitch
             FROM players WHERE player_id=?1",
            [player_id.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, f32>(3)?,
                    row.get::<_, f32>(4)?,
                    row.get::<_, f32>(5)?,
                    row.get::<_, f32>(6)?,
                    row.get::<_, f32>(7)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error("load player"))?;
    let Some((session, inventory_revision, resume_revision, x, y, z, yaw, pitch)) = row else {
        return Ok(None);
    };
    let current_edit_session = session
        .map(|session| {
            let session =
                EditSessionId::from_bytes(decode_fixed_bytes(&session, "player edit session")?);
            if session.is_nil() {
                return Err(EditAuthorityError(
                    "durable player edit session is nil".to_owned(),
                ));
            }
            Ok(session)
        })
        .transpose()?;
    let mut counts = [0; MATERIAL_INVENTORY_SLOTS];
    let mut statement = connection
        .prepare("SELECT material,units FROM player_inventory WHERE player_id=?1 ORDER BY material")
        .map_err(sql_error("prepare inventory load"))?;
    let rows = statement
        .query_map([player_id.as_bytes().as_slice()], |row| {
            Ok((row.get::<_, u16>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(sql_error("load inventory"))?;
    let mut slots = 0;
    for row in rows {
        let (material_id, units) = row.map_err(sql_error("decode inventory row"))?;
        let material = decode_material(material_id, "inventory")?;
        if units < 0 {
            return Err(EditAuthorityError(
                "durable inventory count is negative".to_owned(),
            ));
        }
        counts[usize::from(material.id())] = units as u64;
        slots += 1;
    }
    if slots != MATERIAL_INVENTORY_SLOTS || counts[usize::from(Material::Air.id())] != 0 {
        return Err(EditAuthorityError(
            "durable inventory is incomplete or contains Air".to_owned(),
        ));
    }
    let inventory = MaterialInventory {
        revision: positive_u64(inventory_revision, "inventory revision")?,
        counts,
    };
    let resume = PlayerResume {
        revision: positive_u64(resume_revision, "resume revision")?,
        eye_position_metres: [x, y, z],
        look_yaw_radians: yaw,
        look_pitch_radians: pitch,
    };
    validate_resume(resume)?;
    Ok(Some(DurablePlayer {
        current_edit_session,
        inventory,
        resume,
    }))
}

fn load_operation(
    connection: &Connection,
    player_id: PlayerId,
    edit_session_id: EditSessionId,
    operation_id: u64,
) -> Result<Option<StoredOperation>, EditAuthorityError> {
    let key = OperationKey::new(player_id, edit_session_id, operation_id);
    let base = connection
        .query_row(
            "SELECT action,x,y,z,argument,revision FROM edit_operations
             WHERE player_id=?1 AND edit_session_id=?2 AND operation_id=?3",
            params![key.player(), key.session(), key.operation()],
            |row| {
                Ok((
                    row.get::<_, u8>(0)?,
                    VoxelCoord::new(row.get(1)?, row.get(2)?, row.get(3)?),
                    row.get::<_, u16>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error("load edit operation"))?;
    let Some((action, coord, argument, revision)) = base else {
        return Ok(None);
    };
    let action = decode_action(action, coord, argument)?;
    let mutations = load_operation_mutations(connection, &key)?;
    let affected_chunks = load_operation_chunks(connection, &key)?;
    let affected_surface_tiles = load_operation_surfaces(connection, &key)?;
    Ok(Some(StoredOperation {
        action,
        revision: positive_u64(revision, "operation revision")?,
        mutations,
        affected_chunks,
        affected_surface_tiles,
    }))
}

fn load_operation_mutations(
    connection: &Connection,
    key: &OperationKey,
) -> Result<Vec<VoxelMutation>, EditAuthorityError> {
    let mut statement = connection
        .prepare(
            "SELECT x,y,z,material FROM edit_operation_mutations
             WHERE player_id=?1 AND edit_session_id=?2 AND operation_id=?3 ORDER BY ordinal",
        )
        .map_err(sql_error("prepare operation mutations"))?;
    let rows = statement
        .query_map(
            params![key.player(), key.session(), key.operation()],
            |row| {
                Ok((
                    VoxelCoord::new(row.get(0)?, row.get(1)?, row.get(2)?),
                    row.get::<_, u16>(3)?,
                ))
            },
        )
        .map_err(sql_error("load operation mutations"))?;
    rows.map(|row| {
        let (coord, material) = row.map_err(sql_error("decode operation mutation"))?;
        Ok(VoxelMutation {
            coord,
            material: decode_material(material, "operation mutation")?,
        })
    })
    .collect()
}

fn load_operation_chunks(
    connection: &Connection,
    key: &OperationKey,
) -> Result<Vec<ChunkCoord>, EditAuthorityError> {
    let mut statement = connection
        .prepare(
            "SELECT x,y,z FROM edit_operation_chunks
             WHERE player_id=?1 AND edit_session_id=?2 AND operation_id=?3 ORDER BY ordinal",
        )
        .map_err(sql_error("prepare operation chunks"))?;
    let rows = statement
        .query_map(
            params![key.player(), key.session(), key.operation()],
            |row| Ok(ChunkCoord::new(row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(sql_error("load operation chunks"))?;
    rows.map(|row| row.map_err(sql_error("decode operation chunk")))
        .collect()
}

fn load_operation_surfaces(
    connection: &Connection,
    key: &OperationKey,
) -> Result<Vec<SurfaceTileCoord>, EditAuthorityError> {
    let mut statement = connection
        .prepare(
            "SELECT stride,x,z FROM edit_operation_surfaces
             WHERE player_id=?1 AND edit_session_id=?2 AND operation_id=?3 ORDER BY ordinal",
        )
        .map_err(sql_error("prepare operation surfaces"))?;
    let rows = statement
        .query_map(
            params![key.player(), key.session(), key.operation()],
            |row| {
                Ok((
                    row.get::<_, i32>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                ))
            },
        )
        .map_err(sql_error("load operation surfaces"))?;
    rows.map(|row| {
        let (stride, x, z) = row.map_err(sql_error("decode operation surface"))?;
        let level = SurfaceLodLevel::from_stride_voxels(stride).ok_or_else(|| {
            EditAuthorityError(format!("unknown durable surface stride {stride}"))
        })?;
        Ok(SurfaceTileCoord::new(level, x, z))
    })
    .collect()
}

fn plan_action(
    source: &dyn WorldSourceEngine,
    edits: &EditMap,
    mut inventory: MaterialInventory,
    action: EditAction,
) -> Result<(Vec<VoxelMutation>, MaterialInventory), EditAuthorityError> {
    match action {
        EditAction::Dig { hit, face } => {
            let normal = face.normal();
            let (min_x, max_x) = dig_axis_bounds(hit.x, normal[0])?;
            let (min_y, max_y) = dig_axis_bounds(hit.y, normal[1])?;
            let (min_z, max_z) = dig_axis_bounds(hit.z, normal[2])?;
            let min = VoxelCoord::new(min_x, min_y, min_z);
            let max = VoxelCoord::new(max_x, max_y, max_z);
            let snapshot = source_voxel_block(source, min, DIG_SAMPLE_SHAPE)?;
            let mut mutations = Vec::with_capacity(DIG_MAX_MUTATIONS);
            for x in min.x..=max.x {
                for y in min.y..=max.y {
                    for z in min.z..=max.z {
                        let coord = VoxelCoord::new(x, y, z);
                        let generated = snapshot.sample(coord).ok_or_else(|| {
                            EditAuthorityError("source omitted a requested dig voxel".to_owned())
                        })?;
                        let material = edits.resolve_generated(coord, generated);
                        if material == Material::Air {
                            continue;
                        }
                        let slot = &mut inventory.counts[usize::from(material.id())];
                        *slot = slot.checked_add(1).ok_or_else(|| {
                            EditAuthorityError("material inventory overflowed".to_owned())
                        })?;
                        mutations.push(VoxelMutation {
                            coord,
                            material: Material::Air,
                        });
                    }
                }
            }
            Ok((mutations, inventory))
        }
        EditAction::Place { coord, material } => {
            if material == Material::Air {
                return Err(EditAuthorityError("Air cannot be placed".to_owned()));
            }
            let snapshot = source_voxel_block(source, coord, [1, 1, 1])?;
            let generated = snapshot.sample(coord).ok_or_else(|| {
                EditAuthorityError("source omitted the placement voxel".to_owned())
            })?;
            if edits.resolve_generated(coord, generated) != Material::Air {
                return Err(EditAuthorityError(
                    "placement target is occupied".to_owned(),
                ));
            }
            let slot = &mut inventory.counts[usize::from(material.id())];
            if *slot == 0 {
                return Err(EditAuthorityError(format!(
                    "no {} inventory remains",
                    material.id()
                )));
            }
            *slot -= 1;
            Ok((vec![VoxelMutation { coord, material }], inventory))
        }
    }
}

fn dig_axis_bounds(value: i32, outward_normal: i8) -> Result<(i32, i32), EditAuthorityError> {
    let error = || EditAuthorityError("dig volume exceeds world coordinates".to_owned());
    match outward_normal {
        -1 => Ok((
            value,
            value.checked_add(DIG_EDGE_VOXELS - 1).ok_or_else(error)?,
        )),
        0 => Ok((
            value.checked_sub(DIG_RADIUS_VOXELS).ok_or_else(error)?,
            value.checked_add(DIG_RADIUS_VOXELS).ok_or_else(error)?,
        )),
        1 => Ok((
            value.checked_sub(DIG_EDGE_VOXELS - 1).ok_or_else(error)?,
            value,
        )),
        _ => Err(EditAuthorityError("dig face normal is invalid".to_owned())),
    }
}

fn source_voxel_block(
    source: &dyn WorldSourceEngine,
    min: VoxelCoord,
    sample_shape: [u32; 3],
) -> Result<voxels_world::VoxelBlockSnapshot, EditAuthorityError> {
    let request = VoxelBlockRequest { min, sample_shape };
    let result = source
        .generate_batch(WorldProductBatch {
            priority: WorldProductPriority::CollisionCritical,
            requests: vec![WorldProductRequest::VoxelBlock(request)],
        })
        .map_err(|error| EditAuthorityError(format!("sample edit target: {error}")))?;
    if result.source_identity_hash != source.identity().identity_hash() || result.items.len() != 1 {
        return Err(EditAuthorityError(
            "world source returned a mismatched edit sample batch".to_owned(),
        ));
    }
    let item = result
        .items
        .into_iter()
        .next()
        .ok_or_else(|| EditAuthorityError("world source omitted an edit sample".to_owned()))?;
    match (item.request, item.result) {
        (WorldProductRequest::VoxelBlock(returned), Ok(WorldProduct::VoxelBlock(snapshot)))
            if returned == request
                && snapshot.request == request
                && snapshot.source_identity_hash == source.identity().identity_hash() =>
        {
            Ok(snapshot)
        }
        (_, Err(error)) => Err(EditAuthorityError(format!("sample edit target: {error}"))),
        _ => Err(EditAuthorityError(
            "world source returned a mismatched edit sample".to_owned(),
        )),
    }
}

fn affected_products(
    source: &dyn WorldSourceEngine,
    before: &EditMap,
    after: &EditMap,
    mutations: &[VoxelMutation],
) -> Result<(Vec<ChunkCoord>, Vec<SurfaceTileCoord>), EditAuthorityError> {
    let mut chunks = BTreeSet::new();
    let mut surfaces = BTreeSet::new();
    for mutation in mutations {
        chunks.extend(EditMap::affected_chunks(mutation.coord));
        for level in SurfaceLodLevel::ALL {
            surfaces.extend(source.surface_tiles_affected_by_voxel(before, level, mutation.coord));
            surfaces.extend(source.surface_tiles_affected_by_voxel(after, level, mutation.coord));
        }
    }
    if mutations.len() > MAX_EDIT_MUTATIONS
        || chunks.len() > MAX_EDIT_AFFECTED_CHUNKS
        || surfaces.len() > MAX_EDIT_AFFECTED_SURFACE_TILES
    {
        return Err(EditAuthorityError(format!(
            "edit outcome exceeds protocol bounds: {} mutations, {} chunks, {} surface tiles",
            mutations.len(),
            chunks.len(),
            surfaces.len(),
        )));
    }
    Ok((chunks.into_iter().collect(), surfaces.into_iter().collect()))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one atomic persistence boundary needs every changed world and player product"
)]
fn persist_action(
    connection: &mut Connection,
    player_id: PlayerId,
    command: EditCommand,
    revision: u64,
    inventory: MaterialInventory,
    mutations: &[VoxelMutation],
    affected_chunks: &[ChunkCoord],
    affected_surface_tiles: &[SurfaceTileCoord],
) -> Result<(), EditAuthorityError> {
    let transaction = connection
        .transaction()
        .map_err(sql_error("begin edit transaction"))?;
    if !mutations.is_empty() {
        for mutation in mutations {
            transaction
                .execute(
                    "INSERT INTO voxel_edits(x,y,z,material,revision) VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(x,y,z) DO UPDATE SET
                        material=excluded.material, revision=excluded.revision",
                    params![
                        mutation.coord.x,
                        mutation.coord.y,
                        mutation.coord.z,
                        mutation.material.id(),
                        to_sql_i64(revision, "edit revision")?,
                    ],
                )
                .map_err(sql_error("persist voxel mutation"))?;
        }
        transaction
            .execute(
                "UPDATE metadata SET revision=?1 WHERE singleton=1",
                [to_sql_i64(revision, "edit revision")?],
            )
            .map_err(sql_error("persist edit revision"))?;
        transaction
            .execute(
                "UPDATE players SET inventory_revision=?1 WHERE player_id=?2",
                params![
                    to_sql_i64(inventory.revision, "inventory revision")?,
                    player_id.as_bytes().as_slice()
                ],
            )
            .map_err(sql_error("persist inventory revision"))?;
        persist_inventory(&transaction, player_id, inventory)?;
    }
    persist_operation(
        &transaction,
        player_id,
        command,
        revision,
        inventory.revision,
        mutations,
        affected_chunks,
        affected_surface_tiles,
    )?;
    transaction
        .commit()
        .map_err(sql_error("commit edit action"))
}

#[allow(
    clippy::too_many_arguments,
    reason = "the normalized operation journal stores each independently validated outcome set"
)]
fn persist_operation(
    transaction: &Transaction<'_>,
    player_id: PlayerId,
    command: EditCommand,
    revision: u64,
    inventory_revision: u64,
    mutations: &[VoxelMutation],
    affected_chunks: &[ChunkCoord],
    affected_surface_tiles: &[SurfaceTileCoord],
) -> Result<(), EditAuthorityError> {
    let (action, coord, argument) = encode_action(command.action);
    transaction
        .execute(
            "INSERT INTO edit_operations(
                player_id,edit_session_id,operation_id,action,x,y,z,argument,revision,inventory_revision
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                player_id.as_bytes().as_slice(),
                command.edit_session_id.as_bytes().as_slice(),
                command.operation_id.to_le_bytes().as_slice(),
                action,
                coord.x,
                coord.y,
                coord.z,
                argument,
                to_sql_i64(revision, "operation revision")?,
                to_sql_i64(inventory_revision, "operation inventory revision")?,
            ],
        )
        .map_err(sql_error("persist edit operation"))?;
    for (ordinal, mutation) in mutations.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO edit_operation_mutations(
                    player_id,edit_session_id,operation_id,ordinal,x,y,z,material
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    player_id.as_bytes().as_slice(),
                    command.edit_session_id.as_bytes().as_slice(),
                    command.operation_id.to_le_bytes().as_slice(),
                    ordinal as i64,
                    mutation.coord.x,
                    mutation.coord.y,
                    mutation.coord.z,
                    mutation.material.id(),
                ],
            )
            .map_err(sql_error("persist operation mutation"))?;
    }
    for (ordinal, coord) in affected_chunks.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO edit_operation_chunks(
                    player_id,edit_session_id,operation_id,ordinal,x,y,z
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    player_id.as_bytes().as_slice(),
                    command.edit_session_id.as_bytes().as_slice(),
                    command.operation_id.to_le_bytes().as_slice(),
                    ordinal as i64,
                    coord.x,
                    coord.y,
                    coord.z,
                ],
            )
            .map_err(sql_error("persist operation chunk"))?;
    }
    for (ordinal, coord) in affected_surface_tiles.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO edit_operation_surfaces(
                    player_id,edit_session_id,operation_id,ordinal,stride,x,z
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    player_id.as_bytes().as_slice(),
                    command.edit_session_id.as_bytes().as_slice(),
                    command.operation_id.to_le_bytes().as_slice(),
                    ordinal as i64,
                    coord.level.stride_voxels(),
                    coord.x,
                    coord.z,
                ],
            )
            .map_err(sql_error("persist operation surface"))?;
    }
    Ok(())
}

fn persist_inventory(
    transaction: &Transaction<'_>,
    player_id: PlayerId,
    inventory: MaterialInventory,
) -> Result<(), EditAuthorityError> {
    for material in Material::ALL {
        let units = inventory.counts[usize::from(material.id())];
        transaction
            .execute(
                "INSERT INTO player_inventory(player_id,material,units) VALUES(?1,?2,?3)
                 ON CONFLICT(player_id,material) DO UPDATE SET units=excluded.units",
                params![
                    player_id.as_bytes().as_slice(),
                    material.id(),
                    to_sql_i64(units, "material inventory count")?,
                ],
            )
            .map_err(sql_error("persist material inventory"))?;
    }
    Ok(())
}

#[cfg(test)]
fn starting_inventory(units: u32) -> MaterialInventory {
    let mut counts = [u64::from(units); MATERIAL_INVENTORY_SLOTS];
    counts[usize::from(Material::Air.id())] = 0;
    MaterialInventory {
        revision: INITIAL_REVISION,
        counts,
    }
}

fn new_edit_session_id() -> EditSessionId {
    loop {
        let session = EditSessionId::from_bytes(*Uuid::new_v4().as_bytes());
        if !session.is_nil() {
            return session;
        }
    }
}

fn validate_resume(resume: PlayerResume) -> Result<(), EditAuthorityError> {
    let position_limit = (i32::MAX as f32 - 64.0) * VOXEL_SIZE_METRES;
    if resume.revision == 0
        || !resume
            .eye_position_metres
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= position_limit)
        || !resume.look_yaw_radians.is_finite()
        || !(-std::f32::consts::PI..=std::f32::consts::PI).contains(&resume.look_yaw_radians)
        || !resume.look_pitch_radians.is_finite()
        || !(-1.5..=1.5).contains(&resume.look_pitch_radians)
    {
        return Err(EditAuthorityError(
            "player resume state is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn encode_action(action: EditAction) -> (u8, VoxelCoord, u16) {
    match action {
        EditAction::Dig { hit, face } => (1, hit, u16::from(face.id())),
        EditAction::Place { coord, material } => (2, coord, material.id()),
    }
}

fn decode_action(
    action: u8,
    coord: VoxelCoord,
    argument: u16,
) -> Result<EditAction, EditAuthorityError> {
    match action {
        1 => {
            let id = u8::try_from(argument).map_err(|_| {
                EditAuthorityError(format!("unknown durable dig face id {argument}"))
            })?;
            let face = VoxelFace::from_id(id).ok_or_else(|| {
                EditAuthorityError(format!("unknown durable dig face id {argument}"))
            })?;
            Ok(EditAction::Dig { hit: coord, face })
        }
        2 => {
            let material = decode_material(argument, "operation")?;
            if material == Material::Air {
                return Err(EditAuthorityError(
                    "durable placement operation contains Air".to_owned(),
                ));
            }
            Ok(EditAction::Place { coord, material })
        }
        _ => Err(EditAuthorityError(
            "durable edit operation has an invalid action".to_owned(),
        )),
    }
}

struct OperationKey {
    player: [u8; 16],
    session: [u8; 16],
    operation: [u8; 8],
}

impl OperationKey {
    fn new(player_id: PlayerId, edit_session_id: EditSessionId, operation_id: u64) -> Self {
        Self {
            player: *player_id.as_bytes(),
            session: *edit_session_id.as_bytes(),
            operation: operation_id.to_le_bytes(),
        }
    }

    fn player(&self) -> &[u8] {
        &self.player
    }

    fn session(&self) -> &[u8] {
        &self.session
    }

    fn operation(&self) -> &[u8] {
        &self.operation
    }
}

fn decode_fixed_bytes<const N: usize>(
    bytes: &[u8],
    context: &'static str,
) -> Result<[u8; N], EditAuthorityError> {
    bytes
        .try_into()
        .map_err(|_| EditAuthorityError(format!("durable {context} has the wrong byte length")))
}

fn decode_material(id: u16, context: &'static str) -> Result<Material, EditAuthorityError> {
    Material::from_id(id)
        .ok_or_else(|| EditAuthorityError(format!("unknown durable {context} material id {id}")))
}

fn positive_u64(value: i64, context: &'static str) -> Result<u64, EditAuthorityError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| EditAuthorityError(format!("durable {context} is not positive")))
}

fn to_sql_i64(value: u64, context: &'static str) -> Result<i64, EditAuthorityError> {
    i64::try_from(value)
        .map_err(|_| EditAuthorityError(format!("{context} exceeded SQLite INTEGER")))
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

    fn resume(revision: u64) -> PlayerResume {
        PlayerResume {
            revision,
            eye_position_metres: [0.05, 21.2, 0.05],
            look_yaw_radians: 0.0,
            look_pitch_radians: 0.0,
        }
    }

    fn place(
        operation_id: u64,
        session: EditSessionId,
        coord: VoxelCoord,
        material: Material,
    ) -> EditCommand {
        EditCommand {
            operation_id,
            edit_session_id: session,
            action: EditAction::Place { coord, material },
        }
    }

    fn dig(operation_id: u64, session: EditSessionId, hit: VoxelCoord) -> EditCommand {
        dig_face(operation_id, session, hit, VoxelFace::PositiveY)
    }

    fn dig_face(
        operation_id: u64,
        session: EditSessionId,
        hit: VoxelCoord,
        face: VoxelFace,
    ) -> EditCommand {
        EditCommand {
            operation_id,
            edit_session_id: session,
            action: EditAction::Dig { hit, face },
        }
    }

    fn admit_player(
        authority: &EditAuthority,
        player_id: PlayerId,
        resume: PlayerResume,
    ) -> (LoadedPlayer, EditSessionId) {
        let player = authority.load_player(player_id, resume).unwrap();
        let session = authority.begin_player_session(player_id).unwrap();
        (player, session)
    }

    #[test]
    fn new_players_can_place_only_materials_earned_by_digging() {
        let source = ProceduralWorldSource::new(0xdecafbad);
        let authority = EditAuthority::in_memory(world_id(30), &source, 8).unwrap();
        let player = player_id(30);
        let (opened, session) = admit_player(&authority, player, resume(1));
        assert_eq!(opened.inventory, MaterialInventory::EMPTY);

        let placement = VoxelCoord::new(0, 300, 0);
        let rejected = authority
            .apply(
                &source,
                player,
                300,
                place(1, session, placement, Material::Stone),
            )
            .unwrap_err();
        assert!(rejected.to_string().contains("no 3 inventory"));

        let dug = authority
            .apply(
                &source,
                player,
                300,
                dig(2, session, VoxelCoord::new(0, -100, 0)),
            )
            .unwrap();
        let earned = dug.commit.editor_inventory.unwrap();
        let material = Material::ALL
            .into_iter()
            .find(|material| *material != Material::Air && earned.count(*material) > 0)
            .expect("solid dig must earn at least one material");
        let before_place = earned.count(material);

        let placed = authority
            .apply(&source, player, 300, place(3, session, placement, material))
            .unwrap();
        assert_eq!(
            placed.commit.editor_inventory.unwrap().count(material),
            before_place - 1
        );
        let never_earned = Material::ALL.into_iter().find(|candidate| {
            *candidate != Material::Air && *candidate != material && earned.count(*candidate) == 0
        });
        if let Some(never_earned) = never_earned {
            let error = authority
                .apply(
                    &source,
                    player,
                    300,
                    place(4, session, VoxelCoord::new(1, 300, 0), never_earned),
                )
                .unwrap_err();
            assert!(error.to_string().contains("inventory remains"));
        }
    }

    #[test]
    fn solid_and_mixed_digs_yield_exact_material_histograms() {
        let source = ProceduralWorldSource::new(42);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(1), &source, 8, 8).unwrap();
        let (opened, edit_session_id) = admit_player(&authority, player_id(2), resume(1));
        for material in Material::ALL {
            let expected = if material == Material::Air { 0 } else { 8 };
            assert_eq!(opened.inventory.count(material), expected);
        }

        let solid_hit = VoxelCoord::new(0, -100, 0);
        let solid_min = VoxelCoord::new(-2, -104, -2);
        let solid_source = source_voxel_block(&source, solid_min, DIG_SAMPLE_SHAPE).unwrap();
        let mut expected_histogram = [0_u64; MATERIAL_INVENTORY_SLOTS];
        for x in solid_min.x..solid_min.x + DIG_EDGE_VOXELS {
            for y in solid_min.y..solid_min.y + DIG_EDGE_VOXELS {
                for z in solid_min.z..solid_min.z + DIG_EDGE_VOXELS {
                    let material = solid_source.sample(VoxelCoord::new(x, y, z)).unwrap();
                    expected_histogram[usize::from(material.id())] += 1;
                }
            }
        }
        assert_eq!(expected_histogram[Material::Air.id() as usize], 0);

        let solid = authority
            .apply(
                &source,
                player_id(2),
                22,
                dig(1, edit_session_id, solid_hit),
            )
            .unwrap();
        assert!(solid.changed);
        assert_eq!(solid.commit.revision, INITIAL_REVISION + 1);
        assert_eq!(
            solid.commit.editor_inventory.unwrap().revision,
            opened.inventory.revision + 1
        );
        assert_eq!(solid.commit.mutations.len(), 125);
        assert!(solid.commit.mutations.len() <= 125);
        assert!(
            solid
                .commit
                .mutations
                .iter()
                .all(|mutation| mutation.material == Material::Air)
        );
        let solid_inventory = solid.commit.editor_inventory.unwrap();
        for material in Material::ALL {
            assert_eq!(
                solid_inventory
                    .count(material)
                    .saturating_sub(opened.inventory.count(material)),
                expected_histogram[usize::from(material.id())],
                "dig credited the wrong number of {} voxels",
                material.id()
            );
        }

        let sky = VoxelCoord::new(50, 300, 50);
        let materials = [Material::Stone, Material::Wood, Material::Water];
        for (index, material) in materials.into_iter().enumerate() {
            authority
                .apply(
                    &source,
                    player_id(2),
                    22,
                    place(
                        10 + index as u64,
                        edit_session_id,
                        VoxelCoord::new(sky.x + index as i32, sky.y, sky.z),
                        material,
                    ),
                )
                .unwrap();
        }
        let before = load_durable_player(&authority.lock().connection, player_id(2))
            .unwrap()
            .unwrap()
            .inventory;
        let mixed = authority
            .apply(
                &source,
                player_id(2),
                22,
                dig(20, edit_session_id, VoxelCoord::new(52, 300, 50)),
            )
            .unwrap();
        assert_eq!(mixed.commit.mutations.len(), 3);
        let after = mixed.commit.editor_inventory.unwrap();
        for material in materials {
            assert_eq!(after.count(material), before.count(material) + 1);
        }
    }

    #[test]
    fn placement_debits_inventory_and_rejects_air_occupied_and_out_of_stock() {
        let source = ProceduralWorldSource::new(7);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(2), &source, 8, 1).unwrap();
        let (_opened, edit_session_id) = admit_player(&authority, player_id(3), resume(1));
        let first = VoxelCoord::new(0, 300, 0);
        let applied = authority
            .apply(
                &source,
                player_id(3),
                30,
                place(1, edit_session_id, first, Material::Wood),
            )
            .unwrap();
        assert_eq!(
            applied
                .commit
                .editor_inventory
                .unwrap()
                .count(Material::Wood),
            0
        );
        assert!(
            authority
                .apply(
                    &source,
                    player_id(3),
                    30,
                    place(2, edit_session_id, first, Material::Stone),
                )
                .unwrap_err()
                .to_string()
                .contains("occupied")
        );
        assert!(
            authority
                .apply(
                    &source,
                    player_id(3),
                    30,
                    place(
                        3,
                        edit_session_id,
                        VoxelCoord::new(1, 300, 0),
                        Material::Wood,
                    ),
                )
                .unwrap_err()
                .to_string()
                .contains("no 8 inventory")
        );
        assert!(
            authority
                .apply(
                    &source,
                    player_id(3),
                    30,
                    place(
                        4,
                        edit_session_id,
                        VoxelCoord::new(2, 300, 0),
                        Material::Air,
                    ),
                )
                .unwrap_err()
                .to_string()
                .contains("Air cannot")
        );
    }

    #[test]
    fn dig_geometry_matrix_is_exact_sorted_unique_and_chunk_complete() {
        let source = ProceduralWorldSource::new(0xface);
        let inventory = starting_inventory(1);
        let boundary_locals = [0, 1, 2, 29, 30, 31];

        for axis in 0..3 {
            let chunk_bases = if axis == 1 { [-1_024, -992] } else { [-64, 0] };
            for chunk_base in chunk_bases {
                for local in boundary_locals {
                    let mut hit = [7, -1_000, -11];
                    hit[axis] = chunk_base + local;
                    if axis == 1 {
                        hit[0] = 7;
                        hit[2] = -11;
                    }
                    let hit = VoxelCoord::new(hit[0], hit[1], hit[2]);
                    let (mutations, after) = plan_action(
                        &source,
                        &EditMap::default(),
                        inventory,
                        EditAction::Dig {
                            hit,
                            face: VoxelFace::PositiveY,
                        },
                    )
                    .unwrap();
                    assert_eq!(mutations.len(), DIG_MAX_MUTATIONS, "hit {hit:?}");
                    assert!(
                        mutations
                            .windows(2)
                            .all(|pair| pair[0].coord < pair[1].coord),
                        "dig mutations must be strictly sorted and unique for {hit:?}"
                    );
                    let min = VoxelCoord::new(
                        hit.x - DIG_RADIUS_VOXELS,
                        hit.y - DIG_EDGE_VOXELS + 1,
                        hit.z - DIG_RADIUS_VOXELS,
                    );
                    let max = VoxelCoord::new(
                        min.x + DIG_EDGE_VOXELS - 1,
                        hit.y,
                        min.z + DIG_EDGE_VOXELS - 1,
                    );
                    let actual = mutations
                        .iter()
                        .map(|mutation| mutation.coord)
                        .collect::<BTreeSet<_>>();
                    let expected = (min.x..=max.x)
                        .flat_map(|x| {
                            (min.y..=max.y).flat_map(move |y| {
                                (min.z..=max.z).map(move |z| VoxelCoord::new(x, y, z))
                            })
                        })
                        .collect::<BTreeSet<_>>();
                    assert_eq!(actual, expected, "wrong dig volume for {hit:?}");

                    let affected = mutations
                        .iter()
                        .flat_map(|mutation| EditMap::affected_chunks(mutation.coord))
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>();
                    assert!(affected.len() <= MAX_EDIT_AFFECTED_CHUNKS);
                    assert!(affected.windows(2).all(|pair| pair[0] < pair[1]));
                    assert_eq!(
                        Material::ALL
                            .into_iter()
                            .map(|material| {
                                after
                                    .count(material)
                                    .saturating_sub(inventory.count(material))
                            })
                            .sum::<u64>(),
                        DIG_MAX_MUTATIONS as u64
                    );
                }
            }
        }
    }

    #[test]
    fn every_dig_face_cuts_five_layers_inward_from_the_clicked_surface() {
        let source = ProceduralWorldSource::new(0xf00d);
        let hit = VoxelCoord::new(7, -1_000, -11);
        for face in [
            VoxelFace::NegativeX,
            VoxelFace::PositiveX,
            VoxelFace::NegativeY,
            VoxelFace::PositiveY,
            VoxelFace::NegativeZ,
            VoxelFace::PositiveZ,
        ] {
            let (mutations, _) = plan_action(
                &source,
                &EditMap::default(),
                starting_inventory(1),
                EditAction::Dig { hit, face },
            )
            .unwrap();
            assert_eq!(mutations.len(), DIG_MAX_MUTATIONS);
            let normal = face.normal();
            let values = mutations
                .iter()
                .map(|mutation| mutation.coord.as_array())
                .collect::<Vec<_>>();
            let hit = hit.as_array();
            for axis in 0..3 {
                let min = values.iter().map(|coord| coord[axis]).min().unwrap();
                let max = values.iter().map(|coord| coord[axis]).max().unwrap();
                match normal[axis] {
                    -1 => assert_eq!((min, max), (hit[axis], hit[axis] + 4)),
                    0 => assert_eq!((min, max), (hit[axis] - 2, hit[axis] + 2)),
                    1 => assert_eq!((min, max), (hit[axis] - 4, hit[axis])),
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn dig_coordinate_extrema_fail_atomically_instead_of_wrapping() {
        let source = ProceduralWorldSource::new(0xbeef);
        for (hit, face) in [
            (VoxelCoord::new(i32::MIN + 1, -100, 0), VoxelFace::PositiveY),
            (VoxelCoord::new(i32::MAX - 1, -100, 0), VoxelFace::PositiveY),
            (VoxelCoord::new(0, i32::MIN + 3, 0), VoxelFace::PositiveY),
            (VoxelCoord::new(0, i32::MAX - 3, 0), VoxelFace::NegativeY),
            (VoxelCoord::new(0, -100, i32::MIN + 1), VoxelFace::PositiveY),
            (VoxelCoord::new(0, -100, i32::MAX - 1), VoxelFace::PositiveY),
        ] {
            let error = plan_action(
                &source,
                &EditMap::default(),
                starting_inventory(1),
                EditAction::Dig { hit, face },
            )
            .unwrap_err();
            assert!(error.to_string().contains("exceeds world coordinates"));
        }
    }

    #[test]
    fn place_occupied_dig_noop_and_replace_preserve_every_revision_invariant() {
        let source = ProceduralWorldSource::new(0x1234);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(20), &source, 8, 2).unwrap();
        let (opened, session) = admit_player(&authority, player_id(20), resume(1));
        let target = VoxelCoord::new(-65, 300, 31);

        let placed = authority
            .apply(
                &source,
                player_id(20),
                20,
                place(1, session, target, Material::Wood),
            )
            .unwrap();
        assert_eq!(placed.commit.revision, 2);
        assert_eq!(placed.commit.editor_inventory.unwrap().revision, 2);
        assert_eq!(placed.commit.mutations.len(), 1);

        let occupied = authority
            .apply(
                &source,
                player_id(20),
                20,
                place(2, session, target, Material::Stone),
            )
            .unwrap_err();
        assert!(occupied.to_string().contains("occupied"));
        assert_eq!(authority.revision(), 2);

        let dug = authority
            .apply(&source, player_id(20), 20, dig(3, session, target))
            .unwrap();
        assert!(dug.changed);
        assert_eq!(dug.commit.revision, 3);
        assert_eq!(dug.commit.mutations.len(), 1);
        let after_dig = dug.commit.editor_inventory.unwrap();
        assert_eq!(after_dig.revision, 3);
        assert_eq!(
            after_dig.count(Material::Wood),
            opened.inventory.count(Material::Wood)
        );

        let noop = authority
            .apply(&source, player_id(20), 20, dig(4, session, target))
            .unwrap();
        assert!(!noop.changed);
        assert_eq!(noop.commit.revision, 3);
        assert!(noop.commit.mutations.is_empty());
        assert!(noop.commit.affected_chunks.is_empty());
        assert!(noop.commit.affected_surface_tiles.is_empty());
        assert_eq!(noop.commit.editor_inventory, Some(after_dig));

        let replaced = authority
            .apply(
                &source,
                player_id(20),
                20,
                place(5, session, target, Material::Basalt),
            )
            .unwrap();
        assert_eq!(replaced.commit.revision, 4);
        assert_eq!(replaced.commit.editor_inventory.unwrap().revision, 4);
        assert_eq!(
            authority
                .snapshot_chunks(&[target.chunk()])
                .edits
                .override_at(target),
            Some(Material::Basalt)
        );
        assert_eq!(authority.revision(), 4);
    }

    #[test]
    fn overlapping_digs_never_credit_the_same_voxel_twice() {
        let source = ProceduralWorldSource::new(0x5678);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(21), &source, 8, 1).unwrap();
        let (opened, session) = admit_player(&authority, player_id(21), resume(1));
        let first = authority
            .apply(
                &source,
                player_id(21),
                21,
                dig(1, session, VoxelCoord::new(0, -100, 0)),
            )
            .unwrap();
        let second = authority
            .apply(
                &source,
                player_id(21),
                21,
                dig(2, session, VoxelCoord::new(3, -100, 0)),
            )
            .unwrap();
        assert_eq!(first.commit.mutations.len(), 125);
        assert_eq!(second.commit.mutations.len(), 75);
        let first_coords = first
            .commit
            .mutations
            .iter()
            .map(|mutation| mutation.coord)
            .collect::<BTreeSet<_>>();
        let second_coords = second
            .commit
            .mutations
            .iter()
            .map(|mutation| mutation.coord)
            .collect::<BTreeSet<_>>();
        assert!(first_coords.is_disjoint(&second_coords));
        let inventory = second.commit.editor_inventory.unwrap();
        let gained = Material::ALL
            .into_iter()
            .map(|material| inventory.count(material) - opened.inventory.count(material))
            .sum::<u64>();
        assert_eq!(gained, 200);
    }

    #[test]
    fn concurrent_players_contend_atomically_for_placement_and_overlapping_digs() {
        let source = Arc::new(ProceduralWorldSource::new(0x9abc));
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(22), source.as_ref(), 8, 2).unwrap();
        let players = [player_id(22), player_id(23)];
        let sessions = players.map(|player| admit_player(&authority, player, resume(1)).1);
        let target = VoxelCoord::new(10, 300, -10);

        let placements = players
            .into_iter()
            .zip(sessions)
            .enumerate()
            .map(|(index, (player, session))| {
                let source = Arc::clone(&source);
                let authority = Arc::clone(&authority);
                std::thread::spawn(move || {
                    authority.apply(
                        source.as_ref(),
                        player,
                        100 + index as u64,
                        place(1, session, target, Material::Wood),
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(placements.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            placements
                .iter()
                .filter(|result| result
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains("occupied")))
                .count(),
            1
        );
        let total_wood = players
            .into_iter()
            .map(|player| {
                load_durable_player(&authority.lock().connection, player)
                    .unwrap()
                    .unwrap()
                    .inventory
                    .count(Material::Wood)
            })
            .sum::<u64>();
        assert_eq!(
            total_wood, 3,
            "exactly one of four starting units was spent"
        );

        let dig_source = Arc::new(ProceduralWorldSource::new(0xdef0));
        let dig_authority =
            EditAuthority::in_memory_with_inventory(world_id(23), dig_source.as_ref(), 8, 1)
                .unwrap();
        let dig_players = [player_id(24), player_id(25)];
        let dig_sessions =
            dig_players.map(|player| admit_player(&dig_authority, player, resume(1)).1);
        let digs = dig_players
            .into_iter()
            .zip(dig_sessions)
            .enumerate()
            .map(|(index, (player, session))| {
                let source = Arc::clone(&dig_source);
                let authority = Arc::clone(&dig_authority);
                std::thread::spawn(move || {
                    authority
                        .apply(
                            source.as_ref(),
                            player,
                            200 + index as u64,
                            dig(1, session, VoxelCoord::new(index as i32 * 3, -100, 0)),
                        )
                        .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        let mut revisions = digs
            .iter()
            .map(|edit| edit.commit.revision)
            .collect::<Vec<_>>();
        revisions.sort_unstable();
        assert_eq!(revisions, vec![2, 3]);
        let sets = digs
            .iter()
            .map(|edit| {
                edit.commit
                    .mutations
                    .iter()
                    .map(|mutation| mutation.coord)
                    .collect::<BTreeSet<_>>()
            })
            .collect::<Vec<_>>();
        assert!(sets[0].is_disjoint(&sets[1]));
        assert_eq!(sets.iter().map(BTreeSet::len).sum::<usize>(), 200);
    }

    #[test]
    fn edit_sessions_allow_exact_old_retries_but_not_new_operations() {
        let source = ProceduralWorldSource::new(8);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(3), &source, 8, 4).unwrap();
        let (_first, first_session) = admit_player(&authority, player_id(4), resume(1));
        let command = place(
            1,
            first_session,
            VoxelCoord::new(0, 300, 0),
            Material::Stone,
        );
        let applied = authority.apply(&source, player_id(4), 40, command).unwrap();
        let loaded_again = authority.load_player(player_id(4), resume(1)).unwrap();
        assert_eq!(loaded_again.inventory.count(Material::Stone), 3);
        authority
            .apply(
                &source,
                player_id(4),
                40,
                place(
                    2,
                    first_session,
                    VoxelCoord::new(1, 300, 0),
                    Material::Stone,
                ),
            )
            .expect("a non-rotating load must not invalidate the admitted session");
        let second_session = authority.begin_player_session(player_id(4)).unwrap();
        assert_ne!(first_session, second_session);
        let retry = authority.apply(&source, player_id(4), 41, command).unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.commit.revision, applied.commit.revision);
        let lost_before_commit = place(
            3,
            first_session,
            VoxelCoord::new(2, 300, 0),
            Material::Stone,
        );
        assert_eq!(
            authority
                .apply(&source, player_id(4), 41, lost_before_commit)
                .unwrap_err()
                .to_string(),
            EDIT_SESSION_NOT_CURRENT
        );
        let reissued = lost_before_commit
            .reissue_after_session_rotation(4, second_session)
            .expect("rotated session must reissue an absent operation");
        assert!(
            authority
                .apply(&source, player_id(4), 41, reissued)
                .expect("reissued operation")
                .changed
        );
        assert!(
            authority
                .apply(
                    &source,
                    player_id(4),
                    41,
                    place(
                        1,
                        first_session,
                        VoxelCoord::new(2, 300, 0),
                        Material::Stone,
                    ),
                )
                .unwrap_err()
                .to_string()
                .contains("different command")
        );
    }

    #[test]
    fn sqlite_restart_preserves_world_inventory_and_latest_resume() {
        let source = ProceduralWorldSource::new(91);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "voxels-edit-authority-v5-{}-{unique}.sqlite3",
            std::process::id()
        ));
        let player = player_id(5);
        let coord = VoxelCoord::new(-57, 300, 89);
        {
            let authority =
                EditAuthority::open_with_inventory(&path, world_id(4), &source, 4, 2).unwrap();
            let (_opened, edit_session_id) = admit_player(&authority, player, resume(1));
            authority
                .apply(
                    &source,
                    player,
                    50,
                    place(1, edit_session_id, coord, Material::GlowCrystal),
                )
                .unwrap();
            let mut saved = resume(7);
            saved.eye_position_metres = [4.0, 22.0, -3.0];
            saved.look_yaw_radians = 0.8;
            authority.save_player_resume(player, saved).unwrap();
            let mut stale = resume(3);
            stale.eye_position_metres = [99.0, 99.0, 99.0];
            authority.save_player_resume(player, stale).unwrap();
        }
        {
            let reopened =
                EditAuthority::open_with_inventory(&path, world_id(4), &source, 4, 99).unwrap();
            let player = reopened.load_player(player, resume(1)).unwrap();
            assert_eq!(player.inventory.count(Material::GlowCrystal), 1);
            assert_eq!(player.resume.revision, 7);
            assert_eq!(player.resume.eye_position_metres, [4.0, 22.0, -3.0]);
            assert_eq!(
                reopened
                    .snapshot_chunks(&[coord.chunk()])
                    .edits
                    .override_at(coord),
                Some(Material::GlowCrystal)
            );
        }
        remove_sqlite_files(&path);
    }

    #[test]
    fn previous_schema_is_rejected_without_migration() {
        let source = ProceduralWorldSource::new(17);
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 4).unwrap();
        let error = EditAuthority::from_connection(connection, world_id(6), &source, 4, None)
            .err()
            .unwrap();
        assert!(error.to_string().contains("schema 4; expected 5"));
        assert!(
            error
                .to_string()
                .contains("migrations are intentionally unsupported")
        );
    }

    #[test]
    fn new_database_records_the_current_schema_version() {
        let source = ProceduralWorldSource::new(17);
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(
            &mut connection,
            world_id(6),
            source.identity().identity_hash(),
        )
        .unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, EDIT_DATABASE_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn observer_publication_omits_private_inventory() {
        let source = ProceduralWorldSource::new(23);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(7), &source, 4, 2).unwrap();
        let (_opened, edit_session_id) = admit_player(&authority, player_id(8), resume(1));
        let applied = authority
            .apply(
                &source,
                player_id(8),
                80,
                place(
                    1,
                    edit_session_id,
                    VoxelCoord::new(0, 300, 0),
                    Material::Stone,
                ),
            )
            .unwrap();
        let mut editor = authority.subscribe(80);
        let mut observer = authority.subscribe(81);
        authority.publish(&applied.commit, &BTreeSet::from([80, 81]));
        assert!(
            editor
                .receiver
                .recv()
                .await
                .unwrap()
                .editor_inventory
                .is_some()
        );
        assert!(
            observer
                .receiver
                .recv()
                .await
                .unwrap()
                .editor_inventory
                .is_none()
        );
    }

    #[tokio::test]
    async fn overflow_discards_every_stale_commit_before_the_resync_boundary() {
        let source = ProceduralWorldSource::new(24);
        let authority =
            EditAuthority::in_memory_with_inventory(world_id(8), &source, 1, 3).unwrap();
        let (_opened, session) = admit_player(&authority, player_id(9), resume(1));
        let mut subscription = authority.subscribe(90);

        let first = authority
            .apply(
                &source,
                player_id(9),
                90,
                place(1, session, VoxelCoord::new(0, 300, 0), Material::Stone),
            )
            .unwrap();
        let second = authority
            .apply(
                &source,
                player_id(9),
                90,
                place(2, session, VoxelCoord::new(1, 300, 0), Material::Stone),
            )
            .unwrap();
        authority.publish(&first.commit, &BTreeSet::from([90]));
        authority.publish(&second.commit, &BTreeSet::from([90]));

        let popped_stale = subscription.receiver.recv().await.unwrap();
        assert_eq!(popped_stale.revision, first.commit.revision);
        assert!(subscription.discard_stale_after_overflow());
        assert!(matches!(
            subscription.receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let third = authority
            .apply(
                &source,
                player_id(9),
                90,
                place(3, session, VoxelCoord::new(2, 300, 0), Material::Stone),
            )
            .unwrap();
        authority.publish(&third.commit, &BTreeSet::from([90]));
        assert_eq!(
            subscription.receiver.recv().await.unwrap().revision,
            third.commit.revision,
            "new commits after the resync boundary must still flow"
        );
        assert!(!subscription.discard_stale_after_overflow());
    }

    fn remove_sqlite_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    }
}
