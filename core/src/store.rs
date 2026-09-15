//! SQLite persistence for chores and room reference states.
//!
//! Chores are persistent entities: they survive app restarts and can transition
//! through a lifecycle. Reference images are stored on disk; their metadata
//! lives here.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::domain::{
    now_unix, BoundingBox, ChoreEntity, ChoreStatus, FingerprintKind, FingerprintRecord, Landmark,
    ReferenceState,
};

/// Errors produced by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A database error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// A filesystem error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A stored value could not be decoded.
    #[error("decode error: {0}")]
    Decode(String),
}

/// Async SQLite store for chores and room references.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

/// DDL for the chores table.
const DDL_CHORES: &str = "CREATE TABLE IF NOT EXISTS chores (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    target TEXT NOT NULL,
    action TEXT NOT NULL,
    estimated_seconds INTEGER NOT NULL,
    status INTEGER NOT NULL,
    ymin INTEGER, xmin INTEGER, ymax INTEGER, xmax INTEGER,
    confidence REAL NOT NULL,
    subtasks TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    how_to TEXT NOT NULL DEFAULT ''
)";

/// DDL for the room references table.
const DDL_REFERENCES: &str = "CREATE TABLE IF NOT EXISTS room_references (
    room_id TEXT PRIMARY KEY,
    image_id TEXT NOT NULL,
    description TEXT NOT NULL,
    captured_at_unix INTEGER NOT NULL
)";

/// DDL for the room landmarks table.
const DDL_LANDMARKS: &str = "CREATE TABLE IF NOT EXISTS room_landmarks (
    room_id TEXT NOT NULL,
    label TEXT NOT NULL,
    ymin INTEGER, xmin INTEGER, ymax INTEGER, xmax INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (room_id, label)
)";

/// DDL for the room fingerprints table.
const DDL_FINGERPRINTS: &str = "CREATE TABLE IF NOT EXISTS room_fingerprints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    room_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    grid_hash BLOB NOT NULL,
    captured_at_unix INTEGER NOT NULL
)";

/// DDL for the expected objects table.
const DDL_EXPECTED: &str = "CREATE TABLE IF NOT EXISTS expected_objects (
    room_id TEXT NOT NULL,
    object_label TEXT NOT NULL,
    PRIMARY KEY (room_id, object_label)
)";

