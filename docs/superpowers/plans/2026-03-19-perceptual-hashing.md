# FR-002: Perceptual Hashing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add dHash-based perceptual hashing as a second-pass duplicate detection layer to catch near-duplicate images that differ only in metadata, compression, resolution, or format.

**Architecture:** Two-pass approach — content hashing runs first (existing behavior), then perceptual hashing computes dHashes for supported image formats and compares non-exact eval files against all reference dHashes using Hamming distance. Results are split into three groups: exact matches, similar matches, and uniques.

**Tech Stack:** Rust (image crate for decoding, existing rayon/rusqlite), React/TypeScript frontend, Tauri IPC.

**Spec:** `docs/superpowers/specs/2026-03-19-perceptual-hashing-design.md`

---

### Task 1: Add `image` crate dependency

**Files:**
- Modify: `src-tauri/Cargo.toml:14-29`

- [ ] **Step 1: Add the image crate to Cargo.toml**

Add after the `hex = "0.4"` line:

```toml
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "tiff", "bmp", "webp"] }
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: Compiles successfully (downloads image crate and dependencies)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "add image crate dependency for perceptual hashing"
```

---

### Task 2: Create `perceptual.rs` module with tests

**Files:**
- Create: `src-tauri/src/perceptual.rs`
- Modify: `src-tauri/src/lib.rs:5` (add `mod perceptual;`)
- Modify: `src-tauri/src/tests.rs` (add perceptual hash tests)

- [ ] **Step 1: Write the unit tests in tests.rs**

Add at the bottom of `src-tauri/src/tests.rs`:

```rust
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
    // Create a simple 10x10 PNG with known pixel data
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

    // Create a gradient image
    let mut img1 = image::ImageBuffer::new(100, 100);
    for (x, _y, pixel) in img1.enumerate_pixels_mut() {
        let val = ((x * 255) / 99) as u8;
        *pixel = image::Rgb([val, val, val]);
    }
    let path1 = dir.path().join("gradient1.png");
    img1.save(&path1).unwrap();

    // Create a very slightly modified version (add tiny noise)
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

    // All black
    let img1 = image::ImageBuffer::from_fn(100, 100, |_x, _y| image::Rgb([0u8, 0, 0]));
    let path1 = dir.path().join("black.png");
    img1.save(&path1).unwrap();

    // Horizontal gradient (left-to-right brightness increase)
    let img2 = image::ImageBuffer::from_fn(100, 100, |x, _y| {
        let val = ((x * 255) / 99) as u8;
        image::Rgb([val, val, val])
    });
    let path2 = dir.path().join("gradient.png");
    img2.save(&path2).unwrap();

    let hash1 = crate::perceptual::compute_dhash(&path1).unwrap();
    let hash2 = crate::perceptual::compute_dhash(&path2).unwrap();
    let dist = crate::perceptual::hamming_distance(hash1, hash2);
    assert!(dist > 10, "Expected different images to have distance > 10, got {dist}");
}
```

- [ ] **Step 2: Register the module in lib.rs**

Add `mod perceptual;` after `mod hasher;` in `src-tauri/src/lib.rs` (line 5). The existing module list:

```rust
mod actionlog;
mod cache;
mod commands;
mod fileops;
mod hasher;
mod perceptual;
```

- [ ] **Step 3: Create perceptual.rs with the implementation**

Create `src-tauri/src/perceptual.rs`:

```rust
//! Perceptual hashing (dHash) for near-duplicate image detection.

use std::path::Path;

/// Compute a 64-bit difference hash (dHash) for an image file.
///
/// The image is decoded, resized to 9x8 grayscale, and each pixel is
/// compared to its right neighbor. The result is a 64-bit hash in
/// row-major order (bit index = y * 8 + x).
///
/// Returns `None` if the image cannot be decoded (unsupported format,
/// corrupt file, or file not found).
pub fn compute_dhash(path: &Path) -> Option<u64> {
    let img = image::open(path).ok()?;
    let gray = img
        .resize_exact(9, 8, image::imageops::FilterType::Lanczos3)
        .to_luma8();

    let mut hash: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            if gray.get_pixel(x, y)[0] > gray.get_pixel(x + 1, y)[0] {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    Some(hash)
}

/// Hamming distance between two 64-bit hashes (number of differing bits).
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test`
Expected: All new perceptual tests pass, all existing tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/perceptual.rs src-tauri/src/lib.rs src-tauri/src/tests.rs
git commit -m "add perceptual hashing module with dHash and hamming distance"
```

---

### Task 3: Update cache for perceptual hash storage

**Files:**
- Modify: `src-tauri/src/cache.rs`
- Modify: `src-tauri/src/tests.rs` (add cache tests)

- [ ] **Step 1: Write cache tests**

Add at the bottom of `src-tauri/src/tests.rs`:

```rust
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
    // Verify u64 values above i64::MAX survive the SQLite round-trip
    let cache = HashCache::open_in_memory().unwrap();
    let high_val: u64 = 0xFFFFFFFFFFFFFFFF; // u64::MAX
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test cache_stores_and_retrieves_perceptual_hash cache_stores_null_perceptual_hash cache_perceptual_hash_high_bit_roundtrip 2>&1 | head -30`
Expected: Compilation errors (CachedFile doesn't have perceptual_hash field yet, get() returns wrong type)

- [ ] **Step 3: Update cache.rs**

Replace the full content of `src-tauri/src/cache.rs` with:

```rust
//! SQLite-backed hash cache. Avoids re-hashing files whose size and mtime haven't changed.

