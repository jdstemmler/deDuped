# Code Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address all findings from the comprehensive code review — data safety fixes, test coverage gaps, documentation accuracy, and code polish.

**Architecture:** Four phases executed sequentially. Phase 1 (data safety) must land before merging to main. Phases 2-4 can follow incrementally.

**Tech Stack:** Rust (Tauri 2, rusqlite, rayon), React/TypeScript

**IMPORTANT:** Before running any cargo/rust commands, always set PATH: `export PATH="$HOME/.cargo/bin:$PATH"`

---

## Phase 1: Critical Data Safety (must fix before merge)

### Task 1: Validate reference and eval directories are different

**Files:**
- Modify: `src-tauri/src/commands.rs:136-145`
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Add validation in `scan_folders`**

In `src-tauri/src/commands.rs`, after the existing directory existence checks (line 144), add:

```rust
    // Prevent scanning the same folder as both reference and eval
    if ref_dir == eval_dir {
        return Err("Reference and eval folders must be different".to_string());
    }
    // Prevent one being a subdirectory of the other
    if ref_dir.starts_with(&eval_dir) || eval_dir.starts_with(&ref_dir) {
        return Err("Reference and eval folders cannot be nested inside each other".to_string());
    }
```

- [ ] **Step 2: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cd src-tauri && cargo test`
Expected: All existing tests pass (new validation doesn't affect existing tests since they use separate dirs).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "validate reference and eval directories are different

Prevent scanning the same folder as both reference and eval, and
prevent one being a subdirectory of the other. Either case would
flag all files as duplicates of themselves."
```

---

### Task 2: Add copy verification to cross-filesystem move

**Files:**
- Modify: `src-tauri/src/fileops.rs:59-67`
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Update the cross-filesystem fallback in `move_file`**

Replace lines 59-67 of `src-tauri/src/fileops.rs`:

```rust
    fs::rename(file_path, &final_target).or_else(|_| -> Result<(), String> {
        // fs::rename fails across filesystem boundaries. Fall back to copy + delete,
        // which is not atomic: if interrupted after copy, the file exists in both locations.
        fs::copy(file_path, &final_target)
            .map_err(|e| format!("Failed to copy to {}: {e}", final_target.display()))?;
        fs::remove_file(file_path)
            .map_err(|e| format!("Failed to remove source {}: {e}", file_path.display()))?;
        Ok(())
    })?;
```

With:

```rust
    fs::rename(file_path, &final_target).or_else(|rename_err| -> Result<(), String> {
        // fs::rename fails across filesystem boundaries. Fall back to copy + delete.
        fs::copy(file_path, &final_target)
            .map_err(|e| format!("Failed to move {} (rename: {rename_err}, copy: {e})", file_path.display()))?;

        // Verify the copy is complete before deleting the source
        let src_size = fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
        let dst_size = fs::metadata(&final_target).map(|m| m.len()).unwrap_or(0);
        if src_size != dst_size {
            let _ = fs::remove_file(&final_target);
            return Err(format!(
                "Copy verification failed for {} (src={src_size}, dst={dst_size})",
                file_path.display()
            ));
        }

        fs::remove_file(file_path)
            .map_err(|e| format!("Failed to remove source {}: {e}", file_path.display()))?;
        Ok(())
    })?;
```

- [ ] **Step 2: Apply the same fix to the undo path**

In `src-tauri/src/commands.rs`, replace the undo rename fallback (lines 553-558):

```rust
        let res = fs::rename(&dest, &target).or_else(|_| {
            fs::copy(&dest, &target)
                .map_err(|e| format!("Failed to copy {} back: {e}", dest.display()))?;
            fs::remove_file(&dest)
                .map_err(|e| format!("Failed to remove {}: {e}", dest.display()))?;
            Ok::<(), String>(())
        });
```

With:

