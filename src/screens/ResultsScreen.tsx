import { useState } from "react";
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

export default function ResultsScreen({ config, result, onNewScan }: Props) {
  const isReview = config.dupe_mode.type === "ReviewFirst";
  const [reviewAction, setReviewAction] = useState<"trash" | "move" | "nothing">("trash");
  const [reviewDest, setReviewDest] = useState("");
  const [actionDone, setActionDone] = useState(false);
  const [actionResult, setActionResult] = useState<ActionResult | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [executing, setExecuting] = useState(false);

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setReviewDest(selected);
  };

  const handleExport = async (format: "csv" | "json") => {
    const ext = format === "csv" ? "csv" : "json";
    const filePath = await save({
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      defaultPath: `scan-report.${ext}`,
    });
    if (!filePath) return;
    try {
      await invoke("export_report", {
        results: result,
        format,
        destPath: filePath,
      });
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  const canAct = isReview
    ? reviewAction !== "move" || reviewDest !== ""
    : true;

  const handleAction = async (dupeFiles: EvalFile[], mode: ActionMode) => {
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
      return; // unreachable: non-review mode is always Trash or MoveToFolder
    }
    handleAction(result.duplicates, mode);
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
            {[...result.duplicates, ...result.uniques].map((file) => (
              <div key={file.path} className="file-row">
                <span className={`status-dot ${file.is_duplicate ? "dot-dupe" : "dot-unique"}`} />
                <span className="file-path">{file.relative_path}</span>
                <span className="file-size">{formatSize(file.size)}</span>
                <span className={`tag ${file.is_duplicate ? "tag-dupe" : "tag-unique"}`}>
                  {file.is_duplicate ? "Duplicate" : "Unique"}
                </span>
              </div>
            ))}
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
            <button className="btn-small" onClick={() => handleExport("csv")}>Export CSV</button>
            <button className="btn-small" onClick={() => handleExport("json")}>Export JSON</button>
          </div>

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
                  {executing ? "Processing..." : reviewAction === "trash" ? "Move to Trash" : reviewAction === "move" ? "Move Files" : "Done"}
                </button>
              </div>
            </div>
          ) : (
            <div className="action-panel">
              <div className="action-buttons">
                <button
                  className={`btn-primary ${config.dupe_mode.type === "Trash" ? "btn-danger" : ""}`}
                  disabled={executing}
                  onClick={handleConfirmAction}
                >
                  {executing
                    ? "Processing..."
                    : config.dupe_mode.type === "Trash"
                      ? `Move ${result.duplicates.length} to Trash`
                      : `Move ${result.duplicates.length} Files`}
                </button>
              </div>
            </div>
          )}

          <button className="btn-link" onClick={onNewScan}>← New Scan</button>
        </div>
      </div>
    </div>
  );
}
