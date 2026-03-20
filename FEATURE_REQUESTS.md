# deDuped — Feature Requests

## FR-001: Configurable file type categories — SHIPPED (v0.2.0)

### Problem

The app currently only processes image/photo files. Users may want to dedup video libraries, document archives, or mixed-media folders. Hardcoding to photo extensions limits the tool's usefulness.

### Proposed solution

Add a file type selector to the setup screen (between the folder pickers and the duplicate handling options). The user picks which categories of files to include in the scan.

### Categories and default extensions

**Images** (on by default)
- Raw: `.cr2`, `.cr3`, `.nef`, `.arw`, `.orf`, `.rw2`, `.dng`, `.raf`, `.pef`, `.srw`, `.x3f`
- Standard: `.jpg`, `.jpeg`, `.png`, `.tif`, `.tiff`, `.bmp`, `.webp`
- Apple/mobile: `.heic`, `.heif`

**Videos**
- `.mp4`, `.mov`, `.avi`, `.mkv`, `.m4v`, `.wmv`, `.flv`, `.webm`, `.mts`, `.m2ts`

**Documents**
- Office: `.pdf`, `.doc`, `.docx`, `.xls`, `.xlsx`, `.ppt`, `.pptx`
- Text: `.txt`, `.rtf`, `.md`, `.csv`
- Design: `.psd`, `.ai`, `.indd`, `.sketch`, `.fig`

**Audio**
- `.mp3`, `.flac`, `.aac`, `.wav`, `.aiff`, `.ogg`, `.m4a`, `.wma`, `.alac`

**All files**
- Hashes everything regardless of extension. Skip known junk (`.DS_Store`, `Thumbs.db`, `.` prefixed hidden files).

### UI behavior

#### Category selector

Present categories as a multi-select group (pill buttons or checkboxes) on the setup screen. "Images" is selected by default. Selecting "All files" deselects the individual categories (and vice versa). Multiple categories can be combined — selecting "Images" + "Videos" scans for both.

#### Extension customization

Each category should be expandable (disclosure triangle or "customize" link) to reveal its extension list. From there the user can:

- **See all default extensions** for that category as a list of removable tags/pills
- **Add custom extensions** via a text input (type `.cr4` or just `cr4`, app normalizes to lowercase with leading dot)
- **Remove extensions** they don't care about (click the X on a tag)

Custom extensions persist across sessions (store in app preferences or the SQLite cache DB). They should be visually distinct from defaults (e.g., a small "custom" badge or different pill color) so the user knows what they added.

#### Example UI sketch

```
File types to scan
─────────────────────────────────────
[● Images]  [ Videos ]  [ Documents ]  [ Audio ]  [ All files ]

▾ Images (14 extensions)
┌──────────────────────────────────────────────┐
│ .cr2  .cr3  .nef  .arw  .orf  .rw2  .dng    │
│ .raf  .pef  .srw  .x3f  .jpg  .jpeg  .png   │
│ .tif  .tiff  .bmp  .webp  .heic  .heif      │
│ .cr4 ✕ [custom]                              │
│                                              │
│ [+ Add extension...]                         │
└──────────────────────────────────────────────┘
```

### Edge cases to handle

- **Sidecar files**: When "Images" is selected, optionally include `.xmp` sidecars. These aren't images themselves but are tightly coupled to raw files. Consider a sub-toggle: "Include XMP sidecars" (default off). When on, if a photo is moved/trashed, its sidecar follows.
- **Case sensitivity**: Extensions should be matched case-insensitively. `.CR3` and `.cr3` are the same.
- **No category selected**: Disable the scan button if no categories are selected and "All files" isn't checked.
- **Overlapping extensions**: If the user adds `.mp4` as a custom extension under "Images" but it already exists under "Videos", handle gracefully — deduplicate the extension list internally, don't error.
- **Empty categories**: If the user removes all extensions from a category, treat it as deselected.

### Data model

```rust
struct FileTypeConfig {
    // Which categories are enabled
    enabled_categories: HashSet<Category>,
    
    // Custom extensions per category (user-added)
    custom_extensions: HashMap<Category, Vec<String>>,
    
    // Removed defaults per category (user doesn't want these)
    removed_defaults: HashMap<Category, Vec<String>>,
    
    // Whether "All files" mode is active (overrides categories)
    all_files: bool,
}

enum Category {
    Images,
    Videos,
    Documents,
    Audio,
}
```

Persist `custom_extensions` and `removed_defaults` to app preferences so they survive across sessions. `enabled_categories` and `all_files` are per-scan settings and don't need to persist (or optionally remember last-used selection).

### Impact on existing architecture

- **Hasher**: Currently filters by a hardcoded extension set. Replace with the resolved extension list from `FileTypeConfig`.
- **Cache**: No changes needed — cache is keyed by path, doesn't care about file type.
- **Frontend**: New UI section on setup screen. Category state passed to backend with the scan command.
- **Results view**: Could optionally show a file type icon or extension badge on each result row to help distinguish when multiple categories are active.

### Status

SHIPPED in v0.2.0. Category selector with Images/Videos/Documents/Audio/All Files pills, plus collapsible per-category extension customization (add/remove/undo).

---

## FR-002: Perceptual hashing for near-duplicate detection — SHIPPED (v1.0.0)

### Problem

Content hashing (SHA-256/xxHash) compares files byte-for-byte. Two copies of the same photo with different metadata, compression levels, resolutions, or formats will have different content hashes and be treated as unique. This is the most common false-negative in photo dedup workflows — the same image re-exported from Lightroom, synced through iCloud with recompression, or saved with updated EXIF/XMP metadata.

### Algorithm: dHash (Difference Hash)

dHash is the best balance of speed, simplicity, and accuracy for this use case:

