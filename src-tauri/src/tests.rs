use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::cache::{CachedFile, HashCache};
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

    let hash = hasher::hash_file(&file).unwrap();
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

    let hash = hasher::hash_file(&file).unwrap();
    // SHA-256 of empty input
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hash_file_nonexistent() {
    let result = hasher::hash_file(&PathBuf::from("/does/not/exist.jpg"));
    assert!(result.is_err());
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
    let image_exts = hasher::resolve_extensions(&["images".to_string()], false).unwrap();
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

    let result = fileops::move_file(&file, src_dir.path(), dest_dir.path()).unwrap();

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

    let result = fileops::move_file(&file, src_dir.path(), dest_dir.path()).unwrap();

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
    };

    cache.set(&entry).unwrap();

    let result = cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 500);
    assert_eq!(result, Some("abc123".to_string()));
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
    };

    cache.set(&entry).unwrap();

    // Different size
    assert_eq!(cache.get("/tmp/test/photo.jpg", 2048, 1700000000, 500), None);
    // Different mtime
    assert_eq!(cache.get("/tmp/test/photo.jpg", 1024, 1700000001, 500), None);
    // Different path
    assert_eq!(cache.get("/tmp/test/other.jpg", 1024, 1700000000, 500), None);
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
    };
    cache.set(&entry1).unwrap();

    let entry2 = CachedFile {
        path: "/tmp/test/photo.jpg".to_string(),
        hash: "new_hash".to_string(),
        size: 2048,
        mtime_secs: 1700000001,
        mtime_nanos: 0,
    };
    cache.set(&entry2).unwrap();

    // Old metadata should no longer match
    assert_eq!(cache.get("/tmp/test/photo.jpg", 1024, 1700000000, 0), None);
    // New metadata should match
    assert_eq!(
        cache.get("/tmp/test/photo.jpg", 2048, 1700000001, 0),
        Some("new_hash".to_string())
    );
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
    };
    cache.set(&entry).unwrap();

    let pruned = cache.prune().unwrap();
    assert_eq!(pruned, 1);

    // Entry should be gone
    assert_eq!(
        cache.get("/definitely/does/not/exist/photo.jpg", 100, 1700000000, 0),
        None
    );
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
    };
    cache.set(&entry).unwrap();

    let pruned = cache.prune().unwrap();
    assert_eq!(pruned, 0);

    // Entry should still be there
    assert_eq!(
        cache.get(&path_str, 4, 1700000000, 0),
        Some("exists".to_string())
    );
}

// ---------------------------------------------------------------------------
// resolve_extensions: builds correct extension sets
// ---------------------------------------------------------------------------

#[test]
fn resolve_extensions_all_files_returns_none() {
    let result = hasher::resolve_extensions(&["images".to_string()], true);
    assert!(result.is_none());
}

#[test]
fn resolve_extensions_single_category() {
    let result = hasher::resolve_extensions(&["images".to_string()], false).unwrap();
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
    )
    .unwrap();
    assert!(result.contains("jpg"));
    assert!(result.contains("mp4"));
    assert!(result.contains("mov"));
    assert!(!result.contains("pdf"));
}

#[test]
fn resolve_extensions_empty_categories() {
    let result = hasher::resolve_extensions(&[], false).unwrap();
    assert!(result.is_empty());
}

#[test]
fn resolve_extensions_unknown_category_ignored() {
    let result = hasher::resolve_extensions(&["unknown".to_string()], false).unwrap();
    assert!(result.is_empty());
}
