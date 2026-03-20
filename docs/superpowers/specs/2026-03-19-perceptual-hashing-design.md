# FR-002: Perceptual Hashing for Near-Duplicate Detection — Design Spec

## Overview

Add perceptual hashing (dHash) as a second-pass duplicate detection layer on top of existing content hashing. This catches near-duplicates that differ only in metadata, compression level, resolution, or format — the most common false-negative in photo dedup workflows.

## Decisions

- **Algorithm:** dHash (difference hash) — 64-bit, fast, simple, well-understood accuracy profile
- **Supported formats:** JPEG, PNG, TIFF, BMP, WebP (what the `image` crate decodes natively)
- **Unsupported formats fall back to content-hash only:** RAW (CR2, NEF, ARW, etc.), HEIC/HEIF, video, audio, documents
- **Threshold presets only** (no custom slider): Strict (≤5), Moderate (≤10), Loose (≤15)
- **Results grouping:** Exact and Similar matches displayed in separate sections
- **Selection controls:** "Select All", "Select Exact", "Select Similar"
- **No intra-eval perceptual matching** in this release — only eval-vs-reference
- **Two-pass architecture:** Content hash first, then perceptual hash for non-exact eval files + all reference files

## Scan Flow

1. Content hash both reference and eval folders (existing behavior)
2. Exact match — eval files whose content hash appears in the reference set (includes intra-eval content-hash duplicates, classified as `"exact"`)
3. If perceptual matching enabled:
   a. dHash the full reference folder (supported formats only, cached)
   b. dHash only non-exact-match eval files (supported formats only, cached)
   c. Compare each eval dHash against all reference dHashes
   d. Flag eval files with Hamming distance ≤ threshold as "Similar"
4. Return three groups: exact matches, similar matches, uniques

## Backend

### New module: `src-tauri/src/perceptual.rs`

Two public functions:

```rust
/// Compute dHash for an image file.
/// Resizes to 9x8 grayscale, compares each pixel to its right neighbor.
/// Returns None if the format can't be decoded.
pub fn compute_dhash(path: &Path) -> Option<u64>

/// Hamming distance between two 64-bit hashes.
pub fn hamming_distance(a: u64, b: u64) -> u32
```

dHash algorithm:
1. Decode image, resize to 9 wide x 8 tall grayscale (72 pixels)
2. For each of 8 rows, compare each pixel to its right neighbor (8 comparisons per row)
3. 8 x 8 = 64 comparisons = 64-bit hash, row-major order (bit index = `y * 8 + x`), each bit = 1 if left pixel brighter than right neighbor

New Cargo dependency:
```toml
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "tiff", "bmp", "webp"] }
```

### Cache changes: `src-tauri/src/cache.rs`

Add nullable `perceptual_hash INTEGER` column to `file_hashes` table.

- Add `perceptual_hash INTEGER` to the `CREATE TABLE IF NOT EXISTS` statement so new databases include it natively
- Add migration block (same pattern as existing `has_algorithm` check) to add the column for existing databases:
  ```sql
  ALTER TABLE file_hashes ADD COLUMN perceptual_hash INTEGER
  ```
- Nullable because non-image files and unsupported formats don't produce a perceptual hash

**Type handling:** SQLite stores `INTEGER` as signed 64-bit. dHash values span the full `u64` range. Use `u64` in Rust and cast via `as i64` / `as u64` at the SQLite boundary (bitwise reinterpretation, not numeric conversion).

**`CachedFile`** gains `perceptual_hash: Option<i64>` (SQLite-facing type).

**`get()` return type** changes from `Option<String>` to `Option<CacheHit>`:
```rust
pub struct CacheHit {
    pub hash: String,
    pub perceptual_hash: Option<u64>,  // cast from i64 at retrieval
}
```

**`set()`** — `CachedFile` gains the `perceptual_hash` field; `INSERT OR REPLACE` SQL updated to include the new column.

Cache key unchanged: `(path, algorithm)`. Staleness logic unchanged (invalidate on size/mtime change).

### Hasher changes: `src-tauri/src/hasher.rs`

**`HashedFile`** gains `perceptual_hash: Option<u64>`.

`hash_files_cached`:
- On cache hit: populate `perceptual_hash` from `CacheHit.perceptual_hash`
- On cache miss: after content hashing, compute dHash for supported image formats. Store both to cache.
- dHash computation runs in the same Rayon parallel block as content hashing.

### Command changes: `src-tauri/src/commands.rs`

**`ScanConfig`** gains:
- `perceptual_matching: bool` — `#[serde(default)]`, defaults to `false`
- `perceptual_threshold: u32` — `#[serde(default = "default_threshold")]` where `fn default_threshold() -> u32 { 10 }`

**`EvalFile`** changes:
- **Remove** `is_duplicate: bool`
- **Add** `match_type: String` — values: `"exact"`, `"similar"`, `"unique"`
- **Add** `hamming_distance: Option<u32>` — populated for `"similar"` matches, `None` for exact/unique

**`ScanResult`** changes from `duplicates`/`uniques` to:
- `exact_matches: Vec<EvalFile>`
- `similar_matches: Vec<EvalFile>`
- `uniques: Vec<EvalFile>`
- `total_eval`, `skipped`, `stats` retained