```rust
        let res = fs::rename(&dest, &target).or_else(|rename_err| {
            fs::copy(&dest, &target)
                .map_err(|e| format!("Failed to restore {} (rename: {rename_err}, copy: {e})", dest.display()))?;
            let src_size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            let dst_size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            if src_size != dst_size {
                let _ = fs::remove_file(&target);
                return Err(format!(
                    "Copy verification failed restoring {} (src={src_size}, dst={dst_size})",
                    dest.display()
                ));
            }
            fs::remove_file(&dest)
                .map_err(|e| format!("Failed to remove {}: {e}", dest.display()))?;
            Ok::<(), String>(())
        });
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/fileops.rs src-tauri/src/commands.rs
git commit -m "verify copy integrity before deleting source in cross-fs moves

Check that destination file size matches source before removing the
original. Preserves the original rename error in fallback messages.
Applies to both move_file and undo_last_action paths."
```

---

### Task 3: Surface action log write failures

**Files:**
- Modify: `src-tauri/src/commands.rs:441-464`

- [ ] **Step 1: Replace the silent action log write**

Replace lines 441-464 in `src-tauri/src/commands.rs`:

```rust
    // Record the batch in the action log (best-effort).
    if !log_entries.is_empty() {
        let action_type = match &action {
            ActionMode::Trash => "trash",
            ActionMode::MoveToFolder { .. } => "move",
            ActionMode::Nothing => "nothing",
        };

        let millis = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let batch = ActionBatch {
            id: format!("{millis}"),
            timestamp: now,
            action_type: action_type.to_string(),
            entries: log_entries,
            eval_dir: eval_dir.clone(),
        };

        if let Ok(log) = ActionLog::default() {
            let _ = log.append(batch);
        }
    }
```

With:

```rust
    // Record the batch in the action log for undo support.
    if !log_entries.is_empty() {
        let action_type = match &action {
            ActionMode::Trash => "trash",
            ActionMode::MoveToFolder { .. } => "move",
            ActionMode::Nothing => "nothing",
        };

        let millis = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let batch = ActionBatch {
            id: format!("{millis}"),
            timestamp: now,
            action_type: action_type.to_string(),
            entries: log_entries,
            eval_dir: eval_dir.clone(),
        };

        let log_result = ActionLog::default().and_then(|log| log.append(batch));
        if let Err(e) = log_result {
            errors.push(format!("Warning: failed to record action for undo: {e}"));
        }
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "surface action log write failures instead of swallowing

If the undo log fails to write, the error is now reported in
ActionResult.errors so the user knows undo won't be available."
```

---

### Task 4: Fix partial undo removing entire batch

**Files:**
- Modify: `src-tauri/src/commands.rs:596-597`
- Modify: `src-tauri/src/actionlog.rs` (add `update_entries` method)

- [ ] **Step 1: Add `update_entries` to ActionLog**

In `src-tauri/src/actionlog.rs`, add a method to update a batch's entries (keeping only failed entries for retry):

```rust
    /// Replace a batch's entries with a subset (e.g., after partial undo).
    pub fn update_entries(&self, batch_id: &str, remaining_entries: Vec<ActionEntry>) -> Result<(), String> {
        let mut batches = self.load()?;
        if let Some(batch) = batches.iter_mut().find(|b| b.id == batch_id) {
            if remaining_entries.is_empty() {
                // All entries handled — remove the batch entirely
                batches.retain(|b| b.id != batch_id);
            } else {
                batch.entries = remaining_entries;
            }
        }
        self.write(&batches)
    }
```

- [ ] **Step 2: Update undo_last_action to handle partial undo**

In `src-tauri/src/commands.rs`, replace the batch removal at lines 596-597:

```rust
    let batch_id = batch.id.clone();
    log.remove_batch(&batch_id)?;
```

With:

