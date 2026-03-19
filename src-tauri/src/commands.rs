use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::cache::HashCache;
use crate::fileops;
use crate::hasher;

// ── Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    pub reference_dir: String,
    pub eval_dir: String,
    pub dupe_mode: DupeMode,
    pub move_uniques: bool,
    pub unique_dest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DupeMode {
    Trash,
    MoveToFolder { dest: String },
    ReviewFirst,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub total_eval: usize,
    pub duplicates: Vec<EvalFile>,
    pub uniques: Vec<EvalFile>,
}

#[derive(Debug, Clone, Serialize)]
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

// ── Commands ─────────────────────────────────────────────────────────

/// Emit a progress event to the frontend.
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

    // Move ALL blocking work off the async runtime
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let result = scan_folders_blocking(&config, &app, &ref_dir, &eval_dir);
        let _ = tx.send(result);
    });

    rx.await.map_err(|_| "Scan thread dropped unexpectedly".to_string())?
}

fn scan_folders_blocking(
    config: &ScanConfig,
    app: &AppHandle,
    ref_dir: &Path,
    eval_dir: &Path,
) -> Result<ScanResult, String> {
    // Open cache
    let cache = HashCache::open()?;
    let _ = cache.prune();

    // Phase 1a: Collect reference files
    emit_progress(app, "Collecting reference files...", 0, 0);
    let ref_files = hasher::collect_files(ref_dir);
    let ref_total = ref_files.len();

    // Phase 1b: Hash reference folder
    let ref_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Hashing reference folder...".to_string(),
        ref_progress.clone(),
        ref_total,
    );

    let ref_hashed = {
        let cache = HashCache::open()?;
        hasher::hash_files_cached(&ref_files, &cache, ref_progress)
    };
    let _ = reporter.join();

    // Build hash set from reference
    let ref_hashes: HashSet<String> = ref_hashed.iter().map(|f| f.hash.clone()).collect();

    // Phase 2a: Collect eval files
    emit_progress(app, "Collecting eval files...", 0, 0);
    let eval_files = hasher::collect_files(eval_dir);
    let eval_total = eval_files.len();

    // Phase 2b: Hash eval folder
    let eval_progress = Arc::new(AtomicUsize::new(0));
    let reporter = spawn_progress_reporter(
        app.clone(),
        "Checking eval folder...".to_string(),
        eval_progress.clone(),
        eval_total,
    );

    let eval_hashed = {
        let cache = HashCache::open()?;
        hasher::hash_files_cached(&eval_files, &cache, eval_progress)
    };
    let _ = reporter.join();

    // Detect intra-eval duplicates: track which hashes we've already seen
    emit_progress(app, "Comparing files...", 0, 0);
    let mut seen_eval_hashes: HashMap<String, usize> = HashMap::new();

    let mut duplicates = Vec::new();
    let mut uniques = Vec::new();

    for (idx, ef) in eval_hashed.iter().enumerate() {
        let is_ref_dupe = ref_hashes.contains(&ef.hash);
        let is_intra_dupe = seen_eval_hashes.contains_key(&ef.hash);
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

        if !is_ref_dupe {
            seen_eval_hashes.entry(ef.hash.clone()).or_insert(idx);
        }
    }

    Ok(ScanResult {
        total_eval: eval_hashed.len(),
        duplicates,
        uniques,
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
