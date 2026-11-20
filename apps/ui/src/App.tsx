import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import "./App.css";
import { EditorHistory, resolveEditorShortcut } from "./editor-history.mjs";
import {
  hostAdapter,
  type Availability,
  type EditorMonitor,
  type EditorPreview,
  type ImportFailure,
  type ImportResult,
  type LibraryEntry,
  type LibraryPreview,
  type WallpaperEditorSettings,
} from "./host-adapter";

const colorControls: Array<{
  key: keyof WallpaperEditorSettings["color"];
  label: string;
  min: number;
  max: number;
}> = [
  { key: "brightness", label: "Brightness", min: 0, max: 4 },
  { key: "contrast", label: "Contrast", min: 0, max: 4 },
  { key: "saturation", label: "Saturation", min: 0, max: 4 },
  { key: "gamma", label: "Gamma", min: 0.1, max: 4 },
  { key: "opacity", label: "Opacity", min: 0, max: 1 },
  { key: "temperature", label: "Color temperature", min: -1, max: 1 },
  { key: "red", label: "Red balance", min: 0, max: 4 },
  { key: "green", label: "Green balance", min: 0, max: 4 },
  { key: "blue", label: "Blue balance", min: 0, max: 4 },
];

function defaultEditorSettings(wallpaperId: number): WallpaperEditorSettings {
  return {
    schemaVersion: 1,
    wallpaperId,
    render: {
      placementMode: "cover",
      positionPx: { x: 0, y: 0 },
      sizePx: null,
      scale: { x: 1, y: 1 },
      rotationDegrees: 0,
      anchor: { x: 0.5, y: 0.5 },
      crop: { x: 0, y: 0, width: 1, height: 1 },
    },
    color: {
      brightness: 1,
      contrast: 1,
      saturation: 1,
      gamma: 1,
      opacity: 1,
      temperature: 0,
      red: 1,
      green: 1,
      blue: 1,
    },
  };
}

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

type EditorPointer = {
  kind: "move" | "resize";
  startX: number;
  startY: number;
  position: { x: number; y: number };
  size: { width: number; height: number };
  aspect: number;
};

