import test from "node:test";
import assert from "node:assert/strict";

import { formatTimecode, isTimelineCommitKey, isValidSegment, moveSegmentHandle, nominalFrameStep100ns, parseSegmentDraft, parseTimecode, runPlaybackAction } from "../src/playback-time.mjs";

test("parses and formats millisecond timecodes without losing precision", () => {
  assert.equal(parseTimecode("01:02:03.456"), 3_723_456);
  assert.equal(formatTimecode(3_723_456), "01:02:03.456");
  assert.equal(formatTimecode(parseTimecode("123:59:59.999")), "123:59:59.999");
});

test("rejects malformed, out-of-range and unsafe timecodes", () => {
  for (const value of ["1:02:03.456", "00:60:00.000", "00:00:60.000", "00:00:00.1", "abc", "999999999999999999:00:00.000"]) {
    assert.equal(parseTimecode(value), null, value);
  }
});

test("validates segments against the 500 ms minimum and duration", () => {
  assert.equal(isValidSegment(100, 600, 1000), true);
  assert.equal(isValidSegment(100, 599, 1000), false);
  assert.equal(isValidSegment(600, 500, 1000), false);
  assert.equal(isValidSegment(0, 1001, 1000), false);
});

test("keyboard timeline changes commit and segment handles preserve valid bounds", () => {
  for (const key of ["ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"]) {
    assert.equal(isTimelineCommitKey(key), true);
  }
  assert.equal(isTimelineCommitKey("Tab"), false);

  const afterStartDrag = moveSegmentHandle("start", 250, 200, 700, 1000);
  assert.deepEqual(afterStartDrag, { startMs: 250, endMs: 750 });
  const afterEndDrag = moveSegmentHandle("end", 800, afterStartDrag.startMs, afterStartDrag.endMs, 1000);
  assert.deepEqual(afterEndDrag, { startMs: 250, endMs: 800 });
  assert.deepEqual(parseSegmentDraft("00:00:00.250", "00:00:00.800", 1000), afterEndDrag);
  assert.equal(parseSegmentDraft("00:00:00.250", "00:00:00.749", 1000), null);
  assert.equal(parseSegmentDraft("invalid", "00:00:00.800", 1000), null);
});

test("failed playback IPC result contains an error and no successful value", async () => {
  const result = await runPlaybackAction(async () => {
    throw new Error("Wallpaper host is not running.");
  }, "Playback failed.");
  assert.deepEqual(result, { ok: false, message: "Wallpaper host is not running." });
  assert.equal("value" in result, false);
});

test("computes nominal source-frame steps from rational frame rates", () => {
  assert.equal(nominalFrameStep100ns(30, 1), 333_333);
  assert.equal(nominalFrameStep100ns(24_000, 1_001), 417_083);
  assert.equal(nominalFrameStep100ns(4_294_967_295, 4_294_967_295), 10_000_000);
  assert.equal(nominalFrameStep100ns(0, 1), null);
});
