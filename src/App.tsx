import { useState } from "react";
import SetupScreen from "./screens/SetupScreen";
import ScanningScreen from "./screens/ScanningScreen";
import ResultsScreen from "./screens/ResultsScreen";
import type { ScanConfig, ScanResult } from "./types";

type Screen = "setup" | "scanning" | "results";

export default function App() {
  const [screen, setScreen] = useState<Screen>("setup");
  const [config, setConfig] = useState<ScanConfig | null>(null);
  const [result, setResult] = useState<ScanResult | null>(null);

  const handleStartScan = (cfg: ScanConfig) => {
    setConfig(cfg);
    setScreen("scanning");
  };

  const handleScanComplete = (res: ScanResult) => {
    setResult(res);
    setScreen("results");
  };

  const handleNewScan = () => {
    setResult(null);
    setScreen("setup");
  };

  return (
    <div className="app">
      <header className="app-header">
        <h1>deDuped</h1>
        <span className="subtitle">One-way photo deduplication</span>
      </header>
      <main className="app-main">
        {screen === "setup" && (
          <SetupScreen onStart={handleStartScan} initialConfig={config} />
        )}
        {screen === "scanning" && config && (
          <ScanningScreen config={config} onComplete={handleScanComplete} />
        )}
        {screen === "results" && config && result && (
          <ResultsScreen
            config={config}
            result={result}
            onNewScan={handleNewScan}
          />
        )}
      </main>
    </div>
  );
}
