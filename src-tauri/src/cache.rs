use rusqlite::{Connection, params};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
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

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_hashes (
                path TEXT PRIMARY KEY,
                hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL
            );"
        ).map_err(|e| format!("Failed to create cache table: {e}"))?;

        Ok(Self { conn })
    }

    fn db_path() -> Result<PathBuf, String> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| "Could not determine application data directory".to_string())?;
        Ok(data_dir.join("com.photodedup").join("cache.db"))
    }

    /// Look up a cached hash. Returns Some(hash) if the file hasn't changed.
    pub fn get(&self, path: &str, size: u64, mtime_secs: i64, mtime_nanos: u32) -> Option<String> {
        self.conn
            .query_row(
                "SELECT hash FROM file_hashes WHERE path = ?1 AND size = ?2 AND mtime_secs = ?3 AND mtime_nanos = ?4",
                params![path, size as i64, mtime_secs, mtime_nanos],
                |row| row.get(0),
            )
            .ok()
    }

    /// Insert or update a hash entry.
    pub fn set(&self, entry: &CachedFile) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO file_hashes (path, hash, size, mtime_secs, mtime_nanos) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![entry.path, entry.hash, entry.size as i64, entry.mtime_secs, entry.mtime_nanos],
            )
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        Ok(())
    }

    /// Load all cached hashes into a set for fast lookup.
    pub fn load_hash_set(&self) -> Result<HashSet<String>, String> {
        let mut stmt = self.conn
            .prepare("SELECT hash FROM file_hashes")
            .map_err(|e| format!("Failed to prepare query: {e}"))?;
        let hashes = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Failed to query cache: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(hashes)
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
