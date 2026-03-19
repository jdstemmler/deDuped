# deDuped

A macOS desktop app for **one-way photo deduplication**. Point it at your photo library (reference) and a pile of unsorted files (eval) — it hashes everything, finds duplicates, and lets you trash or move them. The reference folder is never modified.

## Why this exists

Every dedup tool does two-way dedup. That's dangerous when you have a 60k+ file library and incoming files — you don't want to accidentally flag library copies for deletion. deDuped's core differentiator is the **one-directional model**: reference is read-only, only eval files get acted on.

## Tech stack

- **Backend**: Rust (Tauri 2)
  - Parallel SHA-256 hashing via `rayon`
  - SQLite hash cache — skip rehashing unchanged files across runs
  - Safe file operations — macOS Trash via `trash` crate, move with structure preservation
  - XMP sidecar handling — sidecars follow their parent file
- **Frontend**: React + TypeScript (Vite)
  - Setup screen with folder pickers and config options
  - Real-time scanning progress via Tauri events
  - Results view with action panel

## Getting started

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) (v18+)
- macOS (uses native APIs for trash, folder pickers, etc.)

### Development

```bash
# Install frontend dependencies
npm install

# Run in dev mode (hot-reloading frontend + Rust backend)
cargo tauri dev
```

### Build

```bash
# Production build — outputs .app and .dmg
cargo tauri build
```

The built app is at `src-tauri/target/release/bundle/macos/deDuped.app`.

## Usage

1. **Select folders** — pick your reference (photo library) and eval (incoming files) folders
2. **Configure** — choose how to handle duplicates:
   - **Move to trash** (default) — recoverable via macOS Trash
   - **Move to folder** — relocate dupes to a specific directory, preserving subfolder structure
   - **Review first** — scan only, decide after seeing results
3. **Scan** — the app hashes both folders and identifies duplicates
4. **Act** — confirm the action on the results screen

Optionally toggle "move unique files" to sort non-duplicates into a separate folder.

## How it works

- Files are hashed with SHA-256 for content-based comparison
- A SQLite cache (`~/Library/Application Support/com.photodedup/cache.db`) stores hashes keyed by path + size + mtime — unchanged files aren't rehashed
- Supported file types: common photo formats (JPEG, PNG, HEIC, RAW variants), video (MP4, MOV, AVI, MKV)
- Hidden files, `.DS_Store`, and non-media files are skipped
- Intra-eval duplicates are detected (if eval contains two identical files, only one is kept)
- XMP sidecar files (`.xmp`) are automatically moved/trashed alongside their parent photo
- Empty directories in the eval folder are cleaned up after file operations

## Project structure

```
src/                    # React frontend
  screens/              # Setup, Scanning, Results screens
  types.ts              # Shared TypeScript types
  styles.css            # Design tokens and component styles
src-tauri/
  src/
    cache.rs            # SQLite hash cache
    hasher.rs           # File collection + parallel SHA-256 hashing
    fileops.rs          # Trash, move, sidecar handling, dir cleanup
    commands.rs         # Tauri commands (scan_folders, execute_action)
    lib.rs / main.rs    # App entry points
  tauri.conf.json       # Tauri app configuration
```
