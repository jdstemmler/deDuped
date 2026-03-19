use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

use crate::cache::{CachedFile, HashCache};

pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "bmp", "webp", "heic", "heif",
    "cr2", "cr3", "nef", "arw", "orf", "rw2", "dng", "raf", "pef", "srw", "x3f",
];

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "m4v", "wmv", "flv", "webm", "mts", "m2ts",
];

pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "md",
    "csv", "psd", "ai", "indd", "sketch", "fig",
];

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "aac", "wav", "aiff", "ogg", "m4a", "wma", "alac",
];

/// Resolve category names into an optional set of allowed extensions.
/// Returns `None` if `all_files` is true (accept everything).
/// Returns `Some(set)` with the combined extensions from all enabled categories.
pub fn resolve_extensions(categories: &[String], all_files: bool) -> Option<HashSet<String>> {
    if all_files {
        return None;
    }
    let mut set = HashSet::new();
    for cat in categories {
        let exts: &[&str] = match cat.to_lowercase().as_str() {
            "images" => IMAGE_EXTENSIONS,
            "videos" => VIDEO_EXTENSIONS,
            "documents" => DOCUMENT_EXTENSIONS,
            "audio" => AUDIO_EXTENSIONS,
            _ => continue,
        };
        for ext in exts {
            set.insert(ext.to_string());
        }
    }
    Some(set)
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().map_or(false, |s| s.starts_with('.'))
}

pub fn collect_files(dir: &Path, allowed_extensions: Option<&HashSet<String>>) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            match allowed_extensions {
                None => true,
                Some(exts) => {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| exts.contains(&ext.to_lowercase()))
                        .unwrap_or(false)
                }
            }
        })
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