```rust
    let batch_id = batch.id.clone();
    if errors.is_empty() {
        // All entries restored successfully — remove the batch
        log.remove_batch(&batch_id)?;
    } else if processed > 0 {
        // Partial undo — keep only the entries that failed
        let failed_sources: HashSet<String> = errors.iter()
            .filter_map(|e| {
                // Extract source paths from error messages for failed entries
                // We track which entries succeeded instead
                None::<String>
            })
            .collect();

        // Simpler approach: rebuild from the entries that were NOT successfully processed
        // We know which dest files we successfully restored (they no longer exist at dest)
        let remaining: Vec<ActionEntry> = batch.entries.iter()
            .filter(|entry| {
                // If the dest file still exists, this entry was NOT successfully undone
                entry.dest_path.as_ref()
                    .map(|d| Path::new(d).exists())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        log.update_entries(&batch_id, remaining)?;
    }
    // If processed == 0, leave the batch as-is for retry
```

Note: `ActionEntry` needs to derive `Clone`. Check if it already does. If not, add `#[derive(Clone)]` to it in `actionlog.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/actionlog.rs
git commit -m "fix partial undo removing entire action log batch

On partial undo, keep only the failed entries in the batch so the
user can retry. Only remove the batch when all entries succeed."
```

---

### Task 5: Surface sidecar operation warnings

**Files:**
- Modify: `src-tauri/src/fileops.rs:32-89`

- [ ] **Step 1: Update `trash_file` to return sidecar warnings**

Replace the `trash_file` function in `src-tauri/src/fileops.rs`:

```rust
/// Also trashes any associated sidecars. Returns warnings for sidecar failures.
pub fn trash_file(path: &Path) -> Result<Vec<String>, String> {
    let sidecars = find_sidecars(path);
    trash::delete(path).map_err(|e| format!("Failed to trash {}: {e}", path.display()))?;
    let mut warnings = Vec::new();
    for sidecar in sidecars {
        if let Err(e) = trash::delete(&sidecar) {
            warnings.push(format!("Sidecar {} could not be trashed: {e}", sidecar.display()));
        }
    }
    Ok(warnings)
}
```

- [ ] **Step 2: Update `move_file` to return sidecar warnings**

Change the return type of `move_file` to `Result<(PathBuf, Vec<String>), String>`. Replace the sidecar loop (lines 71-86):

```rust
    // Best-effort sidecar handling — failures are returned as warnings.
    let mut warnings = Vec::new();
    for sidecar in sidecars {
        if !sidecar.exists() {
            continue;
        }
        if let Ok(sidecar_relative) = sidecar.strip_prefix(base_dir) {
            let sidecar_target = resolve_collision(&dest_dir.join(sidecar_relative));
            if let Some(parent) = sidecar_target.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    warnings.push(format!("Sidecar dir {}: {e}", parent.display()));
                    continue;
                }
            }
            if let Err(e) = fs::rename(&sidecar, &sidecar_target).or_else(|_| {
                fs::copy(&sidecar, &sidecar_target)?;
                fs::remove_file(&sidecar)?;
                Ok::<(), std::io::Error>(())
            }) {
                warnings.push(format!("Sidecar {}: {e}", sidecar.display()));
            }
        }
    }

    Ok((final_target, warnings))
```

- [ ] **Step 3: Update callers in commands.rs**

