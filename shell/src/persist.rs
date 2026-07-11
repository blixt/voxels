//! SQLite-on-OPFS persistence. All database and filesystem access stays inside the Rust worker.

use rusqlite::{Connection, OptionalExtension, params};
use sqlite_wasm_vfs::sahpool::{OpfsSAHPoolCfg, install as install_opfs_sahpool};
use voxels_core::CameraState;
use wasm_bindgen::JsValue;

const DATABASE_NAME: &str = "voxels.db";
const SCHEMA_VERSION: i64 = 2;

pub struct Store {
    connection: Connection,
}

impl Store {
    pub async fn open(world_seed: u64, generator_version: u32) -> Result<Self, JsValue> {
        install_opfs_sahpool::<sqlite_wasm_rs::WasmOsCallback>(&OpfsSAHPoolCfg::default(), true)
            .await
            .map_err(|error| js_error("install OPFS SQLite VFS", error))?;
        let connection = Connection::open(DATABASE_NAME)
            .map_err(|error| js_error("open SQLite database", error))?;
        connection
            .execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")
            .map_err(|error| js_error("configure SQLite", error))?;
        migrate(&connection)?;
        ensure_world(&connection, world_seed, generator_version)?;
        Ok(Self { connection })
    }

    pub fn load_camera(&self) -> Result<Option<CameraState>, JsValue> {
        self.connection
            .query_row(
                "SELECT x, y, z, yaw, pitch FROM camera WHERE id = 0",
                [],
                |row| {
                    Ok(CameraState::from_persisted(
                        glam::Vec3::new(row.get(0)?, row.get(1)?, row.get(2)?),
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| js_error("load camera", error))
    }

    pub fn save_camera(&self, camera: &CameraState) -> Result<(), JsValue> {
        self.connection
            .execute(
                "INSERT INTO camera (id, x, y, z, yaw, pitch) VALUES (0, ?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(id) DO UPDATE SET x=excluded.x, y=excluded.y, z=excluded.z, \
                 yaw=excluded.yaw, pitch=excluded.pitch",
                params![
                    camera.position.x,
                    camera.position.y,
                    camera.position.z,
                    camera.yaw,
                    camera.pitch
                ],
            )
            .map(|_| ())
            .map_err(|error| js_error("save camera", error))
    }
}

fn migrate(connection: &Connection) -> Result<(), JsValue> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| js_error("read schema version", error))?;
    if version > SCHEMA_VERSION {
        return Err(JsValue::from_str(&format!(
            "local database schema {version} is newer than this build ({SCHEMA_VERSION})"
        )));
    }
    if version == 0 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE worlds (
                   id INTEGER PRIMARY KEY CHECK (id = 0),
                   seed BLOB NOT NULL CHECK (length(seed) = 8),
                   generator_version INTEGER NOT NULL,
                   created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE camera (
                   id INTEGER PRIMARY KEY CHECK (id = 0),
                   x REAL NOT NULL,
                   y REAL NOT NULL,
                   z REAL NOT NULL,
                   yaw REAL NOT NULL,
                   pitch REAL NOT NULL
                 );
                 CREATE TABLE chunks (
                   world_id INTEGER NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
                   x INTEGER NOT NULL,
                   y INTEGER NOT NULL,
                   z INTEGER NOT NULL,
                   revision INTEGER NOT NULL,
                   codec_version INTEGER NOT NULL,
                   payload BLOB NOT NULL,
                   content_hash BLOB NOT NULL CHECK (length(content_hash) = 32),
                   updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                   PRIMARY KEY (world_id, x, y, z)
                 ) WITHOUT ROWID;
                 PRAGMA user_version=2;
                 COMMIT;",
            )
            .map_err(|error| js_error("apply schema migration 1", error))?;
    }
    if version == 1 {
        // Schema 1 stored positions in whole-voxel units. Schema 2 uses SI metres so that the
        // 10 cm voxel resolution is explicit throughout simulation, rendering and persistence.
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 UPDATE camera SET x=x*0.1, y=y*0.1, z=z*0.1;
                 PRAGMA user_version=2;
                 COMMIT;",
            )
            .map_err(|error| js_error("apply schema migration 2", error))?;
    }
    Ok(())
}

fn ensure_world(
    connection: &Connection,
    world_seed: u64,
    generator_version: u32,
) -> Result<(), JsValue> {
    let stored: Option<(Vec<u8>, u32)> = connection
        .query_row(
            "SELECT seed, generator_version FROM worlds WHERE id = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| js_error("load world identity", error))?;
    if let Some((seed, version)) = stored {
        if seed.as_slice() != world_seed.to_le_bytes() || version != generator_version {
            return Err(JsValue::from_str(
                "saved world identity does not match this generator build",
            ));
        }
    } else {
        connection
            .execute(
                "INSERT INTO worlds (id, seed, generator_version) VALUES (0, ?1, ?2)",
                params![world_seed.to_le_bytes().as_slice(), generator_version],
            )
            .map_err(|error| js_error("create world identity", error))?;
    }
    Ok(())
}

fn js_error(context: &str, error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("{context}: {error}"))
}