use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub algorithm: String,
    pub perceptual_hash: Option<i64>,
}

/// Returned by `get()` when a cache hit is found.
#[derive(Debug, Clone)]
pub struct CacheHit {
    pub hash: String,
    pub perceptual_hash: Option<u64>,
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

        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory database: {e}"))?;

        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_hashes (
                path TEXT NOT NULL,
                hash TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                algorithm TEXT NOT NULL DEFAULT 'sha256',
                perceptual_hash INTEGER,
                PRIMARY KEY (path, algorithm)
            );"
        ).map_err(|e| format!("Failed to create cache table: {e}"))?;

        // Migration for legacy databases that lack the `algorithm` column.
        // SQLite doesn't support ALTER PRIMARY KEY, so we recreate the table
        // with the composite PK (path, algorithm) and copy data over.
        let has_algorithm: bool = conn
            .prepare("PRAGMA table_info(file_hashes)")
            .map_err(|e| format!("Failed to read table info: {e}"))?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Failed to iterate table info: {e}"))?
            .filter_map(|r| r.ok())
            .any(|name| name == "algorithm");

        if !has_algorithm {
            conn.execute_batch(
                "ALTER TABLE file_hashes ADD COLUMN algorithm TEXT NOT NULL DEFAULT 'sha256';
                 -- SQLite doesn't support DROP PRIMARY KEY, so we recreate the table.
                 CREATE TABLE IF NOT EXISTS file_hashes_new (
                     path TEXT NOT NULL,
                     hash TEXT NOT NULL,
                     size INTEGER NOT NULL,
                     mtime_secs INTEGER NOT NULL,
                     mtime_nanos INTEGER NOT NULL,
                     algorithm TEXT NOT NULL DEFAULT 'sha256',
                     perceptual_hash INTEGER,
                     PRIMARY KEY (path, algorithm)
                 );
                 INSERT OR IGNORE INTO file_hashes_new SELECT path, hash, size, mtime_secs, mtime_nanos, algorithm, NULL FROM file_hashes;
                 DROP TABLE file_hashes;
                 ALTER TABLE file_hashes_new RENAME TO file_hashes;"
            ).map_err(|e| format!("Failed to migrate cache schema: {e}"))?;
        }

        // Migration for databases that lack the `perceptual_hash` column.
        let has_perceptual: bool = conn
            .prepare("PRAGMA table_info(file_hashes)")
            .map_err(|e| format!("Failed to read table info: {e}"))?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("Failed to iterate table info: {e}"))?
            .filter_map(|r| r.ok())
            .any(|name| name == "perceptual_hash");

        if !has_perceptual {
            conn.execute_batch(
                "ALTER TABLE file_hashes ADD COLUMN perceptual_hash INTEGER"
            ).map_err(|e| format!("Failed to add perceptual_hash column: {e}"))?;
        }

        Ok(())
    }

    fn db_path() -> Result<PathBuf, String> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| "Could not determine application data directory".to_string())?;
        Ok(data_dir.join("com.photodedup").join("cache.db"))
    }

    /// Returns a `CacheHit` if the file hasn't changed since it was cached.
    ///
    /// Staleness check uses (size, mtime). This won't detect edits that
    /// preserve both (e.g. `touch -r`), but it's correct for normal use.
    pub fn get(&self, path: &str, size: u64, mtime_secs: i64, mtime_nanos: u32, algorithm: &str) -> Option<CacheHit> {
        self.conn
            .query_row(
                "SELECT hash, perceptual_hash FROM file_hashes WHERE path = ?1 AND size = ?2 AND mtime_secs = ?3 AND mtime_nanos = ?4 AND algorithm = ?5",
                params![path, size as i64, mtime_secs, mtime_nanos, algorithm],
                |row| {
                    let hash: String = row.get(0)?;
                    let phash: Option<i64> = row.get(1)?;
                    Ok(CacheHit {
                        hash,
                        perceptual_hash: phash.map(|v| v as u64),
                    })
                },
            )
            .ok()
    }

    pub fn set(&self, entry: &CachedFile) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO file_hashes (path, hash, size, mtime_secs, mtime_nanos, algorithm, perceptual_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![entry.path, entry.hash, entry.size as i64, entry.mtime_secs, entry.mtime_nanos, entry.algorithm, entry.perceptual_hash],
            )
            .map_err(|e| format!("Failed to write cache: {e}"))?;
        Ok(())
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
```

- [ ] **Step 4: Fix existing tests that use CachedFile or cache.get()**

Two categories of changes needed in `src-tauri/src/tests.rs`:

**A) CachedFile constructors:** Search for all existing `CachedFile {` constructors and add `perceptual_hash: None,` to each one.

**B) cache.get() assertions:** `cache.get()` now returns `Option<CacheHit>` instead of `Option<String>`. Search for all assertions like `assert_eq!(cache.get(...), Some("hash".to_string()))` and update them to extract the hash, e.g.:

```rust
// Before:
assert_eq!(cache.get("path", size, secs, nanos, "sha256"), Some("abc123".to_string()));

// After:
let hit = cache.get("path", size, secs, nanos, "sha256").unwrap();
assert_eq!(hit.hash, "abc123");
assert_eq!(hit.perceptual_hash, None);
```

Also add `CacheHit` to the test imports at the top of tests.rs:

```rust
use crate::cache::{CachedFile, CacheHit, HashCache};
```

- [ ] **Step 5: Fix existing code in hasher.rs that calls cache.get() and cache.set()**

In `src-tauri/src/hasher.rs`:

At line 14, update the import:
```rust
use crate::cache::{CachedFile, CacheHit, HashCache};
```

At line 216, `cache.get()` now returns `Option<CacheHit>` instead of `Option<String>`. Update:
```rust
if let Some(hit) = cache.get(&path_str, size, mtime_secs, mtime_nanos, algorithm) {
    progress.fetch_add(1, Ordering::Relaxed);
    cache_hits += 1;
    results.push(HashedFile { path: path_str, hash: hit.hash, size });
```

At line 255 (the `cache.set` call), add `perceptual_hash: None`:
```rust
let _ = cache.set(&CachedFile {
    path: fm.path_str.clone(),
    hash: hash.clone(),
    size: fm.size,
    mtime_secs: fm.mtime_secs,
    mtime_nanos: fm.mtime_nanos,
    algorithm: algorithm.to_string(),
    perceptual_hash: None,
});
```

- [ ] **Step 6: Run all tests**

Run: `cd src-tauri && cargo test`
Expected: All tests pass (existing + new cache tests)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/cache.rs src-tauri/src/hasher.rs src-tauri/src/tests.rs
git commit -m "add perceptual hash column to cache with migration"
```

---

### Task 4: Update hasher to compute and cache perceptual hashes

**Files:**
- Modify: `src-tauri/src/hasher.rs`
- Modify: `src-tauri/src/tests.rs`

- [ ] **Step 1: Write hasher test**

Add to `src-tauri/src/tests.rs`:

```rust
// ---------------------------------------------------------------------------
// hash_files_cached: populates perceptual hash for supported images
// ---------------------------------------------------------------------------

#[test]
fn hash_files_cached_computes_perceptual_hash_for_png() {
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

    // Create a text file (should NOT get a perceptual hash)
    let txt_path = root.join("notes.txt");
    fs::write(&txt_path, b"hello").unwrap();

    let cache = HashCache::open_in_memory().unwrap();
    let progress = Arc::new(AtomicUsize::new(0));

    let result = hasher::hash_files_cached(
        &[png_path.clone(), txt_path.clone()],
        &cache,
        progress,
        "sha256",
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

    // First pass: computes and caches
    let progress = Arc::new(AtomicUsize::new(0));
    let result1 = hasher::hash_files_cached(&[png_path.clone()], &cache, progress, "sha256");
    let hash1 = result1.hashed[0].perceptual_hash;
    assert!(hash1.is_some());
    assert_eq!(result1.cache_hits, 0);

    // Second pass: should come from cache
    let progress = Arc::new(AtomicUsize::new(0));
    let result2 = hasher::hash_files_cached(&[png_path.clone()], &cache, progress, "sha256");
    assert_eq!(result2.cache_hits, 1);
    assert_eq!(result2.hashed[0].perceptual_hash, hash1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test hash_files_cached_computes_perceptual 2>&1 | head -20`
Expected: Fails — `HashedFile` doesn't have `perceptual_hash` field yet

- [ ] **Step 3: Update HashedFile and hash_files_cached in hasher.rs**

Add `perceptual_hash: Option<u64>` to `HashedFile`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct HashedFile {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub perceptual_hash: Option<u64>,
}
```

Add a constant for the supported perceptual hash extensions (formats the `image` crate can decode):

```rust
/// Extensions that support perceptual hashing (image crate can decode these).
const PERCEPTUAL_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "bmp", "webp"];
```

Add a helper to check if a path supports perceptual hashing:

```rust
fn supports_perceptual_hash(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| PERCEPTUAL_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}
```

Update the import at the top of hasher.rs:

```rust
use crate::cache::{CachedFile, CacheHit, HashCache};
use crate::perceptual;
```

Update `hash_files_cached` — the cache hit path (around line 216):

```rust
if let Some(hit) = cache.get(&path_str, size, mtime_secs, mtime_nanos, algorithm) {
    progress.fetch_add(1, Ordering::Relaxed);
    cache_hits += 1;
    results.push(HashedFile {
        path: path_str,
        hash: hit.hash,
        size,
        perceptual_hash: hit.perceptual_hash,
    });
```

Update the parallel hashing closure (around line 236) — after computing the content hash, also compute dHash:

```rust
.map(|fm| {
    match hash_file(&fm.path, &algo) {
        Ok(hash) => {
            let phash = if supports_perceptual_hash(&fm.path) {
                perceptual::compute_dhash(&fm.path)
            } else {
                None
            };
            progress_clone.fetch_add(1, Ordering::Relaxed);
            Ok((fm, hash, phash))
        }
        Err(reason) => {
            progress_clone.fetch_add(1, Ordering::Relaxed);
            let path = fm.path_str.clone();
            Err(SkippedFile { path, reason })
        }
    }
})
```

Update the serial cache-update loop (around line 252):

```rust
for item in newly_hashed {
    match item {
        Ok((fm, hash, phash)) => {
            let _ = cache.set(&CachedFile {
                path: fm.path_str.clone(),
                hash: hash.clone(),
                size: fm.size,
                mtime_secs: fm.mtime_secs,
                mtime_nanos: fm.mtime_nanos,
                algorithm: algorithm.to_string(),
                perceptual_hash: phash.map(|v| v as i64),
            });
            results.push(HashedFile {
                path: fm.path_str,
                hash,
                size: fm.size,
                perceptual_hash: phash,
            });
        }
        Err(sf) => skipped.push(sf),
    }
}
```

- [ ] **Step 4: Run all tests**

Run: `cd src-tauri && cargo test`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/hasher.rs src-tauri/src/tests.rs
git commit -m "compute and cache perceptual hashes in hasher pipeline"
```

---

### Task 5: Update commands.rs — data structures and scan logic

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Update ScanConfig with new fields**

Add two fields to `ScanConfig` after `removed_extensions`:

```rust
    #[serde(default)]
    pub perceptual_matching: bool,
    #[serde(default = "default_perceptual_threshold")]
    pub perceptual_threshold: u32,
```

Add the default function below the struct:

```rust
fn default_perceptual_threshold() -> u32 { 10 }
```

- [ ] **Step 2: Update EvalFile**

Replace `is_duplicate: bool` with:

```rust
    pub match_type: String,
    pub hamming_distance: Option<u32>,
```

- [ ] **Step 3: Update ScanResult**

Replace `duplicates: Vec<EvalFile>` and `uniques: Vec<EvalFile>` with:

```rust
    pub exact_matches: Vec<EvalFile>,
    pub similar_matches: Vec<EvalFile>,
    pub uniques: Vec<EvalFile>,
```

- [ ] **Step 4: Update ScanStats**

Add one field (dHash computation time is included in `ref_hash_ms`/`eval_hash_ms` since it runs inline during hashing):

```rust
    pub perceptual_compare_ms: u64,
```

- [ ] **Step 5: Update scan_folders_blocking**

Add `use crate::perceptual;` to the imports at the top of commands.rs.

Replace the comparison section (lines 225-283) with the new three-group logic. The full replacement for the section starting at `emit_progress(app, "Comparing files...", 0, 0);`:

```rust
    // -- Content comparison --
    emit_progress(app, "Comparing files...", 0, 0);
    let mut seen_eval_hashes: HashSet<String> = HashSet::new();

    let mut exact_matches = Vec::new();
    let mut non_exact_eval = Vec::new();
    let mut uniques_candidate = Vec::new();

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

    if config.perceptual_matching {
        // Collect reference perceptual hashes (already computed during hashing).
        // Collect reference perceptual hashes (already computed during hashing).
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

                    if min_dist <= config.perceptual_threshold {
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
```

- [ ] **Step 6: Update export_report**

Replace the CSV section with:

```rust
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
```

The JSON path needs no change — `serde_json::to_string_pretty(&results)` will serialize the new struct automatically.

- [ ] **Step 7: Fix the handleAction unique files reference in execute_action**

In `handleAction` on the frontend, `result.uniques` is still valid — no change needed in execute_action itself.

- [ ] **Step 8: Fix tests.rs references to the old ScanResult/EvalFile fields**

This requires updating several areas in `src-tauri/src/tests.rs`:

**A) EvalFile constructors:** Replace all `is_duplicate: true/false` with:
- `is_duplicate: true` → `match_type: "exact".to_string(), hamming_distance: None`
- `is_duplicate: false` → `match_type: "unique".to_string(), hamming_distance: None`

**B) ScanResult constructors:** Replace all `duplicates:` and `uniques:` fields:
- `duplicates: vec![...]` → `exact_matches: vec![...], similar_matches: vec![]`
- `uniques: vec![...]` → `uniques: vec![...]` (unchanged field name)