impl Store {
    /// Open (creating if needed) the database under `data_dir`.
    pub async fn connect(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = data_dir.as_ref();
        tokio::fs::create_dir_all(dir).await?;
        let db_path = dir.join("lares.db");
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Create tables if they do not exist.
    async fn migrate(&self) -> Result<(), StoreError> {
        self.create_table(DDL_CHORES).await?;
        self.create_table(DDL_REFERENCES).await?;
        self.create_table(DDL_LANDMARKS).await?;
        self.create_table(DDL_FINGERPRINTS).await?;
        self.create_table(DDL_EXPECTED).await?;
        self.ensure_how_to_column().await?;
        Ok(())
    }

    /// Add the `how_to` column to pre-existing chore tables.
    async fn ensure_how_to_column(&self) -> Result<(), StoreError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM pragma_table_info('chores') WHERE name = 'how_to'",
        )
        .fetch_one(&self.pool)
        .await?;
        let count: i64 = row.try_get("n")?;
        if count == 0 {
            sqlx::query("ALTER TABLE chores ADD COLUMN how_to TEXT NOT NULL DEFAULT ''")
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Execute a DDL statement against the store.
    async fn create_table(&self, sql: &'static str) -> Result<(), StoreError> {
        sqlx::query(sql).execute(&self.pool).await?;
        Ok(())
    }

    /// Insert or replace chores within a single transaction.
    pub async fn upsert_chores(&self, chores: &[ChoreEntity]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for chore in chores {
            sqlx::query(
                "INSERT INTO chores
                    (id, room_id, target, action, estimated_seconds, status,
                     ymin, xmin, ymax, xmax, confidence, subtasks, updated_at, how_to)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    room_id=excluded.room_id, target=excluded.target,
                    action=excluded.action, estimated_seconds=excluded.estimated_seconds,
                    status=excluded.status, ymin=excluded.ymin, xmin=excluded.xmin,
                    ymax=excluded.ymax, xmax=excluded.xmax, confidence=excluded.confidence,
                    subtasks=excluded.subtasks, updated_at=excluded.updated_at,
                    how_to=excluded.how_to",
            )
            .bind(&chore.id)
            .bind(&chore.room_id)
            .bind(&chore.target)
            .bind(&chore.action)
            .bind(chore.estimated_seconds as i64)
            .bind(chore.status as i64)
            .bind(chore.r#box.as_ref().map(|b| b.ymin as i64))
            .bind(chore.r#box.as_ref().map(|b| b.xmin as i64))
            .bind(chore.r#box.as_ref().map(|b| b.ymax as i64))
            .bind(chore.r#box.as_ref().map(|b| b.xmax as i64))
            .bind(chore.confidence as f64)
            .bind(serde_json::to_string(&chore.subtasks).unwrap_or_default())
            .bind(now_unix())
            .bind(serde_json::to_string(&chore.how_to).unwrap_or_default())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// List chores, optionally filtered by room, ordered by confidence.
    pub async fn list_chores(
        &self,
        room_id: Option<&str>,
    ) -> Result<Vec<ChoreEntity>, StoreError> {
        let rows = match room_id {
            Some(room) => {
                sqlx::query(
                    "SELECT * FROM chores WHERE room_id = ? ORDER BY confidence DESC",
                )
                .bind(room)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query("SELECT * FROM chores ORDER BY confidence DESC")
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        rows.iter().map(row_to_chore).collect()
    }

    /// Transition a chore's status; returns the updated chore or `None`.
    pub async fn set_status(
        &self,
        id: &str,
        status: ChoreStatus,
    ) -> Result<Option<ChoreEntity>, StoreError> {
        let updated = sqlx::query("UPDATE chores SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status as i64)
            .bind(now_unix())
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if updated == 0 {
            return Ok(None);
        }
        let row = sqlx::query("SELECT * FROM chores WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(Some(row_to_chore(&row)?))
    }

    /// Insert or replace the agreed reference state for a room.
    pub async fn save_reference(&self, reference: &ReferenceState) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO room_references (room_id, image_id, description, captured_at_unix)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(room_id) DO UPDATE SET
                image_id=excluded.image_id,
                description=excluded.description,
                captured_at_unix=excluded.captured_at_unix",
        )
        .bind(&reference.room_id)
        .bind(&reference.image_id)
        .bind(&reference.description)
        .bind(reference.captured_at_unix)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch the agreed reference state for a room, if any.
    pub async fn get_reference(
        &self,
        room_id: &str,
    ) -> Result<Option<ReferenceState>, StoreError> {
        let row = sqlx::query(
            "SELECT room_id, image_id, description, captured_at_unix
             FROM room_references WHERE room_id = ?",
        )
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_reference).transpose()
    }

    /// Replace a room's landmark set within a single transaction.
    pub async fn upsert_landmarks(
        &self,
        room_id: &str,
        landmarks: &[Landmark],
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM room_landmarks WHERE room_id = ?")
            .bind(room_id)
            .execute(&mut *tx)
            .await?;
        let mut seen = std::collections::HashSet::new();
        for landmark in landmarks {
            if !seen.insert(landmark.label.clone()) {
                continue;
            }
            sqlx::query(
                "INSERT INTO room_landmarks
                    (room_id, label, ymin, xmin, ymax, xmax, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(room_id)
            .bind(&landmark.label)
            .bind(landmark.r#box.as_ref().map(|b| b.ymin as i64))
            .bind(landmark.r#box.as_ref().map(|b| b.xmin as i64))
            .bind(landmark.r#box.as_ref().map(|b| b.ymax as i64))
            .bind(landmark.r#box.as_ref().map(|b| b.xmax as i64))
            .bind(now_unix())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Fetch a room's current landmark set.
    pub async fn get_landmarks(&self, room_id: &str) -> Result<Vec<Landmark>, StoreError> {
        let rows = sqlx::query(
            "SELECT label, ymin, xmin, ymax, xmax FROM room_landmarks
             WHERE room_id = ? ORDER BY label",
        )
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_landmark).collect()
    }

    /// Store a scene fingerprint, capping HISTORY kind to the newest 20.
    pub async fn save_fingerprint(
        &self,
        room_id: &str,
        kind: FingerprintKind,
        grid_hash: &[u8],
    ) -> Result<(), StoreError> {
        let kind_str = kind.as_str();
        sqlx::query(
            "INSERT INTO room_fingerprints (room_id, kind, grid_hash, captured_at_unix)
             VALUES (?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(kind_str)
        .bind(grid_hash)
        .bind(now_unix())
        .execute(&self.pool)
        .await?;
        if kind == FingerprintKind::History {
            sqlx::query(
                "DELETE FROM room_fingerprints WHERE room_id = ? AND kind = ?
                 AND id NOT IN (
                    SELECT id FROM room_fingerprints WHERE room_id = ? AND kind = ?
                    ORDER BY captured_at_unix DESC, id DESC LIMIT 20
                 )",
            )
            .bind(room_id)
            .bind(kind_str)
            .bind(room_id)
            .bind(kind_str)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Fetch stored fingerprints for a room and kind, newest first.
    pub async fn get_fingerprints(
        &self,
        room_id: &str,
        kind: FingerprintKind,
    ) -> Result<Vec<FingerprintRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT room_id, kind, grid_hash, captured_at_unix FROM room_fingerprints
             WHERE room_id = ? AND kind = ? ORDER BY captured_at_unix DESC",
        )
        .bind(room_id)
        .bind(kind.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_fingerprint).collect()
    }

    /// Mark an object label as expected in a room.
    pub async fn add_expected(&self, room_id: &str, label: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR IGNORE INTO expected_objects (room_id, object_label) VALUES (?, ?)",
        )
        .bind(room_id)
        .bind(label)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove an expected object label from a room.
    pub async fn remove_expected(&self, room_id: &str, label: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM expected_objects WHERE room_id = ? AND object_label = ?")
            .bind(room_id)
            .bind(label)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List all expected object labels for a room.
    pub async fn get_expected(&self, room_id: &str) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query(
            "SELECT object_label FROM expected_objects WHERE room_id = ? ORDER BY object_label",
        )
        .bind(room_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| r.try_get("object_label"))
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::Database)
    }
}

/// Convert a chores table row into a contract entity.
fn row_to_chore(row: &sqlx::sqlite::SqliteRow) -> Result<ChoreEntity, StoreError> {
    let subtasks_raw: String = row.try_get("subtasks")?;
    let subtasks: Vec<String> = serde_json::from_str(&subtasks_raw)
        .map_err(|e| StoreError::Decode(format!("subtasks: {e}")))?;
    let how_to_raw: String = row.try_get("how_to").unwrap_or_default();
    let how_to: Vec<String> = serde_json::from_str(&how_to_raw)
        .map_err(|e| StoreError::Decode(format!("how_to: {e}")))?;
    let ymin = row.try_get::<Option<i64>, _>("ymin")?;
    let xmin = row.try_get::<Option<i64>, _>("xmin")?;
    let ymax = row.try_get::<Option<i64>, _>("ymax")?;
    let xmax = row.try_get::<Option<i64>, _>("xmax")?;
    let box_ = match (ymin, xmin, ymax, xmax) {
        (Some(a), Some(b), Some(c), Some(d)) => Some(BoundingBox {
            ymin: a as i32,
            xmin: b as i32,
            ymax: c as i32,
            xmax: d as i32,
        }),
        _ => None,
    };
    Ok(ChoreEntity {
        id: row.try_get("id")?,
        room_id: row.try_get("room_id")?,
        target: row.try_get("target")?,
        action: row.try_get("action")?,
        estimated_seconds: row.try_get::<i64, _>("estimated_seconds")? as u32,
        status: row.try_get::<i64, _>("status")? as i32,
        r#box: box_,
        confidence: row.try_get("confidence")?,
        subtasks,
        how_to,
        last_seen_unix: Some(row.try_get::<i64, _>("updated_at")?),
        ..Default::default()
    })
}

/// Convert a room_references table row into a contract entity.
fn row_to_reference(row: sqlx::sqlite::SqliteRow) -> Result<ReferenceState, StoreError> {
    Ok(ReferenceState {
        room_id: row.try_get("room_id")?,
        image_id: row.try_get("image_id")?,
        description: row.try_get("description")?,
        captured_at_unix: row.try_get("captured_at_unix")?,
    })
}

/// Convert a room_landmarks table row into a contract entity.
fn row_to_landmark(row: &sqlx::sqlite::SqliteRow) -> Result<Landmark, StoreError> {
    let ymin = row.try_get::<Option<i64>, _>("ymin")?;
    let xmin = row.try_get::<Option<i64>, _>("xmin")?;
    let ymax = row.try_get::<Option<i64>, _>("ymax")?;
    let xmax = row.try_get::<Option<i64>, _>("xmax")?;
    let r#box = match (ymin, xmin, ymax, xmax) {
        (Some(a), Some(b), Some(c), Some(d)) => Some(BoundingBox {
            ymin: a as i32,
            xmin: b as i32,
            ymax: c as i32,
            xmax: d as i32,
        }),
        _ => None,
    };
    Ok(Landmark {
        label: row.try_get("label")?,
        r#box,
    })
}

/// Convert a room_fingerprints table row into a contract entity.
fn row_to_fingerprint(row: &sqlx::sqlite::SqliteRow) -> Result<FingerprintRecord, StoreError> {
    let kind_str: String = row.try_get("kind")?;
    Ok(FingerprintRecord {
        room_id: row.try_get("room_id")?,
        kind: FingerprintKind::parse(&kind_str) as i32,
        grid_hash: row.try_get("grid_hash")?,
        captured_at_unix: row.try_get("captured_at_unix")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a store on a fresh temporary directory.
    async fn temp_store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lares-test-{}", uuid::Uuid::new_v4()));
        let store = Store::connect(&dir).await.unwrap();
        (store, dir)
    }

    /// A sample chore for persistence tests.
    fn sample_chore() -> ChoreEntity {
        ChoreEntity {
            id: "c1".to_string(),
            room_id: "kitchen".to_string(),
            target: "mugs".to_string(),
            action: "load dishwasher".to_string(),
            estimated_seconds: 60,
            status: ChoreStatus::Discovered as i32,
            r#box: Some(BoundingBox { ymin: 1, xmin: 2, ymax: 3, xmax: 4 }),
            confidence: 0.9,
            subtasks: vec!["load".to_string()],
            how_to: vec![
                "Pick up the mug".to_string(),
                "Open the dishwasher".to_string(),
                "Place the mug on the rack".to_string(),
            ],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn round_trips_chores_and_status() {
        let (store, dir) = temp_store().await;
        let chore = sample_chore();
        store.upsert_chores(std::slice::from_ref(&chore)).await.unwrap();
        let listed = store.list_chores(Some("kitchen")).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "c1");
        assert_eq!(listed[0].subtasks, vec!["load".to_string()]);
        assert_eq!(listed[0].how_to.len(), 3);
        assert!(listed[0].how_to[0].contains("mug"));
        assert_eq!(listed[0].status, ChoreStatus::Discovered as i32);

        let updated = store.set_status("c1", ChoreStatus::Done).await.unwrap();
        assert!(updated.is_some());
        assert_eq!(updated.unwrap().status, ChoreStatus::Done as i32);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_filters_by_room() {
        let (store, dir) = temp_store().await;
        let mut other = sample_chore();
        other.id = "c2".to_string();
        other.room_id = "bedroom".to_string();
        store.upsert_chores(&[sample_chore(), other]).await.unwrap();
        let kitchen = store.list_chores(Some("kitchen")).await.unwrap();
        assert_eq!(kitchen.len(), 1);
        assert_eq!(kitchen[0].room_id, "kitchen");
        let all = store.list_chores(None).await.unwrap();
        assert_eq!(all.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reference_round_trips() {
        let (store, dir) = temp_store().await;
        let reference = ReferenceState {
            room_id: "kitchen".to_string(),
            image_id: "img1".to_string(),
            description: "counters clear, dishes empty".to_string(),
            captured_at_unix: 1_700_000_000,
        };
        store.save_reference(&reference).await.unwrap();
        let loaded = store.get_reference("kitchen").await.unwrap();
        assert_eq!(loaded.unwrap().description, "counters clear, dishes empty");
        assert!(store.get_reference("bathroom").await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A sample landmark for persistence tests.
    fn sample_landmark(label: &str) -> Landmark {
        Landmark {
            label: label.to_string(),
            r#box: Some(BoundingBox { ymin: 100, xmin: 200, ymax: 400, xmax: 600 }),
        }
    }

    #[tokio::test]
    async fn landmarks_round_trip_and_replace() {
        let (store, dir) = temp_store().await;
        store.upsert_landmarks("kitchen", &[sample_landmark("sink"), sample_landmark("hamper")]).await.unwrap();
        let loaded = store.get_landmarks("kitchen").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].label, "hamper");
        store.upsert_landmarks("kitchen", &[sample_landmark("sink")]).await.unwrap();
        let reloaded = store.get_landmarks("kitchen").await.unwrap();
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].label, "sink");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn fingerprints_store_and_retrieve_by_kind() {
        let (store, dir) = temp_store().await;
        store.save_fingerprint("kitchen", FingerprintKind::Latest, &[1, 2, 3]).await.unwrap();
        store.save_fingerprint("kitchen", FingerprintKind::Clean, &[4, 5, 6]).await.unwrap();
        let latest = store.get_fingerprints("kitchen", FingerprintKind::Latest).await.unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].grid_hash, vec![1, 2, 3]);
        let clean = store.get_fingerprints("kitchen", FingerprintKind::Clean).await.unwrap();
        assert_eq!(clean.len(), 1);
        let all_history = store.get_fingerprints("kitchen", FingerprintKind::History).await.unwrap();
        assert!(all_history.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}