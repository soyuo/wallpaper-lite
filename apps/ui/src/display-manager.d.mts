import type { DisplayManagerSnapshot, DisplayProfile } from "./host-adapter";

export function makeDisplayEdits(snapshot: DisplayManagerSnapshot): {
  mode: DisplayProfile["presentationMode"];
  edits: Record<string, { wallpaperId: number | ""; fpsOverride: string }>;
};
export function buildDisplayProfiles(
  snapshot: DisplayManagerSnapshot,
  mode: DisplayProfile["presentationMode"],
  edits: Record<string, { wallpaperId: number | string; fpsOverride: string }>,
): DisplayProfile[];
export function applyDisplayProfiles(
  profiles: DisplayProfile[],
  apply: (profiles: DisplayProfile[]) => Promise<{ applied: boolean; failedTargets: { deviceName: string; stage: string }[] }>,
): Promise<{ ok: boolean; message: string }>;
