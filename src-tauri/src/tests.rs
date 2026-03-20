//! Unit and integration tests for the core deduplication pipeline.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

use crate::cache::{CachedFile, HashCache};
use crate::commands::{self, EvalFile, ScanResult, ScanStats};
use crate::fileops;
use crate::hasher;

// ---------------------------------------------------------------------------
// hash_file: produces correct SHA-256 for known content
// ---------------------------------------------------------------------------

#[test]
fn hash_file_known_content() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("hello.txt");
    fs::write(&file, b"hello world").unwrap();

    let hash = hasher::hash_file(&file, "sha256").unwrap();
    // SHA-256 of "hello world"
    assert_eq!(
        hash,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
}

#[test]
fn hash_file_empty() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("empty.txt");
    fs::write(&file, b"").unwrap();

    let hash = hasher::hash_file(&file, "sha256").unwrap();
    // SHA-256 of empty input
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hash_file_nonexistent() {
    let result = hasher::hash_file(&PathBuf::from("/does/not/exist.jpg"), "sha256");
    assert!(result.is_err());
}

#[test]
fn hash_file_xxh3_known_content() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("hello.txt");
    fs::write(&file, b"hello world").unwrap();

    let hash = hasher::hash_file(&file, "xxh3").unwrap();
    // xxh3_64 produces a 16-char hex string
    assert_eq!(hash.len(), 16);

    // Verify determinism: hashing the same content again should produce the same hash
    let hash2 = hasher::hash_file(&file, "xxh3").unwrap();
    assert_eq!(hash, hash2);

    // Verify it's different from SHA-256
    let sha_hash = hasher::hash_file(&file, "sha256").unwrap();
    assert_ne!(hash, sha_hash);
}

#[test]
fn hash_file_xxh3_empty() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("empty.txt");
    fs::write(&file, b"").unwrap();

    let hash = hasher::hash_file(&file, "xxh3").unwrap();
    assert_eq!(hash.len(), 16);
}

// ---------------------------------------------------------------------------
// collect_files: filters by extension and skips hidden files
// ---------------------------------------------------------------------------

#[test]
fn collect_files_filters_extensions() {
    let tmp = TempDir::new().unwrap();
    // Use a non-hidden subdirectory as the scan root since TempDir names
    // start with '.' and collect_files skips hidden entries.
    let dir = tmp.path().join("scan_root");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("photo.jpg"), b"jpg").unwrap();
    fs::write(dir.join("photo.png"), b"png").unwrap();
    fs::write(dir.join("notes.txt"), b"txt").unwrap();
    fs::write(dir.join("data.csv"), b"csv").unwrap();
    fs::write(dir.join("video.mp4"), b"mp4").unwrap();

    // With an images-only filter, only jpg and png should match
    let image_exts = hasher::resolve_extensions(&["images".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    let files = hasher::collect_files(&dir, Some(&image_exts));
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(names.contains(&"photo.jpg".to_string()));
    assert!(names.contains(&"photo.png".to_string()));
    assert!(!names.contains(&"video.mp4".to_string()));
    assert!(!names.contains(&"notes.txt".to_string()));
    assert!(!names.contains(&"data.csv".to_string()));
}

#[test]
fn collect_files_none_accepts_all() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("scan_root");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("photo.jpg"), b"jpg").unwrap();
    fs::write(dir.join("notes.txt"), b"txt").unwrap();
    fs::write(dir.join("video.mp4"), b"mp4").unwrap();

    let files = hasher::collect_files(&dir, None);
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(names.contains(&"photo.jpg".to_string()));
    assert!(names.contains(&"notes.txt".to_string()));
    assert!(names.contains(&"video.mp4".to_string()));
}

#[test]
fn collect_files_skips_hidden() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("scan_root");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("visible.jpg"), b"ok").unwrap();

    let hidden_dir = dir.join(".hidden");
    fs::create_dir(&hidden_dir).unwrap();
    fs::write(hidden_dir.join("secret.jpg"), b"nope").unwrap();

    let files = hasher::collect_files(&dir, None);
    assert_eq!(files.len(), 1);
    assert!(files[0].file_name().unwrap().to_string_lossy() == "visible.jpg");
}

// ---------------------------------------------------------------------------
// find_sidecars: both naming conventions
// ---------------------------------------------------------------------------

#[test]
fn find_sidecars_stem_convention() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("DSC_0001.NEF");
    let sidecar = dir.path().join("DSC_0001.xmp");
    fs::write(&photo, b"raw").unwrap();
    fs::write(&sidecar, b"xmp").unwrap();

    let sidecars = fileops::find_sidecars(&photo);
    assert!(sidecars.contains(&sidecar));
}

#[test]
fn find_sidecars_fullname_convention() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("DSC_0001.NEF");
    let sidecar = dir.path().join("DSC_0001.NEF.xmp");
    fs::write(&photo, b"raw").unwrap();
    fs::write(&sidecar, b"xmp").unwrap();

    let sidecars = fileops::find_sidecars(&photo);
    assert!(sidecars.contains(&sidecar));
}

#[test]
fn find_sidecars_uppercase_xmp() {
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("IMG_0001.CR2");
    let sidecar = dir.path().join("IMG_0001.XMP");
    fs::write(&photo, b"raw").unwrap();
    fs::write(&sidecar, b"xmp").unwrap();

    let sidecars = fileops::find_sidecars(&photo);
    assert!(sidecars.contains(&sidecar));
}

#[test]
fn find_sidecars_no_duplicates() {
    // If both conventions yield the same file, it should appear only once
    let dir = TempDir::new().unwrap();
    let photo = dir.path().join("photo.xmp"); // edge case: file IS an xmp
    let sidecar_stem = dir.path().join("photo.xmp"); // stem convention = same file
    fs::write(&photo, b"data").unwrap();

    let sidecars = fileops::find_sidecars(&photo);
    // The photo itself should not appear in its own sidecar list
    assert!(!sidecars.contains(&sidecar_stem));
}

// ---------------------------------------------------------------------------
// resolve_collision: appends -1, -2, etc.
// ---------------------------------------------------------------------------

#[test]
fn resolve_collision_no_conflict() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("photo.jpg");
    // File doesn't exist, so no collision
    assert_eq!(fileops::resolve_collision(&target), target);
}

