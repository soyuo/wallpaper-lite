import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import "./styles/tokens.css";
import "./styles/base.css";
import "./App.css";
import { EditorHistory, resolveEditorShortcut } from "./editor-history.mjs";
import {
  HostAdapterError,
  hostAdapter,
  type EditorMonitor,
  type EditorPreview,
  type LibraryEntry,
  type WallpaperEditorPreset,
  type WallpaperEditorSettings,
} from "./host-adapter";
import { PerformancePanels } from "./features/performance/PerformancePanels";
import { usePerformance } from "./features/performance/usePerformance";
import { DisplayPanel } from "./features/displays/DisplayPanel";
import { useDisplays } from "./features/displays/useDisplays";
import { PlaybackPanelView } from "./features/playback/PlaybackPanelView";
import { usePlayback } from "./features/playback/usePlayback";
import { LibraryPage } from "./features/library/LibraryPage";
import { useLibrary } from "./features/library/useLibrary";

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

type AppPage = "library" | "editor" | "displays" | "playback" | "performance";

const APP_PAGES: { id: AppPage; label: string }[] = [
  { id: "library", label: "Library" },
  { id: "editor", label: "Editor" },
  { id: "displays", label: "Monitors" },
  { id: "playback", label: "Playback" },
  { id: "performance", label: "Performance" },
];

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

type EditorPreviewLike = { monitorWidth: number; monitorHeight: number; settings: WallpaperEditorSettings };

// The preview bitmap is rendered once per load/apply; reflect pending move/resize drags on it live.
function editorDragTransform(preview: EditorPreviewLike, draft: WallpaperEditorSettings | null): React.CSSProperties | undefined {
  if (!draft) return undefined;
  const { monitorWidth: mw, monitorHeight: mh } = preview;
  if (mw <= 0 || mh <= 0) return undefined;
  const base = preview.settings.render;
  const next = draft.render;
  const baseSize = base.sizePx ?? { width: mw, height: mh };
  const nextSize = next.sizePx ?? { width: mw, height: mh };
  const dx = ((next.positionPx.x - base.positionPx.x) / mw) * 100;
  const dy = ((next.positionPx.y - base.positionPx.y) / mh) * 100;
  const sx = nextSize.width / baseSize.width;
  const sy = nextSize.height / baseSize.height;
  if (dx === 0 && dy === 0 && sx === 1 && sy === 1) return undefined;
  return {
    transformOrigin: `${(base.positionPx.x / mw) * 100}% ${(base.positionPx.y / mh) * 100}%`,
    transform: `translate(${dx}%, ${dy}%) scale(${sx}, ${sy})`,
  };
}

