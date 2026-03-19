#!/usr/bin/env python3
"""
Photo deduplication script.

Hashes all files in a library folder, then checks an inbox folder against it.
- Duplicates are deleted from the inbox.
- Unique files are moved to an import staging folder.

Usage:
    python3 dedup_photos.py [--dry-run]
    python3 dedup_photos.py --base /path/to/photos [--dry-run]
"""

import argparse
import hashlib
import os
import shutil
import sys
from pathlib import Path


# Adjust these defaults to match your setup, or pass --base at runtime
DEFAULT_BASE = "/Volumes/Thunderbay/OrganizedFiles/Media/Photos"
LIBRARY_DIR = "LRLibraryAssets"
INBOX_DIR = "SuspectedDuplicates"
IMPORT_DIR = "_to_import"

# Only process files with these extensions (case-insensitive)
PHOTO_EXTENSIONS = {
    ".jpg", ".jpeg", ".png", ".tif", ".tiff", ".heic", ".heif",
    ".cr2", ".cr3", ".nef", ".arw", ".orf", ".rw2", ".dng",
    ".raf", ".pef", ".srw", ".x3f",
    ".mp4", ".mov", ".avi", ".mkv",  # video if you want
}


def hash_file(path: Path, chunk_size: int = 65536) -> str:
    """SHA-256 hash of file contents."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(chunk_size):
            h.update(chunk)
    return h.hexdigest()


def is_photo(path: Path) -> bool:
    return path.suffix.lower() in PHOTO_EXTENSIONS


def build_library_index(library_path: Path) -> set[str]:
    """Walk the library and return a set of content hashes."""
    hashes = set()
    files = [p for p in library_path.rglob("*") if p.is_file() and is_photo(p)]
    total = len(files)
    print(f"Indexing {total:,} files in {library_path.name}...")

    for i, filepath in enumerate(files, 1):
        if i % 500 == 0 or i == total:
            print(f"  [{i:,}/{total:,}] {filepath.name}")
        try:
            hashes.add(hash_file(filepath))
        except (OSError, PermissionError) as e:
            print(f"  WARN: Could not read {filepath}: {e}")

    print(f"Indexed {len(hashes):,} unique hashes.\n")
    return hashes


def process_inbox(
    inbox_path: Path,
    import_path: Path,
    library_hashes: set[str],
    dry_run: bool = False,
) -> tuple[int, int, int]:
    """Check inbox files against library hashes. Delete dupes, move unique."""
    files = [p for p in inbox_path.rglob("*") if p.is_file() and is_photo(p)]
    total = len(files)
    dupes = 0
    unique = 0
    errors = 0

    print(f"Processing {total:,} files in {inbox_path.name}...")
    if dry_run:
        print("  *** DRY RUN — no files will be moved or deleted ***\n")

    for filepath in files:
        try:
            file_hash = hash_file(filepath)
        except (OSError, PermissionError) as e:
            print(f"  ERROR: Could not read {filepath}: {e}")
            errors += 1
            continue

        if file_hash in library_hashes:
            dupes += 1
            print(f"  DUP:  {filepath.relative_to(inbox_path)}")
            if not dry_run:
                filepath.unlink()
        else:
            unique += 1
            # Preserve any subfolder structure from the inbox
            relative = filepath.relative_to(inbox_path)
            dest = import_path / relative
            print(f"  MOVE: {relative} -> {import_path.name}/{relative}")
            if not dry_run:
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(filepath), str(dest))
            # Add hash so subsequent dupes within the inbox are caught too
            library_hashes.add(file_hash)

    return dupes, unique, errors


def cleanup_empty_dirs(path: Path, dry_run: bool = False):
    """Remove empty directories left behind after moves/deletes."""
    # Walk bottom-up so children are removed before parents
    for dirpath in sorted(path.rglob("*"), reverse=True):
        if dirpath.is_dir() and not any(dirpath.iterdir()):
            print(f"  RMDIR: {dirpath.relative_to(path)}")
            if not dry_run:
                dirpath.rmdir()


def main():
    parser = argparse.ArgumentParser(description="Dedup photos against library.")
    parser.add_argument(
        "--base",
        type=Path,
        default=Path(DEFAULT_BASE),
        help=f"Base photos directory (default: {DEFAULT_BASE})",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show what would happen without making changes.",
    )
    args = parser.parse_args()

    library_path = args.base / LIBRARY_DIR
    inbox_path = args.base / INBOX_DIR
    import_path = args.base / IMPORT_DIR

    # Sanity checks
    for name, path in [("Library", library_path), ("Inbox", inbox_path)]:
        if not path.is_dir():
            print(f"ERROR: {name} directory not found: {path}")
            sys.exit(1)

    import_path.mkdir(parents=True, exist_ok=True)

    # Build index of what's already in the library
    library_hashes = build_library_index(library_path)

    # Process the inbox
    dupes, unique, errors = process_inbox(
        inbox_path, import_path, library_hashes, dry_run=args.dry_run
    )

    # Clean up empty dirs in the inbox
    print(f"\nCleaning up empty directories in {inbox_path.name}...")
    cleanup_empty_dirs(inbox_path, dry_run=args.dry_run)

    # Summary
    print(f"\n{'=' * 40}")
    print(f"{'DRY RUN ' if args.dry_run else ''}SUMMARY")
    print(f"{'=' * 40}")
    print(f"  Duplicates deleted:  {dupes:,}")
    print(f"  Unique files moved:  {unique:,}")
    print(f"  Errors:              {errors:,}")
    print(f"{'=' * 40}")

    if args.dry_run and (dupes or unique):
        print("\nRun again without --dry-run to apply changes.")


if __name__ == "__main__":
    main()