1. Decode image and resize to 9x8 grayscale (72 pixels)
2. Compare each pixel to its right neighbor (8x8 = 64 comparisons)
3. Output a 64-bit hash where each bit = "left pixel brighter than right neighbor"

Two images are "perceptually similar" if the Hamming distance (number of differing bits) between their dHashes is below a threshold. Empirically:
- 0 = identical image
- 1–5 = same image, minor differences (metadata, slight recompression)
- 6–10 = likely the same image with moderate changes (quality level, minor crop)
- 11–15 = possibly the same image with significant changes
- 16+ = probably different images

### Scope

**Supported for perceptual hashing** (formats the Rust `image` crate can decode):
- JPEG, PNG, TIFF, BMP, WebP

**Falls back to content-hash only** (would require heavy C library bindings):
- RAW formats (NEF, CR2, CR3, ARW, ORF, RW2, DNG, RAF, PEF, SRW, X3F)
- Apple formats (HEIC, HEIF)
- Video (MP4, MOV, AVI, MKV)

Content hashing still runs for ALL files regardless. Perceptual matching is an additional layer on top.

### What this catches that content hashing misses

- Same photo with different metadata (XMP, EXIF, IPTC edits)
- Same photo re-exported at different JPEG quality levels
- Same photo with minor crops or resolution changes
- Same photo saved in different formats (JPEG vs PNG vs TIFF)

### What this won't catch

- Heavily edited versions (color grading, filters, major crops)
- RAW vs processed versions (can't decode RAW without libraw)
- Video files

### Architecture

Perceptual matching is a **second pass**, not a replacement for content hashing:

1. Content hash comparison runs first → "Exact" duplicates
2. For eval files that weren't exact content matches, compare dHash Hamming distance against all reference files' perceptual hashes
3. Files below the threshold → "Similar" duplicates
4. Also detect intra-eval perceptual duplicates

### Backend implementation

**New dependency**: `image = "0.25"` in Cargo.toml

**New module**: `src-tauri/src/perceptual.rs`

```rust
/// Compute dHash for an image file. Returns None if the format can't be decoded.
pub fn compute_dhash(path: &Path) -> Option<u64> {
    let img = image::open(path).ok()?;
    let gray = img.resize_exact(9, 8, image::imageops::Lanczos3).to_luma8();
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

/// Hamming distance between two 64-bit hashes.
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
```

**Cache changes** (`cache.rs`):
- Add nullable `perceptual_hash INTEGER` column to the `file_hashes` table
- Migration for existing databases
- Update `get` to return the perceptual hash alongside the content hash
- Update `set` to store the perceptual hash

**Hasher changes** (`hasher.rs`):
- After content hashing, compute perceptual hash for supported image types
- Add `perceptual_hash: Option<u64>` to `HashedFile`
- Cache perceptual hashes alongside content hashes

**Command changes** (`commands.rs`):
- Add `perceptual_matching: bool` and `perceptual_threshold: u32` to `ScanConfig`
- After content comparison, run perceptual comparison pass for non-exact-match eval files
- Add `match_type: String` to `EvalFile` — values: "exact", "similar", "unique"
- Perceptual comparison is O(eval × ref) but each comparison is XOR + popcount on 64-bit integers — 60k ref × 1k eval = 60M ops finishes in seconds

### Frontend implementation

**Setup screen** (`SetupScreen.tsx`):
- "Perceptual matching" toggle below the hash algorithm selector
- When enabled, show threshold presets: Strict (≤5), Moderate (≤10), Loose (≤15)
- Disabled when no image categories are selected (perceptual hashing only works on images)
- Persist setting to localStorage

**Results screen** (`ResultsScreen.tsx`):
- Three match types with distinct colors:
  - Red "Exact" — content hash match (byte-for-byte identical)
  - Amber "Similar" — perceptual match (visually identical, different bytes)
  - Green "Unique" — no match
- Summary bar gets a fourth stat for similar matches
- Checkboxes apply to both exact and similar matches
- File rows for similar matches show Hamming distance as a similarity percentage

**Types** (`types.ts`):
- Add `perceptual_matching: boolean` and `perceptual_threshold: number` to `ScanConfig`
- Add `match_type: string` to `EvalFile`

### Performance considerations

- dHash computation requires decoding the full image → slower than content hashing (~10-50ms per image depending on size)
- Cached after first computation — subsequent scans use cached perceptual hashes
- Perceptual comparison loop is extremely fast (64-bit XOR + popcount)
- Only runs when the toggle is enabled — no impact when disabled
- For very large reference libraries (100k+), consider building a spatial index (BK-tree) for sub-linear Hamming distance search. Not needed for the initial implementation.

### Testing

- Unit test: known image → known dHash value
- Unit test: identical image with different metadata → Hamming distance 0
- Unit test: completely different images → Hamming distance >> 10
- Unit test: rotated/scaled version → Hamming distance within threshold
- Integration test: scan with perceptual matching enabled, verify "similar" matches are found

### Priority

High. This is the #1 differentiator that would set deDuped apart from basic hash-comparison tools. The TIF metadata example proves the need exists in real workflows.

### Status

SHIPPED in v1.0.0. dHash perceptual matching with Strict/Moderate/Loose presets, grouped results (Exact/Similar/Unique), and per-file similarity percentage display. Supported formats: JPEG, PNG, TIFF, BMP, WebP.

### Future scope

- **HEIC/HEIF support:** Via `libheif` bindings for native Apple format decoding
- **Intra-eval perceptual matching:** Detect similar files within the eval folder itself (O(n^2) pairwise comparison)
- **BK-tree index:** Sub-linear Hamming distance search for very large reference libraries (100k+ files)
