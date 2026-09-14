//! SQLite persistence for chores and room reference states.
//!
//! Chores are persistent entities: they survive app restarts and can transition
//! through a lifecycle. Reference images are stored on disk; their metadata
//! lives here.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::domain::{BoundingBox, ChoreEntity, ChoreStatus, ReferenceState, now_unix};

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
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chores (
                id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                target TEXT NOT NULL,
                action TEXT NOT NULL,
                estimated_seconds INTEGER NOT NULL,
                status INTEGER NOT NULL,
                ymin INTEGER, xmin INTEGER, ymax INTEGER, xmax INTEGER,
                confidence REAL NOT NULL,
                subtasks TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS room_references (
                room_id TEXT PRIMARY KEY,
                image_id TEXT NOT NULL,
                description TEXT NOT NULL,
                captured_at_unix INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert or replace chores within a single transaction.
    pub async fn upsert_chores(&self, chores: &[ChoreEntity]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for chore in chores {
            sqlx::query(
                "INSERT INTO chores
                    (id, room_id, target, action, estimated_seconds, status,
                     ymin, xmin, ymax, xmax, confidence, subtasks, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    room_id=excluded.room_id, target=excluded.target,
                    action=excluded.action, estimated_seconds=excluded.estimated_seconds,
                    status=excluded.status, ymin=excluded.ymin, xmin=excluded.xmin,
                    ymax=excluded.ymax, xmax=excluded.xmax, confidence=excluded.confidence,
                    subtasks=excluded.subtasks, updated_at=excluded.updated_at",
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
}

/// Convert a chores table row into a contract entity.
fn row_to_chore(row: &sqlx::sqlite::SqliteRow) -> Result<ChoreEntity, StoreError> {
    let subtasks_raw: String = row.try_get("subtasks")?;
    let subtasks: Vec<String> = serde_json::from_str(&subtasks_raw)
        .map_err(|e| StoreError::Decode(format!("subtasks: {e}")))?;
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
}