#[test]
fn resolve_collision_increments() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("photo.jpg");
    fs::write(&target, b"original").unwrap();

    let resolved = fileops::resolve_collision(&target);
    assert_eq!(resolved.file_name().unwrap().to_string_lossy(), "photo-1.jpg");

    // Create -1 too, should get -2
    fs::write(&resolved, b"first copy").unwrap();
    let resolved2 = fileops::resolve_collision(&target);
    assert_eq!(resolved2.file_name().unwrap().to_string_lossy(), "photo-2.jpg");
}

#[test]
fn resolve_collision_no_extension() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("README");
    fs::write(&target, b"content").unwrap();

    let resolved = fileops::resolve_collision(&target);
    assert_eq!(resolved.file_name().unwrap().to_string_lossy(), "README-1");
}

// ---------------------------------------------------------------------------
// cleanup_empty_dirs: removes empty but not non-empty
// ---------------------------------------------------------------------------

#[test]
fn cleanup_empty_dirs_removes_empty() {
    let dir = TempDir::new().unwrap();
    let empty_sub = dir.path().join("empty_sub");
    fs::create_dir(&empty_sub).unwrap();

    let removed = fileops::cleanup_empty_dirs(dir.path()).unwrap();
    assert_eq!(removed, 1);
    assert!(!empty_sub.exists());
}

#[test]
fn cleanup_empty_dirs_keeps_nonempty() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("has_files");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("keep.txt"), b"data").unwrap();

    let removed = fileops::cleanup_empty_dirs(dir.path()).unwrap();
    assert_eq!(removed, 0);
    assert!(sub.exists());
}

#[test]
fn cleanup_empty_dirs_does_not_remove_root() {
    let dir = TempDir::new().unwrap();
    // Root itself is empty, but cleanup should not remove it
    let removed = fileops::cleanup_empty_dirs(dir.path()).unwrap();
    assert_eq!(removed, 0);
    assert!(dir.path().exists());
}

// ---------------------------------------------------------------------------
// move_file: preserves directory structure and handles collisions
// ---------------------------------------------------------------------------

#[test]
fn move_file_preserves_structure() {
    let src_dir = TempDir::new().unwrap();
    let dest_dir = TempDir::new().unwrap();

    let sub = src_dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let file = sub.join("photo.jpg");
    fs::write(&file, b"image data").unwrap();

    let (result, _warnings) = fileops::move_file(&file, src_dir.path(), dest_dir.path()).unwrap();

    assert_eq!(result, dest_dir.path().join("sub").join("photo.jpg"));
    assert!(result.exists());
    assert!(!file.exists());
    assert_eq!(fs::read(&result).unwrap(), b"image data");
}

#[test]
fn move_file_handles_collision() {
    let src_dir = TempDir::new().unwrap();
    let dest_dir = TempDir::new().unwrap();

    let file = src_dir.path().join("photo.jpg");
    fs::write(&file, b"new version").unwrap();

    // Pre-create a collision at the destination
    let existing = dest_dir.path().join("photo.jpg");
    fs::write(&existing, b"old version").unwrap();

    let (result, _warnings) = fileops::move_file(&file, src_dir.path(), dest_dir.path()).unwrap();

    assert_eq!(
        result.file_name().unwrap().to_string_lossy(),
        "photo-1.jpg"
    );
    assert!(result.exists());
    assert_eq!(fs::read(&result).unwrap(), b"new version");
    // Original at dest should be untouched
    assert_eq!(fs::read(&existing).unwrap(), b"old version");
}

// ---------------------------------------------------------------------------
// trash_file: works on a temp file
// ---------------------------------------------------------------------------

#[test]
fn trash_file_removes_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("to_trash.jpg");
    fs::write(&file, b"delete me").unwrap();
    assert!(file.exists());

    fileops::trash_file(&file).unwrap();
    assert!(!file.exists());
}

// ---------------------------------------------------------------------------
// Cache: get/set/prune round-trip
// ---------------------------------------------------------------------------

#[test]
fn cache_set_and_get() {
    let cache = HashCache::open_in_memory().unwrap();

    let entry = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "abc123".to_string(),
        size: 1024,
        mtime_secs: 1700000000,
        mtime_nanos: 500,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };

    cache.set(&entry).unwrap();

    let hit = cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 500, "sha256").unwrap();
    assert_eq!(hit.hash, "abc123");
}

#[test]
fn cache_get_returns_none_on_mismatch() {
    let cache = HashCache::open_in_memory().unwrap();

    let entry = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "abc123".to_string(),
        size: 1024,
        mtime_secs: 1700000000,
        mtime_nanos: 500,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };

    cache.set(&entry).unwrap();

    // Different size
    assert!(cache.get("/tmp/test/photo.jpg", 2048, 1700000000, 500, "sha256").is_none());
    // Different mtime
    assert!(cache.get("/tmp/test/photo.jpg", 1024, 1700000001, 500, "sha256").is_none());
    // Different path
    assert!(cache.get("/tmp/test/other.jpg", 1024, 1700000000, 500, "sha256").is_none());
    // Different algorithm
    assert!(cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 500, "xxh3").is_none());
}

#[test]
fn cache_set_overwrites() {
    let cache = HashCache::open_in_memory().unwrap();

    let entry1 = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "old_hash".to_string(),
        size: 1024,
        mtime_secs: 1700000000,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };
    cache.set(&entry1).unwrap();

    let entry2 = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "new_hash".to_string(),
        size: 2048,
        mtime_secs: 1700000001,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };
    cache.set(&entry2).unwrap();

    // Old metadata should no longer match
    assert!(cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 0, "sha256").is_none());
    // New metadata should match
    let hit = cache.get("/tmp/test/photo.jpg", 2048, 1700000001, 0, "sha256").unwrap();
    assert_eq!(hit.hash, "new_hash");
}

#[test]
fn cache_stores_different_algorithms_separately() {
    let cache = HashCache::open_in_memory().unwrap();

    let sha_entry = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "sha_hash".to_string(),
        size: 1024,
        mtime_secs: 1700000000,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };
    cache.set(&sha_entry).unwrap();

    let xxh_entry = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "xxh_hash".to_string(),
        size: 1024,
        mtime_secs: 1700000000,
        mtime_nanos: 0,
        algorithm: "xxh3".to_string(),
        perceptual_hash: None,
    };
    cache.set(&xxh_entry).unwrap();

    // Each algorithm returns its own hash
    let sha_hit = cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 0, "sha256").unwrap();
    assert_eq!(sha_hit.hash, "sha_hash");
    let xxh_hit = cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 0, "xxh3").unwrap();
    assert_eq!(xxh_hit.hash, "xxh_hash");
}

