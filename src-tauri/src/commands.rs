//! Tauri command handlers: folder scanning, duplicate action execution, and report export.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::cache::HashCache;
use crate::fileops;
use crate::hasher;

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
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub total_eval: usize,
    pub duplicates: Vec<EvalFile>,
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
    pub is_duplicate: bool,
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
pub async fn scan_folders(config: ScanConfig, app: AppHandle) -> Result<ScanResult, String> {
    let ref_dir = PathBuf::from(&config.reference_dir);
    let eval_dir = PathBuf::from(&config.eval_dir);

    if !ref_dir.is_dir() {
        return Err(format!("Reference folder does not exist: {}", ref_dir.display()));
    }
    if !eval_dir.is_dir() {
        return Err(format!("Eval folder does not exist: {}", eval_dir.display()));
    }

    // Tauri async commands run on Tokio. Blocking I/O (hashing, SQLite) would
    // starve the runtime, so we spawn a dedicated OS thread and bridge back
    // via a oneshot channel.
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let result = scan_folders_blocking(&app, &ref_dir, &eval_dir, &config);
        let _ = tx.send(result);
    });

    rx.await.map_err(|_| "Scan thread dropped unexpectedly".to_string())?
}

fn scan_folders_blocking(
    app: &AppHandle,
    ref_dir: &Path,
    eval_dir: &Path,
    config: &ScanConfig,
) -> Result<ScanResult, String> {
    let scan_start = Instant::now();

    let allowed = hasher::resolve_extensions(
        &config.categories,
        config.all_files,
        &config.custom_extensions,
        &config.removed_extensions,
    );

    let cache = HashCache::open()?;
    let _ = cache.prune();

    // -- Reference: collect --
    emit_progress(app, "Collecting reference files...", 0, 0);
    let t0 = Instant::now();
    let ref_files = hasher::collect_files(ref_dir, allowed.as_ref());
    let ref_collect_ms = t0.elapsed().as_millis() as u64;
    let ref_file_count = ref_files.len();

    // -- Reference: hash --
    let ref_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Hashing reference folder...".to_string(),
        ref_progress.clone(),
        ref_file_count,
    );

    let t0 = Instant::now();
    let ref_result = hasher::hash_files_cached(&ref_files, &cache, ref_progress, &config.hash_algorithm);
    let ref_hash_ms = t0.elapsed().as_millis() as u64;
    let ref_cache_hits = ref_result.cache_hits;
    let _ = reporter.join();

    let ref_hashes: HashSet<String> = ref_result.hashed.iter().map(|f| f.hash.clone()).collect();

    // -- Eval: collect --
    emit_progress(app, "Collecting eval files...", 0, 0);
    let t0 = Instant::now();
    let eval_files = hasher::collect_files(eval_dir, allowed.as_ref());
    let eval_collect_ms = t0.elapsed().as_millis() as u64;
    let eval_file_count = eval_files.len();

    // -- Eval: hash --
    let eval_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Checking eval folder...".to_string(),
        eval_progress.clone(),
        eval_file_count,
    );

    let t0 = Instant::now();
    let eval_result = hasher::hash_files_cached(&eval_files, &cache, eval_progress, &config.hash_algorithm);
    let eval_hash_ms = t0.elapsed().as_millis() as u64;
    let eval_cache_hits = eval_result.cache_hits;
    let _ = reporter.join();

    // Sort by path so intra-eval duplicate detection is deterministic: when two
    // eval files share a hash, the lexicographically-first path is kept as "unique."
    let mut eval_hashed = eval_result.hashed;
    eval_hashed.sort_by(|a, b| a.path.cmp(&b.path));

    let skipped = ref_result.skipped.len() + eval_result.skipped.len();

    // Compute total bytes across both phases
    let total_bytes: u64 = ref_result.hashed.iter().map(|f| f.size).sum::<u64>()
        + eval_hashed.iter().map(|f| f.size).sum::<u64>();

    // Detect intra-eval duplicates: track which hashes we've already seen
    emit_progress(app, "Comparing files...", 0, 0);
    let mut seen_eval_hashes: HashSet<String> = HashSet::new();

    let mut duplicates = Vec::new();
    let mut uniques = Vec::new();

    for ef in &eval_hashed {
        let is_ref_dupe = ref_hashes.contains(&ef.hash);
        let is_intra_dupe = seen_eval_hashes.contains(&ef.hash);
        let is_duplicate = is_ref_dupe || is_intra_dupe;

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
            is_duplicate,
        };

        if is_duplicate {
            duplicates.push(eval_file);
        } else {
            uniques.push(eval_file);
        }

        // Only track non-ref hashes for intra-eval detection. If a hash matches
        // reference, every eval copy is a ref-dupe regardless of how many exist.
        if !is_ref_dupe {
            seen_eval_hashes.insert(ef.hash.clone());
        }
    }

    let total_ms = scan_start.elapsed().as_millis() as u64;

    Ok(ScanResult {
        total_eval: eval_hashed.len(),
        duplicates,
        uniques,
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
        },
    })
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

    for file_str in &files {
        let file_path = PathBuf::from(file_str);
        if !file_path.exists() {
            errors.push(format!("File not found: {file_str}"));
            continue;
        }

        let result = match &action {
            ActionMode::Trash => fileops::trash_file(&file_path),
            ActionMode::MoveToFolder { dest } => {
                fileops::move_file(&file_path, &eval_path, &PathBuf::from(dest)).map(|_| ())
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
        fileops::cleanup_empty_dirs(&eval_path).unwrap_or(0)
    } else {
        0
    };

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

            writeln!(file, "status,relative_path,size_bytes,hash")
                .map_err(|e| format!("Failed to write header: {e}"))?;

            for f in &results.duplicates {
                writeln!(
                    file,
                    "{},{},{},{}",
                    "duplicate",
                    csv_quote(&f.relative_path),
                    f.size,
                    f.hash,
                )
                .map_err(|e| format!("Failed to write row: {e}"))?;
            }
            for f in &results.uniques {
                writeln!(
                    file,
                    "{},{},{},{}",
                    "unique",
                    csv_quote(&f.relative_path),
                    f.size,
                    f.hash,
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
