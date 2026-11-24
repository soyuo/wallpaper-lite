export type HostState = "checking" | "connected" | "disconnected";

export function toggleControl(state: string): "play" | "pause";
export function toggleLabel(state: string): "Play" | "Pause";
export function hostSummary(hostState: HostState): { label: string; detail: string };
export function nowPlayingSummary(
  playback: { wallpaperId: number | null; state: string; loadingVideo: boolean; speedPercent: number; fpsLimit: number } | null | undefined,
  entry: { displayName: string } | null,
): { active: boolean; title: string; detail: string };
export function canPlayFromDashboard(entry: { mediaKind: string; availability: string }): boolean;