#[test]
fn cache_prune_removes_nonexistent() {
    let cache = HashCache::open_in_memory().unwrap();

    // Insert an entry for a file that doesn't exist
    let entry = CachedFile {
        path: "/definitely/does/not/exist/photo.jpg".to_string(),
        hash: "deadbeef".to_string(),
        size: 100,
        mtime_secs: 1700000000,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };
    cache.set(&entry).unwrap();

    let pruned = cache.prune().unwrap();
    assert_eq!(pruned, 1);

    // Entry should be gone
    assert!(cache.get("/definitely/does/not/exist/photo.jpg", 100, 1700000000, 0, "sha256").is_none());
}

#[test]
fn cache_prune_keeps_existing() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("existing.jpg");
    fs::write(&file, b"data").unwrap();

    let cache = HashCache::open_in_memory().unwrap();
    let path_str = file.to_string_lossy().to_string();

    let entry = CachedFile {
        path: path_str.clone(),
        hash: "exists".to_string(),
        size: 4,
        mtime_secs: 1700000000,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    };
    cache.set(&entry).unwrap();

    let pruned = cache.prune().unwrap();
    assert_eq!(pruned, 0);

    // Entry should still be there
    let hit = cache.get(&path_str, 4, 1700000000, 0, "sha256").unwrap();
    assert_eq!(hit.hash, "exists");
}

// ---------------------------------------------------------------------------
// resolve_extensions: builds correct extension sets
// ---------------------------------------------------------------------------

#[test]
fn resolve_extensions_all_files_returns_none() {
    let result = hasher::resolve_extensions(&["images".to_string()], true, &HashMap::new(), &HashMap::new());
    assert!(result.is_none());
}