**C) ScanStats constructors:** Add two new fields to every `ScanStats { ... }`:
```rust
perceptual_compare_ms: 0,
```

**D) The `detect_duplicates` helper function** (~line 570): This function replicates the scan comparison logic and returns `(duplicates, uniques)`. Update it to return `(exact_matches, uniques)` with the new `EvalFile` shape (using `match_type` and `hamming_distance` instead of `is_duplicate`).

**E) The `integration_full_scan_finds_duplicates` test** (~line 621): Update assertions to use `result.exact_matches` instead of `result.duplicates`.

**F) The `integration_csv_export` test** (~line 920): Update the `ScanResult` construction to use `exact_matches`/`similar_matches`/`uniques`, update `EvalFile` constructors, update CSV header assertion to include `hamming_distance` column, and update expected status values from `"duplicate"` to `"exact"`.

- [ ] **Step 9: Run all tests**

Run: `cd src-tauri && cargo test`
Expected: All tests pass

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/tests.rs
git commit -m "update scan pipeline for three-group results with perceptual matching"
```

---

### Task 6: Update frontend types

**Files:**
- Modify: `src/types.ts`

- [ ] **Step 1: Update types.ts**

Update `ScanConfig` — add after `removed_extensions`:

```typescript
  perceptual_matching: boolean;
  perceptual_threshold: number;
