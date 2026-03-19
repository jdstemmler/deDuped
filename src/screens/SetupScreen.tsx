import { useState, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { ScanConfig, DupeMode } from "../types";

interface Props {
  onStart: (config: ScanConfig) => void;
  initialConfig: ScanConfig | null;
}

export default function SetupScreen({ onStart, initialConfig }: Props) {
  const [referenceDir, setReferenceDir] = useState(initialConfig?.reference_dir ?? "");
  const [evalDir, setEvalDir] = useState(initialConfig?.eval_dir ?? "");
  const [dupeMode, setDupeMode] = useState<"trash" | "move" | "review">(
    initialConfig?.dupe_mode.type === "MoveToFolder"
      ? "move"
      : initialConfig?.dupe_mode.type === "ReviewFirst"
        ? "review"
        : "trash"
  );
  const [dupeDest, setDupeDest] = useState(
    initialConfig?.dupe_mode.type === "MoveToFolder" ? initialConfig.dupe_mode.dest : ""
  );
  const [moveUniques, setMoveUniques] = useState(initialConfig?.move_uniques ?? false);
  const [uniqueDest, setUniqueDest] = useState(initialConfig?.unique_dest ?? "");

  const pickFolder = async (setter: (path: string) => void) => {
    const selected = await open({ directory: true, multiple: false });
    if (selected) setter(selected as string);
  };

  const canStart =
    referenceDir !== "" &&
    evalDir !== "" &&
    (dupeMode !== "move" || dupeDest !== "") &&
    (!moveUniques || uniqueDest !== "");

  const handleStart = () => {
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
    });
  };

  return (
    <div className="setup">
      {/* Folder pickers */}
      <div className="folder-pickers">
        <div
          className={`folder-picker ${referenceDir ? "selected" : ""}`}
          onClick={() => pickFolder(setReferenceDir)}
        >
          <span className="badge badge-protected">Protected</span>
          <div className="picker-label">Reference Folder</div>
          {referenceDir ? (
            <div className="picker-path">{referenceDir}</div>
          ) : (
            <div className="picker-hint">Click to select your photo library</div>
          )}
        </div>

        <div
          className={`folder-picker ${evalDir ? "selected" : ""}`}
          onClick={() => pickFolder(setEvalDir)}
        >
          <span className="badge badge-eval">Checking</span>
          <div className="picker-label">Eval Folder</div>
          {evalDir ? (
            <div className="picker-path">{evalDir}</div>
          ) : (
            <div className="picker-hint">Click to select incoming files to check</div>
          )}
        </div>
      </div>

      {/* Dupe handling */}
      <div className="config-section">
        <h3>Duplicate handling</h3>
        <div className="radio-group">
          <label className={`radio-card ${dupeMode === "trash" ? "active" : ""}`}>
            <input
              type="radio"
              name="dupeMode"
              checked={dupeMode === "trash"}
              onChange={() => setDupeMode("trash")}
            />
            <div>
              <strong>Move to trash</strong>
              <span className="radio-desc">Send duplicates to macOS Trash (recoverable)</span>
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
              {dupeMode === "move" && (
                <div className="inline-picker" onClick={(e) => e.stopPropagation()}>
                  <code className="path-display">{dupeDest || "No folder selected"}</code>
                  <button className="btn-small" onClick={() => pickFolder(setDupeDest)}>
                    Browse...
                  </button>
                </div>
              )}
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
              <span className="radio-desc">Scan only — decide what to do after seeing results</span>
            </div>
          </label>
        </div>
      </div>

      {/* Unique file handling */}
      <div className="config-section">
        <h3>Non-duplicate handling</h3>
        <div className="toggle-row">
          <span>Move unique files to a separate folder</span>
          <label className="toggle">
            <input
              type="checkbox"
              checked={moveUniques}
              onChange={(e) => setMoveUniques(e.target.checked)}
            />
            <span className="toggle-slider" />
          </label>
        </div>
        {moveUniques && (
          <div className="inline-picker">
            <code className="path-display">{uniqueDest || "No folder selected"}</code>
            <button className="btn-small" onClick={() => pickFolder(setUniqueDest)}>
              Browse...
            </button>
          </div>
        )}
      </div>

      {/* Start button */}
      <button className="btn-primary btn-start" disabled={!canStart} onClick={handleStart}>
        Start Scan
      </button>
    </div>
  );
}
