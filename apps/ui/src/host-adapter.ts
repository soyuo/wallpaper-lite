import { invoke } from "@tauri-apps/api/core";

export type ImportStatus =
  | "imported"
  | "already_present"
  | "rejected"
  | "unavailable"
  | "scan_failed"
  | "limit_reached";

export type ImportFailure =
  | "unsupported_format"
  | "invalid_media"
  | "codec_unavailable"
  | "source_unavailable"
  | "directory_unreadable"
  | "directory_depth_exceeded"
  | "candidate_limit_reached"
  | "catalog_write_failed";

export interface ImportResult {
  fileName: string;
  status: ImportStatus;
  failure: ImportFailure | null;
  wallpaperId: number | null;
}

export type MediaKind = "image" | "video";
export type Availability = "available" | "file_missing" | "unavailable";

export interface LibraryEntry {
  id: number;
  displayName: string;
  sourcePath: string;
  mediaKind: MediaKind;
  isFavorite: boolean;
  availability: Availability;
  sizeBytes: number | null;
}

export interface LibraryPreview {
  width: number;
  height: number;
  duration100ns: number | null;
  sizeBytes: number;
  thumbnailDataUrl: string;
}

export interface EditorMonitor {
  id: string;
  label: string;
  width: number;
  height: number;
}

export interface EditorPreview {
  width: number;
  height: number;
  monitorWidth: number;
  monitorHeight: number;
  dataUrl: string;
  settings: WallpaperEditorSettings;
}

export interface WallpaperEditorPreset {
  name: string;
  settings: WallpaperEditorSettings;
}

export interface WallpaperEditorSettings {
  schemaVersion: number;
  wallpaperId: number;
  render: {
    placementMode: "cover" | "fit" | "stretch" | "center" | "custom";
    positionPx: { x: number; y: number };
    sizePx: { width: number; height: number } | null;
    scale: { x: number; y: number };
    rotationDegrees: number;
    anchor: { x: number; y: number };
    crop: { x: number; y: number; width: number; height: number };
  };
  color: {
    brightness: number; contrast: number; saturation: number; gamma: number;
    opacity: number; temperature: number; red: number; green: number; blue: number;
  };
}

export interface PlaybackSettings {
  schemaVersion: number;
  wallpaperId: number;
  loopMode: "disabled" | "full" | "segment";
  segment: { startMs: number; endMs: number } | null;
  speedPercent: number;
  fpsLimit: number;
  volumePercent: number;
  muted: boolean;
}

export interface PlaybackState {
  sequence: number;
  wallpaperId: number | null;
  state: "stopped" | "playing" | "paused" | "ended";
  mediaTime100ns: number;
  duration100ns: number | null;
  frameRateNumerator: number | null;
  frameRateDenominator: number | null;
  speedPercent: number;
  fpsLimit: number;
  loadingVideo: boolean;
  lastError: string | null;
}

export interface PlaybackPanel {
  state: PlaybackState;
  settings: PlaybackSettings | null;
}

export type AppErrorCode =
  | "host_unavailable"
  | "timeout"
  | "disconnected"
  | "malformed_message"
  | "malformed_request"
  | "malformed_response"
  | "message_too_large"
  | "protocol_version_mismatch"
  | "unsupported_platform"
  | "transport_error"
  | "library_storage_failed"
  | "invalid_library_entry"
  | "editor_settings_storage_failed"
  | "invalid_editor_settings"
  | "editor_apply_failed"
  | "editor_apply_rollback_failed"
  | "editor_wallpaper_not_active"
  | "playback_settings_storage_failed"
  | "invalid_playback_settings"
  | "playback_control_failed"
  | "playback_settings_rollback_failed"
  | "internal";

interface AppErrorPayload {
  code: AppErrorCode;
  message: string;
}

export class HostAdapterError extends Error {
  constructor(
    readonly code: AppErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "HostAdapterError";
  }
}

export interface HostAdapter {
  ping(): Promise<void>;
  importMedia(paths: string[]): Promise<ImportResult[]>;
  listLibrary(): Promise<LibraryEntry[]>;
  libraryPreview(id: number): Promise<LibraryPreview>;
  editorMonitors(): Promise<EditorMonitor[]>;
  editorPreview(id: number, monitorWidth: number, monitorHeight: number): Promise<EditorPreview>;
  applyEditorSettings(id: number, settings: WallpaperEditorSettings): Promise<void>;
  listEditorPresets(id: number): Promise<WallpaperEditorPreset[]>;
  saveEditorPreset(id: number, name: string, settings: WallpaperEditorSettings): Promise<void>;
  renameEditorPreset(id: number, currentName: string, newName: string): Promise<void>;
  removeEditorPreset(id: number, name: string): Promise<void>;
  recentWallpaperIds(): Promise<number[]>;
  removeLibraryEntry(id: number): Promise<void>;
  renameLibraryEntry(id: number, displayName: string): Promise<void>;
  setLibraryFavorite(id: number, favorite: boolean): Promise<void>;
  relocateLibraryEntry(id: number, path: string): Promise<void>;
  playbackPanel(): Promise<PlaybackPanel>;
  playbackState(): Promise<PlaybackState>;
  loadVideoForPlayback(id: number): Promise<PlaybackPanel>;
  controlPlayback(control: "play" | "pause" | "stop"): Promise<PlaybackState>;
  seekPlayback(positionMs: number): Promise<PlaybackState>;
  seekPlayback100ns(position100ns: number): Promise<PlaybackState>;
  applyPlaybackSettings(id: number, settings: PlaybackSettings): Promise<PlaybackPanel>;
}

