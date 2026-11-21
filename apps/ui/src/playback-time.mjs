const TIME_RE = /^(\d{2,}):([0-5]\d):([0-5]\d)\.(\d{3})$/;

export function parseTimecode(value) {
  const match = TIME_RE.exec(value.trim());
  if (!match) return null;
  const [hours, minutes, seconds, milliseconds] = match.slice(1).map(Number);
  const total = (((hours * 60 + minutes) * 60 + seconds) * 1000) + milliseconds;
  return Number.isSafeInteger(total) ? total : null;
}

export function formatTimecode(milliseconds) {
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) return "00:00:00.000";
  const hours = Math.floor(milliseconds / 3_600_000);
  const minutes = Math.floor(milliseconds / 60_000) % 60;
  const seconds = Math.floor(milliseconds / 1000) % 60;
  const fraction = milliseconds % 1000;
  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(fraction).padStart(3, "0")}`;
}

export function isValidSegment(startMs, endMs, durationMs) {
  return Number.isSafeInteger(startMs)
    && Number.isSafeInteger(endMs)
    && Number.isSafeInteger(durationMs)
    && startMs >= 0
    && endMs <= durationMs
    && endMs - startMs >= 500;
}

export function parseSegmentDraft(startTime, endTime, durationMs) {
  const startMs = parseTimecode(startTime);
  const endMs = parseTimecode(endTime);
  return startMs !== null && endMs !== null && isValidSegment(startMs, endMs, durationMs)
    ? { startMs, endMs }
    : null;
}

export function moveSegmentHandle(handle, valueMs, startMs, endMs, durationMs) {
  if (!Number.isSafeInteger(valueMs) || !Number.isSafeInteger(startMs) || !Number.isSafeInteger(endMs)
    || !Number.isSafeInteger(durationMs) || durationMs < 500) return null;
  const currentStart = Math.min(Math.max(0, startMs), durationMs - 500);
  const currentEnd = Math.min(durationMs, Math.max(currentStart + 500, endMs));
  if (handle === "start") {
    const nextStart = Math.min(Math.max(0, valueMs), durationMs - 500);
    return { startMs: nextStart, endMs: Math.min(durationMs, Math.max(currentEnd, nextStart + 500)) };
  }
  if (handle === "end") {
    const nextEnd = Math.max(Math.min(durationMs, valueMs), currentStart + 500);
    return { startMs: currentStart, endMs: nextEnd };
  }
  return null;
}

export function isTimelineCommitKey(key) {
  return ["ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"].includes(key);
}

export async function runPlaybackAction(action, fallbackMessage) {
  try {
    return { ok: true, value: await action() };
  } catch (error) {
    return { ok: false, message: error instanceof Error ? error.message : fallbackMessage };
  }
}

export function nominalFrameStep100ns(numerator, denominator) {
  if (!Number.isSafeInteger(numerator) || !Number.isSafeInteger(denominator)
    || numerator <= 0 || denominator <= 0) return null;
  const numeratorBig = BigInt(numerator);
  const numeratorTicks = 10_000_000n * BigInt(denominator);
  const step = (numeratorTicks + numeratorBig / 2n) / numeratorBig;
  return step > 0n && step <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(step) : null;
}
