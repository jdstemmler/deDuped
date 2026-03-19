use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

use crate::cache::{CachedFile, HashCache};

const PHOTO_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "heic", "heif",
    "cr2", "cr3", "nef", "arw", "orf", "rw2", "dng", "raf", "pef", "srw", "x3f",
];

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "avi", "mkv"];

/// Check if a file extension matches our supported types.
fn is_supported_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return false,
    };
    PHOTO_EXTENSIONS.contains(&ext.as_str()) || VIDEO_EXTENSIONS.contains(&ext.as_str())
}

/// Check if a path component is hidden (starts with `.`).
fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().map_or(false, |s| s.starts_with('.'))
}

/// Walk a directory and collect all supported file paths.
pub fn collect_files(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_supported_file(e.path()))
        .map(|e| e.into_path())
        .collect()
}

/// Hash a single file with SHA-256.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Get file metadata (size, mtime) for cache comparison.
fn file_meta(path: &Path) -> Result<(u64, i64, u32), String> {
    let meta = fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for {}: {e}", path.display()))?;
    let mtime = meta.modified()
        .map_err(|e| format!("Failed to get mtime for {}: {e}", path.display()))?;
    let duration = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok((meta.len(), duration.as_secs() as i64, duration.subsec_nanos()))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HashedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
}

/// Metadata needed for cache lookups and hashing.
struct FileMeta {
    path: PathBuf,
    path_str: String,
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
}

/// Hash files in parallel, using the cache for already-known files.
/// Strategy: check cache serially (fast), hash misses in parallel, update cache serially.
pub fn hash_files_cached(
    files: &[PathBuf],
    cache: &HashCache,
    progress: Arc<AtomicUsize>,
) -> Vec<HashedFile> {
    // Phase 1: Check cache for each file (serial — rusqlite is not Sync)
    let mut results: Vec<HashedFile> = Vec::with_capacity(files.len());
    let mut needs_hashing: Vec<FileMeta> = Vec::new();

    for path in files {
        let path_str = path.to_string_lossy().to_string();
        let meta = match file_meta(path) {
            Ok(m) => m,
            Err(_) => {
                progress.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        let (size, mtime_secs, mtime_nanos) = meta;

        if let Some(hash) = cache.get(&path_str, size, mtime_secs, mtime_nanos) {
            progress.fetch_add(1, Ordering::Relaxed);
            results.push(HashedFile { path: path_str, hash, size });
        } else {
            needs_hashing.push(FileMeta {
                path: path.clone(),
                path_str,
                size,
                mtime_secs,
                mtime_nanos,
            });
        }
    }

    // Phase 2: Hash cache misses in parallel
    let progress_clone = progress.clone();
    let newly_hashed: Vec<(FileMeta, String)> = needs_hashing
        .into_par_iter()
        .filter_map(|fm| {
            let hash = hash_file(&fm.path).ok()?;
            progress_clone.fetch_add(1, Ordering::Relaxed);
            Some((fm, hash))
        })
        .collect();

    // Phase 3: Update cache and collect results (serial)
    for (fm, hash) in newly_hashed {
        let _ = cache.set(&CachedFile {
            path: fm.path_str.clone(),
            hash: hash.clone(),
            size: fm.size,
            mtime_secs: fm.mtime_secs,
            mtime_nanos: fm.mtime_nanos,
        });
        results.push(HashedFile {
            path: fm.path_str,
            hash,
            size: fm.size,
        });
    }

    results
}
