import { useState, useEffect, useRef } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import type { ScanConfig, DupeMode } from "../types";

const LOCAL_STORAGE_KEY = "deduped-last-config";
const EXT_CUSTOM_KEY = "deduped-custom-extensions";
const EXT_REMOVED_KEY = "deduped-removed-extensions";

/** Default extensions per category — mirrors the Rust constants in hasher.rs */
const DEFAULT_EXTENSIONS: Record<string, string[]> = {
  images: [
    "jpg", "jpeg", "png", "tif", "tiff", "bmp", "webp", "heic", "heif",
    "cr2", "cr3", "nef", "arw", "orf", "rw2", "dng", "raf", "pef", "srw", "x3f",
  ],
  videos: [
    "mp4", "mov", "avi", "mkv", "m4v", "wmv", "flv", "webm", "mts", "m2ts",
  ],
  documents: [
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "md",
    "csv", "psd", "ai", "indd", "sketch", "fig",
  ],
  audio: [
    "mp3", "flac", "aac", "wav", "aiff", "ogg", "m4a", "wma", "alac",
  ],
};

interface SavedConfig {
  reference_dir: string;
  eval_dir: string;
  dupe_mode: "trash" | "move" | "review";
  dupeDest: string;
  moveUniques: boolean;
  uniqueDest: string;
  selectedCategories: string[];
  allFiles: boolean;
  hashAlgorithm?: string;
  perceptualMatching?: boolean;
  perceptualThreshold?: number;
}

function loadSavedConfig(): SavedConfig | null {
  try {
    const raw = localStorage.getItem(LOCAL_STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as SavedConfig;
  } catch (err) {
    console.warn("Failed to load saved config:", err);
    return null;
  }
}

function saveSavedConfig(config: SavedConfig): void {
  localStorage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(config));
}

function loadRecord(key: string): Record<string, string[]> {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return {};
    return JSON.parse(raw) as Record<string, string[]>;
  } catch (err) {
    console.warn("Failed to load saved record:", err);
    return {};
  }
}

function saveRecord(key: string, value: Record<string, string[]>): void {
  localStorage.setItem(key, JSON.stringify(value));
}

function initialDupeMode(initialConfig: ScanConfig | null, saved: SavedConfig | null): "trash" | "move" | "review" {
  if (initialConfig) {
    if (initialConfig.dupe_mode.type === "MoveToFolder") return "move";
    if (initialConfig.dupe_mode.type === "ReviewFirst") return "review";
    return "trash";
  }
  return saved?.dupe_mode ?? "trash";
}

function initialDupeDest(initialConfig: ScanConfig | null, saved: SavedConfig | null): string {
  if (initialConfig?.dupe_mode.type === "MoveToFolder") return initialConfig.dupe_mode.dest;
  return saved?.dupeDest ?? "";
}

interface Props {
  onStart: (config: ScanConfig) => void;
  initialConfig: ScanConfig | null;
}

interface DragDropPayload {
  paths: string[];
  position: { x: number; y: number };
}

