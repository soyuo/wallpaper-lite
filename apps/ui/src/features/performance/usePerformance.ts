import { useEffect, useState } from "react";

import { buildPerformanceRows, isPerformanceSnapshotStale, PERFORMANCE_METRICS_POLL_MS } from "../../performance-dashboard.mjs";
import {
  hostAdapter,
  type DisplayManagerSnapshot,
  type PerformanceMetricsSnapshot,
  type PerformancePolicySettings,
} from "../../host-adapter";

export function usePerformance(displaySnapshot: DisplayManagerSnapshot | null) {
  const [performancePolicy, setPerformancePolicy] = useState<PerformancePolicySettings | null>(null);
  const [performancePolicySaving, setPerformancePolicySaving] = useState(false);
  const [performancePolicyMessage, setPerformancePolicyMessage] = useState("");
  const [performanceMetrics, setPerformanceMetrics] = useState<PerformanceMetricsSnapshot | null>(null);
  const [performanceMetricsError, setPerformanceMetricsError] = useState("");
  const [performanceMetricsNow, setPerformanceMetricsNow] = useState(() => Date.now());

  useEffect(() => {
    let stopped = false;
    void hostAdapter.performancePolicySettings().then((settings) => {
      if (!stopped) setPerformancePolicy(settings);
    }).catch((error: unknown) => {
      if (!stopped) setPerformancePolicyMessage(error instanceof Error ? error.message : "Performance policy settings could not be loaded.");
    });
    return () => { stopped = true; };
  }, []);

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const snapshot = await hostAdapter.performanceMetrics();
        if (!stopped) {
          setPerformanceMetrics(snapshot);
          setPerformanceMetricsError("");
        }
      } catch (error: unknown) {
        if (!stopped) {
          setPerformanceMetricsError(error instanceof Error ? error.message : "Performance metrics could not be refreshed.");
        }
      } finally {
        if (!stopped) {
          setPerformanceMetricsNow(Date.now());
          timer = setTimeout(() => void poll(), PERFORMANCE_METRICS_POLL_MS);
        }
      }
    };
    void poll();
    return () => {
      stopped = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, []);

  function updatePerformancePolicy(updates: Partial<PerformancePolicySettings>) {
    setPerformancePolicy((current) => current ? { ...current, ...updates } : current);
    setPerformancePolicyMessage("");
  }

  async function savePerformancePolicy() {
    if (!performancePolicy) return;
    setPerformancePolicySaving(true);
    setPerformancePolicyMessage("");
    try {
      await hostAdapter.applyPerformancePolicySettings(performancePolicy);
      setPerformancePolicyMessage("Power-saving policy saved and applied to active playback targets.");
    } catch (error: unknown) {
      setPerformancePolicyMessage(error instanceof Error ? error.message : "Power-saving policy could not be applied.");
    } finally {
      setPerformancePolicySaving(false);
    }
  }

  const performanceRows = buildPerformanceRows(performanceMetrics, displaySnapshot);
  const performanceMetricsStale = performanceMetrics !== null
    && isPerformanceSnapshotStale(performanceMetrics.sampledAtUnixMs, performanceMetricsNow);

  return {
    performancePolicy,
    performancePolicySaving,
    performancePolicyMessage,
    performanceMetrics,
    performanceMetricsError,
    performanceRows,
    performanceMetricsStale,
    updatePerformancePolicy,
    savePerformancePolicy,
  };
}

export type PerformanceState = ReturnType<typeof usePerformance>;
