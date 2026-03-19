# Photo Dedup — Claude Code Handoff

## What is this?

A macOS desktop app for **one-way photo deduplication**. The user selects a "reference" folder (their existing photo library — never modified) and an "eval" folder (incoming files to check). The app hashes everything, identifies duplicates, and lets the user decide what to do with dupes and non-dupes.

### Why this exists

Every dedup app out there does two-way dedup: "find duplicates across these folders." That's fine until you have a 60k+ file photo library and a pile of unsorted files — you don't want to accidentally flag your library copies for deletion. This app's core differentiator is the **one-directional model**: the reference folder is read-only, period.

---

## Tech stack decision

**Tauri (Rust backend + web frontend)** is the recommended approach.

Why Tauri over Swift:
- Rust gives us fast, parallelized SHA-256 hashing out of the box (critical for 60k+ file libraries)
- Web frontend means we can reuse the mockup UI almost directly
- Tauri apps are tiny (~5-10MB vs Electron's 100MB+)
- Tauri's dialog API gives us native NSOpenPanel folder pickers
- The user is comfortable with web dev (FastAPI + Node/Vite experience)

Alternative: SwiftUI + Swift. Viable if the user prefers fully native. CryptoKit handles hashing, NSOpenPanel for folder pickers. More polished macOS integration but steeper learning curve if unfamiliar with Swift.

---

## Core architecture

```
┌─────────────────────────────────────┐
│           Tauri Frontend            │
│  (HTML/CSS/JS or React — see UI)    │
│                                     │
│  ┌─────────┐  ┌──────────────────┐  │
│  │ Folder   │  │ Results view     │  │
│  │ pickers  │  │ (file list,      │  │
│  │ + config │  │  stats, actions) │  │
│  └─────────┘  └──────────────────┘  │
└──────────────┬──────────────────────┘
               │ Tauri commands (invoke)
┌──────────────▼──────────────────────┐
│           Rust Backend              │
│                                     │
│  ┌─────────────────────────────┐    │
│  │ Hasher                      │    │
│  │ - SHA-256 content hashing   │    │
│  │ - Parallel with rayon       │    │
│  │ - Progress events to frontend│   │
│  └─────────────────────────────┘    │
│  ┌─────────────────────────────┐    │
│  │ Cache (SQLite)              │    │
│  │ - path, hash, size, mtime  │    │
│  │ - skip rehash if unchanged  │    │
│  └─────────────────────────────┘    │
│  ┌─────────────────────────────┐    │
│  │ File Operations             │    │
│  │ - macOS trash (trash crate) │    │
│  │ - move preserving structure │    │
│  │ - empty dir cleanup         │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

### Rust crates to use

- `sha2` — SHA-256 hashing
- `rayon` — parallel iteration for hashing
- `rusqlite` — SQLite for hash cache
- `trash` — cross-platform "move to trash" (uses macOS trash on macOS)
- `walkdir` — recursive directory traversal
- `tauri` — app framework
- `serde` / `serde_json` — serialization for frontend communication

---

## Hash caching strategy

The reference folder may have 60k+ files. Rehashing every run is slow. Use a SQLite cache:

```sql
CREATE TABLE file_hashes (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_secs INTEGER NOT NULL,
    mtime_nanos INTEGER NOT NULL
);
```

**Cache hit logic**: If `path` exists in cache AND `size` matches AND `mtime` matches → use cached hash. Otherwise, rehash and update cache.

Cache location: `~/Library/Application Support/com.photodedup/cache.db` (or similar Tauri app data path).

Invalidation: If a cached path no longer exists on disk, prune it from the cache on next run.

---

## File type filtering

Only process files with these extensions (case-insensitive):

**Photos**: .jpg, .jpeg, .png, .tif, .tiff, .heic, .heif, .cr2, .cr3, .nef, .arw, .orf, .rw2, .dng, .raf, .pef, .srw, .x3f

**Video** (optional, include by default): .mp4, .mov, .avi, .mkv

Skip hidden files/folders (starting with `.`), `.DS_Store`, thumbs.db, sidecar files (.xmp) — or make sidecar handling configurable later.

---

## Application flow

### Screen 1: Setup

Two large folder picker areas:
1. **Reference folder** — opens NSOpenPanel (via Tauri dialog API), shows full path once selected. Tagged with "Protected" badge. This folder is NEVER modified.
2. **Eval folder** — same picker pattern. Tagged with "Checking" badge.

**Duplicate handling options** (radio group):
- **Move to trash** (default) — uses macOS trash, recoverable. No additional config needed.
- **Move to folder** — shows a "Browse..." path picker for the destination folder. Subfolder structure from eval folder is preserved inside the destination. The action button should be disabled until a folder is selected.
- **Review first** — runs scan without acting. Decision deferred to results screen.

**Non-duplicate handling** (toggle, default OFF):
- OFF = unique files stay in place in the eval folder.
- ON = shows a "Browse..." path picker for destination. Subfolder structure preserved. e.g., `eval/2025-06-19/DSC_0091.NEF` → `dest/2025-06-19/DSC_0091.NEF`

**Start scan button**: Disabled until both reference and eval folders are selected. If "move to folder" is chosen for dupes, also require that folder to be selected.

### Screen 2: Scanning (progress)

Two-phase progress bar:
1. **"Indexing reference folder..."** (0-~70% of time, depending on cache hits)
2. **"Checking eval folder..."** (~30% of time)

Show: phase label, percentage, file count progress (e.g., `[12,450 / 18,323]`).

Backend emits progress events to frontend via Tauri event system.

### Screen 3: Results

**Summary stats** (3 metric cards):
- Total scanned (from eval folder)
- Duplicates found (red)
- Unique files (green)

**File list**: Scrollable list showing each eval file with:
- Red/green status dot
- Relative file path (monospace)
- File size
- "Duplicate" / "Unique" tag

**If mode was "Review first"**: Show an action panel below the file list with:
- Move to trash
- Move to folder (with Browse... picker)
- Leave them alone
- Action button disabled until folder is selected (if "move to folder" chosen)

**If mode was "Move to trash" or "Move to folder"**: Show confirmation action bar:
- Cancel button (goes back to setup)
- Action button (red for trash, standard for move)

**After action completes**: Show a brief completion summary. Clean up empty directories in eval folder.

### Navigation
- "← New scan" link on results screen goes back to setup
- Setup state is preserved when going back

---

## File operations — critical rules

1. **NEVER permanently delete files.** Trash or move only.
2. **NEVER modify, move, or delete anything in the reference folder.**
3. When moving files (dupes or uniques), **preserve subfolder structure** relative to the eval folder root.
4. After all moves/deletes, **clean up empty directories** in the eval folder (walk bottom-up, rmdir if empty).
5. Handle filename collisions at destination: if `dest/2025-06-19/IMG_001.CR3` already exists, append a suffix like `-1`, `-2`, etc. Don't silently overwrite.
6. Catch intra-eval duplicates: if the eval folder itself contains two identical files, only keep/move one copy.

---

## UI design reference

The mockup was built with these design tokens (from Claude.ai's design system, but adapt to your preferred styling):

**General aesthetic**: Clean, flat, minimal. White surfaces, thin borders, generous whitespace. No gradients or shadows.

**Colors**:
- Primary text: near-black
- Secondary text: muted gray
- Borders: very light gray (0.5px)
- Protected/reference badge: green tint (#E1F5EE bg, #085041 text)
- Eval/checking badge: blue tint (#E6F1FB bg, #0C447C text)
- Duplicate status: red (#E24B4A dot, #FCEBEB/#791F1F tag)
- Unique status: green (#1D9E75 dot, #E1F5EE/#085041 tag)
- Trash/danger button: #A32D2D background
- Primary button: dark (near-black) background

**Folder pickers**: Large dashed-border areas, become solid border with tinted background when a folder is selected. Show full filesystem path.

**Path pickers** (for dupe/unique destinations): Inline row with monospace path display + "Browse..." button.

**Radio groups**: Card-style rows with custom radio circles, selected row gets tinted background.

**Toggle**: iOS-style toggle switch for the "move unique files" option.

**Results file list**: Monospace filenames, scrollable container, status dot + tag on each row with file size.

---

## Stretch goals (not MVP)

- **Thumbnail preview**: Show a small thumbnail next to each file in results, and optionally show the matched reference file side-by-side for dupes.
- **Drag-and-drop**: Allow dragging folders onto the picker areas instead of clicking Browse.
- **Configurable hash algorithm**: SHA-256 is the default, but offer xxHash for speed if the user doesn't need cryptographic strength.
- **Export report**: Save scan results as CSV or JSON.
- **Menu bar mode**: Run as a lightweight menu bar app that watches the eval folder and auto-scans on changes (FSEvents).
- **Sidecar handling**: When moving/trashing a photo, also move its .xmp sidecar if one exists alongside it.
- **Remember last-used folders**: Persist the reference and eval folder paths between sessions.

---

## Summary

Build a Tauri macOS app with:
- Rust backend: parallel SHA-256 hashing, SQLite cache, safe file ops (trash crate), progress events
- Web frontend: setup screen (folder pickers + config), scanning progress, results with action panel
- One-way dedup model: reference folder is read-only, eval folder gets acted on
- Three dupe modes: trash, move to folder, review-then-decide
- Optional unique file move with structure preservation
- Never permanently delete anything
