import test from "node:test";
import assert from "node:assert/strict";

import { canPlayFromDashboard, hostSummary, nowPlayingSummary, toggleControl, toggleLabel } from "../src/dashboard-view.mjs";

test("play/pause toggles between the two commands like the tray menu", () => {
  assert.equal(toggleControl("playing"), "pause");
  for (const state of ["paused", "stopped", "ended"]) {
    assert.equal(toggleControl(state), "play");
  }
  assert.equal(toggleLabel("playing"), "Pause");
  assert.equal(toggleLabel("paused"), "Play");
});

test("the host card names each state", () => {
  assert.equal(hostSummary("connected").label, "Running");
  assert.equal(hostSummary("disconnected").label, "Not running");
  assert.equal(hostSummary("checking").label, "Checking…");
  assert.equal(hostSummary(undefined).label, "Checking…");
});

test("nothing playing shows the empty state", () => {
  for (const playback of [null, undefined, { wallpaperId: null, state: "stopped", loadingVideo: false, speedPercent: 100, fpsLimit: 60 }]) {
    const summary = nowPlayingSummary(playback, null);
    assert.equal(summary.active, false);
    assert.equal(summary.title, "Nothing is playing");
  }
});

test("a playing wallpaper is named with its state, speed and frame limit", () => {
  const playback = { wallpaperId: 4, state: "playing", loadingVideo: false, speedPercent: 125, fpsLimit: 30 };
  assert.deepEqual(nowPlayingSummary(playback, { displayName: "Ocean" }), {
    active: true,
    title: "Ocean",
    detail: "Playing · 125% speed · 30 FPS limit",
  });
  assert.equal(nowPlayingSummary({ ...playback, state: "paused" }, { displayName: "Ocean" }).detail.startsWith("Paused"), true);
  assert.equal(nowPlayingSummary({ ...playback, loadingVideo: true }, { displayName: "Ocean" }).detail, "Loading…");
});

test("a playing wallpaper missing from the library is still described", () => {
  const playback = { wallpaperId: 9, state: "playing", loadingVideo: false, speedPercent: 100, fpsLimit: 60 };
  const summary = nowPlayingSummary(playback, null);
  assert.equal(summary.active, true);
  assert.match(summary.title, /no longer in the Library/);
});

test("only available videos can be started from the dashboard", () => {
  assert.equal(canPlayFromDashboard({ mediaKind: "video", availability: "available" }), true);
  assert.equal(canPlayFromDashboard({ mediaKind: "video", availability: "file_missing" }), false);
  assert.equal(canPlayFromDashboard({ mediaKind: "image", availability: "available" }), false);
});
