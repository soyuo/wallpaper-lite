import { formatBytes } from "../../format";
import { formatPerformanceValue } from "../../performance-dashboard.mjs";
import type { PolicyAction } from "../../host-adapter";
import type { PerformanceState } from "./usePerformance";

function policyActionLabel(action: PolicyAction): string {
  switch (action) {
    case "pause_rendering": return "Pause rendering";
    case "pause_playback": return "Pause playback";
    default: return "Continue";
  }
}

export function PerformancePanels({ hidden, state }: { hidden: boolean; state: PerformanceState }) {
  const {
    performancePolicy,
    performancePolicySaving,
    performancePolicyMessage,
    performanceMetrics,
    performanceMetricsError,
    performanceRows,
    performanceMetricsStale,
    updatePerformancePolicy,
    savePerformancePolicy,
  } = state;

  return (
    <>
      <section className="ui-panel performance-dashboard" hidden={hidden} aria-labelledby="performance-dashboard-title">
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

      <section className="ui-panel performance-policy" hidden={hidden} aria-labelledby="performance-policy-title">
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
    </>
  );
}