**`ScanStats`** gains:
- `perceptual_compare_ms: u64` — time for Hamming distance comparisons (0 when disabled). dHash computation time is included in `ref_hash_ms`/`eval_hash_ms` since it runs inline during the hashing pipeline.

**`scan_folders`** command:
- After content comparison, if `perceptual_matching` is enabled:
  - Collect all reference dHashes (from cached `HashedFile` data)
  - For each non-exact eval file with a dHash, compare against all reference dHashes
  - If minimum Hamming distance ≤ threshold → `match_type: "similar"`, record the distance
  - Emit progress events with phase `"Comparing perceptual hashes"`
- Intra-eval content-hash duplicates continue to be detected and classified as `match_type: "exact"`

**`export_report`** updates:
- CSV `status` column values change from `"duplicate"`/`"unique"` to `"exact"`/`"similar"`/`"unique"`
- CSV gains optional `hamming_distance` column (empty for exact/unique)
- JSON export serializes the new `ScanResult` structure with all three groups
- Iterate over `exact_matches`, `similar_matches`, and `uniques` instead of `duplicates`/`uniques`

## Frontend

### Types: `src/types.ts`

**`ScanConfig`** gains:
- `perceptual_matching: boolean` (default: `false`)
- `perceptual_threshold: number` (default: `10`)

**`EvalFile`** changes:
- **Remove** `is_duplicate: boolean`
- **Add** `match_type: string`
- **Add** `hamming_distance: number | null`

**`ScanResult`** changes to:
- `exact_matches: EvalFile[]`
- `similar_matches: EvalFile[]`
- `uniques: EvalFile[]`
- `total_eval`, `skipped`, `stats` retained

**`ScanStats`** gains:
- `perceptual_compare_ms: number` (0 when disabled)

### SetupScreen: `src/screens/SetupScreen.tsx`

New "Perceptual Matching" section below hash algorithm selector:
- Toggle to enable/disable perceptual matching
- When enabled, three preset pills:
  - **Strict (≤5)** — hint: "Metadata changes, recompression"
  - **Moderate (≤10)** — hint: "Quality differences, minor crops"
  - **Loose (≤15)** — hint: "Significant changes — review carefully"
- Disabled (grayed out toggle) when no image categories are selected and "All Files" is not checked
- "All Files" counts as having image categories (since all files includes images)
- If user enables perceptual matching then deselects all image categories, the toggle auto-disables
- Settings persisted to localStorage
- Existing saved configs without the new fields use defaults (`false`, `10`)

### ResultsScreen: `src/screens/ResultsScreen.tsx`

**Summary bar** — four stats:
- Total scanned, Exact matches (red), Similar matches (amber), Unique (green)

**File list** — split into sections with headers:
- "Exact Matches" section — red status dots/badges
- "Similar Matches" section — amber status dots/badges, each row shows similarity percentage: `Math.round((64 - hamming_distance) / 64 * 100)` using the `hamming_distance` field from `EvalFile`
- Uniques section — green status dots

**Selection buttons:**
- "Select All" — selects all exact + similar matches
- "Select Exact" — selects only exact matches
- "Select Similar" — selects only similar matches
- `selectedFiles` remains a single `Set<string>`. Each button sets it to the appropriate subset.

**Initial selection state:** Default to selecting only exact matches (not similar). Similar matches should require explicit user action to select, since they carry higher false-positive risk.

**Actions** apply to all selected files regardless of match type. `handleConfirmAction` filters from `[...result.exact_matches, ...result.similar_matches]` instead of `result.duplicates`.

**Frontend references** to `is_duplicate` and `result.duplicates` replaced:
- `is_duplicate` checks become `match_type !== "unique"`
- `result.duplicates` becomes `result.exact_matches` (and `result.similar_matches` where applicable)

## Performance

- dHash computation: ~10-50ms per image (image decode dominates). Cached after first run.
- Perceptual comparison: XOR + popcount on 64-bit integers. 60k ref x 1k eval = 60M ops finishes in seconds. Linear scan is adequate up to ~100k reference files; beyond that, consider BK-tree (see Future Scope).
- Zero overhead when perceptual matching is disabled — steps 3-5 of the scan flow don't run.

## Testing

- Unit: `hamming_distance` function correctness (known inputs/outputs)
- Unit: `compute_dhash` on a known test image → known hash value (commit a small test fixture image)
- Unit: identical image with different EXIF metadata → Hamming distance 0
- Unit: completely different images → Hamming distance >> 10
- Unit: unsupported format returns `None`
- Integration: scan with perceptual matching enabled, verify "similar" matches found and `match_type` values correct
- Integration: scan with perceptual matching disabled returns same results as current behavior (backward compatibility)
- Frontend: verify grouped display, selection buttons, threshold presets

## Future Scope

- **HEIC/HEIF support:** Via `libheif` bindings for native Apple format decoding
- **Intra-eval perceptual matching:** Detect similar files within the eval folder itself (O(n^2) pairwise comparison)
- **BK-tree index:** Sub-linear Hamming distance search for very large reference libraries (100k+ files)