```

Update `ScanStats` — add:

```typescript
  perceptual_compare_ms: number;
```

Update `ScanResult` — replace `duplicates` and `uniques` with:

```typescript
  exact_matches: EvalFile[];
  similar_matches: EvalFile[];
  uniques: EvalFile[];
```

Update `EvalFile` — replace `is_duplicate: boolean` with:

```typescript
  match_type: string;
  hamming_distance: number | null;
```

- [ ] **Step 2: Verify TypeScript compiles (expect errors in screens)**

Run: `cd /Users/jdstemmler/Repos/fun/deduper && npx tsc --noEmit 2>&1 | head -30`
Expected: Type errors in SetupScreen.tsx and ResultsScreen.tsx (they reference old fields). This is expected — we'll fix them in the next tasks.

- [ ] **Step 3: Commit**

```bash
git add src/types.ts
git commit -m "update TypeScript types for perceptual matching"
```

---

### Task 7: Update SetupScreen with perceptual matching UI

**Files:**
- Modify: `src/screens/SetupScreen.tsx`

- [ ] **Step 1: Add perceptual matching state**

Add after the `hashAlgorithm` state (around line 117):

```typescript
  const [perceptualMatching, setPerceptualMatching] = useState(
    initialConfig?.perceptual_matching ?? false
  );
  const [perceptualThreshold, setPerceptualThreshold] = useState<number>(
    initialConfig?.perceptual_threshold ?? 10
  );
