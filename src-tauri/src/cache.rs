//! SQLite-backed hash cache. Avoids re-hashing files whose size and mtime haven't changed.

use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub algorithm: String,
    pub perceptual_hash: Option<i64>,
}

/// Returned by `get()` when a cache hit is found.
#[derive(Debug, Clone)]
pub struct CacheHit {
    pub hash: String,
    pub perceptual_hash: Option<u64>,
}

pub struct HashCache {
    conn: Connection,
}

impl HashCache {
    pub fn open() -> Result<Self, String> {
        let db_path = Self::db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache directory: {e}"))?;
        }
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Failed to open cache database: {e}"))?;

        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory database: {e}"))?;

        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_hashes (
                path TEXT NOT NULL,
                hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                algorithm TEXT NOT NULL DEFAULT 'sha256',
                perceptual_hash INTEGER,
                PRIMARY KEY (path, algorithm)
            );"
        ).map_err(|e| format!("Failed to create cache table: {e}"))?;

        // Migration for legacy databases that lack the `algorithm` column.
        let has_algorithm: bool = conn
            .prepare("PRAGMA table_info(file_hashes)")
            .map_err(|e| format!("Failed to read table info: {e}"))?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Failed to iterate table info: {e}"))?
            .filter_map(|r| r.ok())
            .any(|name| name == "algorithm");

        if !has_algorithm {
            conn.execute_batch(
                "ALTER TABLE file_hashes ADD COLUMN algorithm TEXT NOT NULL DEFAULT 'sha256';
                 CREATE TABLE IF NOT EXISTS file_hashes_new (
                     path TEXT NOT NULL,
                     hash TEXT NOT NULL,
                     size INTEGER NOT NULL,
                     mtime_secs INTEGER NOT NULL,
                     mtime_nanos INTEGER NOT NULL,
                     algorithm TEXT NOT NULL DEFAULT 'sha256',
                     perceptual_hash INTEGER,
                     PRIMARY KEY (path, algorithm)
                 );
                 INSERT OR IGNORE INTO file_hashes_new SELECT path, hash, size, mtime_secs, mtime_nanos, algorithm, NULL FROM file_hashes;
                 DROP TABLE file_hashes;
                 ALTER TABLE file_hashes_new RENAME TO file_hashes;"
            ).map_err(|e| format!("Failed to migrate cache schema: {e}"))?;
        }

        // Migration for databases that lack the `perceptual_hash` column.
        let has_perceptual: bool = conn
            .prepare("PRAGMA table_info(file_hashes)")
            .map_err(|e| format!("Failed to read table info: {e}"))?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Failed to iterate table info: {e}"))?
            .filter_map(|r| r.ok())
            .any(|name| name == "perceptual_hash");

        if !has_perceptual {
            conn.execute_batch(
                "ALTER TABLE file_hashes ADD COLUMN perceptual_hash INTEGER"
            ).map_err(|e| format!("Failed to add perceptual_hash column: {e}"))?;
        }

        Ok(())
    }

    fn db_path() -> Result<PathBuf, String> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| "Could not determine application data directory".to_string())?;
        Ok(data_dir.join("com.photodedup").join("cache.db"))
    }

    /// Returns a `CacheHit` if the file hasn't changed since it was cached.
    pub fn get(&self, path: &str, size: u64, mtime_secs: i64, mtime_nanos: u32, algorithm: &str) -> Option<CacheHit> {
        self.conn
            .query_row(
                "SELECT hash, perceptual_hash FROM file_hashes WHERE path = ?1 AND size = ?2 AND mtime_secs = ?3 AND mtime_nanos = ?4 AND algorithm = ?5",
                params![path, size as i64, mtime_secs, mtime_nanos, algorithm],
                |row| {
                    let hash: String = row.get(0)?;
                    let phash: Option<i64> = row.get(1)?;
                    Ok(CacheHit {
                        hash,
                        perceptual_hash: phash.map(|v| v as u64),
                    })
                },
            )
            .ok()
    }

    pub fn set(&self, entry: &CachedFile) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO file_hashes (path, hash, size, mtime_secs, mtime_nanos, algorithm, perceptual_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![entry.path, entry.hash, entry.size as i64, entry.mtime_secs, entry.mtime_nanos, entry.algorithm, entry.perceptual_hash],
            )
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        Ok(())
    }

    /// Remove entries for paths that no longer exist on disk.
    pub fn prune(&self) -> Result<usize, String> {
        let mut stmt = self.conn
            .prepare("SELECT path FROM file_hashes")
            .map_err(|e| format!("Failed to prepare prune query: {e}"))?;
        let paths: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Failed to query paths: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        let mut pruned = 0;
        for path in &paths {
            if !Path::new(path).exists() {
                self.conn
                    .execute("DELETE FROM file_hashes WHERE path = ?1", params![path])
                    .map_err(|e| format!("Failed to prune entry: {e}"))?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }
}
