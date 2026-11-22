import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import "./styles/tokens.css";
import "./styles/base.css";
import "./App.css";
import { EditorHistory, resolveEditorShortcut } from "./editor-history.mjs";
import { applyDisplayProfiles, buildDisplayProfiles, makeDisplayEdits } from "./display-manager.mjs";
import {
  buildPerformanceRows,
  formatPerformanceValue,
  isPerformanceSnapshotStale,
  PERFORMANCE_METRICS_POLL_MS,
} from "./performance-dashboard.mjs";
import {
  HostAdapterError,
  hostAdapter,
  type Availability,
  type EditorMonitor,
  type EditorPreview,
  type DisplayManagerSnapshot,
  type DisplayPresentationMode,
  type PerformancePolicySettings,
  type PerformanceMetricsSnapshot,
  type PolicyAction,
  type ImportFailure,
  type ImportResult,
  type LibraryEntry,
  type LibraryPreview,
  type PlaybackPanel,
  type PlaybackSettings,
  type WallpaperEditorPreset,
  type WallpaperEditorSettings,
} from "./host-adapter";
import { formatTimecode, isTimelineCommitKey, moveSegmentHandle, nominalFrameStep100ns, parseSegmentDraft, parseTimecode, runPlaybackAction } from "./playback-time.mjs";

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