export default function SetupScreen({ onStart, initialConfig }: Props) {
  const saved = useRef(loadSavedConfig()).current;

  const [referenceDir, setReferenceDir] = useState(initialConfig?.reference_dir ?? saved?.reference_dir ?? "");
  const [evalDir, setEvalDir] = useState(initialConfig?.eval_dir ?? saved?.eval_dir ?? "");
  const [dupeMode, setDupeMode] = useState<"trash" | "move" | "review">(
    initialDupeMode(initialConfig, saved)
  );
  const [dupeDest, setDupeDest] = useState(
    initialDupeDest(initialConfig, saved)
  );
  const [moveUniques, setMoveUniques] = useState(initialConfig?.move_uniques ?? saved?.moveUniques ?? false);
  const [uniqueDest, setUniqueDest] = useState(initialConfig?.unique_dest ?? saved?.uniqueDest ?? "");

  const [selectedCategories, setSelectedCategories] = useState<Set<string>>(
    () => {
      if (initialConfig) return new Set(initialConfig.categories);
      if (saved?.selectedCategories) return new Set(saved.selectedCategories);
      return new Set(["images"]);
    }
  );
  const [allFiles, setAllFiles] = useState(
    initialConfig?.all_files ?? saved?.allFiles ?? false
  );

  const [hashAlgorithm, setHashAlgorithm] = useState<string>(
    initialConfig?.hash_algorithm ?? saved?.hashAlgorithm ?? "sha256"
  );

  const [perceptualMatching, setPerceptualMatching] = useState(
    initialConfig?.perceptual_matching ?? saved?.perceptualMatching ?? false
  );
  const [perceptualThreshold, setPerceptualThreshold] = useState<number>(
    initialConfig?.perceptual_threshold ?? saved?.perceptualThreshold ?? 10
  );

  const hasImageCategory = allFiles || selectedCategories.has("images");

  useEffect(() => {
    if (!hasImageCategory) {
      setPerceptualMatching(false);
    }
  }, [hasImageCategory]);

  const [customExtensions, setCustomExtensions] = useState<Record<string, string[]>>(
    () => initialConfig?.custom_extensions ?? loadRecord(EXT_CUSTOM_KEY)
  );
  const [removedExtensions, setRemovedExtensions] = useState<Record<string, string[]>>(
    () => initialConfig?.removed_extensions ?? loadRecord(EXT_REMOVED_KEY)
  );

  const [customizerOpen, setCustomizerOpen] = useState(false);
  const [extInput, setExtInput] = useState("");

  const [dragOver, setDragOver] = useState<"reference" | "eval" | null>(null);
  const dragOverRef = useRef<"reference" | "eval" | null>(null);

  // Persist extension customizations to localStorage
  useEffect(() => {
    saveRecord(EXT_CUSTOM_KEY, customExtensions);
  }, [customExtensions]);

  useEffect(() => {
    saveRecord(EXT_REMOVED_KEY, removedExtensions);
  }, [removedExtensions]);

  useEffect(() => {
    let unlistenFn: (() => void) | undefined;

    (async () => {
      try {
        unlistenFn = await listen<DragDropPayload>("tauri://drag-drop", (event) => {
          const paths = event.payload.paths;
          if (paths.length > 0 && dragOverRef.current) {
            const droppedPath = paths[0];
            if (dragOverRef.current === "reference") {
              setReferenceDir(droppedPath);
            } else {
              setEvalDir(droppedPath);
            }
          }
          dragOverRef.current = null;
          setDragOver(null);
        });
      } catch {
        // Tauri drag-drop listener unavailable (e.g. plain browser / Playwright)
      }
    })();

    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, []);

  const pickFolder = async (setter: (path: string) => void) => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setter(selected);
  };

  const canStart =
    referenceDir !== "" &&
    evalDir !== "" &&
    (dupeMode !== "move" || dupeDest !== "") &&
    (!moveUniques || uniqueDest !== "") &&
    (allFiles || selectedCategories.size > 0);

  const handleStart = () => {
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

    let mode: DupeMode;
    if (dupeMode === "trash") mode = { type: "Trash" };
    else if (dupeMode === "move") mode = { type: "MoveToFolder", dest: dupeDest };
    else mode = { type: "ReviewFirst" };

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
  };

  // --- Extension customization helpers ---

  /** Compute the visible extension list for all selected categories */
  const buildExtensionList = () => {
    const entries: { ext: string; category: string; kind: "default" | "custom" | "removed" }[] = [];
    const seen = new Set<string>();

    for (const cat of selectedCategories) {
      const defaults = DEFAULT_EXTENSIONS[cat] ?? [];
      const removed = new Set((removedExtensions[cat] ?? []).map((e) => e.toLowerCase()));
      const custom = (customExtensions[cat] ?? []).map((e) => e.toLowerCase());

      for (const ext of defaults) {
        if (seen.has(ext)) continue;
        seen.add(ext);
        if (removed.has(ext)) {
          entries.push({ ext, category: cat, kind: "removed" });
        } else {
          entries.push({ ext, category: cat, kind: "default" });
        }
      }

      for (const ext of custom) {
        if (seen.has(ext)) continue;
        seen.add(ext);
        entries.push({ ext, category: cat, kind: "custom" });
      }
    }

    return entries;
  };

  const handleRemoveExtension = (ext: string, category: string, kind: "default" | "custom") => {
    if (kind === "custom") {
      // Remove from customExtensions
      setCustomExtensions((prev) => {
        const list = (prev[category] ?? []).filter((e) => e.toLowerCase() !== ext.toLowerCase());
        const next = { ...prev };
        if (list.length === 0) {
          delete next[category];
        } else {
          next[category] = list;
        }
        return next;
      });
    } else {
      // Add to removedExtensions
      setRemovedExtensions((prev) => {
        const list = prev[category] ?? [];
        if (list.some((e) => e.toLowerCase() === ext.toLowerCase())) return prev;
        return { ...prev, [category]: [...list, ext.toLowerCase()] };
      });
    }
  };

  const handleUndoRemove = (ext: string, category: string) => {
    setRemovedExtensions((prev) => {
      const list = (prev[category] ?? []).filter((e) => e.toLowerCase() !== ext.toLowerCase());
      const next = { ...prev };
      if (list.length === 0) {
        delete next[category];
      } else {
        next[category] = list;
      }
      return next;
    });
  };

  const handleAddExtension = (raw: string) => {
    const ext = raw.toLowerCase().replace(/^\.+/, "").trim();
    if (!ext) return;

    // Add to the first selected category
    const categories = Array.from(selectedCategories);
    if (categories.length === 0) return;
    const targetCat = categories[0];

    // Check if it's already a default for any selected category
    for (const cat of categories) {
      const defaults = DEFAULT_EXTENSIONS[cat] ?? [];
      if (defaults.includes(ext)) {
        // If it was removed, undo the removal instead
        const removed = removedExtensions[cat] ?? [];
        if (removed.some((e) => e.toLowerCase() === ext)) {
          handleUndoRemove(ext, cat);
        }
        return;
      }
    }

    // Check if already in custom for any selected category
    for (const cat of categories) {
      const custom = (customExtensions[cat] ?? []).map((e) => e.toLowerCase());
      if (custom.includes(ext)) return; // already exists
    }

    setCustomExtensions((prev) => {
      const list = prev[targetCat] ?? [];
      return { ...prev, [targetCat]: [...list, ext] };
    });
  };

  const extensionEntries = buildExtensionList();
  const showCategoryWarning = !allFiles && selectedCategories.size === 0;

  return (
    <div className="setup">
      <div className="folder-pickers">
        <div
          className={`folder-picker ${referenceDir ? "selected reference-selected" : ""} ${dragOver === "reference" ? "drag-over" : ""}`}
          onClick={() => pickFolder(setReferenceDir)}
          onDragEnter={(e) => { e.preventDefault(); dragOverRef.current = "reference"; setDragOver("reference"); }}
          onDragOver={(e) => e.preventDefault()}
          onDragLeave={() => { dragOverRef.current = dragOverRef.current === "reference" ? null : dragOverRef.current; setDragOver((prev) => prev === "reference" ? null : prev); }}
        >
          <span className="badge badge-protected">Protected</span>
          <div className="picker-label">Reference Folder</div>
          {referenceDir ? (
            <div className="picker-path">{referenceDir}</div>
          ) : (
            <div className="picker-hint">Click or drop a folder from Finder</div>
          )}
        </div>

        <div
          className={`folder-picker ${evalDir ? "selected eval-selected" : ""} ${dragOver === "eval" ? "drag-over" : ""}`}
          onClick={() => pickFolder(setEvalDir)}
          onDragEnter={(e) => { e.preventDefault(); dragOverRef.current = "eval"; setDragOver("eval"); }}
          onDragOver={(e) => e.preventDefault()}
          onDragLeave={() => { dragOverRef.current = dragOverRef.current === "eval" ? null : dragOverRef.current; setDragOver((prev) => prev === "eval" ? null : prev); }}
        >
          <span className="badge badge-eval">Checking</span>
          <div className="picker-label">Eval Folder</div>
          {evalDir ? (
            <div className="picker-path">{evalDir}</div>
          ) : (
            <div className="picker-hint">Click or drop a folder from Finder</div>
          )}
        </div>
      </div>

      <div className="config-section">
        <h3>File types</h3>
        <div className="category-pills">
          {(["images", "videos", "documents", "audio"] as const).map((cat) => (
            <button
              key={cat}
              className={`category-pill ${!allFiles && selectedCategories.has(cat) ? "active" : ""}`}
              onClick={() => {
                setAllFiles(false);
                setSelectedCategories((prev) => {
                  const next = new Set(prev);
                  if (next.has(cat)) next.delete(cat);
                  else next.add(cat);
                  return next;
                });
              }}
            >
              {cat.charAt(0).toUpperCase() + cat.slice(1)}
            </button>
          ))}
          <button
            className={`category-pill ${allFiles ? "active" : ""}`}
            onClick={() => {
              setAllFiles(true);
              setSelectedCategories(new Set());
            }}
          >
            All Files
          </button>
        </div>
        {showCategoryWarning && (
          <div className="category-warning">Select at least one file type to scan</div>
        )}

        {!allFiles && selectedCategories.size > 0 && (
          <div className="extension-customizer">
            <button
              className="customize-toggle"
              onClick={() => setCustomizerOpen((prev) => !prev)}
            >
              {customizerOpen ? "Hide extensions" : "Customize extensions..."}
            </button>
            {customizerOpen && (
              <>
                <div className="extension-tags">
                  {extensionEntries.map(({ ext, category, kind }) =>
                    kind === "removed" ? (
                      <span key={`${category}-${ext}`} className="extension-tag removed">
                        <span className="ext-text">.{ext}</span>
                        <button
                          className="ext-undo"
                          onClick={() => handleUndoRemove(ext, category)}
                          title="Restore"
                        >
                          undo
                        </button>
                      </span>
                    ) : (
                      <span
                        key={`${category}-${ext}`}
                        className={`extension-tag ${kind === "custom" ? "custom" : ""}`}
                      >
                        <span className="ext-text">.{ext}</span>
                        <button
                          className="ext-remove"
                          onClick={() => handleRemoveExtension(ext, category, kind)}
                          title="Remove"
                        >
                          &times;
                        </button>
                      </span>
                    )
                  )}
                </div>
                <input
                  type="text"
                  className="extension-input"
                  placeholder="Add extension..."
                  value={extInput}
                  onChange={(e) => setExtInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      handleAddExtension(extInput);
                      setExtInput("");
                    }
                  }}
                />
              </>
            )}
          </div>
        )}
      </div>

      <div className="config-section">
        <h3>Duplicate handling</h3>
        <div className="radio-group-3col">
          <label className={`radio-card ${dupeMode === "trash" ? "active" : ""}`}>
            <input
              type="radio"
              name="dupeMode"
              checked={dupeMode === "trash"}
              onChange={() => setDupeMode("trash")}
            />
            <div>
              <strong>Move to trash</strong>
              <span className="radio-desc">Send to macOS Trash (recoverable)</span>
            </div>
          </label>

          <label className={`radio-card ${dupeMode === "move" ? "active" : ""}`}>
            <input
              type="radio"
              name="dupeMode"
              checked={dupeMode === "move"}
              onChange={() => setDupeMode("move")}
            />
            <div>
              <strong>Move to folder</strong>
              <span className="radio-desc">Move duplicates to a specific folder</span>
            </div>
          </label>

          <label className={`radio-card ${dupeMode === "review" ? "active" : ""}`}>
            <input
              type="radio"
              name="dupeMode"
              checked={dupeMode === "review"}
              onChange={() => setDupeMode("review")}
            />
            <div>
              <strong>Review first</strong>
              <span className="radio-desc">Scan only -- decide after seeing results</span>
            </div>
          </label>
        </div>
        <div className="options-row">
          <div className="hash-algorithm-row">
            <span className="hash-label">Hash algorithm</span>
            <div className="hash-pills">
              <button
                className={`hash-pill ${hashAlgorithm === "sha256" ? "active" : ""}`}
                onClick={() => setHashAlgorithm("sha256")}
              >
                SHA-256
              </button>
              <button
                className={`hash-pill ${hashAlgorithm === "xxh3" ? "active" : ""}`}
                onClick={() => setHashAlgorithm("xxh3")}
              >
                xxHash
              </button>
            </div>
            <span className="hash-hint">
              {hashAlgorithm === "sha256"
                ? "Cryptographic, slower"
                : "Fast, non-cryptographic"}
            </span>
          </div>
          <div className="dest-picker-slot">
            {dupeMode === "move" && (
              <div className="inline-picker">
                <code className="path-display">{dupeDest || "No folder selected"}</code>
                <button className="btn-small" onClick={() => pickFolder(setDupeDest)}>
                  Browse...
                </button>
              </div>
            )}
          </div>
        </div>
        <div className="perceptual-row">
          <label className="toggle">
            <input
              type="checkbox"
              checked={perceptualMatching}
              onChange={(e) => setPerceptualMatching(e.target.checked)}
              disabled={!hasImageCategory}
            />
            <span className="toggle-slider" />
          </label>
          <span className={`hash-label ${!hasImageCategory ? "disabled" : ""}`}>
            Similar image detection
          </span>
          {perceptualMatching && hasImageCategory && (
            <>
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
            </>
          )}
        </div>
      </div>

      <div className="config-section">
        <h3>Non-duplicate handling</h3>
        <div className="unique-row">
          <div className="toggle-inline">
            <label className="toggle">
              <input
                type="checkbox"
                checked={moveUniques}
                onChange={(e) => setMoveUniques(e.target.checked)}
              />
              <span className="toggle-slider" />
            </label>
            <span>Move unique files to a separate folder</span>
          </div>
          <div className="inline-picker">
            <code className={`path-display ${!moveUniques ? "disabled" : ""}`}>
              {moveUniques ? (uniqueDest || "No folder selected") : "Unique files stay in place"}
            </code>
            <button
              className="btn-small"
              disabled={!moveUniques}
              onClick={() => pickFolder(setUniqueDest)}
            >
              Browse...
            </button>
          </div>
        </div>
      </div>

      <button className="btn-primary btn-start" disabled={!canStart} onClick={handleStart}>
        Start Scan
      </button>
    </div>
  );
}
