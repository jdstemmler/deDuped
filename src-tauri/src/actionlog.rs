//! Persistent action log for file operations, enabling undo of recent batches.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Maximum number of batches retained in the log file.
const MAX_BATCHES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionEntry {
    pub timestamp: String,
    pub action: String,
    pub source_path: String,
    pub dest_path: Option<String>,
    pub eval_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBatch {
    pub id: String,
    pub timestamp: String,
    pub action_type: String,
    pub entries: Vec<ActionEntry>,
    pub eval_dir: String,
}

pub struct ActionLog {
    path: PathBuf,
}

impl ActionLog {
    /// Returns an `ActionLog` backed by the default log file location.
    pub fn default() -> Result<Self, String> {
        let support_dir = dirs::data_dir()
            .ok_or_else(|| "Could not determine Application Support directory".to_string())?
            .join("com.photodedup");

        fs::create_dir_all(&support_dir)
            .map_err(|e| format!("Failed to create app support dir: {e}"))?;

        Ok(Self {
            path: support_dir.join("action_log.json"),
        })
    }

    /// Creates an `ActionLog` at an arbitrary path (useful for testing).
    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Read and parse the log file. Returns an empty vec if the file doesn't exist.
    pub fn load(&self) -> Result<Vec<ActionBatch>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&self.path)
            .map_err(|e| format!("Failed to read action log: {e}"))?;
        if data.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse action log: {e}"))
    }

    /// Append a batch to the log, enforcing the cap of MAX_BATCHES.
    pub fn append(&self, batch: ActionBatch) -> Result<(), String> {
        let mut batches = self.load()?;
        batches.push(batch);

        // Keep only the most recent MAX_BATCHES entries.
        if batches.len() > MAX_BATCHES {
            let drain_count = batches.len() - MAX_BATCHES;
            batches.drain(..drain_count);
        }

        self.write(&batches)
    }

    /// Remove a batch by id (e.g. after a successful undo).
    pub fn remove_batch(&self, batch_id: &str) -> Result<(), String> {
        let mut batches = self.load()?;
        batches.retain(|b| b.id != batch_id);
        self.write(&batches)
    }

    fn write(&self, batches: &[ActionBatch]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(batches)
            .map_err(|e| format!("Failed to serialize action log: {e}"))?;
        fs::write(&self.path, json)
            .map_err(|e| format!("Failed to write action log: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_batch(id: &str, action_type: &str, count: usize) -> ActionBatch {
        let entries: Vec<ActionEntry> = (0..count)
            .map(|i| ActionEntry {
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                action: action_type.to_string(),
                source_path: format!("/src/file_{i}.jpg"),
                dest_path: if action_type == "move" {
                    Some(format!("/dest/file_{i}.jpg"))
                } else {
                    None
                },
                eval_dir: "/src".to_string(),
            })
            .collect();

        ActionBatch {
            id: id.to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            action_type: action_type.to_string(),
            entries,
            eval_dir: "/src".to_string(),
        }
    }

    #[test]
    fn load_returns_empty_when_no_file() {
        let tmp = TempDir::new().unwrap();
        let log = ActionLog::at(tmp.path().join("missing.json"));
        let batches = log.load().unwrap();
        assert!(batches.is_empty());
    }

    #[test]
    fn append_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let log = ActionLog::at(tmp.path().join("log.json"));

        log.append(sample_batch("b1", "trash", 2)).unwrap();
        log.append(sample_batch("b2", "move", 3)).unwrap();

        let batches = log.load().unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].id, "b1");
        assert_eq!(batches[1].id, "b2");
        assert_eq!(batches[1].entries.len(), 3);
    }

    #[test]
    fn remove_batch_by_id() {
        let tmp = TempDir::new().unwrap();
        let log = ActionLog::at(tmp.path().join("log.json"));

        log.append(sample_batch("b1", "trash", 1)).unwrap();
        log.append(sample_batch("b2", "move", 1)).unwrap();
        log.append(sample_batch("b3", "trash", 1)).unwrap();

        log.remove_batch("b2").unwrap();

        let batches = log.load().unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].id, "b1");
        assert_eq!(batches[1].id, "b3");
    }

    #[test]
    fn caps_at_max_batches() {
        let tmp = TempDir::new().unwrap();
        let log = ActionLog::at(tmp.path().join("log.json"));

        for i in 0..60 {
            log.append(sample_batch(&format!("b{i}"), "trash", 1)).unwrap();
        }

        let batches = log.load().unwrap();
        assert_eq!(batches.len(), 50);
        // The oldest 10 should have been dropped; first retained is b10.
        assert_eq!(batches[0].id, "b10");
        assert_eq!(batches[49].id, "b59");
    }
}
