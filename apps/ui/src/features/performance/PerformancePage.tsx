import { NumberField } from "../../components/NumberField";
import { formatBytes, friendlyDisplayName } from "../../format";
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

function ratio(used: unknown, budget: unknown): number | null {
  return typeof used === "number" && typeof budget === "number" && budget > 0 ? Math.min(100, (used / budget) * 100) : null;
}

function StatCard({ label, value, sub, pct }: { label: string; value: string; sub?: string; pct?: number | null }) {
  return (
    <section className="performance-stat">
      <span className="performance-stat-label">{label}</span>
      <strong className="performance-stat-value">{value}</strong>
      {pct != null && <div className="dashboard-progress" aria-hidden="true"><span style={{ width: `${pct}%` }} /></div>}
      {sub && <span className="performance-stat-sub">{sub}</span>}
    </section>
  );
}

const POLICY_OPTIONS: { value: PolicyAction; label: string }[] = [
  { value: "continue", label: "Continue" },
  { value: "pause_rendering", label: "Pause rendering" },
  { value: "pause_playback", label: "Pause playback" },
];

function PolicySegment({ label, value, disabled, onChange }: { label: string; value: PolicyAction; disabled: boolean; onChange(value: PolicyAction): void }) {
  return (
    <div className="performance-policy-field">
      <span className="performance-stat-label">{label}</span>
      <div className="seg" role="group" aria-label={`${label} action`}>
        {POLICY_OPTIONS.map((option) => (
          <button key={option.value} aria-pressed={value === option.value} className={value === option.value ? "on" : undefined} disabled={disabled} onClick={() => onChange(option.value)}>{option.label}</button>
        ))}
      </div>
    </div>
  );
}

export function PerformancePage({ hidden, state }: { hidden: boolean; state: PerformanceState }) {
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
              <StatCard label="Process working set" value={formatBytes(performanceMetrics.processWorkingSetBytes)} />
              <StatCard label="Process private bytes" value={formatBytes(performanceMetrics.processPrivateBytes)} />
              <StatCard label="GPU local" value={formatBytes(performanceMetrics.gpuLocalUsageBytes)} sub={`of ${formatBytes(performanceMetrics.gpuLocalBudgetBytes)}`} pct={ratio(performanceMetrics.gpuLocalUsageBytes, performanceMetrics.gpuLocalBudgetBytes)} />
              <StatCard label="GPU non-local" value={formatBytes(performanceMetrics.gpuNonLocalUsageBytes)} sub={`of ${formatBytes(performanceMetrics.gpuNonLocalBudgetBytes)}`} pct={ratio(performanceMetrics.gpuNonLocalUsageBytes, performanceMetrics.gpuNonLocalBudgetBytes)} />
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
              {performanceRows.length > 0 ? (
                <div className="performance-table-wrap">
                  <table className="performance-table">
                    <thead>
                      <tr><th scope="col">Target</th><th scope="col">Source FPS</th><th scope="col">Rendered FPS</th><th scope="col">Frame interval</th><th scope="col">Dropped</th><th scope="col">Decode</th><th scope="col">Present</th><th scope="col">Status</th></tr>
                    </thead>
                    <tbody>
                      {performanceRows.map((row) => (
                        <tr key={row.id}>
                          <th scope="row">{friendlyDisplayName(row.label)}{row.metrics ? <small>{row.metrics.mediaKind}</small> : null}</th>
                          <td>{row.metrics ? formatPerformanceValue(row.metrics.sourceFps, "FPS") : "—"}</td>
                          <td>{row.metrics ? formatPerformanceValue(row.metrics.renderedFps, "FPS") : "—"}</td>
                          <td>{row.metrics ? formatPerformanceValue(row.metrics.frameIntervalMs, "ms") : "—"}</td>
                          <td>{row.metrics ? row.metrics.droppedFramesTotal.toLocaleString() : "—"}</td>
                          <td>{row.metrics ? formatPerformanceValue(row.metrics.decodeLatencyMs, "ms") : "—"}</td>
                          <td>{row.metrics ? formatPerformanceValue(row.metrics.presentLatencyMs, "ms") : "—"}</td>
                          <td><span className="chip" data-state={row.status === "available" ? "connected" : "disconnected"}><span className="dot" />{row.status === "available" ? "Connected" : row.status === "failed" ? "Target error" : row.status === "disconnected" ? "Disconnected" : row.status === "metrics_unavailable" ? "Metrics unavailable" : "Unmapped target"}</span></td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ) : <p>No display targets are currently registered.</p>}
              {performanceRows.map((row) => (
                <div key={row.id}>
                  {row.status === "failed" && <p className="performance-dashboard-warning" role="alert">{row.label} reported an error{row.failureStage ? ` during ${row.failureStage}` : ""}. Other display metrics remain available.</p>}
                  {row.status === "disconnected" && <p>{row.label}: the saved assignment is waiting for this display to reconnect. Any shown counters are the last host sample for this target.</p>}
                  {row.status === "metrics_unavailable" && <p>{row.label}: no host metrics are currently registered for this connected display.</p>}
                </div>
              ))}
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
          <button className="ui-button--primary" disabled={!performancePolicy || performancePolicySaving} onClick={() => void savePerformancePolicy()}>
            {performancePolicySaving ? "Applying..." : "Apply policies"}
          </button>
        </div>
        {performancePolicy ? (
          <div className="performance-policy-controls">
            <PolicySegment label="Fullscreen app" value={performancePolicy.fullscreenAction} disabled={performancePolicySaving} onChange={(fullscreenAction) => updatePerformancePolicy({ fullscreenAction })} />
            <PolicySegment label="Session lock" value={performancePolicy.sessionLockAction} disabled={performancePolicySaving} onChange={(sessionLockAction) => updatePerformancePolicy({ sessionLockAction })} />
            <PolicySegment
              label="On battery"
              value={performancePolicy.batteryAction}
              disabled={performancePolicySaving}
              onChange={(batteryAction) => updatePerformancePolicy({ batteryAction, batteryFpsLimit: batteryAction === "continue" ? performancePolicy.batteryFpsLimit : null })}
            />
            <label>
              Battery FPS cap (optional, 1–120)
              <NumberField
                aria-label="Battery FPS cap"
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
