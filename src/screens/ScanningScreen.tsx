import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ScanConfig, ScanResult, ProgressEvent } from "../types";

interface Props {
  config: ScanConfig;
  onComplete: (result: ScanResult) => void;
  onBack: () => void;
}

function formatEta(ms: number): string {
  if (ms < 1000) return "< 1s";
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  return `${mins}m ${remSecs}s`;
}

export default function ScanningScreen({ config, onComplete, onBack }: Props) {
  const [phase, setPhase] = useState("Starting scan...");
  const [current, setCurrent] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // React StrictMode double-invokes effects in dev. This ref ensures
  // scan_folders is only called once across mount/remount cycles.
  const scanInvokedRef = useRef(false);
  const cancelledRef = useRef(false);
  const phaseStartRef = useRef<number>(Date.now());
  const lastPhaseRef = useRef<string>("");

  useEffect(() => {
    cancelledRef.current = false;
    let unlisten: (() => void) | undefined;

    const run = async () => {
      unlisten = await listen<ProgressEvent>("scan-progress", (event) => {
        if (cancelledRef.current) return;
        if (event.payload.phase !== lastPhaseRef.current) {
          lastPhaseRef.current = event.payload.phase;
          phaseStartRef.current = Date.now();
        }
        setPhase(event.payload.phase);
        setCurrent(event.payload.current);
        setTotal(event.payload.total);
      });

      if (!scanInvokedRef.current) {
        scanInvokedRef.current = true;
        try {
          const result = await invoke<ScanResult>("scan_folders", { config });
          if (!cancelledRef.current) onComplete(result);
        } catch (err) {
          if (!cancelledRef.current) {
            setError(err instanceof Error ? err.message : String(err));
          }
        }
      }
    };

    run();

    return () => {
      cancelledRef.current = true;
      if (unlisten) unlisten();
    };
  }, []);

  const handleCancel = async () => {
    try {
      await invoke("cancel_scan");
    } catch {
      // Best-effort cancellation
    }
    onBack();
  };

  if (error) {
    return (
      <div className="scanning">
        <div className="scan-error">
          <h2>Scan Failed</h2>
          <p className="error-message">{error}</p>
          <button className="btn-primary" onClick={onBack}>
            &larr; Back to Setup
          </button>
        </div>
      </div>
    );
  }

  const pct = total > 0 ? Math.round((current / total) * 100) : 0;

  const eta = (() => {
    if (current <= 0 || total <= 0 || current >= total) return null;
    const elapsed = Date.now() - phaseStartRef.current;
    if (elapsed < 2000) return null; // wait for stable rate
    const rate = current / elapsed; // items per ms
    const remaining = (total - current) / rate;
    return formatEta(remaining);
  })();

  return (
    <div className="scanning">
      <div className="scan-phase">{phase}</div>
      <div className="progress-bar">
        <div className="progress-fill" style={{ width: `${pct}%` }} />
      </div>
      <div className="scan-stats">
        <span className="scan-pct">{pct}%</span>
        <span className="scan-count">
          [{current.toLocaleString()} / {total.toLocaleString()}]
        </span>
        {eta && <span className="scan-eta">{eta} remaining</span>}
      </div>
      <button className="btn-link" onClick={handleCancel}>Cancel</button>
    </div>
  );
}