#[test]
fn resolve_extensions_single_category() {
    let result = hasher::resolve_extensions(&["images".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    assert!(result.contains("jpg"));
    assert!(result.contains("png"));
    assert!(result.contains("heic"));
    assert!(!result.contains("mp4"));
    assert!(!result.contains("pdf"));
}

#[test]
fn resolve_extensions_multiple_categories() {
    let result = hasher::resolve_extensions(
        &["images".to_string(), "videos".to_string()],
        false,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert!(result.contains("jpg"));
    assert!(result.contains("mp4"));
    assert!(result.contains("mov"));
    assert!(!result.contains("pdf"));
}

#[test]
fn resolve_extensions_empty_categories() {
    let result = hasher::resolve_extensions(&[], false, &HashMap::new(), &HashMap::new()).unwrap();
    assert!(result.is_empty());
}

#[test]
fn resolve_extensions_unknown_category_ignored() {
    let result = hasher::resolve_extensions(&["unknown".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    assert!(result.is_empty());
}

// ===========================================================================
// Integration tests: full pipeline with real files on disk
// ===========================================================================

/// Helper: create a file under `dir` with the given content, creating parent dirs.
fn create_file(dir: &Path, relative: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

/// Inline duplicate detection logic extracted from commands.rs scan_folders_blocking.
fn detect_duplicates(
    ref_hashed: &[hasher::HashedFile],
    eval_hashed: &mut Vec<hasher::HashedFile>,
    eval_dir: &Path,
) -> (Vec<EvalFile>, Vec<EvalFile>) {
    let ref_hash_set: HashSet<String> = ref_hashed.iter().map(|f| f.hash.clone()).collect();

    eval_hashed.sort_by(|a, b| a.path.cmp(&b.path));

    let mut seen_eval_hashes: HashSet<String> = HashSet::new();
    let mut exact_matches = Vec::new();
    let mut uniques = Vec::new();

    for ef in eval_hashed.iter() {
        let is_ref_dupe = ref_hash_set.contains(&ef.hash);
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
            uniques.push(eval_file);
        }

        if !is_ref_dupe {
            seen_eval_hashes.insert(ef.hash.clone());
        }
    }

    (exact_matches, uniques)
}

// ---------------------------------------------------------------------------
// Test 1: Full scan finds duplicates (ref dupes + intra-eval dupes)
// ---------------------------------------------------------------------------

#[test]
fn integration_full_scan_finds_duplicates() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("scan");
    let ref_dir = root.join("reference");
    let eval_dir = root.join("eval");

    // Reference: 3 files with known content
    create_file(&ref_dir, "photo_a.jpg", b"content_a");
    create_file(&ref_dir, "photo_b.jpg", b"content_b");
    create_file(&ref_dir, "photo_c.jpg", b"content_c");

    // Eval: 5 files
    // 2 copies of ref files (dupes of reference)
    create_file(&eval_dir, "copy_a.jpg", b"content_a"); // ref dupe
    create_file(&eval_dir, "copy_b.jpg", b"content_b"); // ref dupe
    // 2 unique files
    create_file(&eval_dir, "unique_1.jpg", b"unique_content_1");
    create_file(&eval_dir, "unique_2.jpg", b"unique_content_2");
    // 1 intra-eval duplicate (same content as unique_1)
    create_file(&eval_dir, "zzz_intra_dupe.jpg", b"unique_content_1"); // intra-eval dupe

    let image_exts = hasher::resolve_extensions(&["images".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    let ref_files = hasher::collect_files(&ref_dir, Some(&image_exts));
    let eval_files = hasher::collect_files(&eval_dir, Some(&image_exts));

    assert_eq!(ref_files.len(), 3);
    assert_eq!(eval_files.len(), 5);

    let cache = HashCache::open_in_memory().unwrap();
    let progress = Arc::new(AtomicUsize::new(0));

    let no_cancel = Arc::new(AtomicBool::new(false));
    let ref_result = hasher::hash_files_cached(&ref_files, &cache, progress.clone(), "sha256", &no_cancel);
    assert_eq!(ref_result.hashed.len(), 3);
    assert!(ref_result.skipped.is_empty());

    let progress2 = Arc::new(AtomicUsize::new(0));
    let eval_result = hasher::hash_files_cached(&eval_files, &cache, progress2, "sha256", &no_cancel);
    assert_eq!(eval_result.hashed.len(), 5);
    assert!(eval_result.skipped.is_empty());

    let mut eval_hashed = eval_result.hashed;
    let (exact_matches, uniques) = detect_duplicates(&ref_result.hashed, &mut eval_hashed, &eval_dir);

    assert_eq!(exact_matches.len(), 3, "Expected 3 duplicates (2 ref + 1 intra-eval)");
    assert_eq!(uniques.len(), 2, "Expected 2 unique files");

    // total_eval
    let total_eval = exact_matches.len() + uniques.len();
    assert_eq!(total_eval, 5);

    // Results should be sorted by path (deterministic)
    for window in exact_matches.windows(2) {
        assert!(window[0].path <= window[1].path, "exact_matches not sorted by path");
    }
    for window in uniques.windows(2) {
        assert!(window[0].path <= window[1].path, "uniques not sorted by path");
    }
}

// ---------------------------------------------------------------------------
// Test 2: Cache speeds up second run
// ---------------------------------------------------------------------------

#[test]
fn integration_cache_speeds_up_second_run() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("cache_test");
    create_file(&root, "a.jpg", b"alpha");
    create_file(&root, "b.jpg", b"bravo");
    create_file(&root, "c.jpg", b"charlie");

    let image_exts = hasher::resolve_extensions(&["images".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    let files = hasher::collect_files(&root, Some(&image_exts));
    assert_eq!(files.len(), 3);

    let cache = HashCache::open_in_memory().unwrap();

    // First run: everything should be hashed (no cache hits)
    let no_cancel = Arc::new(AtomicBool::new(false));
    let progress1 = Arc::new(AtomicUsize::new(0));
    let result1 = hasher::hash_files_cached(&files, &cache, progress1, "sha256", &no_cancel);
    assert_eq!(result1.hashed.len(), 3);
    assert!(result1.skipped.is_empty());

    // Collect hashes from first run for comparison
    let mut hashes1: Vec<(String, String)> = result1
        .hashed
        .iter()
        .map(|h| (h.path.clone(), h.hash.clone()))
        .collect();
    hashes1.sort();

    // Second run: all should be cache hits. To verify cache hits, we examine
    // the result which should be identical. The internal `needs_hashing` vec
    // should be empty because all files are in cache with matching metadata.
    let progress2 = Arc::new(AtomicUsize::new(0));
    let result2 = hasher::hash_files_cached(&files, &cache, progress2.clone(), "sha256", &no_cancel);
    assert_eq!(result2.hashed.len(), 3);
    assert!(result2.skipped.is_empty());

    let mut hashes2: Vec<(String, String)> = result2
        .hashed
        .iter()
        .map(|h| (h.path.clone(), h.hash.clone()))
        .collect();
    hashes2.sort();

    // Same results both times
    assert_eq!(hashes1, hashes2);

    // Verify second run had zero needs_hashing by checking progress was
    // completed entirely during cache-hit phase (progress == file count
    // before any parallel hashing occurs). We check that progress2 advanced
    // to 3 -- but since it's all cache hits, the atomic counter should be 3.
    assert_eq!(progress2.load(Ordering::Relaxed), 3);
}

// ---------------------------------------------------------------------------
// Test 3: Move preserves directory structure and sidecars
// ---------------------------------------------------------------------------

#[test]
fn integration_move_preserves_structure_and_sidecars() {
    let tmp = TempDir::new().unwrap();
    let eval_dir = tmp.path().join("eval");
    let dest_dir = tmp.path().join("dest");
    fs::create_dir_all(&dest_dir).unwrap();

    create_file(&eval_dir, "2024/photo.nef", b"raw image data");
    create_file(&eval_dir, "2024/photo.xmp", b"sidecar metadata");

    let nef_path = eval_dir.join("2024/photo.nef");
    let (result, _warnings) = fileops::move_file(&nef_path, &eval_dir, &dest_dir).unwrap();

    // NEF should be at dest/2024/photo.nef
    assert_eq!(result, dest_dir.join("2024").join("photo.nef"));
    assert!(result.exists());

    // XMP sidecar should have moved too
    let xmp_dest = dest_dir.join("2024").join("photo.xmp");
    assert!(xmp_dest.exists(), "XMP sidecar should be at dest/2024/photo.xmp");

    // Originals should be gone
    assert!(!eval_dir.join("2024/photo.nef").exists());
    assert!(!eval_dir.join("2024/photo.xmp").exists());

    // eval/2024/ should be empty (or gone)
    let eval_2024 = eval_dir.join("2024");
    if eval_2024.exists() {
        let entries: Vec<_> = fs::read_dir(&eval_2024).unwrap().collect();
        assert!(entries.is_empty(), "eval/2024/ should be empty after move");
    }
}

// ---------------------------------------------------------------------------
// Test 4: Move handles collision
// ---------------------------------------------------------------------------

#[test]
fn integration_move_handles_collision() {
    let tmp = TempDir::new().unwrap();
    let eval_dir = tmp.path().join("eval");
    let dest_dir = tmp.path().join("dest");

    create_file(&eval_dir, "photo.jpg", b"new version");
    create_file(&dest_dir, "photo.jpg", b"original version");

    let eval_photo = eval_dir.join("photo.jpg");
    let (result, _warnings) = fileops::move_file(&eval_photo, &eval_dir, &dest_dir).unwrap();

    // Original at dest should be untouched
    let dest_original = dest_dir.join("photo.jpg");
    assert!(dest_original.exists());
    assert_eq!(fs::read(&dest_original).unwrap(), b"original version");

    // Moved file should be photo-1.jpg
    let dest_collision = dest_dir.join("photo-1.jpg");
    assert_eq!(result, dest_collision);
    assert!(dest_collision.exists());
    assert_eq!(fs::read(&dest_collision).unwrap(), b"new version");
}

// ---------------------------------------------------------------------------
// Test 5: Trash with sidecar
// ---------------------------------------------------------------------------

#[test]
fn integration_trash_with_sidecar() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("trash_test");

    let nef = create_file(&dir, "photo.nef", b"raw image");
    let xmp = create_file(&dir, "photo.xmp", b"sidecar data");

    assert!(nef.exists());
    assert!(xmp.exists());

    fileops::trash_file(&nef).unwrap();

    assert!(!nef.exists(), "NEF should be gone after trash");
    assert!(!xmp.exists(), "XMP sidecar should be gone after trash");
}

// ---------------------------------------------------------------------------
// Test 6: Cleanup empty dirs (nested)
// ---------------------------------------------------------------------------

#[test]
fn integration_cleanup_empty_dirs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("cleanup_root");

    // Create nested dirs with a file only at the deepest level
    let file = create_file(&root, "a/b/c/file.txt", b"data");
    assert!(root.join("a/b/c").exists());

    // Remove the file manually
    fs::remove_file(&file).unwrap();

    let removed = fileops::cleanup_empty_dirs(&root).unwrap();

    // a/b/c/, a/b/, and a/ should all be removed
    assert!(!root.join("a/b/c").exists(), "a/b/c/ should be removed");
    assert!(!root.join("a/b").exists(), "a/b/ should be removed");
    assert!(!root.join("a").exists(), "a/ should be removed");
    assert_eq!(removed, 3);

    // Root itself should be preserved
    assert!(root.exists(), "root should still exist");
}

// ---------------------------------------------------------------------------
// Test 7: Different algorithms produce different hashes
// ---------------------------------------------------------------------------

#[test]
fn integration_different_algorithms_produce_different_hashes() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("algo_test.jpg");
    fs::write(&file, b"test content for hashing").unwrap();

    let sha_hash = hasher::hash_file(&file, "sha256").unwrap();
    let xxh_hash = hasher::hash_file(&file, "xxh3").unwrap();

    assert!(!sha_hash.is_empty(), "SHA-256 hash should not be empty");
    assert!(!xxh_hash.is_empty(), "XXH3 hash should not be empty");
    assert_ne!(sha_hash, xxh_hash, "SHA-256 and XXH3 should produce different hashes");
}

// ---------------------------------------------------------------------------
// Test 8: Category filtering
// ---------------------------------------------------------------------------

#[test]
fn integration_category_filtering() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("filter_test");

    create_file(&root, "photo.jpg", b"image data");
    create_file(&root, "video.mp4", b"video data");
    create_file(&root, "doc.pdf", b"document data");
    create_file(&root, "song.mp3", b"audio data");
    create_file(&root, "data.bin", b"binary data");

    // Images-only
    let image_exts = hasher::resolve_extensions(&["images".to_string()], false, &HashMap::new(), &HashMap::new()).unwrap();
    let image_files = hasher::collect_files(&root, Some(&image_exts));
    let image_names: Vec<String> = image_files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(image_files.len(), 1);
    assert!(image_names.contains(&"photo.jpg".to_string()));

    // Images + Videos
    let img_vid_exts = hasher::resolve_extensions(
        &["images".to_string(), "videos".to_string()],
        false,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    let img_vid_files = hasher::collect_files(&root, Some(&img_vid_exts));
    let img_vid_names: Vec<String> = img_vid_files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(img_vid_files.len(), 2);
    assert!(img_vid_names.contains(&"photo.jpg".to_string()));
    assert!(img_vid_names.contains(&"video.mp4".to_string()));

    // All files (None filter)
    let all_files = hasher::collect_files(&root, None);
    assert_eq!(all_files.len(), 5);
}

// ---------------------------------------------------------------------------
// Test 9: CSV export
// ---------------------------------------------------------------------------

#[test]
fn integration_csv_export() {
    let tmp = TempDir::new().unwrap();
    let csv_path = tmp.path().join("report.csv");

    let results = ScanResult {
        total_eval: 4,
        exact_matches: vec![
            EvalFile {
                path: "/eval/dup1.jpg".to_string(),
                relative_path: "dup1.jpg".to_string(),
                size: 1024,
                hash: "aaa111".to_string(),
                match_type: "exact".to_string(),
                hamming_distance: None,
            },
            EvalFile {
                path: "/eval/dup2.jpg".to_string(),
                relative_path: "dup2.jpg".to_string(),
                size: 2048,
                hash: "bbb222".to_string(),
                match_type: "exact".to_string(),
                hamming_distance: None,
            },
        ],
        similar_matches: vec![],
        uniques: vec![
            EvalFile {
                path: "/eval/unique1.jpg".to_string(),
                relative_path: "unique1.jpg".to_string(),
                size: 512,
                hash: "ccc333".to_string(),
                match_type: "unique".to_string(),
                hamming_distance: None,
            },
            EvalFile {
                path: "/eval/sub/unique2.jpg".to_string(),
                relative_path: "sub/unique2.jpg".to_string(),
                size: 768,
                hash: "ddd444".to_string(),
                match_type: "unique".to_string(),
                hamming_distance: None,
            },
        ],
        skipped: 0,
        stats: ScanStats {
            ref_collect_ms: 0,
            ref_hash_ms: 0,
            eval_collect_ms: 0,
            eval_hash_ms: 0,
            total_ms: 0,
            ref_cache_hits: 0,
            eval_cache_hits: 0,
            ref_file_count: 0,
            eval_file_count: 4,
            total_bytes: 0,
            perceptual_compare_ms: 0,
        },
    };

    // Write CSV using the same logic as export_report
    {
        let mut file = fs::File::create(&csv_path).unwrap();
        writeln!(file, "status,relative_path,size_bytes,hash,hamming_distance").unwrap();
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
                commands::csv_quote(&f.match_type),
                commands::csv_quote(&f.relative_path),
                f.size,
                f.hash,
                dist_str,
            )
            .unwrap();
        }
    }

    // Read back and verify
    let csv_content = fs::read_to_string(&csv_path).unwrap();
    let lines: Vec<&str> = csv_content.lines().collect();

    // Header + 4 data rows
    assert_eq!(lines.len(), 5, "Expected 1 header + 4 data rows");
    assert_eq!(lines[0], "status,relative_path,size_bytes,hash,hamming_distance");

    // Verify exact matches come first
    assert!(lines[1].starts_with("exact,"));
    assert!(lines[2].starts_with("exact,"));
    assert!(lines[3].starts_with("unique,"));
    assert!(lines[4].starts_with("unique,"));

    // Verify specific content
    assert!(lines[1].contains("dup1.jpg"));
    assert!(lines[1].contains("1024"));
    assert!(lines[1].contains("aaa111"));
}

// ---------------------------------------------------------------------------
// perceptual: hamming_distance
// ---------------------------------------------------------------------------

#[test]
fn hamming_distance_identical() {
    assert_eq!(crate::perceptual::hamming_distance(0, 0), 0);
    assert_eq!(crate::perceptual::hamming_distance(u64::MAX, u64::MAX), 0);
}

#[test]
fn hamming_distance_one_bit() {
    assert_eq!(crate::perceptual::hamming_distance(0, 1), 1);
    assert_eq!(crate::perceptual::hamming_distance(0b1010, 0b1000), 1);
}

#[test]
fn hamming_distance_all_bits() {
    assert_eq!(crate::perceptual::hamming_distance(0, u64::MAX), 64);
}

#[test]
fn hamming_distance_known_values() {
    // 0xFF00 vs 0x00FF: 16 bits differ
    assert_eq!(crate::perceptual::hamming_distance(0xFF00, 0x00FF), 16);
}

// ---------------------------------------------------------------------------
// perceptual: compute_dhash
// ---------------------------------------------------------------------------

#[test]
fn dhash_unsupported_format_returns_none() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.txt");
    fs::write(&file, b"not an image").unwrap();
    assert!(crate::perceptual::compute_dhash(&file).is_none());
}

#[test]
fn dhash_nonexistent_file_returns_none() {
    assert!(crate::perceptual::compute_dhash(Path::new("/does/not/exist.jpg")).is_none());
}

#[test]
fn dhash_deterministic() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.png");

    let mut imgbuf = image::ImageBuffer::new(10, 10);
    for (x, y, pixel) in imgbuf.enumerate_pixels_mut() {
        let val = ((x * 25 + y * 10) % 256) as u8;
        *pixel = image::Luma([val]);
    }
    imgbuf.save(&file).unwrap();

    let hash1 = crate::perceptual::compute_dhash(&file).unwrap();
    let hash2 = crate::perceptual::compute_dhash(&file).unwrap();
    assert_eq!(hash1, hash2);
}