In `execute_action`, update the `trash_file` and `move_file` call sites to collect sidecar warnings into the `errors` Vec (they're non-fatal, but surfaced to the user):

For the trash path (around line 398):
```rust
            ActionMode::Trash => {
                let res = fileops::trash_file(&file_path);
                match &res {
                    Ok(warnings) => {
                        for w in warnings {
                            errors.push(w.clone());
                        }
                        log_entries.push(ActionEntry { ... });
                    }
                    _ => {}
                }
                res.map(|_| ())
            }
```

For the move path (around line 411):
```rust
            ActionMode::MoveToFolder { dest } => {
                let dest_path_buf = PathBuf::from(dest);
                let res = fileops::move_file(&file_path, &eval_path, &dest_path_buf);
                match &res {
                    Ok((final_dest, warnings)) => {
                        for w in warnings {
                            errors.push(w.clone());
                        }
                        log_entries.push(ActionEntry { ... });
                    }
                    _ => {}
                }
                res.map(|_| ())
            }
```

- [ ] **Step 4: Fix tests that call trash_file and move_file**

Update all test call sites that use `trash_file` and `move_file` to handle the new return types:
- `trash_file` now returns `Result<Vec<String>, String>` — tests using `.unwrap()` still work but tests checking `is_ok()` need to match
- `move_file` now returns `Result<(PathBuf, Vec<String>), String>` — update destructuring from `let result = move_file(...)` to handle the tuple

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/fileops.rs src-tauri/src/commands.rs src-tauri/src/tests.rs
git commit -m "surface sidecar operation warnings to the user

Sidecar trash/move failures are now returned as warnings instead of
being silently swallowed. Users see these in the action result errors."
```

---

## Phase 2: Test Coverage Gaps

### Task 6: Add cross-filesystem move test

**Files:**
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Add test for copy+delete verification**

Since we can't easily create cross-filesystem scenarios in tests, test the verification logic by creating a test that exercises `move_file` normally and validates the return type. Also add a test for `resolve_collision` with an upper bound concern.

Append to `src-tauri/src/tests.rs`:

```rust
// ---------------------------------------------------------------------------
// move_file: verifies return type includes sidecar warnings
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test move_file_returns_empty_warnings`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/tests.rs
git commit -m "add test for move_file sidecar warnings return type"
```

---

### Task 7: Add csv_quote tests

**Files:**
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Add csv_quote unit tests**

Append to `src-tauri/src/tests.rs`:

```rust
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test csv_quote`
Expected: All 5 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/tests.rs
git commit -m "add unit tests for csv_quote RFC 4180 quoting"
```

---

### Task 8: Add resolve_extensions tests for custom/removed extensions

**Files:**
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Add tests**

Append to `src-tauri/src/tests.rs`:

```rust
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

#[test]
fn resolve_extensions_unknown_algorithm() {
    let result = hasher::hash_file(&PathBuf::from("/tmp/fake.txt"), "blake3");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown hash algorithm"));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test resolve_extensions_with`
Expected: All pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/tests.rs
git commit -m "add tests for custom/removed extensions and unknown algorithm"
```

---

### Task 9: Add cleanup_empty_dirs .DS_Store test

**Files:**
- Test: `src-tauri/src/tests.rs`

- [ ] **Step 1: Add test**

Append to `src-tauri/src/tests.rs`:

```rust
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test cleanup_removes_dir_with_only_ds_store`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/tests.rs
git commit -m "add test for cleanup_empty_dirs .DS_Store handling"
```

---

## Phase 3: Documentation Fixes

### Task 10: Fix README inaccuracies

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Fix "no reads" claim**

In `README.md` line 154, change:
```
- The **reference folder is never modified** — no reads, no writes, no deletes.
```
To:
```
- The **reference folder is never modified** — files are read for hashing but never moved, renamed, or deleted.
```

- [ ] **Step 2: Fix test count**

In `README.md` line 68, change `59 tests:` to match the actual count after Phase 2 changes. Use the actual count from running `cargo test`.

In `README.md` line 192, update `tests.rs` line to match.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "fix README: reference folder claim and test count"
```

---

### Task 11: Update stale module doc comments

**Files:**
- Modify: `src-tauri/src/commands.rs:1`
- Modify: `src-tauri/src/hasher.rs:1`
- Modify: `src-tauri/src/hasher.rs:200`

- [ ] **Step 1: Update doc comments**

In `commands.rs` line 1, change:
```rust
//! Tauri command handlers: folder scanning, duplicate action execution, and report export.
```
To:
```rust
//! Tauri command handlers: folder scanning, duplicate action execution, report export, file preview, action log, and undo.
```

In `hasher.rs` line 1, change:
```rust
//! File discovery, hashing (SHA-256 / xxHash), and cache-aware parallel hashing pipeline.
```
To:
```rust
//! File discovery, content hashing (SHA-256 / xxHash), perceptual hashing (dHash), and cache-aware parallel hashing pipeline.
```

In `hasher.rs`, update the `hash_files_cached` doc comment (line ~200):
```rust
/// Check cache serially, hash misses in parallel, update cache serially.
/// Legacy cache entries without perceptual hashes are backfilled on hit
/// for supported image formats.
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/hasher.rs
git commit -m "update module doc comments for perceptual hashing additions"
```

---

## Phase 4: Polish and Suggestions

### Task 12: Improve error surfacing for cache and cleanup operations

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Surface cleanup_empty_dirs errors**

Replace all `fileops::cleanup_empty_dirs(...).unwrap_or(0)` calls (lines ~436, 579, 581, 584) with:

```rust
match fileops::cleanup_empty_dirs(&path) {
    Ok(n) => n,
    Err(e) => {
        eprintln!("Warning: directory cleanup failed: {e}");
        0
    }
}
```

- [ ] **Step 2: Surface cache prune failure**

Replace line 176 (`let _ = cache.prune();`) with:

```rust
    if let Err(e) = cache.prune() {
        eprintln!("Warning: cache prune failed: {e}");
    }
```

- [ ] **Step 3: Run tests and commit**

```bash
cargo test
git add src-tauri/src/commands.rs
git commit -m "log warnings for cache prune and directory cleanup failures"
```

---

### Task 13: Add preview error display in frontend

**Files:**
- Modify: `src/screens/ResultsScreen.tsx:88-94`

- [ ] **Step 1: Capture preview error**

Add `previewError` state alongside `previewData`:

```typescript
const [previewError, setPreviewError] = useState<string | null>(null);
```

Update the catch block (around line 90):

```typescript
    } catch (err) {
      setPreviewData(null);
      setPreviewError(err instanceof Error ? err.message : String(err));
    }
```

Clear the error on new preview click:
```typescript
    setPreviewError(null);
```

Display the error in the preview panel (replace the "Failed to load preview" fallback):
```tsx
    <div className="preview-info">{previewError || "Failed to load preview"}</div>
```

- [ ] **Step 2: Commit**

```bash
git add src/screens/ResultsScreen.tsx
git commit -m "display specific error message when file preview fails"
```

---

### Task 14: Add console.warn to frontend catch blocks

**Files:**
- Modify: `src/screens/SetupScreen.tsx:43-49, 56-64`

- [ ] **Step 1: Add logging to catch blocks**

In `loadSavedConfig` (line ~46):
```typescript
  } catch (err) {
    console.warn("Failed to load saved config:", err);
    return null;
  }
