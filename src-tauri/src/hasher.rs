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

fn is_supported_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return false,
    };
    PHOTO_EXTENSIONS.contains(&ext.as_str()) || VIDEO_EXTENSIONS.contains(&ext.as_str())
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().map_or(false, |s| s.starts_with('.'))
}

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

pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 131_072]; // 128 KB
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

/// Successful hashes plus any files that could not be hashed.
#[derive(Debug)]
pub struct HashResult {
    pub hashed: Vec<HashedFile>,
    pub skipped: Vec<SkippedFile>,
}

struct FileMeta {
    path: PathBuf,
    path_str: String,
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
}

/// Check cache serially (fast), hash misses in parallel, update cache serially.
pub fn hash_files_cached(
    files: &[PathBuf],
    cache: &HashCache,
    progress: Arc<AtomicUsize>,
) -> HashResult {
    let mut results: Vec<HashedFile> = Vec::with_capacity(files.len());
    let mut needs_hashing: Vec<FileMeta> = Vec::new();
    let mut skipped: Vec<SkippedFile> = Vec::new();

    for path in files {
        let path_str = path.to_string_lossy().to_string();
        let meta = match file_meta(path) {
            Ok(m) => m,
            Err(reason) => {
                progress.fetch_add(1, Ordering::Relaxed);
                skipped.push(SkippedFile { path: path_str, reason });
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

    // Hash cache misses in parallel
    let progress_clone = progress.clone();
    let newly_hashed: Vec<Result<(FileMeta, String), SkippedFile>> = needs_hashing
        .into_par_iter()
        .map(|fm| {
            match hash_file(&fm.path) {
                Ok(hash) => {
                    progress_clone.fetch_add(1, Ordering::Relaxed);
                    Ok((fm, hash))
                }
                Err(reason) => {
                    progress_clone.fetch_add(1, Ordering::Relaxed);
                    let path = fm.path_str.clone();
                    Err(SkippedFile { path, reason })
                }
            }
        })
        .collect();

    // Update cache and collect results (serial)
    for item in newly_hashed {
        match item {
            Ok((fm, hash)) => {
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
            Err(sf) => skipped.push(sf),
        }
    }

    HashResult { hashed: results, skipped }
}
