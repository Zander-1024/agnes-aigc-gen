use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use rand::Rng;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetRecord {
    pub id: String,
    pub asset_uri: String,
    pub kind: String,
    pub remote_url: String,
    pub ratio: Option<String>,
    pub size: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationRecord {
    pub id: i64,
    pub kind: String,
    pub prompt: Option<String>,
    pub input_json: Value,
    pub output_json: Value,
    pub asset_id: Option<String>,
    pub created_at: String,
}

/// Local record of an Agnes video async task (`POST /videos` → poll `GET /videos/{id}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoTaskRecord {
    /// Local short id (1, 2, 3…); use with `task show 3` / `task wait 3`.
    pub id: i64,
    /// Vendor task id returned by Agnes API (`POST /videos`).
    pub task_id: String,
    /// API status: `queued`, `in_progress`, `completed`, `failed`
    pub status: String,
    /// Normalized: `processing`, `success`, or `failed`
    pub phase: String,
    pub prompt: Option<String>,
    pub input_json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
}

impl VideoTaskRecord {
    pub fn phase_from_status(status: &str) -> &'static str {
        match status {
            "completed" => "success",
            "failed" => "failed",
            _ => "processing",
        }
    }
}

impl Database {
    pub fn open() -> Result<Self> {
        let path = Self::db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).with_context(|| format!("open sqlite db {}", path.display()))?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn db_path() -> Result<PathBuf> {
        Ok(AppConfig::config_dir()?.join("generations.db"))
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                remote_url TEXT NOT NULL,
                ratio TEXT,
                size TEXT,
                created_at TEXT NOT NULL,
                meta_json TEXT
            );