```

- [ ] **Step 2: Add auto-disable logic**

Compute whether images are available (needed for the disabled state). Add after the state declarations:

```typescript
  const hasImageCategory = allFiles || selectedCategories.has("images");
```

Add a `useEffect` to auto-disable perceptual matching when images are deselected:

```typescript
  useEffect(() => {
    if (!hasImageCategory) {
      setPerceptualMatching(false);
    }
  }, [hasImageCategory]);
```

- [ ] **Step 3: Add perceptual matching fields to handleStart**

In `handleStart`, add the two new fields to the `onStart()` call:

```typescript
    onStart({
      reference_dir: referenceDir,
      eval_dir: evalDir,
      dupe_mode: mode,
      move_uniques: moveUniques,
      unique_dest: moveUniques ? uniqueDest : null,
      categories: Array.from(selectedCategories),
      all_files: allFiles,
      hash_algorithm: hashAlgorithm,
      custom_extensions: customExtensions,
      removed_extensions: removedExtensions,
      perceptual_matching: perceptualMatching,
      perceptual_threshold: perceptualThreshold,
    });
```

Also persist `perceptualMatching` and `perceptualThreshold` in `SavedConfig` and `saveSavedConfig` / `loadSavedConfig`. Add to the `SavedConfig` interface:

```typescript
  perceptualMatching?: boolean;
  perceptualThreshold?: number;
