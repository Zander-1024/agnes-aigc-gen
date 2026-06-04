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
            ",
        )?;
        Ok(())
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
}
