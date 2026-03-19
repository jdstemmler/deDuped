import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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
  const [executing, setExecuting] = useState(false);

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setReviewDest(selected as string);
  };

  const canAct = isReview
    ? reviewAction !== "move" || reviewDest !== ""
    : true;

  const handleAction = async (dupeFiles: EvalFile[], mode: ActionMode) => {
    setExecuting(true);
    try {
      const res = await invoke<ActionResult>("execute_action", {
        evalDir: config.eval_dir,
        files: dupeFiles.map((f) => f.path),
        action: mode,
      });

      // Handle unique files if configured
      if (config.move_uniques && config.unique_dest) {
        await invoke<ActionResult>("execute_action", {
          evalDir: config.eval_dir,
          files: result.uniques.map((f) => f.path),
          action: { type: "MoveToFolder", dest: config.unique_dest },
        });
      }

      setActionResult(res);
      setActionDone(true);
    } catch (err) {
      console.error("Action failed:", err);
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
      mode = { type: "Nothing" };
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
        {/* Left: File list */}
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

        {/* Right: Summary + Actions */}
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
          </div>

          {/* Action panel */}
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