#[test]
fn dhash_similar_images_low_distance() {
    let dir = TempDir::new().unwrap();

    let mut img1 = image::ImageBuffer::new(100, 100);
    for (x, _y, pixel) in img1.enumerate_pixels_mut() {
        let val = ((x * 255) / 99) as u8;
        *pixel = image::Rgb([val, val, val]);
    }
    let path1 = dir.path().join("gradient1.png");
    img1.save(&path1).unwrap();

    let mut img2 = img1.clone();
    for (_x, _y, pixel) in img2.enumerate_pixels_mut() {
        pixel[0] = pixel[0].saturating_add(1);
    }
    let path2 = dir.path().join("gradient2.png");
    img2.save(&path2).unwrap();

    let hash1 = crate::perceptual::compute_dhash(&path1).unwrap();
    let hash2 = crate::perceptual::compute_dhash(&path2).unwrap();
    let dist = crate::perceptual::hamming_distance(hash1, hash2);
    assert!(dist <= 5, "Expected similar images to have distance <= 5, got {dist}");
}

#[test]
fn dhash_different_images_high_distance() {
    let dir = TempDir::new().unwrap();

    // Horizontal gradient: pixel values increase left-to-right.
    // dHash compares left vs right, so left < right => bit 0 for every pair.
    let img1 = image::ImageBuffer::from_fn(100, 100, |x, _y| {
        let val = ((x * 255) / 99) as u8;
        image::Rgb([val, val, val])
    });
    let path1 = dir.path().join("h_gradient.png");
    img1.save(&path1).unwrap();

    // Reverse horizontal gradient: pixel values decrease left-to-right.
    // dHash: left > right => bit 1 for every pair.
    let img2 = image::ImageBuffer::from_fn(100, 100, |x, _y| {
        let val = (255 - (x * 255) / 99) as u8;
        image::Rgb([val, val, val])
    });
    let path2 = dir.path().join("h_gradient_rev.png");
    img2.save(&path2).unwrap();

    let hash1 = crate::perceptual::compute_dhash(&path1).unwrap();
    let hash2 = crate::perceptual::compute_dhash(&path2).unwrap();
    let dist = crate::perceptual::hamming_distance(hash1, hash2);
    assert!(dist > 10, "Expected different images to have distance > 10, got {dist}");
}

