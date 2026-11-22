export const PERFORMANCE_METRICS_POLL_MS: number;
export const PERFORMANCE_METRICS_STALE_MS: number;
export function isPerformanceSnapshotStale(sampledAtUnixMs: number, nowUnixMs: number, staleAfterMs?: number): boolean;
export function formatPerformanceValue(value: number | null, unit: string, digits?: number): string;
export function formatBytes(value: number | null): string;
export function buildPerformanceRows(snapshot: import("./host-adapter").PerformanceMetricsSnapshot | null, displaySnapshot: import("./host-adapter").DisplayManagerSnapshot | null): Array<{
  id: string;
  label: string;
  status: "available" | "failed" | "metrics_unavailable" | "disconnected" | "unmapped";
  failureStage: string | null;
  metrics: import("./host-adapter").TargetPerformanceMetrics | null;
}>;