```

Initialize state from saved config:

```typescript
  const [perceptualMatching, setPerceptualMatching] = useState(
    initialConfig?.perceptual_matching ?? saved?.perceptualMatching ?? false
  );
  const [perceptualThreshold, setPerceptualThreshold] = useState<number>(
    initialConfig?.perceptual_threshold ?? saved?.perceptualThreshold ?? 10
  );
```

Add `perceptualMatching` and `perceptualThreshold` to the explicit `saveSavedConfig` call in `handleStart` (the existing code lists all fields explicitly — add the two new fields alongside the existing ones):

```typescript
    saveSavedConfig({
      reference_dir: referenceDir,
      eval_dir: evalDir,
      dupe_mode: dupeMode,
      dupeDest,
      moveUniques,
      uniqueDest,
      selectedCategories: Array.from(selectedCategories),
      allFiles,
      hashAlgorithm,
      perceptualMatching,
      perceptualThreshold,
    });
```

- [ ] **Step 4: Add perceptual matching UI section**

Add after the hash algorithm section (after the closing `</div>` of the `options-row` div, around line 524), before the "Non-duplicate handling" section:

```tsx
      <div className="config-section">
        <h3>Perceptual matching</h3>
        <div className="toggle-inline">
          <label className="toggle">
            <input
              type="checkbox"
              checked={perceptualMatching}
              onChange={(e) => setPerceptualMatching(e.target.checked)}
              disabled={!hasImageCategory}
            />
            <span className="toggle-slider" />
          </label>
          <span className={!hasImageCategory ? "disabled" : ""}>
            Find visually similar images (not just byte-identical)
          </span>
        </div>
        {!hasImageCategory && (
          <div className="category-warning">
            Requires an image category to be selected
          </div>
        )}
        {perceptualMatching && hasImageCategory && (
          <div className="threshold-presets">
            <span className="hash-label">Sensitivity</span>
            <div className="hash-pills">
              <button
                className={`hash-pill ${perceptualThreshold === 5 ? "active" : ""}`}
                onClick={() => setPerceptualThreshold(5)}
              >
                Strict
              </button>
              <button
                className={`hash-pill ${perceptualThreshold === 10 ? "active" : ""}`}
                onClick={() => setPerceptualThreshold(10)}
              >
                Moderate
              </button>
              <button
                className={`hash-pill ${perceptualThreshold === 15 ? "active" : ""}`}
                onClick={() => setPerceptualThreshold(15)}
              >
                Loose
              </button>
            </div>
            <span className="hash-hint">
              {perceptualThreshold === 5
                ? "Metadata changes, recompression"
                : perceptualThreshold === 10
                  ? "Quality differences, minor crops"
                  : "Significant changes \u2014 review carefully"}
            </span>
          </div>
        )}
      </div>
```

- [ ] **Step 5: Verify TypeScript compiles for SetupScreen**

Run: `npx tsc --noEmit 2>&1 | grep SetupScreen`
Expected: No SetupScreen errors (ResultsScreen errors still expected)

- [ ] **Step 6: Commit**

```bash
git add src/screens/SetupScreen.tsx
git commit -m "add perceptual matching toggle and threshold presets to setup screen"
```

---

### Task 8: Update ResultsScreen for three-group display

**Files:**
- Modify: `src/screens/ResultsScreen.tsx`

- [ ] **Step 1: Update initial selection state**

Replace line 54-55:
```typescript
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(
    () => new Set(result.duplicates.map((f) => f.path))
  );
```
With (default to selecting only exact matches):
```typescript
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(
    () => new Set(result.exact_matches.map((f) => f.path))
  );
```

- [ ] **Step 2: Update derived values**

Replace lines 58-61:
```typescript
  const allDupePaths = result.duplicates.map((f) => f.path);
  const selectedCount = selectedFiles.size;
  const totalDupes = allDupePaths.length;
  const allSelected = selectedCount === totalDupes && totalDupes > 0;
```
With:
```typescript
  const exactPaths = result.exact_matches.map((f) => f.path);
  const similarPaths = result.similar_matches.map((f) => f.path);
  const allMatchPaths = [...exactPaths, ...similarPaths];
  const selectedCount = selectedFiles.size;
  const totalMatches = allMatchPaths.length;