// Approximates crates/renderer placement.rs on the full source so the part outside the display stays visible.
function editorGhostStyle(source: { width: number; height: number }, draft: WallpaperEditorSettings, mw: number, mh: number): React.CSSProperties | undefined {
  const r = draft.render;
  const c = r.crop;
  const ew = source.width * c.width;
  const eh = source.height * c.height;
  if (mw <= 0 || mh <= 0 || ew <= 0 || eh <= 0 || c.width <= 0 || c.height <= 0) return undefined;
  let scale = Math.max(mw / ew, mh / eh);
  if (r.placementMode === "fit") scale = Math.min(mw / ew, mh / eh);
  else if (r.placementMode === "center") scale = 1;
  let boxW = ew * scale;
  let boxH = eh * scale;
  if (r.placementMode === "stretch") { boxW = mw; boxH = mh; }
  // Destination rectangle before sizePx: Cover and Stretch fill the display, Fit/Center use the box.
  const fills = r.placementMode === "cover" || r.placementMode === "custom" || r.placementMode === "stretch";
  const baseW = fills ? mw : boxW;
  const baseH = fills ? mh : boxH;
  const rectW = r.sizePx?.width ?? baseW;
  const rectH = r.sizePx?.height ?? baseH;
  const kx = rectW / baseW;
  const ky = rectH / baseH;
  const boxWs = boxW * kx;
  const boxHs = boxH * ky;
  const rectL = (mw - rectW) / 2;
  const rectT = (mh - rectH) / 2;
  const left = (mw - boxWs) / 2 - c.x * (boxWs / c.width);
  const top = (mh - boxHs) / 2 - c.y * (boxHs / c.height);
  const width = boxWs / c.width;
  const height = boxHs / c.height;
  const ax = rectL + rectW * r.anchor.x;
  const ay = rectT + rectH * r.anchor.y;
  return {
    position: "absolute",
    left: `${(left / mw) * 100}%`,
    top: `${(top / mh) * 100}%`,
    width: `${(width / mw) * 100}%`,
    height: `${(height / mh) * 100}%`,
    clipPath: `inset(${c.y * 100}% ${(1 - c.x - c.width) * 100}% ${(1 - c.y - c.height) * 100}% ${c.x * 100}%)`,
    transformOrigin: `${((ax - left) / width) * 100}% ${((ay - top) / height) * 100}%`,
    transform: `translate(${(r.positionPx.x / width) * 100}%, ${(r.positionPx.y / height) * 100}%) rotate(${r.rotationDegrees}deg) scale(${r.scale.x}, ${r.scale.y})`,
  };
}

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");
  const displaysState = useDisplays();
  const { displaySnapshot } = displaysState;
  const performanceState = usePerformance(displaySnapshot);
  const playbackState = usePlayback();
  const libraryState = useLibrary();
  const { previews, setPreviews, setRecentIds } = libraryState;
  const [editorMonitors, setEditorMonitors] = useState<EditorMonitor[]>([]);
  const [editorMonitorId, setEditorMonitorId] = useState("");
  const [page, setPage] = useState<AppPage>("library");
  const [previewZoom, setPreviewZoom] = useState(1);
  const [theme, setTheme] = useState<"dark" | "light">(() => (localStorage.getItem("wallpaper-lite.theme") === "light" ? "light" : "dark"));

  const zoomPreview = useCallback((delta: number | "reset") => {
    setPreviewZoom((zoom) => (delta === "reset" ? 1 : Math.min(4, Math.max(0.25, Math.round((zoom + delta) * 100) / 100))));
  }, []);

  const previewCanvasRef = useCallback((node: HTMLDivElement | null) => {
    if (!node) return;
    // Non-passive wheel listener so Ctrl+wheel zooms the preview instead of the whole page.
    node.addEventListener("wheel", (event) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      zoomPreview(event.deltaY < 0 ? 0.1 : -0.1);
    }, { passive: false });
  }, [zoomPreview]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("wallpaper-lite.theme", theme);
  }, [theme]);

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
  const editorPointer = useRef<EditorPointer | null>(null);
  const cropPointer = useRef<CropPointer | null>(null);
  const selectedEditorMonitor = editorMonitors.find((item) => item.id === editorMonitorId) ?? null;

  useEffect(() => {
    void hostAdapter.editorMonitors().then((monitors) => {
      setEditorMonitors(monitors);
      setEditorMonitorId(monitors[0]?.id ?? "");
    }).catch(() => setEditorError("Display information is unavailable."));
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

  async function openEditorPreview(entry: LibraryEntry) {
    setPage("editor");
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
    let dx = Math.round((event.clientX - pointer.startX) * monitor.width / bounds.width / previewZoom);
    let dy = Math.round((event.clientY - pointer.startY) * monitor.height / bounds.height / previewZoom);
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
    <div className="app-layout">
      <nav className="app-sidebar" aria-label="Main">
        <p className="app-brand">wallpaper-lite</p>
        <ul>
          {APP_PAGES.map((item) => (
            <li key={item.id}>
              <button className="app-nav-item" aria-current={page === item.id ? "page" : undefined} onClick={() => setPage(item.id)}>
                {item.label}
              </button>
            </li>
          ))}
        </ul>
        <button className="app-theme-toggle" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
          {theme === "dark" ? "Light mode" : "Dark mode"}
        </button>
      </nav>
    <main className="app-shell">
      <header className="app-header">
        <div>
          <h1>{APP_PAGES.find((item) => item.id === page)?.label}</h1>
          <p>Desktop wallpaper</p>
        </div>
        <div className="app-header-actions">
          <p aria-live="polite" className="host-status" role="status">{hostStatus}</p>
          <button className="ui-button--primary" disabled={checkingHost} onClick={checkHost}>
            {checkingHost ? "Checking..." : "Check wallpaper host"}
          </button>
        </div>
      </header>

      <PerformancePanels hidden={page !== "performance"} state={performanceState} />

      <DisplayPanel hidden={page !== "displays"} state={displaysState} />

      <PlaybackPanelView hidden={page !== "playback"} state={playbackState} />
      {page === "editor" && !editorEntry && (<p className="app-empty">Choose a wallpaper in the Library and press Edit.</p>)}
      {editorEntry && (
        <section className="ui-panel editor-preview-panel" hidden={page !== "editor"} aria-labelledby="editor-preview-title">
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
              ref={previewCanvasRef}
              tabIndex={0}
              className="editor-preview-canvas"
              onKeyDown={(event) => {
                if (!(event.ctrlKey || event.metaKey)) return;
                if (event.key === "+" || event.key === "=") zoomPreview(0.1);
                else if (event.key === "-") zoomPreview(-0.1);
                else if (event.key === "0") zoomPreview("reset");
                else return;
                event.preventDefault();
                event.stopPropagation();
              }}
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
              <div className="editor-preview-ghost" aria-hidden="true">
                <div className="editor-preview-zoom" style={{ transform: `scale(${previewZoom})` }}>
                  {previews[editorEntry.id] && editorSettingsDraft && (
                    <img draggable={false} src={previews[editorEntry.id].thumbnailDataUrl} alt="" style={editorGhostStyle(previews[editorEntry.id], editorSettingsDraft, editorPreview.monitorWidth, editorPreview.monitorHeight)} />
                  )}
                </div>
              </div>
              <div className="editor-preview-clip">
                <div className="editor-preview-zoom" style={{ transform: `scale(${previewZoom})` }}>
                <img draggable={false} src={editorPreview.dataUrl} alt={`Rendered ${editorEntry.displayName}`} style={editorDragTransform(editorPreview, editorSettingsDraft)} />
              </div>
              </div>
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

      <LibraryPage hidden={page !== "library"} state={libraryState} onOpenEditor={(entry) => void openEditorPreview(entry)} onStartPlayback={(entry) => void playbackState.startPlayback(entry)} />
    </main>
    </div>
  );
}

export default App;
