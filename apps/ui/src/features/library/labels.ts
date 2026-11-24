import type { Availability, ImportFailure, ImportResult } from "../../host-adapter";

export const failureLabels: Record<ImportFailure, string> = {
  unsupported_format: "Unsupported format",
  invalid_media: "The file is corrupt or could not be decoded",
  codec_unavailable: "Unsupported video codec or required decoder unavailable",
  source_unavailable: "The source file is unavailable",
  directory_unreadable: "A folder could not be read",
  directory_depth_exceeded: "Folder nesting limit reached",
  candidate_limit_reached: "Import limit reached (4,096 files)",
  catalog_write_failed: "The library could not be saved",
};

export function resultLabel(result: ImportResult): string {
  if (result.failure) return failureLabels[result.failure];
  if (result.status === "already_present") return "Already in library";
  if (result.status === "imported") return "Added to library";
  return "Import could not be completed";
}

export function formatDuration(duration100ns: number | null): string | null {
  if (duration100ns === null) return null;
  const totalSeconds = Math.floor(duration100ns / 10_000_000);
  return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

export function availabilityLabel(value: Availability): string {
  if (value === "file_missing") return "FILE_MISSING";
  if (value === "unavailable") return "Unavailable";
  return "Available";
}

/** Mirrors the host rule: another entry with the same name (any case) gets " 2", " 3", ... appended. */
export function uniqueDisplayName(entries: { id: number; displayName: string }[], id: number, draft: string): string {
  const name = draft.trim();
  const taken = (candidate: string) =>
    entries.some((entry) => entry.id !== id && entry.displayName.toLowerCase() === candidate.toLowerCase());
  if (!taken(name)) return name;
  for (let number = 2; ; number += 1) {
    const candidate = `${name} ${number}`;
    if (!taken(candidate)) return candidate;
  }
}
