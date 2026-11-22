import test from "node:test";
import assert from "node:assert/strict";
import {
  buildPerformanceRows,
  formatBytes,
  formatPerformanceValue,
  isPerformanceSnapshotStale,
  PERFORMANCE_METRICS_POLL_MS,
} from "../src/performance-dashboard.mjs";

test("dashboard polling matches the documented one-second metric cadence", () => {
  assert.equal(PERFORMANCE_METRICS_POLL_MS, 1000);
});

test("new snapshots update rows and a failed target does not hide healthy metrics", () => {
  const displays = {
    displays: [
      { id: "DISPLAY1", name: "Main", deviceName: "\\\\.\\DISPLAY1" },
      { id: "DISPLAY2", name: "Side", deviceName: "\\\\.\\DISPLAY2" },
    ],
    profiles: [],
    targetFailures: [{ deviceName: "\\\\.\\DISPLAY2", stage: "present" }],
  };
  const first = buildPerformanceRows({ targets: [
    { displayId: "DISPLAY1", sourceFps: 24, renderedFps: 23.5, droppedFramesTotal: 2 },
  ] }, displays);
  assert.equal(first.length, 2);
  assert.equal(first[0].status, "available");
  assert.equal(first[0].metrics.sourceFps, 24);
  assert.equal(first[1].status, "failed");
  assert.equal(first[1].failureStage, "present");

  const updated = buildPerformanceRows({ targets: [
    { displayId: "DISPLAY1", sourceFps: 30, renderedFps: 30, droppedFramesTotal: 2 },
  ] }, displays);
  assert.equal(updated[0].metrics.sourceFps, 30);
  assert.equal(updated[1].status, "failed");
});

test("dashboard represents missing counters and disconnected assignments explicitly", () => {
  const rows = buildPerformanceRows({ targets: [
    { displayId: "DISPLAY1", mediaKind: "image", sourceFps: null, renderedFps: null },
  ] }, {
    displays: [{ id: "DISPLAY1", name: "Main", deviceName: "DISPLAY1" }],
    profiles: [{ displayId: "DISPLAY2" }],
    targetFailures: [],
  });
  assert.equal(rows[0].status, "available");
  assert.equal(rows[0].metrics.sourceFps, null);
  assert.equal(rows[1].status, "disconnected");
  assert.equal(formatPerformanceValue(null, "fps"), "Unavailable");
  assert.equal(formatBytes(null), "Unavailable");
});

test("disconnected profiles stay labeled disconnected when a metrics row remains", () => {
  const rows = buildPerformanceRows({ targets: [{
    displayId: "DISPLAY-2",
    mediaKind: "video",
    sourceFps: 24,
    renderedFps: 23.5,
    droppedFramesTotal: 1,
  }] }, {
    displays: [],
    profiles: [{ displayId: "DISPLAY-2", wallpaperId: 7 }],
  });
  assert.equal(rows.length, 1);
  assert.equal(rows[0].status, "disconnected");
  assert.equal(rows[0].metrics.displayId, "DISPLAY-2");
});

test("stale metrics are identified after the freshness window", () => {
  assert.equal(isPerformanceSnapshotStale(10_000, 13_500), false);
  assert.equal(isPerformanceSnapshotStale(10_000, 13_501), true);
  assert.equal(isPerformanceSnapshotStale(0, 10_000), true);
});