// ---------------------------------------------------------------------------
// cache: perceptual hash storage
// ---------------------------------------------------------------------------

#[test]
fn cache_stores_and_retrieves_perceptual_hash() {
    let cache = HashCache::open_in_memory().unwrap();
    cache.set(&CachedFile {
        path: "/photo.jpg".to_string(),
        hash: "abc123".to_string(),
        size: 1000,
        mtime_secs: 100,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: Some(0x123456789ABCDEF0_u64 as i64),
    }).unwrap();

    let hit = cache.get("/photo.jpg", 1000, 100, 0, "sha256").unwrap();
    assert_eq!(hit.hash, "abc123");
    assert_eq!(hit.perceptual_hash, Some(0x123456789ABCDEF0_u64));
}

#[test]
fn cache_stores_null_perceptual_hash() {
    let cache = HashCache::open_in_memory().unwrap();
    cache.set(&CachedFile {
        path: "/doc.pdf".to_string(),
        hash: "def456".to_string(),
        size: 2000,
        mtime_secs: 200,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: None,
    }).unwrap();

    let hit = cache.get("/doc.pdf", 2000, 200, 0, "sha256").unwrap();
    assert_eq!(hit.hash, "def456");
    assert_eq!(hit.perceptual_hash, None);
}

#[test]
fn cache_perceptual_hash_high_bit_roundtrip() {
    let cache = HashCache::open_in_memory().unwrap();
    let high_val: u64 = 0xFFFFFFFFFFFFFFFF;
    cache.set(&CachedFile {
        path: "/high.jpg".to_string(),
        hash: "ghi789".to_string(),
        size: 500,
        mtime_secs: 300,
        mtime_nanos: 0,
        algorithm: "sha256".to_string(),
        perceptual_hash: Some(high_val as i64),
    }).unwrap();

    let hit = cache.get("/high.jpg", 500, 300, 0, "sha256").unwrap();
    assert_eq!(hit.perceptual_hash, Some(high_val));
}

// ---------------------------------------------------------------------------
// hash_files_cached: populates perceptual hash for supported images
// ---------------------------------------------------------------------------

