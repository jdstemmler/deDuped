import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { ScanConfig, ScanResult, ActionMode, ActionResult, EvalFile } from "../types";

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
  const [statsOpen, setStatsOpen] = useState(false);
  const exportTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Selective file actions: track which duplicate files are selected
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(
    () => new Set(result.duplicates.map((f) => f.path))
  );

  const allDupePaths = result.duplicates.map((f) => f.path);
  const selectedCount = selectedFiles.size;
  const totalDupes = allDupePaths.length;
  const allSelected = selectedCount === totalDupes && totalDupes > 0;

  const toggleFile = (path: string) => {
    setSelectedFiles((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleAll = () => {
    if (allSelected) {
      setSelectedFiles(new Set());
    } else {
      setSelectedFiles(new Set(allDupePaths));
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
      setActionResult({ processed: 0, errors: [], dirs_cleaned: 0 });
      setActionDone(true);
      return;
    }

    setExecuting(true);
    setActionError(null);
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
          files: result.uniques.map((f) => f.path),
          action: { type: "MoveToFolder", dest: config.unique_dest },
        });
        merged = {
          processed: merged.processed + uniqueRes.processed,
          errors: [...merged.errors, ...uniqueRes.errors],
          dirs_cleaned: merged.dirs_cleaned + uniqueRes.dirs_cleaned,
        };
      }

      setActionResult(merged);
      setActionDone(true);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setExecuting(false);
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
    const filesToAct = result.duplicates.filter((f) => selectedFiles.has(f.path));
    handleAction(filesToAct, mode);
  };

  if (actionDone && actionResult) {
    return (
      <div className="results">
        <div className="completion">
          <h2>Done</h2>
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
          <button className="btn-primary" onClick={onNewScan}>
            ← New Scan
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="results">
      <div className="results-layout">
        <div className="results-files">
          <div className="file-list">
            <div className="file-list-header">
              <label>
                <input
                  type="checkbox"
                  className="file-row-checkbox"
                  checked={allSelected}
                  onChange={toggleAll}
                />
                Select all duplicates
              </label>
              <span className="selection-count">
                {selectedCount} of {totalDupes} selected
              </span>
            </div>
            {[...result.duplicates, ...result.uniques].map((file) => {
              const isDupe = file.is_duplicate;
              const isSelected = isDupe && selectedFiles.has(file.path);
              const rowClass = [
                "file-row",
                isDupe && isSelected ? "selected-dupe" : "",
                isDupe && !isSelected ? "deselected" : "",
              ]
                .filter(Boolean)
                .join(" ");

              return (
                <div key={file.path} className={rowClass}>
                  {isDupe ? (
                    <input
                      type="checkbox"
                      className="file-row-checkbox"
                      checked={isSelected}
                      onChange={() => toggleFile(file.path)}
                    />
                  ) : (
                    <span style={{ width: 14, flexShrink: 0 }} />
                  )}
                  <span className={`status-dot ${isDupe ? "dot-dupe" : "dot-unique"}`} />
                  <span className="file-path">{file.relative_path}</span>
                  <span className="file-size">{formatSize(file.size)}</span>
                  <span className={`tag ${isDupe ? "tag-dupe" : "tag-unique"}`}>
                    {isDupe ? "Duplicate" : "Unique"}
                  </span>
                </div>
              );
            })}
          </div>
        </div>

        <div className="results-sidebar">
          <div className="summary-bar">
            <div className="stat">
              <span className="stat-value">{result.total_eval}</span>
              <span className="stat-label">scanned</span>
            </div>
            <div className="stat stat-danger">
              <span className="stat-value">{result.duplicates.length}</span>
              <span className="stat-label">dupes</span>
            </div>
            <div className="stat stat-success">
              <span className="stat-value">{result.uniques.length}</span>
              <span className="stat-label">unique</span>
            </div>
            {result.skipped > 0 && (
              <div className="stat stat-warning">
                <span className="stat-value">{result.skipped}</span>
                <span className="stat-label">skipped</span>
              </div>
            )}
          </div>

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
                    ? "Processing..."
                    : reviewAction === "trash"
                      ? `Move ${selectedCount} of ${totalDupes} to Trash`
                      : reviewAction === "move"
                        ? `Move ${selectedCount} of ${totalDupes} Files`
                        : "Done"}
                </button>
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
                    ? "Processing..."
                    : config.dupe_mode.type === "Trash"
                      ? `Move ${selectedCount} of ${totalDupes} to Trash`
                      : `Move ${selectedCount} of ${totalDupes} Files`}
                </button>
              </div>
            </div>
          )}

          <div className="stats-section">
            <button className="stats-toggle" onClick={() => setStatsOpen(!statsOpen)}>
              Scan stats {statsOpen ? "\u25BE" : "\u25B8"}
            </button>
            {statsOpen && (() => {
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
            })()}
          </div>

          <button className="btn-link" onClick={onNewScan}>&larr; New Scan</button>
        </div>
      </div>
    </div>
  );
}
