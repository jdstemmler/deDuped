//! File operations: move, trash, sidecar handling, collision resolution, and empty-dir cleanup.

use std::fs;
use std::path::{Path, PathBuf};

/// Checks two common naming conventions:
///   photo.xmp       (stem + .xmp)
///   photo.NEF.xmp   (full filename + .xmp)
pub(crate) fn find_sidecars(path: &Path) -> Vec<PathBuf> {
    let Some(parent) = path.parent() else { return Vec::new() };

    let stem = path.file_stem().and_then(|s| s.to_str());
    let name = path.file_name().and_then(|s| s.to_str());

    let candidates: Vec<PathBuf> = [
        stem.map(|s| parent.join(format!("{s}.xmp"))),
        stem.map(|s| parent.join(format!("{s}.XMP"))),
        name.map(|n| parent.join(format!("{n}.xmp"))),
        name.map(|n| parent.join(format!("{n}.XMP"))),
    ]
    .into_iter()
    .flatten()
    .filter(|c| c.exists() && c != path)
    .collect();

    let mut sidecars = candidates;
    sidecars.sort();
    sidecars.dedup();
    sidecars
}

/// Also trashes any associated sidecars.
pub fn trash_file(path: &Path) -> Result<(), String> {
    let sidecars = find_sidecars(path);
    trash::delete(path).map_err(|e| format!("Failed to trash {}: {e}", path.display()))?;
    for sidecar in sidecars {
        let _ = trash::delete(&sidecar);
    }
    Ok(())
}

/// Preserves subfolder structure relative to `base_dir`.
/// Also moves any associated sidecar files. Handles filename collisions with `-1`, `-2`, etc.
pub fn move_file(file_path: &Path, base_dir: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let relative = file_path
        .strip_prefix(base_dir)
        .map_err(|e| format!("Failed to compute relative path: {e}"))?;
    let target = dest_dir.join(relative);

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }

    let final_target = resolve_collision(&target);

    let sidecars = find_sidecars(file_path);

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

    // Best-effort: sidecar failures are silently ignored because losing an
    // .xmp is far less harmful than aborting the primary file operation.
    for sidecar in sidecars {
        if !sidecar.exists() {
            continue;
        }
        if let Ok(sidecar_relative) = sidecar.strip_prefix(base_dir) {
            let sidecar_target = resolve_collision(&dest_dir.join(sidecar_relative));
            if let Some(parent) = sidecar_target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&sidecar, &sidecar_target).or_else(|_| {
                fs::copy(&sidecar, &sidecar_target)?;
                fs::remove_file(&sidecar)?;
                Ok::<(), std::io::Error>(())
            });
        }
    }

    Ok(final_target)
}

/// Appends `-1`, `-2`, etc. before the extension if `target` already exists.
pub(crate) fn resolve_collision(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }

    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let parent = target.parent().unwrap_or(Path::new("."));

    for i in 1.. {
        let name = if ext.is_empty() {
            format!("{stem}-{i}")
        } else {
            format!("{stem}-{i}.{ext}")
        };
        let candidate = parent.join(&name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

pub fn cleanup_empty_dirs(dir: &Path) -> Result<usize, String> {
    let mut removed = 0;
    cleanup_empty_dirs_recursive(dir, dir, &mut removed)?;
    Ok(removed)
}

fn cleanup_empty_dirs_recursive(
    current: &Path,
    root: &Path,
    removed: &mut usize,
) -> Result<(), String> {
    if !current.is_dir() {
        return Ok(());
    }

    let entries: Vec<_> = fs::read_dir(current)
        .map_err(|e| format!("Failed to read dir {}: {e}", current.display()))?
        .filter_map(|e| e.ok())
        .collect();

    for entry in &entries {
        if entry.path().is_dir() {
            cleanup_empty_dirs_recursive(&entry.path(), root, removed)?;
        }
    }

    // Re-read after recursion -- children may have been removed
    if current != root {
        let still_entries: Vec<_> = fs::read_dir(current)
            .map_err(|e| format!("Failed to re-read dir {}: {e}", current.display()))?
            .filter_map(|e| e.ok())
            .collect();

        // Treat .DS_Store as junk — macOS creates these automatically and they
        // shouldn't prevent an otherwise-empty directory from being cleaned up.
        let non_junk: Vec<_> = still_entries
            .iter()
            .filter(|e| {
                e.file_name().to_str().map_or(true, |n| n != ".DS_Store")
            })
            .collect();

        if non_junk.is_empty() {
            // Remove any remaining .DS_Store before rmdir
            for entry in &still_entries {
                let _ = fs::remove_file(entry.path());
            }
            fs::remove_dir(current)
                .map_err(|e| format!("Failed to remove dir {}: {e}", current.display()))?;
            *removed += 1;
        }
    }

    Ok(())
}