type CropPointer = {
  kind: "create" | "move" | "resize";
  startX: number; startY: number;
  origin: { x: number; y: number; width: number; height: number };
};

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
  const [editorSettingsDraft, setEditorSettingsDraft] = useState<WallpaperEditorSettings | null>(null);
  const [editorDirty, setEditorDirty] = useState(false);
  const editorHistory = useRef(new EditorHistory<WallpaperEditorSettings>());
  const [editorSaveBusy, setEditorSaveBusy] = useState(false);
  const [createCropMode, setCreateCropMode] = useState(false);
  const [editorBusy, setEditorBusy] = useState(false);
  const [editorError, setEditorError] = useState("");
  const previewRequests = useRef(new Set<number>());
  const editorPointer = useRef<EditorPointer | null>(null);
  const cropPointer = useRef<CropPointer | null>(null);
  const selectedEditorMonitor = editorMonitors.find((item) => item.id === editorMonitorId) ?? null;

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

  useEffect(() => {
    function handleEditorShortcut(event: KeyboardEvent) {
      if (!editorEntry || !editorSettingsDraft) return;
      const target = event.target;
      const editableTarget = target instanceof HTMLElement && (
        target.isContentEditable
        || target.closest("input, textarea, select") !== null
        || target.closest("[contenteditable]") !== null
      );
      const command = resolveEditorShortcut({
        key: event.key,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        shiftKey: event.shiftKey,
        editableTarget,
      });
      if (!command) return;

      const handled = command === "undo"
        ? undoEditorChange()
        : command === "redo"
          ? redoEditorChange()
          : resetEditorSettings();
      if (handled) event.preventDefault();
    }

    window.addEventListener("keydown", handleEditorShortcut);
    return () => window.removeEventListener("keydown", handleEditorShortcut);
  }, [editorEntry, editorSettingsDraft]);

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
      if (!previews[entry.id]) {
        const sourcePreview = await hostAdapter.libraryPreview(entry.id);
        setPreviews((current) => ({ ...current, [entry.id]: sourcePreview }));
      }
      const result = await hostAdapter.editorPreview(entry.id, monitor.width, monitor.height);
      setEditorPreview(result);
      setEditorSettingsDraft(result.settings);
      editorHistory.current.reset(result.settings);
      setEditorDirty(false);
    } catch {
      setEditorError("The wallpaper preview could not be rendered.");
    } finally {
      setEditorBusy(false);
    }
  }

  async function changeEditorMonitor(monitorId: string) {
    const monitor = editorMonitors.find((item) => item.id === monitorId);
    if (!monitor) return;
    if (!editorEntry) return;
    setEditorBusy(true);
    setEditorPreview(null);
    setEditorError("");
    try {
      if (editorDirty && editorSettingsDraft) {
        await hostAdapter.saveEditorSettings(editorEntry.id, editorSettingsDraft);
        setEditorDirty(false);
      }
      setEditorMonitorId(monitor.id);
      const result = await hostAdapter.editorPreview(editorEntry.id, monitor.width, monitor.height);
      setEditorPreview(result);
      setEditorSettingsDraft(result.settings);
      setEditorDirty(false);
    } catch {
      setEditorError("The wallpaper preview could not be rendered for this display.");
    } finally {
      setEditorBusy(false);
    }
  }

  async function saveEditorSettingsDraft() {
    const monitor = editorMonitors.find((item) => item.id === editorMonitorId);
    if (!editorEntry || !editorSettingsDraft || !monitor) return;
    setEditorSaveBusy(true);
    setEditorError("");
    let saved = false;
    try {
      await hostAdapter.saveEditorSettings(editorEntry.id, editorSettingsDraft);
      saved = true;
      setEditorDirty(false);
      const result = await hostAdapter.editorPreview(editorEntry.id, monitor.width, monitor.height);
      setEditorPreview(result);
      setEditorSettingsDraft(result.settings);
      setEditorDirty(false);
    } catch {
      setEditorError(saved
        ? "Settings were saved, but the preview could not be rendered."
        : "These edits are invalid or could not be saved. The previous settings remain active.");
    } finally {
      setEditorSaveBusy(false);
    }
  }

  function beginEditorPointer(event: ReactPointerEvent<HTMLElement>, kind: "move" | "resize") {
    if (!editorSettingsDraft || !editorPreview) return;
    const monitor = editorMonitors.find((item) => item.id === editorMonitorId);
    if (!monitor) return;
    const size = editorSettingsDraft.render.sizePx ?? { width: monitor.width, height: monitor.height };
    editorPointer.current = {
      kind,
      startX: event.clientX,
      startY: event.clientY,
      position: editorSettingsDraft.render.positionPx,
      size,
      aspect: size.width / size.height,
    };
    beginEditorGesture(`canvas-${kind}`);
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function moveEditorPointer(event: ReactPointerEvent<HTMLDivElement>) {
    const pointer = editorPointer.current;
    const monitor = editorMonitors.find((item) => item.id === editorMonitorId);
    if (!pointer || !monitor || !editorSettingsDraft) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return;
    let dx = Math.round((event.clientX - pointer.startX) * monitor.width / bounds.width);
    let dy = Math.round((event.clientY - pointer.startY) * monitor.height / bounds.height);
    if (event.shiftKey && pointer.kind === "resize") {
      const ratioDelta = Math.abs(dx) > Math.abs(dy * pointer.aspect) ? dx : dy * pointer.aspect;
      dx = Math.round(ratioDelta);
      dy = Math.round(ratioDelta / pointer.aspect);
    }
    if (pointer.kind === "move") {
      updateEditorRender({
        placementMode: "custom",
        positionPx: {
          x: Math.max(-32_768, Math.min(32_768, pointer.position.x + dx)),
          y: Math.max(-32_768, Math.min(32_768, pointer.position.y + dy)),
        },
      });
      return;
    }
    const width = Math.max(1, Math.min(32_768, pointer.size.width + dx * (event.altKey ? 2 : 1)));
    const height = Math.max(1, Math.min(32_768, pointer.size.height + dy * (event.altKey ? 2 : 1)));
    updateEditorRender({
      placementMode: "custom",
      positionPx: event.altKey
        ? pointer.position
        : { x: pointer.position.x + Math.round((width - pointer.size.width) / 2), y: pointer.position.y + Math.round((height - pointer.size.height) / 2) },
      sizePx: { width, height },
    });
  }

  function setCropPreset(preset: string) {
    if (!editorSettingsDraft || !editorEntry) return;
    const current = editorSettingsDraft.render.crop;
    const source = previews[editorEntry.id];
    const sourceAspect = source && source.height > 0 ? source.width / source.height : 1;
    if (preset === "free") return;
    const [ratioWidth, ratioHeight] = preset.split(":").map(Number);
    const targetRatio = ratioWidth / ratioHeight;
    let width = current.width;
    let height = current.height;
    if (width * sourceAspect / height > targetRatio) width = height * targetRatio / sourceAspect;
    else height = width * sourceAspect / targetRatio;
    width = Math.min(1, width);
    height = Math.min(1, height);
    updateEditorRender({ crop: {
      x: Math.max(0, Math.min(1 - width, current.x + (current.width - width) / 2)),
      y: Math.max(0, Math.min(1 - height, current.y + (current.height - height) / 2)),
      width,
      height,
    } });
  }

  function beginCropPointer(event: ReactPointerEvent<HTMLElement>, kind: CropPointer["kind"]) {
    if (!editorSettingsDraft) return;
    const surface = event.currentTarget.closest<HTMLElement>(".editor-source-crop");
    if (!surface) return;
    const bounds = surface.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const crop = editorSettingsDraft.render.crop;
    cropPointer.current = {
      kind,
      startX: (event.clientX - bounds.left) / bounds.width,
      startY: (event.clientY - bounds.top) / bounds.height,
      origin: crop,
    };
    beginEditorGesture(`crop-${kind}`);
    surface.setPointerCapture(event.pointerId);
    event.preventDefault();
    event.stopPropagation();
  }

  function moveCropPointer(event: ReactPointerEvent<HTMLElement>) {
    const active = cropPointer.current;
    if (!active || !editorSettingsDraft) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const x = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
    const y = Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height));
    const dx = x - active.startX;
    const dy = y - active.startY;
    let crop = active.origin;
    if (active.kind === "create") {
      const cropX = Math.min(0.99, Math.min(active.startX, x));
      const cropY = Math.min(0.99, Math.min(active.startY, y));
      crop = {
        x: cropX,
        y: cropY,
        width: Math.min(1 - cropX, Math.max(0.01, Math.abs(dx))),
        height: Math.min(1 - cropY, Math.max(0.01, Math.abs(dy))),
      };
    } else if (active.kind === "move") {
      crop = { ...active.origin, x: Math.max(0, Math.min(1 - active.origin.width, active.origin.x + dx)), y: Math.max(0, Math.min(1 - active.origin.height, active.origin.y + dy)) };
    } else {
      const maxWidth = 1 - active.origin.x;
      const maxHeight = 1 - active.origin.y;
      crop = {
        ...active.origin,
        width: Math.min(maxWidth, Math.max(Math.min(0.01, maxWidth), active.origin.width + dx)),
        height: Math.min(maxHeight, Math.max(Math.min(0.01, maxHeight), active.origin.height + dy)),
      };
    }
    updateEditorRender({ crop });
  }

  function updateEditorRender(patch: Partial<WallpaperEditorSettings["render"]>) {
    if (!editorSettingsDraft) return;
    updateEditorSettings({
      ...editorSettingsDraft,
      render: { ...editorSettingsDraft.render, ...patch },
    });
  }

  function updateEditorColor(patch: Partial<WallpaperEditorSettings["color"]>) {
    if (!editorSettingsDraft) return;
    updateEditorSettings({
      ...editorSettingsDraft,
      color: { ...editorSettingsDraft.color, ...patch },
    });
  }

  function updateEditorSettings(settings: WallpaperEditorSettings) {
    setEditorSettingsDraft(editorHistory.current.update(settings));
    setEditorDirty(true);
  }

  function beginEditorGesture(key: string) {
    editorHistory.current.beginGesture(key);
  }

  function endEditorGesture(key?: string) {
    editorHistory.current.endGesture(key);
  }

  function undoEditorChange(): boolean {
    if (!editorHistory.current.canUndo) return false;
    const settings = editorHistory.current.undo();
    if (!settings) return false;
    setEditorSettingsDraft(settings);
    setEditorDirty(true);
    return true;
  }

  function redoEditorChange(): boolean {
    if (!editorHistory.current.canRedo) return false;
    const settings = editorHistory.current.redo();
    if (!settings) return false;
    setEditorSettingsDraft(settings);
    setEditorDirty(true);
    return true;
  }

  function resetEditorSettings(): boolean {
    if (!editorEntry || !editorSettingsDraft) return false;
    const settings = defaultEditorSettings(editorEntry.id);
    if (JSON.stringify(settings) === JSON.stringify(editorSettingsDraft)) return false;
    setEditorSettingsDraft(editorHistory.current.record(settings));
    setEditorDirty(true);
    return true;
  }

  function resetEditorColor() {
    updateEditorColor({
      brightness: 1,
      contrast: 1,
      saturation: 1,
      gamma: 1,
      opacity: 1,
      temperature: 0,
      red: 1,
      green: 1,
      blue: 1,
    });
  }

  function selectPlacementMode(mode: WallpaperEditorSettings["render"]["placementMode"]) {
    if (mode === "custom") {
      updateEditorRender({ placementMode: mode });
      return;
    }
    updateEditorRender({
      placementMode: mode,
      positionPx: { x: 0, y: 0 },
      sizePx: null,
      scale: { x: 1, y: 1 },
      rotationDegrees: 0,
      anchor: { x: 0.5, y: 0.5 },
    });
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
            <button onClick={() => {
              editorHistory.current.clear();
              setEditorEntry(null);
              setEditorPreview(null);
              setEditorSettingsDraft(null);
              setEditorDirty(false);
              setEditorError("");
            }}>
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
              onPointerDown={(event) => beginEditorPointer(event, "move")}
              onPointerMove={moveEditorPointer}
              onPointerUp={() => { editorPointer.current = null; endEditorGesture(); }}
              onPointerCancel={() => { editorPointer.current = null; endEditorGesture(); }}
              style={{
                aspectRatio: `${editorPreview.width} / ${editorPreview.height}`,
                width: `min(100%, ${Math.min(56, 32 * editorPreview.width / editorPreview.height)}rem)`,
              }}
            >
              <img draggable={false} src={editorPreview.dataUrl} alt={`Rendered ${editorEntry.displayName}`} />
              <button
                className="editor-resize-handle"
                aria-label="Resize wallpaper; hold Shift to lock aspect ratio and Alt to resize from center"
                onPointerDown={(event) => { event.stopPropagation(); beginEditorPointer(event, "resize"); }}
              />
            </div>
          ) : null}
          {editorSettingsDraft && editorPreview && !editorBusy && (
            <div className="editor-controls">
              <div className="editor-history-controls" aria-label="Edit history">
                <button type="button" disabled={!editorHistory.current.canUndo} onClick={undoEditorChange}>Undo</button>
                <button type="button" disabled={!editorHistory.current.canRedo} onClick={redoEditorChange}>Redo</button>
                <button type="button" onClick={resetEditorSettings}>Reset settings</button>
              </div>
              <p className="editor-history-shortcuts">Ctrl/Cmd+Z undo · Ctrl/Cmd+Y redo · Delete resets settings (shortcuts pause in text controls)</p>
              <label>Placement
                <select value={editorSettingsDraft.render.placementMode} onChange={(event) => selectPlacementMode(event.currentTarget.value as WallpaperEditorSettings["render"]["placementMode"])}>
                  <option value="cover">Cover</option><option value="fit">Fit</option><option value="stretch">Stretch</option><option value="center">Center</option><option value="custom">Custom</option>
                </select>
              </label>
              <label>Zoom
                <input
                  type="range" min="0.01" max="8" step="0.01" value={editorSettingsDraft.render.scale.x}
                  onPointerDown={() => beginEditorGesture("scale-slider")}
                  onPointerUp={() => endEditorGesture("scale-slider")}
                  onPointerCancel={() => endEditorGesture("scale-slider")}
                  onBlur={() => endEditorGesture("scale-slider")}
                  onKeyDown={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) beginEditorGesture("scale-slider"); }}
                  onKeyUp={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) endEditorGesture("scale-slider"); }}
                  onChange={(event) => { const value = Number(event.currentTarget.value); updateEditorRender({ placementMode: "custom", scale: { x: value, y: value } }); }}
                />
                <output>{editorSettingsDraft.render.scale.x.toFixed(2)}×</output>
              </label>
              <label>Rotation
                <input type="number" min="-3600" max="3600" step="0.5" value={editorSettingsDraft.render.rotationDegrees} onChange={(event) => updateEditorRender({ placementMode: "custom", rotationDegrees: Number(event.currentTarget.value) || 0 })} />
              </label>
              <label>Position X (physical px)
                <input type="number" min="-32768" max="32768" value={editorSettingsDraft.render.positionPx.x} onChange={(event) => updateEditorRender({ placementMode: "custom", positionPx: { ...editorSettingsDraft.render.positionPx, x: Number(event.currentTarget.value) || 0 } })} />
              </label>
              <label>Position Y (physical px)
                <input type="number" min="-32768" max="32768" value={editorSettingsDraft.render.positionPx.y} onChange={(event) => updateEditorRender({ placementMode: "custom", positionPx: { ...editorSettingsDraft.render.positionPx, y: Number(event.currentTarget.value) || 0 } })} />
              </label>
              {editorSettingsDraft.render.placementMode === "custom" && (
                <>
                  <label>Width (physical px)
                    <input type="number" min="1" max="32768" value={editorSettingsDraft.render.sizePx?.width ?? selectedEditorMonitor?.width ?? 1} onChange={(event) => updateEditorRender({ sizePx: { width: Math.max(1, Number(event.currentTarget.value) || 1), height: editorSettingsDraft.render.sizePx?.height ?? selectedEditorMonitor?.height ?? 1 } })} />
                  </label>
                  <label>Height (physical px)
                    <input type="number" min="1" max="32768" value={editorSettingsDraft.render.sizePx?.height ?? selectedEditorMonitor?.height ?? 1} onChange={(event) => updateEditorRender({ sizePx: { width: editorSettingsDraft.render.sizePx?.width ?? selectedEditorMonitor?.width ?? 1, height: Math.max(1, Number(event.currentTarget.value) || 1) } })} />
                  </label>
                </>
              )}
              <fieldset className="editor-color-controls">
                <legend>Color adjustments</legend>
                <div className="editor-color-grid">
                  {colorControls.map(({ key, label, min, max }) => (
                    <label key={key}>
                      <span>{label}</span>
                      <input
                        type="range"
                        min={min}
                        max={max}
                        step="0.01"
                        value={editorSettingsDraft.color[key]}
                        aria-label={label}
                        onPointerDown={() => beginEditorGesture(`color-${key}`)}
                        onPointerUp={() => endEditorGesture(`color-${key}`)}
                        onPointerCancel={() => endEditorGesture(`color-${key}`)}
                        onBlur={() => endEditorGesture(`color-${key}`)}
                        onKeyDown={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) beginEditorGesture(`color-${key}`); }}
                        onKeyUp={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) endEditorGesture(`color-${key}`); }}
                        onChange={(event) => updateEditorColor({ [key]: Number(event.currentTarget.value) })}
                      />
                      <output aria-label={`${label} value`}>{editorSettingsDraft.color[key].toFixed(2)}</output>
                    </label>
                  ))}
                </div>
                <p className="editor-color-note">Temperature ranges from cool (-1) to warm (+1). Opacity scales adjusted color toward black; the desktop surface is not transparent.</p>
                <button type="button" onClick={resetEditorColor}>Reset color</button>
              </fieldset>
              <fieldset className="editor-crop-controls">
                <legend>Normalized crop</legend>
                {previews[editorEntry.id] && (
                  <div
                    className="editor-source-crop"
                    aria-label="Crop source image; drag to select a crop, drag inside the box to move it"
                    style={{ aspectRatio: `${previews[editorEntry.id].width} / ${previews[editorEntry.id].height}` }}
                    onPointerDown={(event) => { if (createCropMode) beginCropPointer(event, "create"); }}
                    onPointerMove={moveCropPointer}
                    onPointerUp={() => { cropPointer.current = null; setCreateCropMode(false); endEditorGesture(); }}
                    onPointerCancel={() => { cropPointer.current = null; setCreateCropMode(false); endEditorGesture(); }}
                  >
                    <img draggable={false} src={previews[editorEntry.id].thumbnailDataUrl} alt="Original source crop" />
                    <div
                      className={`editor-crop-selection${createCropMode ? " is-disabled" : ""}`}
                      style={{ left: `${editorSettingsDraft.render.crop.x * 100}%`, top: `${editorSettingsDraft.render.crop.y * 100}%`, width: `${editorSettingsDraft.render.crop.width * 100}%`, height: `${editorSettingsDraft.render.crop.height * 100}%` }}
                      onPointerDown={(event) => { if (!createCropMode) beginCropPointer(event, "move"); }}
                    >
                      <button aria-label="Resize crop selection" onPointerDown={(event) => { event.stopPropagation(); beginCropPointer(event, "resize"); }} />
                    </div>
                  </div>
                )}
                <label>Aspect preset
                  <select defaultValue="free" onChange={(event) => setCropPreset(event.currentTarget.value)}>
                    <option value="free">Free</option><option value="16:9">16:9</option><option value="4:3">4:3</option><option value="1:1">1:1</option><option value="9:16">9:16</option>
                  </select>
                </label>
                <button type="button" onClick={() => setCreateCropMode(true)}>Select crop</button>
                {(["x", "y", "width", "height"] as const).map((key) => (
                  <label key={key}>{key}
                    <input type="number" min="0" max="1" step="0.01" value={editorSettingsDraft.render.crop[key]} onChange={(event) => updateEditorRender({ crop: { ...editorSettingsDraft.render.crop, [key]: Number(event.currentTarget.value) || 0 } })} />
                  </label>
                ))}
                <button onClick={() => updateEditorRender({ crop: { x: 0, y: 0, width: 1, height: 1 } })}>Reset crop</button>
              </fieldset>
              <button disabled={editorSaveBusy} onClick={() => void saveEditorSettingsDraft()}>
                {editorSaveBusy ? "Saving and rendering..." : "Save and preview"}
              </button>
              <p>Drag the preview to position. Drag the corner handle to resize; hold Shift to lock its aspect ratio or Alt to resize around its center.</p>
            </div>
          )}
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
