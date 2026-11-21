import test from "node:test";
import assert from "node:assert/strict";

import { applyDisplayProfiles, buildDisplayProfiles, makeDisplayEdits } from "../src/display-manager.mjs";

const snapshot = {
  displays: [
    { id: "display-a", name: "Main", isPrimary: true },
    { id: "display-b", name: "Side", isPrimary: false },
  ],
  profiles: [
    { displayId: "display-a", wallpaperId: 1, presentationMode: "independent", fpsOverride: null },
    { displayId: "display-disconnected", wallpaperId: 2, presentationMode: "independent", fpsOverride: 30 },
  ],
  wallpapers: [
    { id: 1, name: "Red", mediaKind: "image", availability: "available" },
    { id: 2, name: "Video", mediaKind: "video", availability: "available" },
    { id: 3, name: "Missing", mediaKind: "image", availability: "file_missing" },
  ],
};

test("initial edits retain saved values and default an unassigned display to available media", () => {
  const result = makeDisplayEdits(snapshot);
  assert.equal(result.mode, "independent");
  assert.deepEqual(result.edits["display-a"], { wallpaperId: 1, fpsOverride: "" });
  assert.deepEqual(result.edits["display-b"], { wallpaperId: 1, fpsOverride: "" });
});

test("independent profiles preserve disconnected records and validate per-display FPS", () => {
  const edits = {
    "display-a": { wallpaperId: 1, fpsOverride: "60" },
    "display-b": { wallpaperId: 2, fpsOverride: "30" },
  };
  const result = buildDisplayProfiles(snapshot, "independent", edits);
  assert.equal(result.find((profile) => profile.displayId === "display-b").wallpaperId, 2);
  assert.equal(result.find((profile) => profile.displayId === "display-b").fpsOverride, 30);
  assert.ok(result.some((profile) => profile.displayId === "display-disconnected"));
  assert.throws(() => buildDisplayProfiles(snapshot, "independent", {
    ...edits,
    "display-b": { wallpaperId: 2, fpsOverride: "121" },
  }), /1–120/);
});

test("duplicate and span profiles fan out the primary wallpaper but keep target FPS", () => {
  const edits = {
    "display-a": { wallpaperId: 2, fpsOverride: "60" },
    "display-b": { wallpaperId: 1, fpsOverride: "30" },
  };
  for (const mode of ["duplicate", "span"]) {
    const result = buildDisplayProfiles(snapshot, mode, edits);
    const connected = result.filter((profile) => profile.displayId === "display-a" || profile.displayId === "display-b");
    assert.deepEqual(connected.map((profile) => profile.wallpaperId), [2, 2]);
    assert.deepEqual(connected.map((profile) => profile.fpsOverride), [60, 30]);
  }
});

test("unavailable media is rejected before a profile is applied", () => {
  assert.throws(() => buildDisplayProfiles(snapshot, "independent", {
    "display-a": { wallpaperId: 3, fpsOverride: "" },
    "display-b": { wallpaperId: 1, fpsOverride: "" },
  }), /available wallpaper/);
});

test("a failed host apply returns an error state and never a success message", async () => {
  const result = await applyDisplayProfiles([], async () => { throw new Error("host unavailable"); });
  assert.equal(result.ok, false);
  assert.equal(result.message, "host unavailable");
});

test("target failures are shown after the saved profiles are restored", async () => {
  const result = await applyDisplayProfiles([], async () => ({
    applied: false,
    failedTargets: [{ deviceName: "\\\\.\\DISPLAY2", stage: "video_load" }],
  }));
  assert.equal(result.ok, false);
  assert.match(result.message, /DISPLAY2/);
  assert.match(result.message, /video_load/);
  assert.match(result.message, /restored/);
});