            CREATE TABLE IF NOT EXISTS generations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                prompt TEXT,
                input_json TEXT NOT NULL,
                output_json TEXT NOT NULL,
                asset_id TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (asset_id) REFERENCES assets(id)
            );

            CREATE INDEX IF NOT EXISTS idx_generations_created ON generations(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_assets_created ON assets(created_at DESC);

            CREATE TABLE IF NOT EXISTS video_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                prompt TEXT,
                input_json TEXT,
                progress INTEGER,
                video_url TEXT,
                asset_id TEXT,
                error_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (asset_id) REFERENCES assets(id)
            );

            CREATE INDEX IF NOT EXISTS idx_video_tasks_updated ON video_tasks(updated_at DESC);
            ",
        )?;
        self.migrate_video_tasks_local_id()?;
        Ok(())
    }

    /// Upgrade legacy `video_tasks` (task_id PRIMARY KEY) to include local `id`.
    fn migrate_video_tasks_local_id(&self) -> Result<()> {
        if self.table_has_column("video_tasks", "id")? {
            return Ok(());
        }
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='video_tasks'", [], |r| {
                r.get(0)
            })?;
        if count == 0 {
            return Ok(());
        }
        self.conn.execute_batch(
            "
            CREATE TABLE video_tasks_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                prompt TEXT,
                input_json TEXT,
                progress INTEGER,
                video_url TEXT,
                asset_id TEXT,
                error_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (asset_id) REFERENCES assets(id)
            );
            INSERT INTO video_tasks_new (task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at)
                SELECT task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at
                FROM video_tasks;
            DROP TABLE video_tasks;
            ALTER TABLE video_tasks_new RENAME TO video_tasks;
            CREATE INDEX IF NOT EXISTS idx_video_tasks_updated ON video_tasks(updated_at DESC);
            ",
        )?;
        Ok(())
    }

    fn table_has_column(&self, table: &str, column: &str) -> Result<bool> {
        let sql = format!("PRAGMA table_info({table})");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Resolve `asset://id` to the stored remote URL; pass through anything else.
    pub fn resolve_reference(&self, raw: &str) -> Result<String> {
        let raw = raw.trim();
        if let Some(id) = raw.strip_prefix("asset://") {
            let asset = self.get_asset(id)?;
            return Ok(asset.remote_url);
        }
        Ok(raw.to_string())
    }

    pub fn insert_asset(
        &self,
        kind: &str,
        remote_url: &str,
        ratio: Option<&str>,
        size: Option<&str>,
    ) -> Result<AssetRecord> {
        let id = new_asset_id();
        let created_at = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO assets (id, kind, remote_url, ratio, size, created_at, meta_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![id, kind, remote_url, ratio, size, created_at],
        )?;
        Ok(AssetRecord {
            id: id.clone(),
            asset_uri: format!("asset://{id}"),
            kind: kind.to_string(),
            remote_url: remote_url.to_string(),
            ratio: ratio.map(str::to_string),
            size: size.map(str::to_string),
            created_at,
        })
    }

    pub fn get_asset(&self, id: &str) -> Result<AssetRecord> {
        let id = id.strip_prefix("asset://").unwrap_or(id);
        self.conn
            .query_row(
                "SELECT id, kind, remote_url, ratio, size, created_at FROM assets WHERE id = ?1",
                params![id],
                |row| {
                    let id: String = row.get(0)?;
                    Ok(AssetRecord {
                        asset_uri: format!("asset://{id}"),
                        id,
                        kind: row.get(1)?,
                        remote_url: row.get(2)?,
                        ratio: row.get(3)?,
                        size: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .with_context(|| format!("asset not found: asset://{id}"))
    }

    pub fn insert_generation(
        &self,
        kind: &str,
        prompt: Option<&str>,
        input: &Value,
        output: &Value,
        asset_id: Option<&str>,
    ) -> Result<i64> {
        let created_at = Utc::now().to_rfc3339();
        let input_json = serde_json::to_string(input).context("serialize input")?;
        let output_json = serde_json::to_string(output).context("serialize output")?;
        self.conn.execute(
            "INSERT INTO generations (kind, prompt, input_json, output_json, asset_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![kind, prompt, input_json, output_json, asset_id, created_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_generations(&self, limit: usize) -> Result<Vec<GenerationRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, prompt, input_json, output_json, asset_id, created_at
             FROM generations ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(GenerationRecord {
                id: row.get(0)?,
                kind: row.get(1)?,
                prompt: row.get(2)?,
                input_json: parse_json_col(row.get::<_, String>(3)?)?,
                output_json: parse_json_col(row.get::<_, String>(4)?)?,
                asset_id: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("list generations")
    }

    pub fn get_generation(&self, id: i64) -> Result<GenerationRecord> {
        self.conn
            .query_row(
                "SELECT id, kind, prompt, input_json, output_json, asset_id, created_at
                 FROM generations WHERE id = ?1",
                params![id],
                |row| {
                    Ok(GenerationRecord {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        prompt: row.get(2)?,
                        input_json: parse_json_col(row.get::<_, String>(3)?)?,
                        output_json: parse_json_col(row.get::<_, String>(4)?)?,
                        asset_id: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .with_context(|| format!("generation #{id} not found"))
    }

    pub fn list_assets(&self, limit: usize) -> Result<Vec<AssetRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, remote_url, ratio, size, created_at FROM assets
             ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let id: String = row.get(0)?;
            Ok(AssetRecord {
                asset_uri: format!("asset://{id}"),
                id,
                kind: row.get(1)?,
                remote_url: row.get(2)?,
                ratio: row.get(3)?,
                size: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("list assets")
    }

    pub fn insert_video_task(
        &self,
        task_id: &str,
        status: &str,
        prompt: Option<&str>,
        input: Option<&Value>,
        progress: Option<i32>,
    ) -> Result<VideoTaskRecord> {
        let now = Utc::now().to_rfc3339();
        let input_json = input.map(|v| serde_json::to_string(v)).transpose()?;
        self.conn.execute(
            "INSERT INTO video_tasks (task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, ?6, ?6)",
            params![task_id, status, prompt, input_json, progress, now],
        )?;
        self.get_video_task(task_id)
    }

    pub fn update_video_task(
        &self,
        task_id: &str,
        status: &str,
        progress: Option<i32>,
        video_url: Option<&str>,
        asset_id: Option<&str>,
        error: Option<&Value>,
    ) -> Result<VideoTaskRecord> {
        let updated_at = Utc::now().to_rfc3339();
        let error_json = error.map(|v| serde_json::to_string(v)).transpose()?;
        self.conn.execute(
            "UPDATE video_tasks SET status = ?2, progress = ?3, video_url = ?4, asset_id = ?5, error_json = ?6, updated_at = ?7
             WHERE task_id = ?1",
            params![task_id, status, progress, video_url, asset_id, error_json, updated_at],
        )?;
        self.get_video_task(task_id)
    }

    pub fn get_video_task(&self, task_id: &str) -> Result<VideoTaskRecord> {
        self.conn
            .query_row(
                "SELECT id, task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at
                 FROM video_tasks WHERE task_id = ?1",
                params![task_id],
                |row| map_video_task_row(row),
            )
            .with_context(|| format!("video task not found: {task_id}"))
    }

    pub fn get_video_task_by_local_id(&self, local_id: i64) -> Result<VideoTaskRecord> {
        self.conn
            .query_row(
                "SELECT id, task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at
                 FROM video_tasks WHERE id = ?1",
                params![local_id],
                |row| map_video_task_row(row),
            )
            .with_context(|| format!("video task #{local_id} not found"))
    }

    /// Map a CLI reference to the vendor task id: local id (`3`, `#3`) or full vendor id.
    pub fn resolve_video_task_ref(&self, reference: &str) -> Result<String> {
        let reference = reference.trim();
        if reference.is_empty() {
            anyhow::bail!("task reference required");
        }
        let local_ref = reference.strip_prefix('#').unwrap_or(reference);
        if local_ref.chars().all(|c| c.is_ascii_digit()) {
            let local_id: i64 = local_ref.parse().context("invalid local task id")?;
            return Ok(self.get_video_task_by_local_id(local_id)?.task_id);
        }
        Ok(reference.to_string())
    }

    pub fn list_video_tasks(&self, limit: usize) -> Result<Vec<VideoTaskRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, status, prompt, input_json, progress, video_url, asset_id, error_json, created_at, updated_at
             FROM video_tasks ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| map_video_task_row(row))?;
        rows.collect::<Result<Vec<_>, _>>().context("list video tasks")
    }
}

fn map_video_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoTaskRecord> {
    let status: String = row.get(2)?;
    let asset_id: Option<String> = row.get(7)?;
    Ok(VideoTaskRecord {
        id: row.get(0)?,
        task_id: row.get(1)?,
        phase: VideoTaskRecord::phase_from_status(&status).to_string(),
        status,
        prompt: row.get(3)?,
        input_json: row
            .get::<_, Option<String>>(4)?
            .map(|s| parse_json_col(s))
            .transpose()?,
        progress: row.get(5)?,
        uri: row.get(6)?,
        asset_uri: asset_id.map(|id| format!("asset://{id}")),
        error: row
            .get::<_, Option<String>>(8)?
            .map(|s| parse_json_col(s))
            .transpose()?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn parse_json_col(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn new_asset_id() -> String {
    const CHARSET: &[u8] = b"0123456789abcdef";
    let mut rng = rand::thread_rng();
    (0..12).map(|_| CHARSET[rng.gen_range(0..16)] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_mem() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        let db = Database { conn };
        db.migrate().unwrap();
        db
    }

    #[test]
    fn asset_roundtrip() {
        let db = open_mem();
        let asset = db
            .insert_asset("image", "https://example.com/a.png", Some("1:1"), Some("1024x1024"))
            .unwrap();
        assert_eq!(
            db.resolve_reference(&asset.asset_uri).unwrap(),
            "https://example.com/a.png"
        );
    }

    #[test]
    fn video_task_roundtrip() {
        let db = open_mem();
        let row = db
            .insert_video_task("task_abc", "queued", Some("ocean"), None, Some(0))
            .unwrap();
        assert_eq!(row.id, 1);
        assert_eq!(row.phase, "processing");
        assert_eq!(db.resolve_video_task_ref("1").unwrap(), "task_abc");
        assert_eq!(db.resolve_video_task_ref("#1").unwrap(), "task_abc");
        let updated = db
            .update_video_task(
                "task_abc",
                "completed",
                Some(100),
                Some("https://example.com/v.mp4"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(updated.phase, "success");
        assert_eq!(updated.uri.as_deref(), Some("https://example.com/v.mp4"));
        assert_eq!(db.list_video_tasks(10).unwrap().len(), 1);
    }
}
