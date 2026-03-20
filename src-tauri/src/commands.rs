//! Tauri command handlers: folder scanning, duplicate action execution, report export, file preview, action log, and undo.

use base64::Engine as _;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Global cancellation flag for the active scan.
static SCAN_CANCELLED: std::sync::LazyLock<Arc<AtomicBool>> =
    std::sync::LazyLock::new(|| Arc::new(AtomicBool::new(false)));
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::actionlog::{ActionBatch, ActionEntry, ActionLog};
use crate::cache::HashCache;
use crate::fileops;
use crate::hasher;
use crate::perceptual;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    pub reference_dir: String,
    pub eval_dir: String,
    pub dupe_mode: DupeMode,
    pub move_uniques: bool,
    pub unique_dest: Option<String>,
    pub categories: Vec<String>,
    pub all_files: bool,
    pub hash_algorithm: String,
    #[serde(default)]
    pub custom_extensions: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub removed_extensions: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub perceptual_matching: bool,
    #[serde(default = "default_perceptual_threshold")]
    pub perceptual_threshold: u32,
}

fn default_perceptual_threshold() -> u32 { 10 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DupeMode {
    Trash,
    MoveToFolder { dest: String },
    ReviewFirst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    pub ref_collect_ms: u64,
    pub ref_hash_ms: u64,
    pub eval_collect_ms: u64,
    pub eval_hash_ms: u64,
    pub total_ms: u64,
    pub ref_cache_hits: usize,
    pub eval_cache_hits: usize,
    pub ref_file_count: usize,
    pub eval_file_count: usize,
    pub total_bytes: u64,
    pub perceptual_compare_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub total_eval: usize,
    pub exact_matches: Vec<EvalFile>,
    pub similar_matches: Vec<EvalFile>,
    pub uniques: Vec<EvalFile>,
    pub skipped: usize,
    pub stats: ScanStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalFile {
    pub path: String,
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
    pub match_type: String,
    pub hamming_distance: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub current: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ActionMode {
    Trash,
    MoveToFolder { dest: String },
    Nothing,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub processed: usize,
    pub errors: Vec<String>,
    pub dirs_cleaned: usize,
}

fn emit_progress(app: &AppHandle, phase: &str, current: usize, total: usize) {
    let _ = app.emit("scan-progress", ProgressEvent {
        phase: phase.to_string(),
        current,
        total,
    });
}

/// Spawn a thread that polls an AtomicUsize and emits progress events until done.
fn spawn_progress_reporter(
    app: AppHandle,
    phase: String,
    progress: Arc<AtomicUsize>,
    total: usize,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            let current = progress.load(Ordering::Relaxed);
            emit_progress(&app, &phase, current, total);
            if current >= total {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}

#[tauri::command]
pub async fn cancel_scan() {
    SCAN_CANCELLED.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub async fn scan_folders(config: ScanConfig, app: AppHandle) -> Result<ScanResult, String> {
    let ref_dir = PathBuf::from(&config.reference_dir);
    let eval_dir = PathBuf::from(&config.eval_dir);

    if !ref_dir.is_dir() {
        return Err(format!("Reference folder does not exist: {}", ref_dir.display()));
    }
    if !eval_dir.is_dir() {
        return Err(format!("Eval folder does not exist: {}", eval_dir.display()));
    }
    if ref_dir == eval_dir {
        return Err("Reference and eval folders must be different".to_string());
    }
    if ref_dir.starts_with(&eval_dir) || eval_dir.starts_with(&ref_dir) {
        return Err("Reference and eval folders cannot be nested inside each other".to_string());
    }

    // Tauri async commands run on Tokio. Blocking I/O (hashing, SQLite) would
    // starve the runtime, so we spawn a dedicated OS thread and bridge back
    // via a oneshot channel.
    let (tx, rx) = tokio::sync::oneshot::channel();

    SCAN_CANCELLED.store(false, Ordering::Relaxed);
    let cancelled = SCAN_CANCELLED.clone();

    std::thread::spawn(move || {
        let result = scan_folders_blocking(&app, &ref_dir, &eval_dir, &config, &cancelled);
        let _ = tx.send(result);
    });

    rx.await.map_err(|_| "Scan thread dropped unexpectedly".to_string())?
}

fn scan_folders_blocking(
    app: &AppHandle,
    ref_dir: &Path,
    eval_dir: &Path,
    config: &ScanConfig,
    cancelled: &Arc<AtomicBool>,
) -> Result<ScanResult, String> {
    let scan_start = Instant::now();

    let allowed = hasher::resolve_extensions(
        &config.categories,
        config.all_files,
        &config.custom_extensions,
        &config.removed_extensions,
    );

    let cache = HashCache::open()?;
    if let Err(e) = cache.prune() {
        eprintln!("Warning: cache prune failed: {e}");
    }

    // -- Reference: collect --
    emit_progress(app, "Collecting reference files...", 0, 0);
    let t0 = Instant::now();
    let ref_files = hasher::collect_files(ref_dir, allowed.as_ref());
    let ref_collect_ms = t0.elapsed().as_millis() as u64;
    let ref_file_count = ref_files.len();

    if cancelled.load(Ordering::Relaxed) {
        return Err("Scan cancelled".to_string());
    }

    // -- Reference: hash --
    let ref_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Hashing reference folder...".to_string(),
        ref_progress.clone(),
        ref_file_count,
    );

    let t0 = Instant::now();
    let ref_result = hasher::hash_files_cached(&ref_files, &cache, ref_progress, &config.hash_algorithm, cancelled);
    let ref_hash_ms = t0.elapsed().as_millis() as u64;
    let ref_cache_hits = ref_result.cache_hits;
    let _ = reporter.join();

    if cancelled.load(Ordering::Relaxed) {
        return Err("Scan cancelled".to_string());
    }

    let ref_hashes: HashSet<String> = ref_result.hashed.iter().map(|f| f.hash.clone()).collect();

    // -- Eval: collect --
    emit_progress(app, "Collecting eval files...", 0, 0);
    let t0 = Instant::now();
    let eval_files = hasher::collect_files(eval_dir, allowed.as_ref());
    let eval_collect_ms = t0.elapsed().as_millis() as u64;
    let eval_file_count = eval_files.len();

    if cancelled.load(Ordering::Relaxed) {
        return Err("Scan cancelled".to_string());
    }

    // -- Eval: hash --
    let eval_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Checking eval folder...".to_string(),
        eval_progress.clone(),
        eval_file_count,
    );

    let t0 = Instant::now();
    let eval_result = hasher::hash_files_cached(&eval_files, &cache, eval_progress, &config.hash_algorithm, cancelled);
    let eval_hash_ms = t0.elapsed().as_millis() as u64;
    let eval_cache_hits = eval_result.cache_hits;
    let _ = reporter.join();

    if cancelled.load(Ordering::Relaxed) {
        return Err("Scan cancelled".to_string());
    }

    // Sort by path so intra-eval duplicate detection is deterministic: when two
    // eval files share a hash, the lexicographically-first path is kept as "unique."
    let mut eval_hashed = eval_result.hashed;
    eval_hashed.sort_by(|a, b| a.path.cmp(&b.path));

    let skipped = ref_result.skipped.len() + eval_result.skipped.len();

    // Compute total bytes across both phases
    let total_bytes: u64 = ref_result.hashed.iter().map(|f| f.size).sum::<u64>()
        + eval_hashed.iter().map(|f| f.size).sum::<u64>();

    // -- Content comparison --
    emit_progress(app, "Comparing files...", 0, 0);
    let mut seen_eval_hashes: HashSet<String> = HashSet::new();

    let mut exact_matches = Vec::new();
    let mut non_exact_eval = Vec::new();

    for ef in &eval_hashed {
        let is_ref_dupe = ref_hashes.contains(&ef.hash);
        let is_intra_dupe = seen_eval_hashes.contains(&ef.hash);
        let is_exact = is_ref_dupe || is_intra_dupe;

        let relative_path = Path::new(&ef.path)
            .strip_prefix(eval_dir)
            .unwrap_or(Path::new(&ef.path))
            .to_string_lossy()
            .to_string();

        let eval_file = EvalFile {
            path: ef.path.clone(),
            relative_path,
            size: ef.size,
            hash: ef.hash.clone(),
            match_type: if is_exact { "exact".to_string() } else { "unique".to_string() },
            hamming_distance: None,
        };

        if is_exact {
            exact_matches.push(eval_file);
        } else {
            non_exact_eval.push((eval_file, ef.perceptual_hash));
        }

        if !is_ref_dupe {
            seen_eval_hashes.insert(ef.hash.clone());
        }
    }

    // -- Perceptual comparison (optional) --
    let mut similar_matches = Vec::new();
    let mut perceptual_compare_ms: u64 = 0;
    let uniques_candidate;

    if config.perceptual_matching {
        let ref_phashes: Vec<u64> = ref_result.hashed.iter()
            .filter_map(|f| f.perceptual_hash)
            .collect();

        if !ref_phashes.is_empty() {
            let compare_total = non_exact_eval.len();
            emit_progress(app, "Comparing perceptual hashes...", 0, compare_total);
            let t0 = Instant::now();

            let mut still_unique = Vec::new();
            for (i, (mut eval_file, eval_phash)) in non_exact_eval.into_iter().enumerate() {
                emit_progress(app, "Comparing perceptual hashes...", i + 1, compare_total);
                if let Some(eval_ph) = eval_phash {
                    let min_dist = ref_phashes.iter()
                        .map(|&rph| perceptual::hamming_distance(eval_ph, rph))
                        .min()
                        .unwrap_or(u32::MAX);

                    if min_dist <= 15 {
                        // Always use the loosest threshold (15) on the backend.
                        // The frontend filters by the user's chosen threshold,
                        // allowing instant re-classification without re-scanning.
                        eval_file.match_type = "similar".to_string();
                        eval_file.hamming_distance = Some(min_dist);
                        similar_matches.push(eval_file);
                    } else {
                        still_unique.push(eval_file);
                    }
                } else {
                    still_unique.push(eval_file);
                }
            }
            uniques_candidate = still_unique;
            perceptual_compare_ms = t0.elapsed().as_millis() as u64;
        } else {
            uniques_candidate = non_exact_eval.into_iter().map(|(f, _)| f).collect();
        }
    } else {
        uniques_candidate = non_exact_eval.into_iter().map(|(f, _)| f).collect();
    }

    let total_ms = scan_start.elapsed().as_millis() as u64;

    Ok(ScanResult {
        total_eval: eval_hashed.len(),
        exact_matches,
        similar_matches,
        uniques: uniques_candidate,
        skipped,
        stats: ScanStats {
            ref_collect_ms,
            ref_hash_ms,
            eval_collect_ms,
            eval_hash_ms,
            total_ms,
            ref_cache_hits,
            eval_cache_hits,
            ref_file_count,
            eval_file_count,
            total_bytes,
            perceptual_compare_ms,
        },
    })
}

/// Generate an ISO 8601 timestamp from the current system time.
fn iso_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_ymd(days as i64);

    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[tauri::command]
pub async fn execute_action(
    eval_dir: String,
    files: Vec<String>,
    action: ActionMode,
) -> Result<ActionResult, String> {
    let eval_path = PathBuf::from(&eval_dir);
    let mut processed = 0;
    let mut errors = Vec::new();
    let mut log_entries: Vec<ActionEntry> = Vec::new();

    let now = iso_now();

    for file_str in &files {
        let file_path = PathBuf::from(file_str);
        if !file_path.exists() {
            errors.push(format!("File not found: {file_str}"));
            continue;
        }

        let result = match &action {
            ActionMode::Trash => {
                let res = fileops::trash_file(&file_path);
                if let Ok(warnings) = &res {
                    for w in warnings {
                        errors.push(w.clone());
                    }
                    log_entries.push(ActionEntry {
                        timestamp: now.clone(),
                        action: "trash".to_string(),
                        source_path: file_str.clone(),
                        dest_path: None,
                        eval_dir: eval_dir.clone(),
                    });
                }
                res.map(|_| ())
            }
            ActionMode::MoveToFolder { dest } => {
                let dest_path_buf = PathBuf::from(dest);
                let res = fileops::move_file(&file_path, &eval_path, &dest_path_buf);
                if let Ok((final_dest, warnings)) = &res {
                    for w in warnings {
                        errors.push(w.clone());
                    }
                    log_entries.push(ActionEntry {
                        timestamp: now.clone(),
                        action: "move".to_string(),
                        source_path: file_str.clone(),
                        dest_path: Some(final_dest.to_string_lossy().to_string()),
                        eval_dir: eval_dir.clone(),
                    });
                }
                res.map(|_| ())
            }
            ActionMode::Nothing => Ok(()),
        };

        match result {
            Ok(()) => processed += 1,
            Err(e) => errors.push(e),
        }
    }

    // Clean up empty directories in eval folder
    let dirs_cleaned = if !matches!(action, ActionMode::Nothing) {
        match fileops::cleanup_empty_dirs(&eval_path) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Warning: directory cleanup failed: {e}");
                0
            }
        }
    } else {
        0
    };

    // Record the batch in the action log (best-effort).
    if !log_entries.is_empty() {
        let action_type = match &action {
            ActionMode::Trash => "trash",
            ActionMode::MoveToFolder { .. } => "move",
            ActionMode::Nothing => "nothing",
        };

        let millis = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let batch = ActionBatch {
            id: format!("{millis}"),
            timestamp: now,
            action_type: action_type.to_string(),
            entries: log_entries,
            eval_dir: eval_dir.clone(),
        };

        let log_result = ActionLog::default().and_then(|log| log.append(batch));
        if let Err(e) = log_result {
            errors.push(format!("Warning: failed to record action for undo: {e}"));
        }
    }

    Ok(ActionResult {
        processed,
        errors,
        dirs_cleaned,
    })
}

/// Lightweight representation of a batch for the frontend (no full entry list).
#[derive(Debug, Clone, Serialize)]
pub struct ActionBatchSummary {
    pub id: String,
    pub timestamp: String,
    pub action_type: String,
    pub entry_count: usize,
    pub eval_dir: String,
}

#[tauri::command]
pub async fn get_action_log() -> Result<Vec<ActionBatchSummary>, String> {
    let log = ActionLog::default()?;
    let batches = log.load()?;

    let mut summaries: Vec<ActionBatchSummary> = batches
        .iter()
        .map(|b| ActionBatchSummary {
            id: b.id.clone(),
            timestamp: b.timestamp.clone(),
            action_type: b.action_type.clone(),
            entry_count: b.entries.len(),
            eval_dir: b.eval_dir.clone(),
        })
        .collect();
    summaries.reverse();
    Ok(summaries)
}

#[tauri::command]
pub async fn undo_last_action() -> Result<ActionResult, String> {
    let log = ActionLog::default()?;
    let batches = log.load()?;

    let batch = batches.last().ok_or("No actions to undo")?;

    if batch.action_type == "trash" {
        return Err(
            "Trash undo is not supported \u{2014} restore files manually from Trash.".to_string(),
        );
    }

    if batch.action_type != "move" {
        return Err(format!("Cannot undo action type: {}", batch.action_type));
    }

    let mut processed = 0;
    let mut errors = Vec::new();

    for entry in &batch.entries {
        let dest = match &entry.dest_path {
            Some(d) => PathBuf::from(d),
            None => {
                errors.push(format!("No destination recorded for {}", entry.source_path));
                continue;
            }
        };

        if !dest.exists() {
            errors.push(format!(
                "Destination file no longer exists: {}",
                dest.display()
            ));
            continue;
        }

        let source = PathBuf::from(&entry.source_path);

        if let Some(parent) = source.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                errors.push(format!(
                    "Failed to create directory {}: {e}",
                    parent.display()
                ));
                continue;
            }
        }

        let target = fileops::resolve_collision(&source);
        let res = fs::rename(&dest, &target).or_else(|rename_err| {
            fs::copy(&dest, &target)
                .map_err(|e| format!("Failed to restore {} (rename: {rename_err}, copy: {e})", dest.display()))?;
            let src_size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            let dst_size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            if src_size != dst_size {
                let _ = fs::remove_file(&target);
                return Err(format!(
                    "Copy verification failed restoring {} (src={src_size}, dst={dst_size})",
                    dest.display()
                ));
            }
            fs::remove_file(&dest)
                .map_err(|e| format!("Failed to remove {}: {e}", dest.display()))?;
            Ok::<(), String>(())
        });

        match res {
            Ok(()) => processed += 1,
            Err(e) => errors.push(format!("{e}")),
        }
    }

    // Clean up empty directories in the destination folder after undo.
    let dirs_cleaned = if let Some(first_entry) = batch.entries.first() {
        if let Some(dest_path) = &first_entry.dest_path {
            let dest_file = PathBuf::from(dest_path);
            if let Some(parent) = dest_file.parent() {
                let source_path = PathBuf::from(&first_entry.source_path);
                let eval_dir = PathBuf::from(&first_entry.eval_dir);
                if let Ok(relative) = source_path.strip_prefix(&eval_dir) {
                    let relative_str = relative.to_string_lossy();
                    let dest_str = dest_file.to_string_lossy();
                    if let Some(root_str) = dest_str.strip_suffix(&*relative_str) {
                        let dest_root = PathBuf::from(root_str.trim_end_matches('/'));
                        match fileops::cleanup_empty_dirs(&dest_root) {
                            Ok(n) => n,
                            Err(e) => {
                                eprintln!("Warning: directory cleanup failed: {e}");
                                0
                            }
                        }
                    } else {
                        match fileops::cleanup_empty_dirs(parent) {
                            Ok(n) => n,
                            Err(e) => {
                                eprintln!("Warning: directory cleanup failed: {e}");
                                0
                            }
                        }
                    }
                } else {
                    match fileops::cleanup_empty_dirs(parent) {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("Warning: directory cleanup failed: {e}");
                            0
                        }
                    }
                }
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    };

    let batch_id = batch.id.clone();
    if errors.is_empty() {
        // All entries restored — remove the batch
        log.remove_batch(&batch_id)?;
    } else if processed > 0 {
        // Partial undo — keep only entries whose destination still exists (not restored)
        let remaining: Vec<_> = batch.entries.iter()
            .filter(|entry| {
                entry.dest_path.as_ref()
                    .map(|d| std::path::Path::new(d).exists())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        log.update_entries(&batch_id, remaining)?;
    }
    // If processed == 0, leave the batch as-is for retry

    Ok(ActionResult {
        processed,
        errors,
        dirs_cleaned,
    })
}

/// RFC 4180 CSV field quoting: wraps in double-quotes when the field
/// contains commas, quotes, or newlines.
pub(crate) fn csv_quote(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[tauri::command]
pub async fn export_report(
    results: ScanResult,
    format: String,
    dest_path: String,
) -> Result<(), String> {
    let path = PathBuf::from(&dest_path);

    match format.as_str() {
        "csv" => {
            let mut file = fs::File::create(&path)
                .map_err(|e| format!("Failed to create file: {e}"))?;

            writeln!(file, "status,relative_path,size_bytes,hash,hamming_distance")
                .map_err(|e| format!("Failed to write header: {e}"))?;

            let all_files = results.exact_matches.iter()
                .chain(results.similar_matches.iter())
                .chain(results.uniques.iter());

            for f in all_files {
                let dist_str = f.hamming_distance
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                writeln!(
                    file,
                    "{},{},{},{},{}",
                    csv_quote(&f.match_type),
                    csv_quote(&f.relative_path),
                    f.size,
                    f.hash,
                    dist_str,
                )
                .map_err(|e| format!("Failed to write row: {e}"))?;
            }

            Ok(())
        }
        "json" => {
            let json = serde_json::to_string_pretty(&results)
                .map_err(|e| format!("Failed to serialize JSON: {e}"))?;
            fs::write(&path, json)
                .map_err(|e| format!("Failed to write file: {e}"))?;
            Ok(())
        }
        _ => Err(format!("Unsupported format: {format}. Use \"csv\" or \"json\".")),
    }
}

// ── File Preview ────────────────────────────────────────


#[derive(Debug, Clone, Serialize)]
pub struct FilePreview {
    pub path: String,
    pub size: u64,
    pub mime_type: String,
    pub is_image: bool,
    pub thumbnail_data: Option<String>,
}

/// Infer a MIME type from the file extension. Falls back to
/// "application/octet-stream" for unknown extensions.
fn mime_from_extension(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        // RAW formats
        "nef" => "image/x-nikon-nef",
        "cr2" => "image/x-canon-cr2",
        "cr3" => "image/x-canon-cr3",
        "arw" => "image/x-sony-arw",
        "orf" => "image/x-olympus-orf",
        "raf" => "image/x-fuji-raf",
        "dng" => "image/x-adobe-dng",
        "rw2" => "image/x-panasonic-rw2",
        // Video
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Whether the extension is one the `image` crate can decode for thumbnail generation.
fn can_thumbnail(ext: &str) -> bool {
    matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff")
}

/// Maximum thumbnail dimension (width or height) in pixels.
const THUMBNAIL_MAX_DIM: u32 = 800;

/// Decode an image, resize it to fit within THUMBNAIL_MAX_DIM, and return
/// a JPEG-encoded base64 data URI. Works for any format the `image` crate
/// supports, regardless of file size.
fn generate_thumbnail(file_path: &Path) -> Option<String> {
    let img = image::open(file_path).ok()?;
    let thumb = img.thumbnail(THUMBNAIL_MAX_DIM, THUMBNAIL_MAX_DIM);
    let mut buf = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Jpeg).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    Some(format!("data:image/jpeg;base64,{b64}"))
}

#[tauri::command]
pub async fn get_file_preview(path: String) -> Result<FilePreview, String> {
    let file_path = PathBuf::from(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {path}"));
    }

    let metadata = fs::metadata(&file_path)
        .map_err(|e| format!("Failed to read metadata: {e}"))?;
    let size = metadata.len();

    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mime_type = mime_from_extension(&ext).to_string();
    let is_image = mime_type.starts_with("image/");

    let thumbnail_data = if can_thumbnail(&ext) {
        generate_thumbnail(&file_path)
    } else {
        None
    };

    Ok(FilePreview {
        path,
        size,
        mime_type,
        is_image,
        thumbnail_data,
    })
}
