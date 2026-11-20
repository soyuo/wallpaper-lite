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
  removeLibraryEntry(id: number): Promise<void>;
  renameLibraryEntry(id: number, displayName: string): Promise<void>;
  setLibraryFavorite(id: number, favorite: boolean): Promise<void>;
  relocateLibraryEntry(id: number, path: string): Promise<void>;
}

function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  if (typeof value !== "object" || value === null) return false;

  const payload = value as Record<string, unknown>;
  return (
    isAppErrorCode(payload.code) &&
    typeof payload.message === "string"
  );
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
};