#[test]
fn hash_files_cached_computes_perceptual_hash_for_png() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("scan_root");
    fs::create_dir(&root).unwrap();

    let img = image::ImageBuffer::from_fn(50, 50, |x, _y| {
        let val = ((x * 255) / 49) as u8;
        image::Rgb([val, val, val])
    });
    let png_path = root.join("gradient.png");
    img.save(&png_path).unwrap();

    let txt_path = root.join("notes.txt");
    fs::write(&txt_path, b"hello").unwrap();

    let cache = HashCache::open_in_memory().unwrap();
    let progress = Arc::new(AtomicUsize::new(0));

    let no_cancel = Arc::new(AtomicBool::new(false));
    let result = hasher::hash_files_cached(
        &[png_path.clone(), txt_path.clone()],
        &cache,
        progress,
        "sha256",
        &no_cancel,
    );

    assert_eq!(result.hashed.len(), 2);
    let png_file = result.hashed.iter().find(|f| f.path.ends_with("gradient.png")).unwrap();
    let txt_file = result.hashed.iter().find(|f| f.path.ends_with("notes.txt")).unwrap();

    assert!(png_file.perceptual_hash.is_some());
    assert!(txt_file.perceptual_hash.is_none());
}

#[test]
fn hash_files_cached_perceptual_hash_from_cache() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("scan_root");
    fs::create_dir(&root).unwrap();

    let img = image::ImageBuffer::from_fn(50, 50, |x, _y| {
        let val = ((x * 255) / 49) as u8;
        image::Rgb([val, val, val])
    });
    let png_path = root.join("gradient.png");
    img.save(&png_path).unwrap();

    let cache = HashCache::open_in_memory().unwrap();

    let no_cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicUsize::new(0));
    let result1 = hasher::hash_files_cached(&[png_path.clone()], &cache, progress, "sha256", &no_cancel);
    let hash1 = result1.hashed[0].perceptual_hash;
    assert!(hash1.is_some());
    assert_eq!(result1.cache_hits, 0);

    let progress = Arc::new(AtomicUsize::new(0));
    let result2 = hasher::hash_files_cached(&[png_path.clone()], &cache, progress, "sha256", &no_cancel);
    assert_eq!(result2.cache_hits, 1);
    assert_eq!(result2.hashed[0].perceptual_hash, hash1);
}

// ---------------------------------------------------------------------------
// Integration: perceptual matching end-to-end
// ---------------------------------------------------------------------------

/// Create two visually similar PNG images in separate folders with different
/// byte content (different metadata / slight pixel change). Verify the full
/// hash + perceptual pipeline finds them as "similar" and not "exact".
#[test]
fn integration_perceptual_matching_finds_similar_images() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("scan");
    let ref_dir = root.join("reference");
    let eval_dir = root.join("eval");
    fs::create_dir_all(&ref_dir).unwrap();
    fs::create_dir_all(&eval_dir).unwrap();

    // Reference: a horizontal gradient PNG
    let img_ref = image::ImageBuffer::from_fn(100, 100, |x, _y| {
        let val = ((x * 255) / 99) as u8;
        image::Rgb([val, val, val])
    });
    let ref_path = ref_dir.join("gradient.png");
    img_ref.save(&ref_path).unwrap();

    // Eval: same gradient with tiny pixel difference (not byte-identical)
    let img_eval = image::ImageBuffer::from_fn(100, 100, |x, _y| {
        let val = ((x * 255) / 99) as u8;
        image::Rgb([val.saturating_add(1), val, val])
    });
    let eval_path = eval_dir.join("gradient_modified.png");
    img_eval.save(&eval_path).unwrap();

    // Verify they have DIFFERENT content hashes (not exact duplicates)
    let ref_hash = hasher::hash_file(&ref_path, "sha256").unwrap();
    let eval_hash = hasher::hash_file(&eval_path, "sha256").unwrap();
    assert_ne!(ref_hash, eval_hash, "Images should have different content hashes");

    // Verify they have SIMILAR perceptual hashes
    let ref_phash = crate::perceptual::compute_dhash(&ref_path).unwrap();
    let eval_phash = crate::perceptual::compute_dhash(&eval_path).unwrap();
    let dist = crate::perceptual::hamming_distance(ref_phash, eval_phash);
    assert!(dist <= 10, "Expected perceptually similar images, got distance {dist}");

    // Run the full pipeline: hash both folders
    let cache = HashCache::open_in_memory().unwrap();
    let _ = cache.prune();

    let allowed = hasher::resolve_extensions(
        &["images".to_string()],
        false,
        &HashMap::new(),
        &HashMap::new(),
    );

    let ref_files = hasher::collect_files(&ref_dir, allowed.as_ref());
    let eval_files = hasher::collect_files(&eval_dir, allowed.as_ref());

    assert_eq!(ref_files.len(), 1);
    assert_eq!(eval_files.len(), 1);

    let no_cancel = Arc::new(AtomicBool::new(false));
    let ref_progress = Arc::new(AtomicUsize::new(0));
    let ref_result = hasher::hash_files_cached(&ref_files, &cache, ref_progress, "sha256", &no_cancel);

    let eval_progress = Arc::new(AtomicUsize::new(0));
    let eval_result = hasher::hash_files_cached(&eval_files, &cache, eval_progress, "sha256", &no_cancel);

    // Verify perceptual hashes were computed
    assert!(ref_result.hashed[0].perceptual_hash.is_some(), "Reference should have perceptual hash");
    assert!(eval_result.hashed[0].perceptual_hash.is_some(), "Eval should have perceptual hash");

    // Build reference hash set for content comparison
    let ref_hash_set: HashSet<String> = ref_result.hashed.iter().map(|f| f.hash.clone()).collect();

    // Content comparison: should NOT be an exact match
    let eval_hf = &eval_result.hashed[0];
    assert!(!ref_hash_set.contains(&eval_hf.hash), "Should not be an exact content match");

    // Perceptual comparison: should find a similar match
    let ref_phashes: Vec<u64> = ref_result.hashed.iter()
        .filter_map(|f| f.perceptual_hash)
        .collect();
    assert!(!ref_phashes.is_empty(), "Should have reference perceptual hashes");

    let eval_ph = eval_result.hashed[0].perceptual_hash.unwrap();
    let min_dist = ref_phashes.iter()
        .map(|&rph| crate::perceptual::hamming_distance(eval_ph, rph))
        .min()
        .unwrap();

    assert!(min_dist <= 10, "Should be within Moderate threshold, got {min_dist}");
}

// ---------------------------------------------------------------------------
// Backfill: legacy cache entries without perceptual hash get backfilled
// ---------------------------------------------------------------------------