```

- [ ] **Step 3: Replace toggleAll with three selection functions**

Replace the `toggleAll` function with:

```typescript
  const selectAll = () => setSelectedFiles(new Set(allMatchPaths));
  const selectExact = () => setSelectedFiles(new Set(exactPaths));
  const selectSimilar = () => setSelectedFiles(new Set(similarPaths));
  const selectNone = () => setSelectedFiles(new Set());
```

- [ ] **Step 4: Update handleConfirmAction**

Replace line 208:
```typescript
    const filesToAct = result.duplicates.filter((f) => selectedFiles.has(f.path));
```
With:
```typescript
    const filesToAct = [...result.exact_matches, ...result.similar_matches].filter((f) => selectedFiles.has(f.path));
```

- [ ] **Step 5: Update the summary bar**

Replace the summary bar section with four stats:

```tsx
          <div className="summary-bar">
            <div className="stat">
              <span className="stat-value">{result.total_eval}</span>
              <span className="stat-label">scanned</span>
            </div>
            <div className="stat stat-danger">
              <span className="stat-value">{result.exact_matches.length}</span>
              <span className="stat-label">exact</span>
            </div>
            {result.similar_matches.length > 0 && (
              <div className="stat stat-warning">
                <span className="stat-value">{result.similar_matches.length}</span>
                <span className="stat-label">similar</span>
              </div>
            )}
            <div className="stat stat-success">
              <span className="stat-value">{result.uniques.length}</span>
              <span className="stat-label">unique</span>
            </div>
            {result.skipped > 0 && (
              <div className="stat stat-warning">
                <span className="stat-value">{result.skipped}</span>
                <span className="stat-label">skipped</span>
              </div>
            )}
          </div>
```

- [ ] **Step 6: Update the file list header**

Replace the file-list-header section with selection buttons:

```tsx
            <div className="file-list-header">
              <div className="selection-buttons">
                <button className="btn-small" onClick={selectAll}>Select All</button>
                <button className="btn-small" onClick={selectExact}>Select Exact</button>
                {result.similar_matches.length > 0 && (
                  <button className="btn-small" onClick={selectSimilar}>Select Similar</button>
                )}
                <button className="btn-small" onClick={selectNone}>Deselect</button>
              </div>
              <span className="selection-count">
                {selectedCount} of {totalMatches} selected
              </span>
            </div>
```

- [ ] **Step 7: Update the file list rendering**

Replace the file list map with grouped sections:

```tsx
            {/* Exact Matches */}
            {result.exact_matches.length > 0 && (
              <>
                <div className="section-header">Exact Matches</div>
                {result.exact_matches.map((file) => {
                  const isSelected = selectedFiles.has(file.path);
                  return (
                    <div
                      key={file.path}
                      className={`file-row ${isSelected ? "selected-dupe" : "deselected"} ${selectedPreview === file.path ? "preview-active" : ""}`}
                    >
                      <input
                        type="checkbox"
                        className="file-row-checkbox"
                        checked={isSelected}
                        onChange={() => toggleFile(file.path)}
                      />
                      <span className="status-dot dot-dupe" />
                      <span
                        className="file-path"
                        style={{ cursor: "pointer" }}
                        onClick={() => handleFileClick(file.path)}
                      >
                        {file.relative_path}
                      </span>
                      <span className="file-size">{formatSize(file.size)}</span>
                      <span className="tag tag-dupe">Exact</span>
                    </div>
                  );
                })}
              </>
            )}

            {/* Similar Matches */}
            {result.similar_matches.length > 0 && (
              <>
                <div className="section-header">Similar Matches</div>
                {result.similar_matches.map((file) => {
                  const isSelected = selectedFiles.has(file.path);
                  const similarity = file.hamming_distance != null
                    ? Math.round((64 - file.hamming_distance) / 64 * 100)
                    : null;
                  return (
                    <div
                      key={file.path}
                      className={`file-row ${isSelected ? "selected-dupe" : "deselected"} ${selectedPreview === file.path ? "preview-active" : ""}`}
                    >
                      <input
                        type="checkbox"
                        className="file-row-checkbox"
                        checked={isSelected}
                        onChange={() => toggleFile(file.path)}
                      />
                      <span className="status-dot dot-similar" />
                      <span
                        className="file-path"
                        style={{ cursor: "pointer" }}
                        onClick={() => handleFileClick(file.path)}
                      >
                        {file.relative_path}
                      </span>
                      <span className="file-size">{formatSize(file.size)}</span>
                      {similarity !== null && (
                        <span className="similarity-badge">{similarity}%</span>
                      )}
                      <span className="tag tag-similar">Similar</span>
                    </div>
                  );
                })}
              </>
            )}

            {/* Uniques */}
            {result.uniques.length > 0 && (
              <>
                <div className="section-header">Unique Files</div>
                {result.uniques.map((file) => (
                  <div
                    key={file.path}
                    className={`file-row ${selectedPreview === file.path ? "preview-active" : ""}`}
                  >
                    <span style={{ width: 14, flexShrink: 0 }} />
                    <span className="status-dot dot-unique" />
                    <span
                      className="file-path"
                      style={{ cursor: "pointer" }}
                      onClick={() => handleFileClick(file.path)}
                    >
                      {file.relative_path}
                    </span>
                    <span className="file-size">{formatSize(file.size)}</span>
                    <span className="tag tag-unique">Unique</span>
                  </div>
                ))}
              </>
            )}
