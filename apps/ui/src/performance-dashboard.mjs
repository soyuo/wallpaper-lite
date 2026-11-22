export const PERFORMANCE_METRICS_POLL_MS = 1000;
export const PERFORMANCE_METRICS_STALE_MS = 3500;

export function isPerformanceSnapshotStale(sampledAtUnixMs, nowUnixMs, staleAfterMs = PERFORMANCE_METRICS_STALE_MS) {
  return !Number.isFinite(sampledAtUnixMs)
    || sampledAtUnixMs <= 0
    || nowUnixMs - sampledAtUnixMs > staleAfterMs;
}

export function formatPerformanceValue(value, unit, digits = 1) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "Unavailable";
  return `${value.toFixed(digits)} ${unit}`;
}

export function formatBytes(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return "Unavailable";
  const units = ["B", "KiB", "MiB", "GiB"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function buildPerformanceRows(snapshot, displaySnapshot) {
  const targets = new Map(snapshot?.targets?.map((target) => [target.displayId, target]) ?? []);
  const savedProfiles = new Map((displaySnapshot?.profiles ?? []).map((profile) => [profile.displayId, profile]));
  const failuresByDevice = new Map(
    (displaySnapshot?.targetFailures ?? []).map((failure) => [failure.deviceName, failure]),
  );
  const rows = [];
  const connectedIds = new Set();
  const representedProfileIds = new Set();

  for (const display of displaySnapshot?.displays ?? []) {
    connectedIds.add(display.id);
    const metrics = targets.get(display.id) ?? null;
    const failure = failuresByDevice.get(display.deviceName) ?? null;
    rows.push({
      id: display.id,
      label: display.name,
      status: failure ? "failed" : metrics ? "available" : "metrics_unavailable",
      failureStage: failure?.stage ?? null,
      metrics,
    });
    if (failure) failuresByDevice.delete(display.deviceName);
  }

  for (const [displayId, metrics] of targets) {
    if (connectedIds.has(displayId)) continue;
    const profile = savedProfiles.get(displayId);
    if (profile) representedProfileIds.add(displayId);
    rows.push({
      id: displayId,
      label: displayId,
      status: profile ? "disconnected" : "unmapped",
      failureStage: null,
      metrics,
    });
  }

  for (const profile of displaySnapshot?.profiles ?? []) {
    if (connectedIds.has(profile.displayId) || representedProfileIds.has(profile.displayId)) continue;
    rows.push({
      id: profile.displayId,
      label: profile.displayId,
      status: "disconnected",
      failureStage: null,
      metrics: null,
    });
  }

  for (const [deviceName, failure] of failuresByDevice) {
    rows.push({
      id: `failure:${deviceName}`,
      label: deviceName,
      status: "failed",
      failureStage: failure.stage,
      metrics: null,
    });
  }
  return rows;
}
