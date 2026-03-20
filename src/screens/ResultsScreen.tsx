import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { ScanConfig, ScanResult, ActionMode, ActionResult, EvalFile, FilePreview, ProgressEvent } from "../types";

interface Props {
  config: ScanConfig;
  result: ScanResult;
  onNewScan: () => void;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatTime(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

export default function ResultsScreen({ config, result, onNewScan }: Props) {
  const isReview = config.dupe_mode.type === "ReviewFirst";
  const [reviewAction, setReviewAction] = useState<"trash" | "move" | "nothing">("trash");
  const [reviewDest, setReviewDest] = useState("");
  const [actionDone, setActionDone] = useState(false);
  const [actionResult, setActionResult] = useState<ActionResult | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const [exportSuccess, setExportSuccess] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [actionCurrent, setActionCurrent] = useState(0);
  const [actionTotal, setActionTotal] = useState(0);
  const [statsOpen, setStatsOpen] = useState(false);
  const exportTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // File preview state
  const [selectedPreview, setSelectedPreview] = useState<string | null>(null);
  const [previewData, setPreviewData] = useState<FilePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);

  // Undo state
  const [lastActionType, setLastActionType] = useState<string | null>(null);
  const [undoing, setUndoing] = useState(false);
  const [undoResult, setUndoResult] = useState<ActionResult | null>(null);
  const [undoError, setUndoError] = useState<string | null>(null);

  // Selective file actions: track which duplicate files are selected
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(
    () => new Set(result.exact_matches.map((f) => f.path))
  );
  const [viewFilter, setViewFilter] = useState<"all" | "exact" | "similar" | "unique">("all");
  const [resultsThreshold, setResultsThreshold] = useState(config.perceptual_threshold);

  // Re-partition similar matches and uniques based on the current threshold.
  // The backend returns all matches with distance <= 15 (loosest). The
  // frontend filters by the user's chosen threshold for instant re-classification.
  const filteredSimilar = result.similar_matches.filter(
    (f) => f.hamming_distance != null && f.hamming_distance <= resultsThreshold
  );
  const reclassifiedUniques = result.similar_matches.filter(
    (f) => f.hamming_distance == null || f.hamming_distance > resultsThreshold
  );
  const allUniques = [...reclassifiedUniques, ...result.uniques];

  const exactPaths = result.exact_matches.map((f) => f.path);
  const similarPaths = filteredSimilar.map((f) => f.path);
  const allMatchPaths = [...exactPaths, ...similarPaths];
  const selectedCount = selectedFiles.size;
  const totalMatches = allMatchPaths.length;

  const toggleFile = (path: string) => {
    setSelectedFiles((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const selectAll = () => setSelectedFiles(new Set(allMatchPaths));
  const selectExact = () => setSelectedFiles(new Set(exactPaths));
  const selectSimilar = () => setSelectedFiles(new Set(similarPaths));
  const selectNone = () => setSelectedFiles(new Set());

  const handleFileClick = async (path: string) => {
    if (selectedPreview === path) {
      setSelectedPreview(null);
      setPreviewData(null);
      return;
    }
    setSelectedPreview(path);
    setPreviewData(null);
    setPreviewError(null);
    setPreviewLoading(true);
    try {
      const data = await invoke<FilePreview>("get_file_preview", { path });
      setPreviewData(data);
    } catch (err) {
      setPreviewData(null);
      setPreviewError(err instanceof Error ? err.message : String(err));
    } finally {
      setPreviewLoading(false);
    }
  };

  useEffect(() => {
    return () => {
      if (exportTimerRef.current) clearTimeout(exportTimerRef.current);
    };
  }, []);

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setReviewDest(selected);
  };

  const handleExport = async (format: "csv" | "json") => {
    setExportError(null);
    const ext = format === "csv" ? "csv" : "json";
    const filePath = await save({
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      defaultPath: `scan-report.${ext}`,
    });
    if (!filePath) return;
    setExporting(true);
    try {
      await invoke("export_report", {
        results: result,
        format,
        destPath: filePath,
      });
      setExportSuccess(true);
      if (exportTimerRef.current) clearTimeout(exportTimerRef.current);
      exportTimerRef.current = setTimeout(() => setExportSuccess(false), 3000);
    } catch (err) {
      setExportError(err instanceof Error ? err.message : String(err));
    } finally {
      setExporting(false);
    }
  };

  const hasSelection = selectedCount > 0;
  const canAct = isReview
    ? (reviewAction === "nothing" || hasSelection) && (reviewAction !== "move" || reviewDest !== "")
    : hasSelection;

  const handleAction = async (dupeFiles: EvalFile[], mode: ActionMode) => {
    if (mode.type === "Nothing") {
      setLastActionType("nothing");
      setActionResult({ processed: 0, errors: [], dirs_cleaned: 0 });
      setActionDone(true);
      return;
    }

    setExecuting(true);
    setActionError(null);
    setActionCurrent(0);
    setActionTotal(dupeFiles.length);

    const unlisten = await listen<ProgressEvent>("scan-progress", (event) => {
      setActionCurrent(event.payload.current);
      setActionTotal(event.payload.total);
    });

    try {
      const res = await invoke<ActionResult>("execute_action", {
        evalDir: config.eval_dir,
        files: dupeFiles.map((f) => f.path),
        action: mode,
      });

      let merged = { ...res };

      if (config.move_uniques && config.unique_dest) {
        const uniqueRes = await invoke<ActionResult>("execute_action", {
          evalDir: config.eval_dir,
          files: allUniques.map((f) => f.path),
          action: { type: "MoveToFolder", dest: config.unique_dest },
        });
        merged = {
          processed: merged.processed + uniqueRes.processed,
          errors: [...merged.errors, ...uniqueRes.errors],
          dirs_cleaned: merged.dirs_cleaned + uniqueRes.dirs_cleaned,
        };
      }

      setLastActionType(mode.type === "Trash" ? "trash" : "move");
      setActionResult(merged);
      setActionDone(true);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      unlisten();
      setExecuting(false);
    }
  };

  const handleUndo = async () => {
    setUndoing(true);
    setUndoError(null);
    try {
      const res = await invoke<ActionResult>("undo_last_action");
      setUndoResult(res);
    } catch (err) {
      setUndoError(err instanceof Error ? err.message : String(err));
    } finally {
      setUndoing(false);
    }
  };

  const handleConfirmAction = () => {
    let mode: ActionMode;
    if (isReview) {
      if (reviewAction === "trash") mode = { type: "Trash" };
      else if (reviewAction === "move") mode = { type: "MoveToFolder", dest: reviewDest };
      else mode = { type: "Nothing" };
    } else if (config.dupe_mode.type === "Trash") {
      mode = { type: "Trash" };
    } else if (config.dupe_mode.type === "MoveToFolder") {
      mode = { type: "MoveToFolder", dest: config.dupe_mode.dest };
    } else {
      return;
    }
    const filesToAct = [...result.exact_matches, ...filteredSimilar].filter((f) => selectedFiles.has(f.path));
    handleAction(filesToAct, mode);
  };

  if (actionDone && actionResult) {
    const canUndo = lastActionType === "move" && !undoResult;
    const isTrash = lastActionType === "trash";

    return (
      <div className="results">
        <div className="completion">
          <h2>{undoResult ? "Undo Complete" : "Done"}</h2>
          {undoResult ? (
            <>
              <p>{undoResult.processed} files restored</p>
              {undoResult.dirs_cleaned > 0 && (
                <p>{undoResult.dirs_cleaned} empty directories cleaned up</p>
              )}
              {undoResult.errors.length > 0 && (
                <div className="error-list">
                  <h4>Errors ({undoResult.errors.length})</h4>
                  {undoResult.errors.map((e, i) => (
                    <div key={i} className="error-item">{e}</div>
                  ))}
                </div>
              )}
            </>
          ) : (
            <>
              <p>{actionResult.processed} files processed</p>
              {actionResult.dirs_cleaned > 0 && (
                <p>{actionResult.dirs_cleaned} empty directories cleaned up</p>
              )}
              {actionResult.errors.length > 0 && (
                <div className="error-list">
                  <h4>Errors ({actionResult.errors.length})</h4>
                  {actionResult.errors.map((e, i) => (
                    <div key={i} className="error-item">{e}</div>
                  ))}
                </div>
              )}
            </>
          )}
          {undoError && (
            <div className="error-list">
              <div className="error-item">{undoError}</div>
            </div>
          )}
          <div className="completion-actions">
            <button className="btn-primary" onClick={onNewScan}>
              &larr; New Scan
            </button>
            {canUndo && (
              <button
                className="undo-btn"
                disabled={undoing}
                onClick={handleUndo}
              >
                {undoing ? "Undoing..." : "Undo Move"}
              </button>
            )}
            {isTrash && !undoResult && (
              <button
                className="undo-btn"
                disabled
                title="Restore from Trash manually"
              >
                Undo (N/A)
              </button>
            )}
          </div>
        </div>
      </div>
    );
  }

  const statsPanel = statsOpen ? (() => {
    const s = result.stats;
    const totalFiles = s.ref_file_count + s.eval_file_count;
    const totalCacheHits = s.ref_cache_hits + s.eval_cache_hits;
    const cacheRate = totalFiles > 0 ? ((totalCacheHits / totalFiles) * 100).toFixed(1) : "0.0";
    const throughputMBs = s.total_ms > 0
      ? ((s.total_bytes / (1024 * 1024)) / (s.total_ms / 1000)).toFixed(1)
      : "0.0";
    return (
      <div className="stats-panel">
        <div className="stats-row">
          <span className="stats-label">Total time</span>
          <span className="stats-value">{formatTime(s.total_ms)}</span>
        </div>
        <div className="stats-row">
          <span className="stats-label">Reference</span>
          <span className="stats-value">
            {s.ref_file_count.toLocaleString()} files ({formatTime(s.ref_collect_ms)}),{" "}
            {s.ref_cache_hits.toLocaleString()} cache hits ({formatTime(s.ref_hash_ms)})
          </span>
        </div>
        <div className="stats-row">
          <span className="stats-label">Eval</span>
          <span className="stats-value">
            {s.eval_file_count.toLocaleString()} files ({formatTime(s.eval_collect_ms)}),{" "}
            {s.eval_cache_hits.toLocaleString()} cache hits ({formatTime(s.eval_hash_ms)})
          </span>
        </div>
        {s.perceptual_compare_ms > 0 && (
          <div className="stats-row">
            <span className="stats-label">Perceptual</span>
            <span className="stats-value">Compare: {formatTime(s.perceptual_compare_ms)}</span>
          </div>
        )}
        <div className="stats-row">
          <span className="stats-label">Throughput</span>
          <span className="stats-value">{throughputMBs} MB/s</span>
        </div>
        <div className="stats-row">
          <span className="stats-label">Cache hit rate</span>
          <span className="stats-value">{cacheRate}%</span>
        </div>
        <div className="stats-row">
          <span className="stats-label">Total data</span>
          <span className="stats-value">{formatSize(s.total_bytes)}</span>
        </div>
      </div>
    );
  })() : null;

  return (
    <div className="results">
      <div className="results-layout">
        <div className="results-files">
          <div className="file-list">
            <div className="file-list-header">
              <div className="filter-pills">
                <button className={`filter-pill ${viewFilter === "all" ? "active" : ""}`} onClick={() => setViewFilter("all")}>
                  All
                </button>
                {result.exact_matches.length > 0 && (
                  <button className={`filter-pill ${viewFilter === "exact" ? "active" : ""}`} onClick={() => setViewFilter("exact")}>
                    Exact ({result.exact_matches.length})
                  </button>
                )}
                {filteredSimilar.length > 0 && (
                  <button className={`filter-pill ${viewFilter === "similar" ? "active" : ""}`} onClick={() => setViewFilter("similar")}>
                    Similar ({filteredSimilar.length})
                  </button>
                )}
                <button className={`filter-pill ${viewFilter === "unique" ? "active" : ""}`} onClick={() => setViewFilter("unique")}>
                  Unique ({allUniques.length})
                </button>
              </div>
              <div className="selection-buttons">
                <button className="btn-small" onClick={selectAll}>Select All</button>
                <button className="btn-small" onClick={selectExact}>Select Exact</button>
                {filteredSimilar.length > 0 && (
                  <button className="btn-small" onClick={selectSimilar}>Select Similar</button>
                )}
                <button className="btn-small" onClick={selectNone}>Deselect</button>
              </div>
              <span className="selection-count">
                {selectedCount} of {totalMatches} selected
              </span>
            </div>
            {/* Exact Matches */}
            {(viewFilter === "all" || viewFilter === "exact") && result.exact_matches.length > 0 && (
              <>
                {viewFilter === "all" && <div className="section-header">Exact Matches</div>}
                {result.exact_matches.map((file) => {
                  const isSelected = selectedFiles.has(file.path);
                  return (
                    <div key={file.path} className={`file-row ${isSelected ? "selected-dupe" : "deselected"} ${selectedPreview === file.path ? "preview-active" : ""}`}>
                      <input type="checkbox" className="file-row-checkbox" checked={isSelected} onChange={() => toggleFile(file.path)} />
                      <span className="status-dot dot-dupe" />
                      <span className="file-path" style={{ cursor: "pointer" }} onClick={() => handleFileClick(file.path)}>{file.relative_path}</span>
                      <span className="file-size">{formatSize(file.size)}</span>
                      <span className="tag tag-dupe">Exact</span>
                    </div>
                  );
                })}
              </>
            )}

            {/* Similar Matches */}
            {(viewFilter === "all" || viewFilter === "similar") && filteredSimilar.length > 0 && (
              <>
                {viewFilter === "all" && <div className="section-header">Similar Matches</div>}
                {filteredSimilar.map((file) => {
                  const isSelected = selectedFiles.has(file.path);
                  const similarity = file.hamming_distance != null ? Math.round((64 - file.hamming_distance) / 64 * 100) : null;
                  return (
                    <div key={file.path} className={`file-row ${isSelected ? "selected-dupe" : "deselected"} ${selectedPreview === file.path ? "preview-active" : ""}`}>
                      <input type="checkbox" className="file-row-checkbox" checked={isSelected} onChange={() => toggleFile(file.path)} />
                      <span className="status-dot dot-similar" />
                      <span className="file-path" style={{ cursor: "pointer" }} onClick={() => handleFileClick(file.path)}>{file.relative_path}</span>
                      <span className="file-size">{formatSize(file.size)}</span>
                      {similarity !== null && <span className="similarity-badge">{similarity}%</span>}
                      <span className="tag tag-similar">Similar</span>
                    </div>
                  );
                })}
              </>
            )}

            {/* Uniques */}
            {(viewFilter === "all" || viewFilter === "unique") && allUniques.length > 0 && (
              <>
                {viewFilter === "all" && <div className="section-header">Unique Files</div>}
                {allUniques.map((file) => (
                  <div key={file.path} className={`file-row ${selectedPreview === file.path ? "preview-active" : ""}`}>
                    <span style={{ width: 14, flexShrink: 0 }} />
                    <span className="status-dot dot-unique" />
                    <span className="file-path" style={{ cursor: "pointer" }} onClick={() => handleFileClick(file.path)}>{file.relative_path}</span>
                    <span className="file-size">{formatSize(file.size)}</span>
                    <span className="tag tag-unique">Unique</span>
                  </div>
                ))}
              </>
            )}
          </div>
        </div>

        <div className="results-sidebar">
          <div className="summary-bar">
            <div className="stat">
              <span className="stat-value">{result.total_eval}</span>
              <span className="stat-label">scanned</span>
            </div>
            <div className="stat stat-danger">
              <span className="stat-value">{result.exact_matches.length}</span>
              <span className="stat-label">exact</span>
            </div>
            {filteredSimilar.length > 0 && (
              <div className="stat stat-warning">
                <span className="stat-value">{filteredSimilar.length}</span>
                <span className="stat-label">similar</span>
              </div>
            )}
            <div className="stat stat-success">
              <span className="stat-value">{allUniques.length}</span>
              <span className="stat-label">unique</span>
            </div>
            {result.skipped > 0 && (
              <div className="stat stat-warning">
                <span className="stat-value">{result.skipped}</span>
                <span className="stat-label">skipped</span>
              </div>
            )}
          </div>

          {config.perceptual_matching && result.similar_matches.length > 0 && (
            <div className="threshold-adjust">
              <span className="threshold-label">Sensitivity</span>
              <div className="hash-pills">
                <button
                  className={`hash-pill ${resultsThreshold === 5 ? "active" : ""}`}
                  onClick={() => setResultsThreshold(5)}
                >
                  Strict
                </button>
                <button
                  className={`hash-pill ${resultsThreshold === 10 ? "active" : ""}`}
                  onClick={() => setResultsThreshold(10)}
                >
                  Moderate
                </button>
                <button
                  className={`hash-pill ${resultsThreshold === 15 ? "active" : ""}`}
                  onClick={() => setResultsThreshold(15)}
                >
                  Loose
                </button>
              </div>
            </div>
          )}

          <div className="export-buttons">
            <button className="btn-small" disabled={exporting} onClick={() => handleExport("csv")}>Export CSV</button>
            <button className="btn-small" disabled={exporting} onClick={() => handleExport("json")}>Export JSON</button>
          </div>
          {exportSuccess && <div style={{ color: "#16a34a", fontSize: "12px", marginTop: "4px" }}>Exported!</div>}
          {exportError && (
            <div className="error-list">
              <h4>Export Failed</h4>
              <div className="error-item">{exportError}</div>
            </div>
          )}

          {actionError && (
            <div className="error-list">
              <h4>Action Failed</h4>
              <div className="error-item">{actionError}</div>
            </div>
          )}

          {/* File Preview Panel */}
          {selectedPreview && (
            <div className="preview-panel">
              <button
                className="preview-close"
                onClick={() => { setSelectedPreview(null); setPreviewData(null); }}
              >
                &times;
              </button>
              {previewLoading ? (
                <div className="preview-info">Loading...</div>
              ) : previewData ? (
                <>
                  {previewData.thumbnail_data ? (
                    <img
                      className="preview-thumbnail"
                      src={previewData.thumbnail_data}
                      alt="Preview"
                    />
                  ) : (
                    <div className="preview-placeholder">
                      Preview not available for this file type
                    </div>
                  )}
                  <div className="preview-info">
                    <div>{previewData.path.split("/").pop()}</div>
                    <div className="preview-detail">{previewData.path}</div>
                    <div className="preview-detail">{formatSize(previewData.size)}</div>
                    <div className="preview-detail">{previewData.mime_type}</div>
                  </div>
                </>
              ) : (
                <div className="preview-info">{previewError || "Failed to load preview"}</div>
              )}
            </div>
          )}

          {isReview ? (
            <div className="action-panel">
              <h3>What to do with duplicates?</h3>
              <div className="radio-group">
                <label className={`radio-card-compact ${reviewAction === "trash" ? "active" : ""}`}>
                  <input
                    type="radio"
                    name="reviewAction"
                    checked={reviewAction === "trash"}
                    onChange={() => setReviewAction("trash")}
                  />
                  <span>Move to trash</span>
                </label>
                <label className={`radio-card-compact ${reviewAction === "move" ? "active" : ""}`}>
                  <input
                    type="radio"
                    name="reviewAction"
                    checked={reviewAction === "move"}
                    onChange={() => setReviewAction("move")}
                  />
                  <span>Move to folder</span>
                </label>
                <label className={`radio-card-compact ${reviewAction === "nothing" ? "active" : ""}`}>
                  <input
                    type="radio"
                    name="reviewAction"
                    checked={reviewAction === "nothing"}
                    onChange={() => setReviewAction("nothing")}
                  />
                  <span>Leave them</span>
                </label>
              </div>
              <div className={`inline-picker ${reviewAction !== "move" ? "invisible" : ""}`}>
                <code className="path-display">{reviewDest || "No folder selected"}</code>
                <button className="btn-small" onClick={pickFolder} disabled={reviewAction !== "move"}>Browse...</button>
              </div>
              <div className="action-buttons">
                <button
                  className={`btn-primary ${reviewAction === "trash" ? "btn-danger" : ""}`}
                  disabled={!canAct || executing}
                  onClick={handleConfirmAction}
                >
                  {executing
                    ? `${actionCurrent.toLocaleString()} / ${actionTotal.toLocaleString()}`
                    : reviewAction === "trash"
                      ? `Move ${selectedCount} of ${totalMatches} to Trash`
                      : reviewAction === "move"
                        ? `Move ${selectedCount} of ${totalMatches} Files`
                        : "Done"}
                </button>
                {executing && actionTotal > 0 && (
                  <>
                    <div className="action-progress">
                      <div className="action-progress-fill" style={{ width: `${Math.round((actionCurrent / actionTotal) * 100)}%` }} />
                    </div>
                    <button className="btn-link" onClick={() => invoke("cancel_scan")}>Cancel</button>
                  </>
                )}
              </div>
            </div>
          ) : (
            <div className="action-panel">
              <div className="action-buttons">
                <button
                  className={`btn-primary ${config.dupe_mode.type === "Trash" ? "btn-danger" : ""}`}
                  disabled={!canAct || executing}
                  onClick={handleConfirmAction}
                >
                  {executing
                    ? `${actionCurrent.toLocaleString()} / ${actionTotal.toLocaleString()}`
                    : config.dupe_mode.type === "Trash"
                      ? `Move ${selectedCount} of ${totalMatches} to Trash`
                      : `Move ${selectedCount} of ${totalMatches} Files`}
                </button>
                {executing && actionTotal > 0 && (
                  <>
                    <div className="action-progress">
                      <div className="action-progress-fill" style={{ width: `${Math.round((actionCurrent / actionTotal) * 100)}%` }} />
                    </div>
                    <button className="btn-link" onClick={() => invoke("cancel_scan")}>Cancel</button>
                  </>
                )}
              </div>
            </div>
          )}

          <div className="stats-section">
            <button className="stats-toggle" onClick={() => setStatsOpen(!statsOpen)}>
              Scan stats {statsOpen ? "\u25BE" : "\u25B8"}
            </button>
            {statsPanel}
          </div>

          <button className="btn-link" onClick={onNewScan}>&larr; New Scan</button>
        </div>
      </div>
    </div>
  );
}