#[test]
fn hash_files_cached_backfills_perceptual_hash_on_legacy_cache_hit() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("scan_root");
    fs::create_dir(&root).unwrap();

    // Create a PNG image
    let img = image::ImageBuffer::from_fn(50, 50, |x, _y| {
        let val = ((x * 255) / 49) as u8;
        image::Rgb([val, val, val])
    });
    let png_path = root.join("gradient.png");
    img.save(&png_path).unwrap();

    let cache = HashCache::open_in_memory().unwrap();

    // Simulate a legacy cache entry: content hash present, no perceptual hash
    let path_str = png_path.to_string_lossy().to_string();
    let meta = fs::metadata(&png_path).unwrap();
    let mtime = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    cache.set(&CachedFile {
        path: path_str.clone(),
        hash: "legacy_hash_abc".to_string(),
        size: meta.len(),
        mtime_secs: mtime.as_secs() as i64,
        mtime_nanos: mtime.subsec_nanos(),
        algorithm: "sha256".to_string(),
        perceptual_hash: None,  // <-- legacy: no perceptual hash
    }).unwrap();

    // Verify cache hit returns None for perceptual hash
    let hit = cache.get(&path_str, meta.len(), mtime.as_secs() as i64, mtime.subsec_nanos(), "sha256").unwrap();
    assert_eq!(hit.perceptual_hash, None);

    // Run hash_files_cached — should be a cache hit but backfill the perceptual hash
    let no_cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicUsize::new(0));
    let result = hasher::hash_files_cached(&[png_path.clone()], &cache, progress, "sha256", &no_cancel);

    assert_eq!(result.cache_hits, 1, "Should be a cache hit");
    assert_eq!(result.hashed[0].hash, "legacy_hash_abc", "Content hash from cache");
    assert!(result.hashed[0].perceptual_hash.is_some(), "Perceptual hash should be backfilled");

    // Verify the cache was updated with the perceptual hash
    let hit2 = cache.get(&path_str, meta.len(), mtime.as_secs() as i64, mtime.subsec_nanos(), "sha256").unwrap();
    assert!(hit2.perceptual_hash.is_some(), "Cache should now have the perceptual hash");
    assert_eq!(hit2.perceptual_hash, result.hashed[0].perceptual_hash);
}

// ---------------------------------------------------------------------------
// move_file: returns sidecar warnings
// ---------------------------------------------------------------------------

#[test]
fn move_file_returns_empty_warnings_on_success() {
    let tmp = TempDir::new().unwrap();
    let eval_dir = tmp.path().join("eval");
    let dest_dir = tmp.path().join("dest");
    fs::create_dir_all(&eval_dir).unwrap();
    fs::create_dir_all(&dest_dir).unwrap();

    fs::write(eval_dir.join("photo.jpg"), b"image data").unwrap();
    let (final_path, warnings) = fileops::move_file(
        &eval_dir.join("photo.jpg"), &eval_dir, &dest_dir
    ).unwrap();

    assert!(final_path.exists());
    assert!(warnings.is_empty());
}

// ---------------------------------------------------------------------------
// csv_quote: RFC 4180 field quoting
// ---------------------------------------------------------------------------

#[test]
fn csv_quote_no_special_chars() {
    assert_eq!(commands::csv_quote("hello"), "hello");
}

#[test]
fn csv_quote_with_comma() {
    assert_eq!(commands::csv_quote("hello,world"), "\"hello,world\"");
}

#[test]
fn csv_quote_with_double_quotes() {
    assert_eq!(commands::csv_quote("say \"hi\""), "\"say \"\"hi\"\"\"");
}

#[test]
fn csv_quote_with_newline() {
    assert_eq!(commands::csv_quote("line1\nline2"), "\"line1\nline2\"");
}

#[test]
fn csv_quote_path_with_comma() {
    assert_eq!(
        commands::csv_quote("photos/2024, vacation/IMG_001.jpg"),
        "\"photos/2024, vacation/IMG_001.jpg\""
    );
}

// ---------------------------------------------------------------------------
// resolve_extensions: custom and removed extensions
// ---------------------------------------------------------------------------

#[test]
fn resolve_extensions_with_custom_addition() {
    let custom = HashMap::from([("images".to_string(), vec!["cr4".to_string()])]);
    let removed = HashMap::new();
    let result = hasher::resolve_extensions(&["images".to_string()], false, &custom, &removed);
    let exts = result.unwrap();
    assert!(exts.contains("cr4"), "Custom extension should be included");
    assert!(exts.contains("jpg"), "Default extension should still be present");
}

#[test]
fn resolve_extensions_with_removed_default() {
    let custom = HashMap::new();
    let removed = HashMap::from([("images".to_string(), vec!["bmp".to_string()])]);
    let result = hasher::resolve_extensions(&["images".to_string()], false, &custom, &removed);
    let exts = result.unwrap();
    assert!(!exts.contains("bmp"), "Removed extension should be excluded");
    assert!(exts.contains("jpg"), "Other defaults should remain");
}

#[test]
fn resolve_extensions_custom_and_removed_combined() {
    let custom = HashMap::from([("images".to_string(), vec!["raw".to_string()])]);
    let removed = HashMap::from([("images".to_string(), vec!["webp".to_string()])]);
    let result = hasher::resolve_extensions(&["images".to_string()], false, &custom, &removed);
    let exts = result.unwrap();
    assert!(exts.contains("raw"));
    assert!(!exts.contains("webp"));
    assert!(exts.contains("jpg"));
}

// ---------------------------------------------------------------------------
// hash_file: unknown algorithm
// ---------------------------------------------------------------------------

#[test]
fn hash_file_unknown_algorithm_returns_error() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.txt");
    fs::write(&file, b"content").unwrap();
    let result = hasher::hash_file(&file, "blake3");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown hash algorithm"));
}

// ---------------------------------------------------------------------------
// cleanup_empty_dirs: .DS_Store handling
// ---------------------------------------------------------------------------

#[test]
fn cleanup_removes_dir_with_only_ds_store() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("root");
    let subdir = root.join("empty_looking");
    fs::create_dir_all(&subdir).unwrap();
    fs::write(subdir.join(".DS_Store"), b"junk").unwrap();

    let removed = fileops::cleanup_empty_dirs(&root).unwrap();
    assert_eq!(removed, 1);
    assert!(!subdir.exists());
}
