import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import "./App.css";
import {
  hostAdapter,
  type Availability,
  type EditorMonitor,
  type EditorPreview,
  type ImportFailure,
  type ImportResult,
  type LibraryEntry,
  type LibraryPreview,
} from "./host-adapter";

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

function formatBytes(value: number | null): string {
  if (value === null) return "Size unavailable";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unit = units[0];
  for (let index = 1; size >= 1024 && index < units.length; index += 1) {
    size /= 1024;
    unit = units[index];
  }
  return `${size.toFixed(size >= 100 ? 0 : 1)} ${unit}`;
}

function formatDuration(duration100ns: number | null): string | null {
  if (duration100ns === null) return null;
  const totalSeconds = Math.floor(duration100ns / 10_000_000);
  return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

function availabilityLabel(value: Availability): string {
  if (value === "file_missing") return "FILE_MISSING";
  if (value === "unavailable") return "Unavailable";
  return "Available";
}

interface LibraryCardProps {
  entry: LibraryEntry;
  preview?: LibraryPreview;
  previewError?: string;
  previewLoading: boolean;
  keepAcknowledged: boolean;
  actionBusy: boolean;
  renaming: boolean;
  renameDraft: string;
  onRequestPreview(id: number): void;
  onLocate(entry: LibraryEntry): void;
  onRemove(entry: LibraryEntry): void;
  onKeep(id: number): void;
  onStartRename(entry: LibraryEntry): void;
  onRenameDraftChange(value: string): void;
  onSubmitRename(entry: LibraryEntry): void;
  onCancelRename(): void;
  onToggleFavorite(entry: LibraryEntry): void;
  onOpenEditorPreview(entry: LibraryEntry): void;
}

function LibraryCard({
  entry,
  preview,
  previewError,
  previewLoading,
  keepAcknowledged,
  actionBusy,
  renaming,
  renameDraft,
  onRequestPreview,
  onLocate,
  onRemove,
  onKeep,
  onStartRename,
  onRenameDraftChange,
  onSubmitRename,
  onCancelRename,
  onToggleFavorite,
  onOpenEditorPreview,
}: LibraryCardProps) {
  const cardRef = useRef<HTMLLIElement>(null);

  useEffect(() => {
    if (entry.availability !== "available" || preview || previewLoading || previewError) return;
    const element = cardRef.current;
    if (!element) return;
    if (!("IntersectionObserver" in window)) {
      onRequestPreview(entry.id);
      return;
    }

    const observer = new IntersectionObserver(
      (observations) => {
        if (observations.some((observation) => observation.isIntersecting)) {
          onRequestPreview(entry.id);
          observer.disconnect();
        }
      },
      { rootMargin: "180px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [entry.availability, entry.id, onRequestPreview, preview, previewError, previewLoading]);

  const duration = formatDuration(preview?.duration100ns ?? null);
  const unavailable = entry.availability !== "available";

  return (
    <li className="library-card" ref={cardRef}>
      <div className="library-thumbnail" aria-label={`${entry.displayName} thumbnail`}>
        {preview ? (
          <img src={preview.thumbnailDataUrl} alt={`${entry.displayName} preview`} />
        ) : unavailable ? (
          <span>{availabilityLabel(entry.availability)}</span>
        ) : previewError ? (
          <span>Preview unavailable</span>
        ) : previewLoading ? (
          <span>Loading preview...</span>
        ) : (
          <span>Preview pending</span>
        )}
      </div>
      <div className="library-card-content">
        {renaming ? (
          <form className="library-rename-form" onSubmit={(event) => {
            event.preventDefault();
            onSubmitRename(entry);
          }}>
            <label>
              <span className="visually-hidden">New name for {entry.displayName}</span>
              <input
                autoFocus
                maxLength={256}
                value={renameDraft}
                onChange={(event) => onRenameDraftChange(event.currentTarget.value)}
              />
            </label>
            <button disabled={actionBusy} type="submit">Save name</button>
            <button disabled={actionBusy} onClick={onCancelRename} type="button">Cancel</button>
          </form>
        ) : (
          <h3>{entry.displayName}</h3>
        )}
        <p className="library-card-path" title={entry.sourcePath}>{entry.sourcePath}</p>
        <p className="library-card-meta">
          {entry.mediaKind === "video" ? "Video" : "Image"} · {formatBytes(preview?.sizeBytes ?? entry.sizeBytes)}
          {preview ? ` · ${preview.width} × ${preview.height}` : ""}
          {duration ? ` · ${duration}` : ""}
        </p>
        <div className="library-card-actions">
          {entry.availability === "available" && (
            <button disabled={actionBusy} onClick={() => onOpenEditorPreview(entry)}>Preview on display</button>
          )}
          <button
            aria-pressed={entry.isFavorite}
            disabled={actionBusy}
            onClick={() => onToggleFavorite(entry)}
          >
            {entry.isFavorite ? "★ Favorited" : "☆ Favorite"}
          </button>
          {!renaming && <button disabled={actionBusy} onClick={() => onStartRename(entry)}>Rename</button>}
          <button disabled={actionBusy} onClick={() => onRemove(entry)}>Remove entry</button>
          {unavailable && (
            <>
              <button disabled={actionBusy} onClick={() => onLocate(entry)}>Locate file</button>
              <button disabled={keepAcknowledged} onClick={() => onKeep(entry.id)}>
                {keepAcknowledged ? "Kept for later" : "Keep entry"}
              </button>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");
  const [importing, setImporting] = useState(false);
  const [importResults, setImportResults] = useState<ImportResult[]>([]);
  const [importStatus, setImportStatus] = useState("");
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [librarySearch, setLibrarySearch] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [libraryMessage, setLibraryMessage] = useState("");
  const [previews, setPreviews] = useState<Record<number, LibraryPreview>>({});
  const [previewErrors, setPreviewErrors] = useState<Record<number, string>>({});
  const [previewLoading, setPreviewLoading] = useState<Record<number, boolean>>({});
  const [busyEntryId, setBusyEntryId] = useState<number | null>(null);
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [keptIds, setKeptIds] = useState<Set<number>>(() => new Set());
  const [visibleCount, setVisibleCount] = useState(24);
  const [editorMonitors, setEditorMonitors] = useState<EditorMonitor[]>([]);
  const [editorMonitorId, setEditorMonitorId] = useState("");
  const [editorEntry, setEditorEntry] = useState<LibraryEntry | null>(null);
  const [editorPreview, setEditorPreview] = useState<EditorPreview | null>(null);
  const [editorBusy, setEditorBusy] = useState(false);
  const [editorError, setEditorError] = useState("");
  const previewRequests = useRef(new Set<number>());

  const refreshLibrary = useCallback(async () => {
    setLibraryBusy(true);
    setLibraryMessage("");
    try {
      setLibrary(await hostAdapter.listLibrary());
    } catch {
      setLibraryMessage("The library could not be loaded. Other application functions remain available.");
    } finally {
      setLibraryBusy(false);
    }
  }, []);

  useEffect(() => {
    void refreshLibrary();
  }, [refreshLibrary]);

  useEffect(() => {
    void hostAdapter.editorMonitors().then((monitors) => {
      setEditorMonitors(monitors);
      setEditorMonitorId(monitors[0]?.id ?? "");
    }).catch(() => setEditorError("Display information is unavailable."));
  }, []);

  const loadPreview = useCallback(async (id: number) => {
    if (previewRequests.current.has(id)) return;
    previewRequests.current.add(id);
    setPreviewLoading((current) => ({ ...current, [id]: true }));
    try {
      const preview = await hostAdapter.libraryPreview(id);
      setPreviews((current) => ({ ...current, [id]: preview }));
    } catch {
      setPreviewErrors((current) => ({ ...current, [id]: "Preview unavailable" }));
    } finally {
      setPreviewLoading((current) => ({ ...current, [id]: false }));
    }
  }, []);

  const filteredLibrary = useMemo(() => {
    const query = librarySearch.trim().toLocaleLowerCase();
    return library.filter((entry) =>
      (!favoritesOnly || entry.isFavorite) &&
      (!query || `${entry.displayName}\n${entry.sourcePath}`.toLocaleLowerCase().includes(query)),
    );
  }, [favoritesOnly, library, librarySearch]);
  const visibleLibrary = filteredLibrary.slice(0, visibleCount);

  async function checkHost() {
    setCheckingHost(true);
    setHostStatus("Checking host connection...");
    try {
      await hostAdapter.ping();
      setHostStatus("Wallpaper host connected.");
    } catch (error) {
      setHostStatus(error instanceof Error ? error.message : "Could not check the wallpaper host.");
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
          : { multiple: true, title: "Import wallpaper files" },
      );
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const results = await hostAdapter.importMedia(paths);
      setImportResults(results);
      const imported = results.filter((result) => result.status === "imported").length;
      setImportStatus(`${imported} item${imported === 1 ? "" : "s"} added; ${results.length} result${results.length === 1 ? "" : "s"}.`);
      await refreshLibrary();
    } catch {
      setImportStatus("Import could not be completed. Check the library and try again.");
    } finally {
      setImporting(false);
    }
  }

  async function locateEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      const path = await open({ multiple: false, title: "Locate wallpaper file" });
      if (typeof path !== "string") return;
      await hostAdapter.relocateLibraryEntry(entry.id, path);
      previewRequests.current.delete(entry.id);
      setPreviews((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setPreviewErrors((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setKeptIds((current) => {
        const next = new Set(current);
        next.delete(entry.id);
        return next;
      });
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not locate "${entry.displayName}". Choose a supported file of the same media type.`);
    } finally {
      setBusyEntryId(null);
    }
  }

  async function removeEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.removeLibraryEntry(entry.id);
      setPreviews((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setPreviewErrors((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      previewRequests.current.delete(entry.id);
      setKeptIds((current) => {
        const next = new Set(current);
        next.delete(entry.id);
        return next;
      });
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not remove "${entry.displayName}" from the library. Its source file remains untouched.`);
    } finally {
      setBusyEntryId(null);
    }
  }

  async function renameEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.renameLibraryEntry(entry.id, renameDraft);
      setRenamingId(null);
      setRenameDraft("");
      setLibraryMessage(`Renamed to "${renameDraft.trim()}".`);
      await refreshLibrary();
    } catch {
      setLibraryMessage("Could not rename this entry. Use a name of 1 to 256 characters.");
    } finally {
      setBusyEntryId(null);
    }
  }

  async function toggleFavorite(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.setLibraryFavorite(entry.id, !entry.isFavorite);
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not update the favorite state for "${entry.displayName}".`);
    } finally {
      setBusyEntryId(null);
    }
  }

  async function openEditorPreview(entry: LibraryEntry) {
    setEditorEntry(entry);
    setEditorPreview(null);
    setEditorError("");
    setEditorBusy(true);
    try {
      const monitors = editorMonitors.length > 0 ? editorMonitors : await hostAdapter.editorMonitors();
      if (monitors.length === 0) {
        setEditorError("No display is available for preview.");
        return;
      }
      const monitor = monitors.find((item) => item.id === editorMonitorId) ?? monitors[0];
      setEditorMonitors(monitors);
      setEditorMonitorId(monitor.id);
      setEditorPreview(await hostAdapter.editorPreview(entry.id, monitor.width, monitor.height));
    } catch {
      setEditorError("The wallpaper preview could not be rendered.");
    } finally {
      setEditorBusy(false);
    }
  }

  async function changeEditorMonitor(monitorId: string) {
    const monitor = editorMonitors.find((item) => item.id === monitorId);
    if (!monitor) return;
    setEditorMonitorId(monitor.id);
    if (!editorEntry) return;
    setEditorBusy(true);
    setEditorPreview(null);
    setEditorError("");
    try {
      setEditorPreview(await hostAdapter.editorPreview(editorEntry.id, monitor.width, monitor.height));
    } catch {
      setEditorError("The wallpaper preview could not be rendered for this display.");
    } finally {
      setEditorBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <h1>wallpaper-lite</h1>
      <p>Desktop application shell</p>
      <button className="host-check" disabled={checkingHost} onClick={checkHost}>
        {checkingHost ? "Checking..." : "Check wallpaper host"}
      </button>
      <p aria-live="polite" className="host-status" role="status">{hostStatus}</p>

      <section className="import-panel" aria-labelledby="import-title">
        <h2 id="import-title">Import media</h2>
        <div className="import-actions">
          <button disabled={importing} onClick={() => void importSelection(false)}>Choose files</button>
          <button disabled={importing} onClick={() => void importSelection(true)}>Choose folder</button>
        </div>
        <p aria-live="polite" className="import-status" role="status">
          {importing ? "Validating selected media..." : importStatus}
        </p>
        {importResults.length > 0 && (
          <ul className="import-results" aria-label="Import results">
            {importResults.map((result, index) => (
              <li key={`${result.fileName}-${index}`}>
                <span>{result.fileName}</span>
                <span>{result.failure ? failureLabels[result.failure] : resultLabel(result)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {editorEntry && (
        <section className="editor-preview-panel" aria-labelledby="editor-preview-title">
          <div className="editor-preview-heading">
            <div>
              <h2 id="editor-preview-title">Preview: {editorEntry.displayName}</h2>
              <p>Uses the saved physical-pixel settings and renderer placement/color rules.</p>
            </div>
            <button onClick={() => { setEditorEntry(null); setEditorPreview(null); setEditorError(""); }}>
              Close preview
            </button>
          </div>
          <label className="editor-monitor-select">
            <span>Preview display</span>
            <select value={editorMonitorId} onChange={(event) => void changeEditorMonitor(event.currentTarget.value)}>
              {editorMonitors.map((monitor) => (
                <option key={monitor.id} value={monitor.id}>{monitor.label}</option>
              ))}
            </select>
          </label>
          {editorBusy ? (
            <p role="status">Rendering preview...</p>
          ) : editorError ? (
            <p role="status">{editorError}</p>
          ) : editorPreview ? (
            <div
              className="editor-preview-canvas"
              aria-label={`Preview canvas ${editorPreview.monitorWidth} by ${editorPreview.monitorHeight}`}
              style={{
                aspectRatio: `${editorPreview.width} / ${editorPreview.height}`,
                width: `min(100%, ${Math.min(56, 32 * editorPreview.width / editorPreview.height)}rem)`,
              }}
            >
              <img src={editorPreview.dataUrl} alt={`Rendered ${editorEntry.displayName}`} />
            </div>
          ) : null}
          <p className="editor-preview-note">Resizing this application window only scales the preview display; it does not change saved wallpaper settings.</p>
        </section>
      )}

      <section className="library-panel" aria-labelledby="library-title">
        <div className="library-heading">
          <div>
            <h2 id="library-title">Wallpaper library</h2>
            <p>{library.length} item{library.length === 1 ? "" : "s"}</p>
          </div>
          <button disabled={libraryBusy} onClick={() => void refreshLibrary()}>
            {libraryBusy ? "Refreshing..." : "Refresh library"}
          </button>
        </div>
        <label className="library-search">
          <span>Search by name or path</span>
          <input
            type="search"
            value={librarySearch}
            onChange={(event) => {
              setLibrarySearch(event.currentTarget.value);
              setVisibleCount(24);
            }}
            placeholder="Search wallpapers"
          />
        </label>
        <label className="library-favorites-filter">
          <input
            type="checkbox"
            checked={favoritesOnly}
            onChange={(event) => {
              setFavoritesOnly(event.currentTarget.checked);
              setVisibleCount(24);
            }}
          />
          <span>Favorites only</span>
        </label>
        <p aria-live="polite" className="library-message" role="status">{libraryMessage}</p>
        {filteredLibrary.length === 0 ? (
          <p className="library-empty">{libraryBusy ? "Loading library..." : "No matching wallpaper entries."}</p>
        ) : (
          <ul className="library-list" aria-label="Wallpaper library entries">
            {visibleLibrary.map((entry) => (
              <LibraryCard
                key={entry.id}
                entry={entry}
                preview={previews[entry.id]}
                previewError={previewErrors[entry.id]}
                previewLoading={Boolean(previewLoading[entry.id])}
                keepAcknowledged={keptIds.has(entry.id)}
                actionBusy={busyEntryId === entry.id}
                renaming={renamingId === entry.id}
                renameDraft={renameDraft}
                onRequestPreview={(id) => void loadPreview(id)}
                onLocate={(item) => void locateEntry(item)}
                onRemove={(item) => void removeEntry(item)}
                onKeep={(id) => setKeptIds((current) => new Set(current).add(id))}
                onStartRename={(item) => {
                  setRenamingId(item.id);
                  setRenameDraft(item.displayName);
                }}
                onRenameDraftChange={setRenameDraft}
                onSubmitRename={(item) => void renameEntry(item)}
                onCancelRename={() => {
                  setRenamingId(null);
                  setRenameDraft("");
                }}
                onToggleFavorite={(item) => void toggleFavorite(item)}
                onOpenEditorPreview={(item) => void openEditorPreview(item)}
              />
            ))}
          </ul>
        )}
        {visibleCount < filteredLibrary.length && (
          <button className="library-more" onClick={() => setVisibleCount((count) => count + 24)}>
            Show more
          </button>
        )}
      </section>
    </main>
  );
}

export default App;