```

- [ ] **Step 8: Update action button text**

Replace references to `totalDupes` with `totalMatches` in the action button text.

- [ ] **Step 9: Update scan stats section**

Add perceptual hash timing to the stats panel (after the eval row):

```tsx
                  {s.perceptual_compare_ms > 0 && (
                    <div className="stats-row">
                      <span className="stats-label">Perceptual</span>
                      <span className="stats-value">
                        Compare: {formatTime(s.perceptual_compare_ms)}
                      </span>
                    </div>
                  )}
```

- [ ] **Step 10: Verify TypeScript compiles clean**

Run: `npx tsc --noEmit`
Expected: No errors

- [ ] **Step 11: Commit**

```bash
git add src/screens/ResultsScreen.tsx
git commit -m "update results screen for three-group display with selection buttons"
```

---

### Task 9: Add CSS for new UI elements

**Files:**
- Modify: `src/styles.css` (or wherever the app's CSS lives — check for the file)

- [ ] **Step 1: Add styles for new elements**

The CSS file is `src/styles.css`.

Add CSS for:
- `.dot-similar` — amber color (`#f59e0b`)
- `.tag-similar` — amber background
- `.similarity-badge` — small inline badge showing percentage
- `.section-header` — visual separator between file list sections
- `.selection-buttons` — horizontal button group
- `.threshold-presets` — layout for the threshold pills and hint
- `.disabled` text style — grayed out text

```css
.dot-similar { background: #f59e0b; }
.tag-similar { background: #fef3c7; color: #92400e; }

.similarity-badge {
  font-size: 11px;
  color: #92400e;
  background: #fef3c7;
  padding: 1px 6px;
  border-radius: 8px;
  flex-shrink: 0;
}

.section-header {
  font-size: 12px;
  font-weight: 600;
  color: #6b7280;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  padding: 8px 12px 4px;
  border-top: 1px solid #e5e7eb;
  margin-top: 4px;
}

.section-header:first-child {
  border-top: none;
  margin-top: 0;
}

.selection-buttons {
  display: flex;
  gap: 4px;
}

.threshold-presets {
  margin-top: 8px;
}

span.disabled {
  opacity: 0.5;
}
```

- [ ] **Step 2: Commit**

```bash
git add src/styles.css
git commit -m "add CSS for perceptual matching UI elements"
```

---

### Task 10: Build verification and integration test

**Files:**
- No new files

- [ ] **Step 1: Run full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: All tests pass

- [ ] **Step 2: Run TypeScript type check**

Run: `npx tsc --noEmit`
Expected: No errors

- [ ] **Step 3: Build the full app**

Run: `cd src-tauri && cargo build`
Expected: Compiles successfully

- [ ] **Step 4: Build the frontend**

Run: `npm run build`
Expected: Builds successfully

- [ ] **Step 5: Commit any fixes needed**

If any fixes were required, commit them with a descriptive message.

---

### Task 11: Update FEATURE_REQUESTS.md status

**Files:**
- Modify: `FEATURE_REQUESTS.md`

- [ ] **Step 1: Add status to FR-002**

Add a `### Status` section at the bottom of FR-002 (before the end of the file), similar to FR-001's status:

```markdown
### Status

SHIPPED in v0.3.0. dHash perceptual matching with Strict/Moderate/Loose presets, grouped results (Exact/Similar/Unique), and per-file similarity percentage display. Supported formats: JPEG, PNG, TIFF, BMP, WebP.
```

- [ ] **Step 2: Update FR-002 header**

Change `## FR-002: Perceptual hashing for near-duplicate detection` to:
`## FR-002: Perceptual hashing for near-duplicate detection — SHIPPED (v0.3.0)`

- [ ] **Step 3: Commit**

```bash
git add FEATURE_REQUESTS.md
git commit -m "mark FR-002 perceptual hashing as shipped"
```