function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  if (typeof value !== "object" || value === null) return false;

  const payload = value as Record<string, unknown>;
  return (
    isAppErrorCode(payload.code) &&
    typeof payload.message === "string"
  );
}

async function invokeAppCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error: unknown) {
    if (isAppErrorPayload(error)) {
      throw new HostAdapterError(error.code, error.message);
    }
    throw error;
  }
}

function isAppErrorCode(value: unknown): value is AppErrorCode {
  return (
    value === "host_unavailable" ||
    value === "timeout" ||
    value === "disconnected" ||
    value === "malformed_message" ||
    value === "malformed_request" ||
    value === "malformed_response" ||
    value === "message_too_large" ||
    value === "protocol_version_mismatch" ||
    value === "unsupported_platform" ||
    value === "transport_error" ||
    value === "library_storage_failed" ||
    value === "invalid_library_entry" ||
    value === "editor_settings_storage_failed" ||
    value === "invalid_editor_settings" ||
    value === "editor_apply_failed" ||
    value === "editor_apply_rollback_failed" ||
    value === "editor_wallpaper_not_active" ||
    value === "playback_settings_storage_failed" ||
    value === "invalid_playback_settings" ||
    value === "playback_control_failed" ||
    value === "playback_settings_rollback_failed" ||
    value === "internal"
  );
}

export const hostAdapter: HostAdapter = {
  async ping() {
    try {
      await invoke<void>("host_ping");
    } catch (error: unknown) {
      if (isAppErrorPayload(error)) {
        throw new HostAdapterError(
          error.code,
          error.message,
        );
      }

      throw new HostAdapterError(
        "internal",
        "Could not check the wallpaper host.",
      );
    }
  },
  importMedia(paths) {
    return invoke<ImportResult[]>("import_media", { paths });
  },
  listLibrary() {
    return invoke<LibraryEntry[]>("list_library");
  },
  libraryPreview(id) {
    return invoke<LibraryPreview>("library_preview", { id });
  },
  editorMonitors() {
    return invoke<EditorMonitor[]>("editor_monitors");
  },
  editorPreview(id, monitorWidth, monitorHeight) {
    return invoke<EditorPreview>("editor_preview", { wallpaperId: id, monitorWidth, monitorHeight });
  },
  applyEditorSettings(id, settings) {
    return invokeAppCommand<void>("apply_editor_settings", { wallpaperId: id, settings });
  },
  listEditorPresets(id) {
    return invokeAppCommand<WallpaperEditorPreset[]>("list_editor_presets", { wallpaperId: id });
  },
  saveEditorPreset(id, name, settings) {
    return invokeAppCommand<void>("save_editor_preset", { wallpaperId: id, name, settings });
  },
  renameEditorPreset(id, currentName, newName) {
    return invokeAppCommand<void>("rename_editor_preset", { wallpaperId: id, currentName, newName });
  },
  removeEditorPreset(id, name) {
    return invokeAppCommand<void>("remove_editor_preset", { wallpaperId: id, name });
  },
  recentWallpaperIds() {
    return invokeAppCommand<number[]>("recent_wallpaper_ids");
  },
  removeLibraryEntry(id) {
    return invoke<void>("remove_library_entry", { id });
  },
  renameLibraryEntry(id, displayName) {
    return invoke<void>("rename_library_entry", { id, displayName });
  },
  setLibraryFavorite(id, favorite) {
    return invoke<void>("set_library_favorite", { id, favorite });
  },
  relocateLibraryEntry(id, path) {
    return invoke<void>("relocate_library_entry", { id, path });
  },
  playbackPanel() {
    return invokeAppCommand<PlaybackPanel>("get_playback_panel");
  },
  playbackState() {
    return invokeAppCommand<PlaybackState>("playback_state");
  },
  loadVideoForPlayback(id) {
    return invokeAppCommand<PlaybackPanel>("load_video_for_playback", { wallpaperId: id });
  },
  controlPlayback(control) {
    return invokeAppCommand<PlaybackState>("control_playback", { control });
  },
  seekPlayback(positionMs) {
    return invokeAppCommand<PlaybackState>("seek_playback", { positionMs });
  },
  seekPlayback100ns(position100ns) {
    return invokeAppCommand<PlaybackState>("seek_playback_100ns", { position100ns });
  },
  applyPlaybackSettings(id, settings) {
    return invokeAppCommand<PlaybackPanel>("apply_playback_settings", { wallpaperId: id, settings });
  },
};
