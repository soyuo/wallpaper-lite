import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import "./App.css";
import { hostAdapter, type ImportFailure, type ImportResult } from "./host-adapter";

const failureLabels: Record<ImportFailure, string> = {
  unsupported_format: "Unsupported format",
  invalid_media: "The file is corrupt or could not be decoded",
  codec_unavailable: "Unsupported video codec or required decoder unavailable",
  source_unavailable: "The source file is unavailable",
  directory_unreadable: "A folder could not be read",
  directory_depth_exceeded: "Folder nesting limit reached",
  candidate_limit_reached: "Import limit reached (4,096 files)",
  catalog_write_failed: "The library could not be saved",
};

function resultLabel(result: ImportResult): string {
  if (result.failure) return failureLabels[result.failure];
  if (result.status === "already_present") return "Already in library";
  if (result.status === "imported") return "Added to library";
  return "Import could not be completed";
}

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");
  const [importing, setImporting] = useState(false);
  const [importResults, setImportResults] = useState<ImportResult[]>([]);
  const [importStatus, setImportStatus] = useState("");

  async function checkHost() {
    setCheckingHost(true);
    setHostStatus("Checking host connection…");

    try {
      await hostAdapter.ping();
      setHostStatus("Wallpaper host connected.");
    } catch (error) {
      setHostStatus(
        error instanceof Error
          ? error.message
          : "Could not check the wallpaper host.",
      );
    } finally {
      setCheckingHost(false);
    }
  }

  async function importSelection(directory: boolean) {
    setImporting(true);
    setImportStatus("");
    setImportResults([]);
    try {
      const selection = await open(
        directory
          ? { directory: true, multiple: false, title: "Import wallpaper folder" }
          : {
              multiple: true,
              title: "Import wallpaper files",
            },
      );
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const results = await hostAdapter.importMedia(paths);
      setImportResults(results);
      const imported = results.filter((result) => result.status === "imported").length;
      setImportStatus(`${imported} item${imported === 1 ? "" : "s"} added; ${results.length} result${results.length === 1 ? "" : "s"}.`);
    } catch {
      setImportStatus("Import could not be completed. Check the library and try again.");
    } finally {
      setImporting(false);
    }
  }

  return (
    <main className="app-shell">
      <h1>wallpaper-lite</h1>
      <p>Desktop application shell</p>
      <button className="host-check" disabled={checkingHost} onClick={checkHost}>
        {checkingHost ? "Checking…" : "Check wallpaper host"}
      </button>
      <p aria-live="polite" className="host-status" role="status">
        {hostStatus}
      </p>
      <section className="import-panel" aria-labelledby="import-title">
        <h2 id="import-title">Import media</h2>
        <div className="import-actions">
          <button disabled={importing} onClick={() => void importSelection(false)}>
            Choose files
          </button>
          <button disabled={importing} onClick={() => void importSelection(true)}>
            Choose folder
          </button>
        </div>
        <p aria-live="polite" className="import-status" role="status">
          {importing ? "Validating selected media…" : importStatus}
        </p>
        {importResults.length > 0 && (
          <ul className="import-results" aria-label="Import results">
            {importResults.map((result, index) => (
              <li key={`${result.fileName}-${index}`}>
                <span>{result.fileName}</span>
                <span>{resultLabel(result)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  );
}

export default App;