function policyActionLabel(action: PolicyAction): string {
  switch (action) {
    case "pause_rendering": return "Pause rendering";
    case "pause_playback": return "Pause playback";
    default: return "Continue";
  }
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
  onStartPlayback(entry: LibraryEntry): void;
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
  onStartPlayback,
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
          {entry.mediaKind === "video" && entry.availability === "available" && (
            <button disabled={actionBusy} onClick={() => onStartPlayback(entry)}>Play on desktop</button>
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
  const [displaySnapshot, setDisplaySnapshot] = useState<DisplayManagerSnapshot | null>(null);
  const [displayMode, setDisplayMode] = useState<DisplayPresentationMode>("independent");
  const [displayEdits, setDisplayEdits] = useState<Record<string, { wallpaperId: number | ""; fpsOverride: string }>>({});
  const [displayStatus, setDisplayStatus] = useState("Display assignments have not been loaded.");
  const [displayLoading, setDisplayLoading] = useState(false);
  const [displaySaving, setDisplaySaving] = useState(false);
  const [performancePolicy, setPerformancePolicy] = useState<PerformancePolicySettings | null>(null);
  const [performancePolicySaving, setPerformancePolicySaving] = useState(false);
  const [performancePolicyMessage, setPerformancePolicyMessage] = useState("");
  const [performanceMetrics, setPerformanceMetrics] = useState<PerformanceMetricsSnapshot | null>(null);
  const [performanceMetricsError, setPerformanceMetricsError] = useState("");
  const [performanceMetricsNow, setPerformanceMetricsNow] = useState(() => Date.now());
  const [playbackPanel, setPlaybackPanel] = useState<PlaybackPanel | null>(null);
  const [playbackSettingsDraft, setPlaybackSettingsDraft] = useState<PlaybackSettings | null>(null);
  const [playbackMessage, setPlaybackMessage] = useState("");
  const [playbackBusy, setPlaybackBusy] = useState(false);
  const [seekTimeDraft, setSeekTimeDraft] = useState("00:00:00.000");
  const [seekTimeDirty, setSeekTimeDirty] = useState(false);
  const [segmentStartDraft, setSegmentStartDraft] = useState("00:00:00.000");
  const [segmentEndDraft, setSegmentEndDraft] = useState("00:00:00.000");
  const [timelineDraftMs, setTimelineDraftMs] = useState<number | null>(null);
  const [importing, setImporting] = useState(false);
  const [importResults, setImportResults] = useState<ImportResult[]>([]);
  const [importStatus, setImportStatus] = useState("");
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [recentIds, setRecentIds] = useState<number[]>([]);
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
  const [editorPresets, setEditorPresets] = useState<WallpaperEditorPreset[]>([]);
  const [presetName, setPresetName] = useState("");
  const [presetRename, setPresetRename] = useState<{ name: string; draft: string } | null>(null);
  const [presetMessage, setPresetMessage] = useState("");
  const [presetBusy, setPresetBusy] = useState(false);
  const editorHistory = useRef(new EditorHistory<WallpaperEditorSettings>());
  const [editorSaveBusy, setEditorSaveBusy] = useState(false);
  const [createCropMode, setCreateCropMode] = useState(false);
  const [editorBusy, setEditorBusy] = useState(false);
  const [editorError, setEditorError] = useState("");
  const previewRequests = useRef(new Set<number>());
  const editorPointer = useRef<EditorPointer | null>(null);
  const cropPointer = useRef<CropPointer | null>(null);
  const selectedEditorMonitor = editorMonitors.find((item) => item.id === editorMonitorId) ?? null;
  const performanceRows = buildPerformanceRows(performanceMetrics, displaySnapshot);
  const performanceMetricsStale = performanceMetrics !== null
    && isPerformanceSnapshotStale(performanceMetrics.sampledAtUnixMs, performanceMetricsNow);

  const refreshLibrary = useCallback(async () => {
    setLibraryBusy(true);
    setLibraryMessage("");
    try {
      const entries = await hostAdapter.listLibrary();
      setLibrary(entries);
      try {
        setRecentIds(await hostAdapter.recentWallpaperIds());
      } catch {
        setRecentIds([]);
      }
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
    let stopped = false;
    void hostAdapter.performancePolicySettings().then((settings) => {
      if (!stopped) setPerformancePolicy(settings);
    }).catch((error: unknown) => {
      if (!stopped) setPerformancePolicyMessage(error instanceof Error ? error.message : "Performance policy settings could not be loaded.");
    });
    return () => { stopped = true; };
  }, []);

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const snapshot = await hostAdapter.performanceMetrics();
        if (!stopped) {
          setPerformanceMetrics(snapshot);
          setPerformanceMetricsError("");
        }
      } catch (error: unknown) {
        if (!stopped) {
          setPerformanceMetricsError(error instanceof Error ? error.message : "Performance metrics could not be refreshed.");
        }
      } finally {
        if (!stopped) {
          setPerformanceMetricsNow(Date.now());
          timer = setTimeout(() => void poll(), PERFORMANCE_METRICS_POLL_MS);
        }
      }
    };
    void poll();
    return () => {
      stopped = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, []);

  function updatePerformancePolicy(updates: Partial<PerformancePolicySettings>) {
    setPerformancePolicy((current) => current ? { ...current, ...updates } : current);
    setPerformancePolicyMessage("");
  }

  async function savePerformancePolicy() {
    if (!performancePolicy) return;
    setPerformancePolicySaving(true);
    setPerformancePolicyMessage("");
    try {
      await hostAdapter.applyPerformancePolicySettings(performancePolicy);
      setPerformancePolicyMessage("Power-saving policy saved and applied to active playback targets.");
    } catch (error: unknown) {
      setPerformancePolicyMessage(error instanceof Error ? error.message : "Power-saving policy could not be applied.");
    } finally {
      setPerformancePolicySaving(false);
    }
  }

  useEffect(() => {
    let stopped = false;
    let polling = false;
    void hostAdapter.playbackPanel().then((panel) => {
      if (stopped) return;
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setPlaybackMessage("");
    }).catch((error: unknown) => {
      if (!stopped) setPlaybackMessage(error instanceof Error ? error.message : "Playback state is unavailable.");
    });
    const timer = window.setInterval(() => {
      if (polling || stopped) return;
      polling = true;
      void hostAdapter.playbackState().then((state) => {
        if (!stopped) setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
      }).catch((error: unknown) => {
        if (!stopped) setPlaybackMessage(error instanceof Error ? error.message : "Playback state is unavailable.");
      }).finally(() => { polling = false; });
    }, 500);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const settings = playbackPanel?.settings;
    if (!settings) return;
    setPlaybackSettingsDraft(settings);
    const segment = settings.segment;
    if (segment) {
      setSegmentStartDraft(formatTimecode(segment.startMs));
      setSegmentEndDraft(formatTimecode(segment.endMs));
    }
  }, [playbackPanel?.settings]);

  useEffect(() => {
    if (playbackPanel?.state && !seekTimeDirty) {
      setSeekTimeDraft(formatTimecode(Math.max(0, Math.floor(playbackPanel.state.mediaTime100ns / 10_000))));
    }
  }, [playbackPanel?.state.mediaTime100ns, seekTimeDirty]);

  useEffect(() => {
    void hostAdapter.editorMonitors().then((monitors) => {
      setEditorMonitors(monitors);
      setEditorMonitorId(monitors[0]?.id ?? "");
    }).catch(() => setEditorError("Display information is unavailable."));
  }, []);

  useEffect(() => {
    void refreshDisplayManager();
  }, []);

  useEffect(() => {
    function handleEditorShortcut(event: KeyboardEvent) {
      if (!editorEntry || !editorSettingsDraft) return;
      if (event.key === "Escape") {
        event.preventDefault();
        cancelEditorChanges();
        return;
      }
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
  const recentLibrary = useMemo(() => recentIds
    .map((id) => library.find((entry) => entry.id === id))
    .filter((entry): entry is LibraryEntry => entry !== undefined)
    .slice(0, 10), [library, recentIds]);

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

  async function refreshDisplayManager() {
    setDisplayLoading(true);
    try {
      const snapshot = await hostAdapter.displayManagerSnapshot();
      const state = makeDisplayEdits(snapshot);
      setDisplaySnapshot(snapshot);
      setDisplayMode(state.mode);
      setDisplayEdits(state.edits);
      const loadedMessage = `Loaded ${snapshot.displays.length} connected display${snapshot.displays.length === 1 ? "" : "s"}.`;
      setDisplayStatus(snapshot.targetFailures.length === 0
        ? loadedMessage
        : `${loadedMessage} ${snapshot.targetFailures.length} target${snapshot.targetFailures.length === 1 ? "" : "s"} could not apply their assignments.`);
    } catch (error) {
      setDisplayStatus(error instanceof Error ? error.message : "Display information could not be loaded.");
    } finally {
      setDisplayLoading(false);
    }
  }

  async function saveDisplayAssignments() {
    if (!displaySnapshot) return;
    let profiles;
    try {
      profiles = buildDisplayProfiles(displaySnapshot, displayMode, displayEdits);
    } catch (error) {
      setDisplayStatus(error instanceof Error ? error.message : "Display assignments are invalid.");
      return;
    }
    setDisplaySaving(true);
    const result = await applyDisplayProfiles(profiles, (candidate) => hostAdapter.applyDisplayProfiles(candidate));
    setDisplayStatus(result.message);
    if (result.ok) await refreshDisplayManager();
    setDisplaySaving(false);
  }

  async function startPlayback(entry: LibraryEntry) {
    setPlaybackBusy(true);
    setPlaybackMessage(`Loading ${entry.displayName}...`);
    try {
      const panel = await hostAdapter.loadVideoForPlayback(entry.id);
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setSeekTimeDraft(formatTimecode(Math.floor(panel.state.mediaTime100ns / 10_000)));
      setSeekTimeDirty(false);
      if (panel.settings?.segment) {
        setSegmentStartDraft(formatTimecode(panel.settings.segment.startMs));
        setSegmentEndDraft(formatTimecode(panel.settings.segment.endMs));
      } else {
        setSegmentStartDraft("00:00:00.000");
        setSegmentEndDraft(formatTimecode(Math.floor((panel.state.duration100ns ?? 0) / 10_000)));
      }
      setPlaybackMessage(`Playing ${entry.displayName}.`);
    } catch (error) {
      setPlaybackMessage(error instanceof Error ? error.message : "The video could not be started.");
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function sendPlaybackControl(control: "play" | "pause" | "stop") {
    setPlaybackBusy(true);
    setPlaybackMessage("");
    const result = await runPlaybackAction(
      () => hostAdapter.controlPlayback(control),
      `Could not ${control} playback.`,
    );
    if (result.ok) {
      const state = result.value;
      setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
    } else {
      setPlaybackMessage(result.message);
    }
    setPlaybackBusy(false);
  }

  async function commitSeek(positionMs: number) {
    if (!Number.isSafeInteger(positionMs) || positionMs < 0) return;
    const durationMs = playbackPanel?.state.duration100ns == null
      ? null
      : Math.floor(playbackPanel.state.duration100ns / 10_000);
    if (durationMs !== null && positionMs > durationMs) {
      setPlaybackMessage("Seek time cannot exceed the video duration.");
      return;
    }
    setPlaybackBusy(true);
    try {
      const state = await hostAdapter.seekPlayback(positionMs);
      setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
      setSeekTimeDraft(formatTimecode(positionMs));
      setSeekTimeDirty(false);
      setPlaybackMessage("");
    } catch (error) {
      setPlaybackMessage(error instanceof Error ? error.message : "The seek command failed.");
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function seekByFrame(direction: -1 | 1) {
    const state = playbackPanel?.state;
    if (!state || state.frameRateNumerator == null || state.frameRateDenominator == null) {
      setPlaybackMessage("Source frame rate is unavailable.");
      return;
    }
    const step = nominalFrameStep100ns(state.frameRateNumerator, state.frameRateDenominator);
    if (step === null) {
      setPlaybackMessage("Source frame rate is invalid.");
      return;
    }
    const duration = state.duration100ns ?? Number.MAX_SAFE_INTEGER;
    const target = Math.max(0, Math.min(duration, state.mediaTime100ns + direction * step));
    setPlaybackBusy(true);
    try {
      const next = await hostAdapter.seekPlayback100ns(target);
      setPlaybackPanel((panel) => panel ? { ...panel, state: next } : { state: next, settings: null });
      setPlaybackMessage("");
    } catch (error) {
      setPlaybackMessage(error instanceof Error ? error.message : "Frame seek failed.");
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function applyPlaybackSettingsDraft() {
    const active = playbackPanel?.state.wallpaperId;
    const draft = playbackSettingsDraft;
    if (active == null || !draft || draft.wallpaperId !== active) {
      setPlaybackMessage("Start a video before applying its playback settings.");
      return;
    }
    let segment = draft.segment;
    if (draft.loopMode === "segment") {
      const durationMs = playbackPanel?.state.duration100ns == null
        ? 0
        : Math.floor(playbackPanel.state.duration100ns / 10_000);
      const parsedSegment = parseSegmentDraft(segmentStartDraft, segmentEndDraft, durationMs);
      if (!parsedSegment) {
        setPlaybackMessage("Segment loop needs valid timecodes, at least 500 ms long, within the video duration.");
        return;
      }
      segment = parsedSegment;
    } else {
      segment = null;
    }
    const candidate = { ...draft, segment };
    setPlaybackBusy(true);
    setPlaybackMessage("");
    try {
      const panel = await hostAdapter.applyPlaybackSettings(active, candidate);
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setPlaybackMessage("Playback settings applied and saved.");
    } catch (error) {
      setPlaybackMessage(error instanceof Error ? error.message : "Playback settings could not be applied.");
    } finally {
      setPlaybackBusy(false);
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
      try {
        setEditorPresets(await hostAdapter.listEditorPresets(entry.id));
      } catch {
        setEditorPresets([]);
        setPresetMessage("Saved presets could not be loaded.");
      }
      setPresetName("");
      setPresetRename(null);
      setPresetMessage("");
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
    const keepDraft = editorDirty;
    setEditorBusy(true);
    setEditorPreview(null);
    setEditorError("");
    try {
      setEditorMonitorId(monitor.id);
      const result = await hostAdapter.editorPreview(editorEntry.id, monitor.width, monitor.height);
      setEditorPreview(result);
      if (!keepDraft) {
        setEditorSettingsDraft(result.settings);
        setEditorDirty(false);
      }
    } catch {
      setEditorError("The wallpaper preview could not be rendered for this display.");
    } finally {
      setEditorBusy(false);
    }
  }

  function cancelEditorChanges() {
    if (editorSaveBusy) return;
    editorHistory.current.clear();
    setEditorEntry(null);
    setEditorPreview(null);
    setEditorSettingsDraft(null);
    setEditorPresets([]);
    setPresetRename(null);
    setEditorDirty(false);
    setEditorError("");
  }

  async function saveEditorSettingsDraft() {
    const monitor = editorMonitors.find((item) => item.id === editorMonitorId);
    if (!editorEntry || !editorSettingsDraft || !monitor) return;
    setEditorSaveBusy(true);
    setEditorError("");
    let saved = false;
    try {
      await hostAdapter.applyEditorSettings(editorEntry.id, editorSettingsDraft);
      saved = true;
      try {
        setRecentIds(await hostAdapter.recentWallpaperIds());
      } catch {
        // Apply already succeeded; the next library refresh can reload recent metadata.
      }
      setEditorDirty(false);
      const result = await hostAdapter.editorPreview(editorEntry.id, monitor.width, monitor.height);
      setEditorPreview(result);
      setEditorSettingsDraft(result.settings);
      setEditorDirty(false);
    } catch (error) {
      if (error instanceof HostAdapterError && error.code === "editor_apply_rollback_failed") {
        setEditorError("Apply failed and the previous desktop settings could not be confirmed. The desktop may differ from saved settings; retry when the host and settings storage are available.");
      } else if (error instanceof HostAdapterError && error.code === "editor_wallpaper_not_active") {
        setEditorError("This library item is not the active desktop wallpaper. Its settings were not changed.");
      } else {
        setEditorError(saved
          ? "Settings were saved, but the preview could not be rendered."
          : "These edits are invalid or could not be saved. The previous settings remain active.");
      }
    } finally {
      setEditorSaveBusy(false);
    }
  }

  async function saveCurrentPreset() {
    if (!editorEntry || !editorSettingsDraft || presetBusy) return;
    setPresetBusy(true);
    setPresetMessage("");
    try {
      await hostAdapter.saveEditorPreset(editorEntry.id, presetName, editorSettingsDraft);
      setEditorPresets(await hostAdapter.listEditorPresets(editorEntry.id));
      setPresetName("");
      setPresetMessage("Preset saved.");
    } catch (error) {
      setPresetMessage(error instanceof Error ? error.message : "The preset could not be saved.");
    } finally {
      setPresetBusy(false);
    }
  }

  function loadPresetToDraft(preset: WallpaperEditorPreset) {
    if (!editorEntry || !editorSettingsDraft || preset.settings.wallpaperId !== editorEntry.id) return;
    editorHistory.current.endGesture();
    setEditorSettingsDraft(editorHistory.current.record(preset.settings));
    setEditorDirty(true);
    setPresetMessage("Preset loaded into the draft. Apply to update the desktop.");
  }

  async function renamePreset(preset: WallpaperEditorPreset) {
    if (!editorEntry || !presetRename || presetBusy) return;
    setPresetBusy(true);
    setPresetMessage("");
    try {
      await hostAdapter.renameEditorPreset(editorEntry.id, preset.name, presetRename.draft);
      setEditorPresets(await hostAdapter.listEditorPresets(editorEntry.id));
      setPresetRename(null);
      setPresetMessage("Preset renamed.");
    } catch (error) {
      setPresetMessage(error instanceof Error ? error.message : "The preset could not be renamed.");
    } finally {
      setPresetBusy(false);
    }
  }

  async function removePreset(preset: WallpaperEditorPreset) {
    if (!editorEntry || presetBusy) return;
    setPresetBusy(true);
    setPresetMessage("");
    try {
      await hostAdapter.removeEditorPreset(editorEntry.id, preset.name);
      setEditorPresets(await hostAdapter.listEditorPresets(editorEntry.id));
      setPresetMessage("Preset removed.");
    } catch (error) {
      setPresetMessage(error instanceof Error ? error.message : "The preset could not be removed.");
    } finally {
      setPresetBusy(false);
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

  const playbackState = playbackPanel?.state ?? null;
  const playbackDurationMs = playbackState?.duration100ns == null
    ? 0
    : Math.max(0, Math.floor(playbackState.duration100ns / 10_000));
  const playbackPositionMs = playbackState
    ? Math.max(0, Math.floor(playbackState.mediaTime100ns / 10_000))
    : 0;
  const seekTimecode = async () => {
    const parsed = parseTimecode(seekTimeDraft);
    if (parsed === null) {
      setPlaybackMessage("Enter time as HH:MM:SS.mmm.");
      return;
    }
    await commitSeek(parsed);
  };

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <h1>wallpaper-lite</h1>
          <p>Desktop application shell</p>
        </div>
        <div className="app-header-actions">
          <p aria-live="polite" className="host-status" role="status">{hostStatus}</p>
          <button className="ui-button--primary" disabled={checkingHost} onClick={checkHost}>
            {checkingHost ? "Checking..." : "Check wallpaper host"}
          </button>
        </div>
      </header>

      <section className="ui-panel performance-dashboard" aria-labelledby="performance-dashboard-title">
        <div className="performance-dashboard-heading">
          <div>
            <h2 id="performance-dashboard-title">Live performance</h2>
            <p>Host metrics poll every second. Null counters are unavailable, not zero.</p>
          </div>
          <span className={performanceMetricsStale ? "performance-stale" : "performance-live"} role="status">
            {performanceMetrics ? performanceMetricsStale ? "Stale data" : "Live" : "Waiting for metrics"}
          </span>
        </div>
        {performanceMetrics ? (
          <>
            {performanceMetricsError && <p className="performance-dashboard-warning" role="status">Refresh failed; retaining the last sample. {performanceMetricsError}</p>}
            {performanceMetricsStale && <p className="performance-dashboard-warning" role="status">The last host sample is older than 3.5 seconds.</p>}
            <div className="performance-memory-grid" aria-label="Process and GPU memory">
              <p>Process working set: <strong>{formatBytes(performanceMetrics.processWorkingSetBytes)}</strong></p>
              <p>Process private bytes: <strong>{formatBytes(performanceMetrics.processPrivateBytes)}</strong></p>
              <p>GPU local: <strong>{formatBytes(performanceMetrics.gpuLocalUsageBytes)}</strong> / {formatBytes(performanceMetrics.gpuLocalBudgetBytes)}</p>
              <p>GPU non-local: <strong>{formatBytes(performanceMetrics.gpuNonLocalUsageBytes)}</strong> / {formatBytes(performanceMetrics.gpuNonLocalBudgetBytes)}</p>
            </div>
            <div className="performance-policy-causes" aria-label="Active performance policy causes">
              <h3>Active policy causes</h3>
              {performanceMetrics.activePolicyCauses ? (
                <>
                  <p>{[
                    performanceMetrics.activePolicyCauses.fullscreen ? "Fullscreen app detected" : null,
                    performanceMetrics.activePolicyCauses.sessionLocked ? "Session locked" : null,
                    performanceMetrics.activePolicyCauses.onBattery ? "On battery" : null,
                  ].filter(Boolean).join(" · ") || "No automatic pause or battery cause is active."}</p>
                  <p>Effective automatic action: <strong>{policyActionLabel(performanceMetrics.activePolicyCauses.effectiveAction)}</strong>
                    {performanceMetrics.activePolicyCauses.onBattery && performanceMetrics.activePolicyCauses.batteryFpsLimit !== null
                      ? ` · Temporary battery cap: ${performanceMetrics.activePolicyCauses.batteryFpsLimit} FPS`
                      : ""}
                  </p>
                </>
              ) : <p>Active policy status is unavailable.</p>}
            </div>
            <div className="performance-target-list" aria-label="Per-display playback metrics">
              {performanceRows.map((row) => (
                <article className="performance-target-card" key={row.id}>
                  <div className="performance-target-heading">
                    <h3>{row.label}</h3>
                    <span>{row.status === "available" ? "Connected" : row.status === "failed" ? "Target error" : row.status === "disconnected" ? "Disconnected" : row.status === "metrics_unavailable" ? "Metrics unavailable" : "Unmapped target"}</span>
                  </div>
                  {row.status === "failed" && <p className="performance-dashboard-warning" role="alert">This target reported an error{row.failureStage ? ` during ${row.failureStage}` : ""}. Other display metrics remain available.</p>}
                  {row.status === "disconnected" && <p>Saved assignment is waiting for this display to reconnect. Any shown counters are the last host sample for this target.</p>}
                  {row.status === "metrics_unavailable" && <p>No host metrics are currently registered for this connected display.</p>}
                  {row.metrics ? (
                    <>
                      <p>Media: <strong>{row.metrics.mediaKind}</strong></p>
                      <dl>
                        <div><dt>Source FPS</dt><dd>{formatPerformanceValue(row.metrics.sourceFps, "FPS")}</dd></div>
                        <div><dt>Rendered FPS</dt><dd>{formatPerformanceValue(row.metrics.renderedFps, "FPS")}</dd></div>
                        <div><dt>Frame interval</dt><dd>{formatPerformanceValue(row.metrics.frameIntervalMs, "ms")}</dd></div>
                        <div><dt>Dropped frames</dt><dd>{row.metrics.droppedFramesTotal.toLocaleString()}</dd></div>
                        <div><dt>Decode latency</dt><dd>{formatPerformanceValue(row.metrics.decodeLatencyMs, "ms")}</dd></div>
                        <div><dt>Present latency</dt><dd>{formatPerformanceValue(row.metrics.presentLatencyMs, "ms")}</dd></div>
                      </dl>
                    </>
                  ) : row.status !== "disconnected" && row.status !== "metrics_unavailable" && <p>No metric sample is available for this target.</p>}
                </article>
              ))}
              {performanceRows.length === 0 && <p>No display targets are currently registered.</p>}
            </div>
            <p className="performance-sample-time">Host sample: {new Date(performanceMetrics.sampledAtUnixMs).toLocaleTimeString()}</p>
          </>
        ) : (
          <p role={performanceMetricsError ? "alert" : "status"} className={performanceMetricsError ? "performance-dashboard-warning" : ""}>
            {performanceMetricsError ? `Performance metrics are unavailable: ${performanceMetricsError}` : "Loading host performance metrics…"}
          </p>
        )}
      </section>

      <section className="ui-panel performance-policy" aria-labelledby="performance-policy-title">
        <div className="performance-policy-heading">
          <div>
            <h2 id="performance-policy-title">Power-saving policies</h2>
            <p>Fullscreen and session actions affect playback; battery options are temporary and keep each display’s saved FPS.</p>
          </div>
          <button disabled={!performancePolicy || performancePolicySaving} onClick={() => void savePerformancePolicy()}>
            {performancePolicySaving ? "Applying..." : "Apply policies"}
          </button>
        </div>
        {performancePolicy ? (
          <div className="performance-policy-controls">
            {([
              ["fullscreenAction", "Fullscreen app"],
              ["sessionLockAction", "Session lock"],
            ] as const).map(([key, label]) => (
              <label key={key}>
                {label}
                <select
                  aria-label={`${label} action`}
                  disabled={performancePolicySaving}
                  value={performancePolicy[key]}
                  onChange={(event) => updatePerformancePolicy({ [key]: event.currentTarget.value as PolicyAction })}
                >
                  <option value="continue">Continue</option>
                  <option value="pause_rendering">Pause rendering</option>
                  <option value="pause_playback">Pause playback</option>
                </select>
              </label>
            ))}
            <label>
              On battery
              <select
                aria-label="Battery action"
                disabled={performancePolicySaving}
                value={performancePolicy.batteryAction}
                onChange={(event) => {
                  const batteryAction = event.currentTarget.value as PolicyAction;
                  updatePerformancePolicy({ batteryAction, batteryFpsLimit: batteryAction === "continue" ? performancePolicy.batteryFpsLimit : null });
                }}
              >
                <option value="continue">Continue</option>
                <option value="pause_rendering">Pause rendering</option>
                <option value="pause_playback">Pause playback</option>
              </select>
            </label>
            <label>
              Battery FPS cap (optional, 1–120)
              <input
                aria-label="Battery FPS cap"
                type="number"
                min={1}
                max={120}
                step={1}
                placeholder="No cap"
                disabled={performancePolicySaving || performancePolicy.batteryAction !== "continue"}
                value={performancePolicy.batteryFpsLimit ?? ""}
                onChange={(event) => {
                  const raw = event.currentTarget.value;
                  const parsed = raw === "" ? null : Number(raw);
                  updatePerformancePolicy({ batteryFpsLimit: parsed === null || Number.isInteger(parsed) ? parsed : performancePolicy.batteryFpsLimit });
                }}
              />
            </label>
          </div>
        ) : <p role="status">Loading power-saving policies…</p>}
        {performancePolicyMessage && <p aria-live="polite" className="performance-policy-status" role="status">{performancePolicyMessage}</p>}
      </section>

      <section className="ui-panel display-manager" aria-labelledby="display-manager-title">
        <div className="display-manager-heading">
          <div>
            <h2 id="display-manager-title">Displays</h2>
            <p>Assign wallpapers and per-display frame-rate limits.</p>
          </div>
          <button disabled={displayLoading || displaySaving} onClick={() => void refreshDisplayManager()}>
            {displayLoading ? "Refreshing..." : "Refresh displays"}
          </button>
        </div>
        {displaySnapshot ? (
          <>
            <label className="display-mode-control">
              Presentation mode
              <select
                aria-label="Presentation mode"
                disabled={displaySaving}
                value={displayMode}
                onChange={(event) => setDisplayMode(event.currentTarget.value as DisplayPresentationMode)}
              >
                <option value="independent">Independent</option>
                <option value="duplicate">Duplicate</option>
                <option value="span">Span</option>
              </select>
            </label>
            <div className="display-manager-list">
              {displaySnapshot.displays.map((display) => {
                const edit = displayEdits[display.id] ?? { wallpaperId: "", fpsOverride: "" };
                const savedProfile = displaySnapshot.profiles.find((profile) => profile.displayId === display.id);
                const assignedWallpaper = savedProfile
                  ? displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === savedProfile.wallpaperId)
                  : null;
                const showWallpaper = displayMode === "independent" || display.isPrimary;
                return (
                  <article className="display-manager-card" key={display.id}>
                    <h3>{display.name}{display.isPrimary ? " · Primary" : ""}</h3>
                    <code className="display-stable-id" title={display.id}>Stable ID: {display.id}</code>
                    <p>{display.deviceName} · {display.width} × {display.height} · {display.refreshRate ?? "Refresh rate unavailable"} · {display.dpiScalePercent}% scale ({display.effectiveDpi.toFixed(1)} DPI)</p>
                    <p className={savedProfile && assignedWallpaper?.availability !== "available" ? "display-assignment-warning" : "display-assignment-state"}>
                      {savedProfile
                        ? assignedWallpaper?.availability === "available"
                          ? `Connected · ${assignedWallpaper.name} assigned`
                          : "Connected · assigned wallpaper is missing from the library or unavailable"
                        : "Connected · no saved assignment"}
                    </p>
                    {showWallpaper && (
                      <label>
                        {displayMode === "independent" ? "Wallpaper" : "Shared wallpaper"}
                        <select
                          aria-label={`${display.name} wallpaper`}
                          disabled={displaySaving}
                          value={edit.wallpaperId}
                          onChange={(event) => {
                            const value = event.currentTarget.value;
                            setDisplayEdits((current) => ({
                              ...current,
                              [display.id]: {
                                ...(current[display.id] ?? edit),
                                wallpaperId: value === "" ? "" : Number(value),
                              },
                            }));
                          }}
                        >
                          <option value="">Choose wallpaper</option>
                          {displaySnapshot.wallpapers.filter((wallpaper) => wallpaper.availability === "available").map((wallpaper) => (
                            <option key={wallpaper.id} value={wallpaper.id}>{wallpaper.name} · {wallpaper.mediaKind}</option>
                          ))}
                        </select>
                      </label>
                    )}
                    <label>
                      FPS override
                      <input
                        aria-label={`${display.name} FPS override`}
                        type="number"
                        min={1}
                        max={120}
                        step={1}
                        placeholder="Default"
                        disabled={displaySaving}
                        value={edit.fpsOverride}
                        onChange={(event) => {
                          const value = event.currentTarget.value;
                          setDisplayEdits((current) => ({
                            ...current,
                            [display.id]: {
                              ...(current[display.id] ?? edit),
                              fpsOverride: value,
                            },
                          }));
                        }}
                      />
                    </label>
                  </article>
                );
              })}
            </div>
            {displaySnapshot.profiles.filter((profile) => !displaySnapshot.displays.some((display) => display.id === profile.displayId)).length > 0 && (
              <div className="display-disconnected">
                <h3>Disconnected display assignments</h3>
                <p>These stable identities are retained and will be restored when their displays reconnect.</p>
                <ul>
                  {displaySnapshot.profiles.filter((profile) => !displaySnapshot.displays.some((display) => display.id === profile.displayId)).map((profile) => (
                    <li key={profile.displayId}>
                      {profile.displayId} · {displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === profile.wallpaperId)?.name ?? `wallpaper ${profile.wallpaperId}`}
                      {displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === profile.wallpaperId)?.availability !== "available" ? " (media missing/unavailable)" : ""}
                      {` · ${profile.presentationMode}`}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <div className="display-manager-actions">
              <button disabled={displaySaving || displayLoading} onClick={() => void saveDisplayAssignments()}>
                {displaySaving ? "Applying..." : "Save and apply"}
              </button>
            </div>
          </>
        ) : (
          <p>No display state is available yet.</p>
        )}
        {displaySnapshot?.targetFailures.map((failure) => (
          <p className="display-manager-error" role="alert" key={`${failure.deviceName}:${failure.stage}`}>
            {failure.deviceName}: {failure.stage}
          </p>
        ))}
        <p className="display-manager-status" aria-live="polite" role="status">{displayStatus}</p>
      </section>

      <section className="ui-panel playback-panel" aria-labelledby="playback-title">
        <h2 id="playback-title">Video playback</h2>
        {playbackState?.wallpaperId == null ? (
          <p>No video is active. Start a video from its library card.</p>
        ) : (
          <>
            <p className="playback-state">{playbackState.loadingVideo ? "Loading" : playbackState.state} · {playbackState.speedPercent}% · {playbackState.fpsLimit} FPS</p>
            {playbackState.lastError && <p className="playback-message" role="status">Host playback error: {playbackState.lastError}</p>}
            <div className="playback-timeline">
              <label htmlFor="playback-seek">Timeline</label>
              <input
                id="playback-seek"
                type="range"
                min={0}
                max={Math.max(1, playbackDurationMs)}
                value={Math.min(playbackDurationMs || playbackPositionMs, timelineDraftMs ?? playbackPositionMs)}
                disabled={playbackBusy || playbackDurationMs <= 0 || playbackState.loadingVideo}
                onChange={(event) => setTimelineDraftMs(Number(event.currentTarget.value))}
                onPointerUp={(event) => {
                  const value = Number(event.currentTarget.value);
                  setTimelineDraftMs(null);
                  void commitSeek(value);
                }}
                onKeyUp={(event) => {
                  if (isTimelineCommitKey(event.key)) {
                    const value = Number(event.currentTarget.value);
                    setTimelineDraftMs(null);
                    void commitSeek(value);
                  }
                }}
              />
              <span>{formatTimecode(playbackPositionMs)} / {formatTimecode(playbackDurationMs)}</span>
            </div>
            <div className="playback-controls" aria-label="Video transport controls">
              <button disabled={playbackBusy || playbackState.loadingVideo || !playbackState.frameRateNumerator} onClick={() => void seekByFrame(-1)}>Previous frame</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("play")}>Play</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("pause")}>Pause</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("stop")}>Stop</button>
              <button disabled={playbackBusy || playbackState.loadingVideo || !playbackState.frameRateNumerator} onClick={() => void seekByFrame(1)}>Next frame</button>
            </div>
            <label className="playback-time-entry">
              <span>Seek time (HH:MM:SS.mmm)</span>
              <input value={seekTimeDraft} onChange={(event) => { setSeekTimeDirty(true); setSeekTimeDraft(event.currentTarget.value); }} onKeyDown={(event) => { if (event.key === "Enter") void seekTimecode(); }} onBlur={() => { if (parseTimecode(seekTimeDraft) === null) setPlaybackMessage("Enter time as HH:MM:SS.mmm."); }} />
              <button disabled={playbackBusy} onClick={() => void seekTimecode()}>Seek</button>
            </label>
            {playbackSettingsDraft?.wallpaperId === playbackState.wallpaperId && (
              <fieldset className="playback-settings">
                <legend>Loop and playback settings</legend>
                <label>
                  <span>Loop mode</span>
                  <select
                    value={playbackSettingsDraft.loopMode}
                    onChange={(event) => {
                      const loopMode = event.currentTarget.value as PlaybackSettings["loopMode"];
                      const segment = loopMode === "segment"
                        ? playbackSettingsDraft.segment ?? { startMs: 0, endMs: playbackDurationMs }
                        : null;
                      setPlaybackSettingsDraft({ ...playbackSettingsDraft, loopMode, segment });
                      if (loopMode === "segment" && segment) {
                        setSegmentStartDraft(formatTimecode(segment.startMs));
                        setSegmentEndDraft(formatTimecode(segment.endMs));
                      }
                    }}
                  >
                    <option value="disabled">Disabled</option>
                    <option value="full">Full video</option>
                    <option value="segment">Segment</option>
                  </select>
                </label>
                {playbackSettingsDraft.loopMode === "segment" && (
                  <div className="playback-segment-fields">
                    <label>
                      <span>Segment start handle</span>
                      <input
                        type="range"
                        min={0}
                        max={Math.max(0, playbackDurationMs - 500)}
                        value={Math.min(Math.max(0, playbackDurationMs - 500), parseTimecode(segmentStartDraft) ?? 0)}
                        onChange={(event) => {
                          const segment = moveSegmentHandle(
                            "start",
                            Number(event.currentTarget.value),
                            parseTimecode(segmentStartDraft) ?? 0,
                            parseTimecode(segmentEndDraft) ?? playbackDurationMs,
                            playbackDurationMs,
                          );
                          if (!segment) return;
                          setSegmentStartDraft(formatTimecode(segment.startMs));
                          setSegmentEndDraft(formatTimecode(segment.endMs));
                        }}
                      />
                    </label>
                    <label>
                      <span>Segment end handle</span>
                      <input
                        type="range"
                        min={Math.min(playbackDurationMs, (parseTimecode(segmentStartDraft) ?? 0) + 500)}
                        max={playbackDurationMs}
                        value={Math.max(Math.min(playbackDurationMs, (parseTimecode(segmentStartDraft) ?? 0) + 500), Math.min(playbackDurationMs, parseTimecode(segmentEndDraft) ?? playbackDurationMs))}
                        onChange={(event) => {
                          const segment = moveSegmentHandle(
                            "end",
                            Number(event.currentTarget.value),
                            parseTimecode(segmentStartDraft) ?? 0,
                            parseTimecode(segmentEndDraft) ?? playbackDurationMs,
                            playbackDurationMs,
                          );
                          if (!segment) return;
                          setSegmentStartDraft(formatTimecode(segment.startMs));
                          setSegmentEndDraft(formatTimecode(segment.endMs));
                        }}
                      />
                    </label>
                    <label><span>Segment start</span><input value={segmentStartDraft} onChange={(event) => setSegmentStartDraft(event.currentTarget.value)} placeholder="00:00:00.000" /></label>
                    <label><span>Segment end</span><input value={segmentEndDraft} onChange={(event) => setSegmentEndDraft(event.currentTarget.value)} placeholder="00:00:00.000" /></label>
                    <p>Segment must be at least 500 ms and fit within the video duration.</p>
                  </div>
                )}
                <label><span>Speed</span><select value={playbackSettingsDraft.speedPercent} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, speedPercent: Number(event.currentTarget.value) })}>{[25, 50, 75, 100, 125, 150, 200].map((value) => <option key={value} value={value}>{value}%</option>)}</select></label>
                <label><span>Frame rate limit</span><select value={playbackSettingsDraft.fpsLimit} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, fpsLimit: Number(event.currentTarget.value) })}>{[15, 24, 30, 60, 120].map((value) => <option key={value} value={value}>{value} FPS</option>)}</select></label>
                <label><span>Volume ({playbackSettingsDraft.volumePercent}%)</span><input aria-label="Playback volume" type="range" min="0" max="100" step="1" value={playbackSettingsDraft.volumePercent} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, volumePercent: Number(event.currentTarget.value) })} /></label>
                <label className="playback-mute"><input type="checkbox" checked={playbackSettingsDraft.muted} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, muted: event.currentTarget.checked })} /><span>Mute audio</span></label>
                {!playbackState.audioAvailable && <p>This video has no audio track.</p>}
                {playbackState.audioAvailable && playbackState.audioEffectiveMuted && playbackSettingsDraft.muted && <p>Audio is muted.</p>}
                {playbackState.audioAvailable && playbackState.audioEffectiveMuted && !playbackSettingsDraft.muted && playbackState.speedPercent !== 100 && <p>Audio is muted while video runs at non-1x speed.</p>}
                <button disabled={playbackBusy} onClick={() => void applyPlaybackSettingsDraft()}>Apply playback settings</button>
              </fieldset>
            )}
          </>
        )}
        <p className="playback-message" aria-live="polite" role="status">{playbackMessage}</p>
      </section>

      <section className="ui-panel import-panel" aria-labelledby="import-title">
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
        <section className="ui-panel editor-preview-panel" aria-labelledby="editor-preview-title">
          <div className="editor-preview-heading">
            <div>
              <h2 id="editor-preview-title">Preview: {editorEntry.displayName}</h2>
              <p>Uses the shared physical-pixel placement and color rules. Apply updates the desktop; Cancel discards the draft.</p>
            </div>
            <button disabled={editorSaveBusy} onClick={cancelEditorChanges}>
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
              <fieldset className="editor-presets">
                <legend>Presets</legend>
                <div className="preset-save-row">
                  <label>Save current draft
                    <input maxLength={64} value={presetName} onChange={(event) => setPresetName(event.currentTarget.value)} placeholder="Preset name" />
                  </label>
                  <button type="button" disabled={presetBusy || !presetName.trim()} onClick={() => void saveCurrentPreset()}>Save preset</button>
                </div>
                {editorPresets.length === 0 ? <p>No saved presets for this wallpaper.</p> : (
                  <ul className="preset-list">
                    {editorPresets.map((preset) => (
                      <li key={preset.name}>
                        {presetRename?.name === preset.name ? (
                          <>
                            <input aria-label={`Rename ${preset.name}`} maxLength={64} value={presetRename.draft} onChange={(event) => setPresetRename({ ...presetRename, draft: event.currentTarget.value })} />
                            <button type="button" disabled={presetBusy || !presetRename.draft.trim()} onClick={() => void renamePreset(preset)}>Save name</button>
                            <button type="button" disabled={presetBusy} onClick={() => setPresetRename(null)}>Cancel rename</button>
                          </>
                        ) : (
                          <>
                            <span>{preset.name}</span>
                            <button type="button" disabled={presetBusy} onClick={() => loadPresetToDraft(preset)}>Load to draft</button>
                            <button type="button" disabled={presetBusy} onClick={() => setPresetRename({ name: preset.name, draft: preset.name })}>Rename</button>
                            <button type="button" disabled={presetBusy} onClick={() => void removePreset(preset)}>Remove</button>
                          </>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
                <p role="status" aria-live="polite">{presetMessage}</p>
                <p>Loading a preset changes this draft only. Use Apply to update the desktop.</p>
              </fieldset>
              <button disabled={editorSaveBusy} onClick={() => void saveEditorSettingsDraft()}>
                {editorSaveBusy ? "Applying and rendering..." : "Apply"}
              </button>
              <button disabled={editorSaveBusy} onClick={cancelEditorChanges}>Cancel</button>
              <p>Drag the preview to position. Drag the corner handle to resize; hold Shift to lock its aspect ratio or Alt to resize around its center.</p>
            </div>
          )}
          <p className="editor-preview-note">Resizing this application window only scales the preview display; it does not change saved wallpaper settings.</p>
        </section>
      )}

      <section className="ui-panel library-panel" aria-labelledby="library-title">
        <div className="library-heading">
          <div>
            <h2 id="library-title">Wallpaper library</h2>
            <p>{library.length} item{library.length === 1 ? "" : "s"}</p>
          </div>
          <button disabled={libraryBusy} onClick={() => void refreshLibrary()}>
            {libraryBusy ? "Refreshing..." : "Refresh library"}
          </button>
        </div>
        {recentLibrary.length > 0 && (
          <section className="recent-wallpapers" aria-labelledby="recent-wallpapers-title">
            <h3 id="recent-wallpapers-title">Recently applied</h3>
            <ul>
              {recentLibrary.map((entry) => (
                <li key={entry.id}>
                  <span>{entry.displayName}</span>
                  <button disabled={entry.availability !== "available"} onClick={() => void openEditorPreview(entry)}>Edit</button>
                </li>
              ))}
            </ul>
          </section>
        )}
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
                onStartPlayback={(item) => void startPlayback(item)}
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