```

In `loadRecord` (line ~61):
```typescript
  } catch (err) {
    console.warn("Failed to load saved record:", err);
    return {};
  }
```

- [ ] **Step 2: Commit**

```bash
git add src/screens/SetupScreen.tsx
git commit -m "add console.warn to frontend config deserialization catch blocks"
```

---

### Task 15: Improve ResultsScreen stats panel readability

**Files:**
- Modify: `src/screens/ResultsScreen.tsx`

- [ ] **Step 1: Extract stats IIFE to a variable**

Find the stats panel IIFE pattern (`{statsOpen && (() => { ... })()}`) and extract it:

```typescript
  const statsPanel = statsOpen ? (() => {
    const s = result.stats;
    // ... existing stats computation
    return (
      <div className="stats-panel">
        {/* ... existing JSX */}
      </div>
    );
  })() : null;
```

Then in JSX:
```tsx
  {statsPanel}
```

- [ ] **Step 2: Commit**

```bash
git add src/screens/ResultsScreen.tsx
git commit -m "extract stats panel from inline IIFE for readability"
```

---

### Task 16: Final verification

- [ ] **Step 1: Run full Rust test suite**

Run: `cargo test`
Expected: All tests pass (count should be ~70+).

- [ ] **Step 2: TypeScript type check**

Run: `npx tsc --noEmit`
Expected: No errors.

- [ ] **Step 3: Build both**

Run: `cargo build && npm run build`
Expected: Both succeed.

- [ ] **Step 4: Update README test count**

Update the test count in README.md to match the actual number from `cargo test` output.

- [ ] **Step 5: Commit and push**

```bash
git add README.md
git commit -m "update test count in README"
git push origin develop
```
