import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import { EditorHistory, resolveEditorShortcut } from "../../editor-history.mjs";
import {
  HostAdapterError,
  hostAdapter,
  type EditorMonitor,
  type EditorPreview,
  type LibraryEntry,
  type LibraryPreview,
  type WallpaperEditorPreset,
  type WallpaperEditorSettings,
} from "../../host-adapter";
import { defaultEditorSettings, type CropPointer, type EditorPointer } from "./editor-utils";

export interface EditorHostDeps {
  previews: Record<number, LibraryPreview>;
  setPreviews: React.Dispatch<React.SetStateAction<Record<number, LibraryPreview>>>;
  setRecentIds: React.Dispatch<React.SetStateAction<number[]>>;
  onOpen(): void;
}

export function useEditor({ previews, setPreviews, setRecentIds, onOpen }: EditorHostDeps) {
  const [editorMonitors, setEditorMonitors] = useState<EditorMonitor[]>([]);
  const [editorMonitorId, setEditorMonitorId] = useState("");
  const [previewZoom, setPreviewZoom] = useState(1);

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

  async function openEditorPreview(entry: LibraryEntry) {
    onOpen();
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

  return {
    beginCropPointer,
    beginEditorGesture,
    beginEditorPointer,
    cancelEditorChanges,
    changeEditorMonitor,
    createCropMode,
    cropPointer,
    editorBusy,
    editorEntry,
    editorError,
    editorHistory,
    editorMonitorId,
    editorMonitors,
    editorPointer,
    editorPresets,
    editorPreview,
    editorSaveBusy,
    editorSettingsDraft,
    endEditorGesture,
    loadPresetToDraft,
    moveCropPointer,
    moveEditorPointer,
    openEditorPreview,
    presetBusy,
    presetMessage,
    presetName,
    presetRename,
    previewCanvasRef,
    previewZoom,
    redoEditorChange,
    removePreset,
    renamePreset,
    resetEditorColor,
    resetEditorSettings,
    saveCurrentPreset,
    saveEditorSettingsDraft,
    selectPlacementMode,
    selectedEditorMonitor,
    setCreateCropMode,
    setCropPreset,
    setPresetName,
    setPresetRename,
    undoEditorChange,
    updateEditorColor,
    updateEditorRender,
    zoomPreview,
  };
}

export type EditorState = ReturnType<typeof useEditor>;
