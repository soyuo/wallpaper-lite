export function parseTimecode(value: string): number | null;
export function formatTimecode(milliseconds: number): string;
export function isValidSegment(startMs: number, endMs: number, durationMs: number): boolean;
export function parseSegmentDraft(startTime: string, endTime: string, durationMs: number): { startMs: number; endMs: number } | null;
export function moveSegmentHandle(handle: "start" | "end", valueMs: number, startMs: number, endMs: number, durationMs: number): { startMs: number; endMs: number } | null;
export function isTimelineCommitKey(key: string): boolean;
export function runPlaybackAction<T>(action: () => Promise<T>, fallbackMessage: string): Promise<{ ok: true; value: T } | { ok: false; message: string }>;
export function nominalFrameStep100ns(numerator: number, denominator: number): number | null;
