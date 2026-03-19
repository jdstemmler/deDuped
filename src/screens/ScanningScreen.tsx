import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ScanConfig, ScanResult, ProgressEvent } from "../types";

interface Props {
  config: ScanConfig;
  onComplete: (result: ScanResult) => void;
}

export default function ScanningScreen({ config, onComplete }: Props) {
  const [phase, setPhase] = useState("Starting scan...");
  const [current, setCurrent] = useState(0);
  const [total, setTotal] = useState(0);
  const scanInvokedRef = useRef(false);
  const cancelledRef = useRef(false);

  useEffect(() => {
    cancelledRef.current = false;
    let unlisten: (() => void) | undefined;

    const run = async () => {
      unlisten = await listen<ProgressEvent>("scan-progress", (event) => {
        if (cancelledRef.current) return;
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
          console.error("Scan failed:", err);
        }
      }
    };

    run();

    return () => {
      cancelledRef.current = true;
      if (unlisten) unlisten();
    };
  }, []);

  const pct = total > 0 ? Math.round((current / total) * 100) : 0;

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
      </div>
    </div>
  );
}
