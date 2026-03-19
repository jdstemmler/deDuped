# Photo Dedup — Feature Requests

## FR-001: Configurable file type categories

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

### Priority

Medium-high. This is a natural generalization that makes the app useful beyond photography workflows. The core dedup logic doesn't change at all — it's purely a filtering concern.
