import { useEffect, useState } from "react";

import { hostAdapter, type LibraryEntry, type PlaybackPanel, type PlaybackSettings } from "../../host-adapter";
import { formatTimecode, nominalFrameStep100ns, parseSegmentDraft, parseTimecode, runPlaybackAction } from "../../playback-time.mjs";
import { describeError } from "../errors/error-message";

export function usePlayback() {
  const [playbackPanel, setPlaybackPanel] = useState<PlaybackPanel | null>(null);
  const [playbackSettingsDraft, setPlaybackSettingsDraft] = useState<PlaybackSettings | null>(null);
  const [playbackMessage, setPlaybackMessage] = useState("");
  const [playbackBusy, setPlaybackBusy] = useState(false);
  const [seekTimeDraft, setSeekTimeDraft] = useState("00:00:00.000");
  const [seekTimeDirty, setSeekTimeDirty] = useState(false);
  const [segmentStartDraft, setSegmentStartDraft] = useState("00:00:00.000");
  const [segmentEndDraft, setSegmentEndDraft] = useState("00:00:00.000");
  const [timelineDraftMs, setTimelineDraftMs] = useState<number | null>(null);

  useEffect(() => {
    let stopped = false;
    let polling = false;
    void hostAdapter.playbackPanel().then((panel) => {
      if (stopped) return;
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setPlaybackMessage("");
    }).catch((error: unknown) => {
      if (!stopped) setPlaybackMessage(describeError(error, "Playback state is unavailable."));
    });
    const timer = window.setInterval(() => {
      if (polling || stopped) return;
      polling = true;
      void hostAdapter.playbackState().then((state) => {
        if (!stopped) setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
      }).catch((error: unknown) => {
        if (!stopped) setPlaybackMessage(describeError(error, "Playback state is unavailable."));
      }).finally(() => { polling = false; });
    }, 500);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const settings = playbackPanel?.settings;
    if (!settings) return;
    setPlaybackSettingsDraft(settings);
    const segment = settings.segment;
    if (segment) {
      setSegmentStartDraft(formatTimecode(segment.startMs));
      setSegmentEndDraft(formatTimecode(segment.endMs));
    }
  }, [playbackPanel?.settings]);

  useEffect(() => {
    if (playbackPanel?.state && !seekTimeDirty) {
      setSeekTimeDraft(formatTimecode(Math.max(0, Math.floor(playbackPanel.state.mediaTime100ns / 10_000))));
    }
  }, [playbackPanel?.state.mediaTime100ns, seekTimeDirty]);

  async function startPlayback(entry: LibraryEntry) {
    setPlaybackBusy(true);
    setPlaybackMessage(`Loading ${entry.displayName}...`);
    try {
      const panel = await hostAdapter.loadVideoForPlayback(entry.id);
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setSeekTimeDraft(formatTimecode(Math.floor(panel.state.mediaTime100ns / 10_000)));
      setSeekTimeDirty(false);
      if (panel.settings?.segment) {
        setSegmentStartDraft(formatTimecode(panel.settings.segment.startMs));
        setSegmentEndDraft(formatTimecode(panel.settings.segment.endMs));
      } else {
        setSegmentStartDraft("00:00:00.000");
        setSegmentEndDraft(formatTimecode(Math.floor((panel.state.duration100ns ?? 0) / 10_000)));
      }
      setPlaybackMessage(`Playing ${entry.displayName}.`);
    } catch (error) {
      setPlaybackMessage(describeError(error, "The video could not be started."));
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function sendPlaybackControl(control: "play" | "pause" | "stop") {
    setPlaybackBusy(true);
    setPlaybackMessage("");
    const result = await runPlaybackAction(
      () => hostAdapter.controlPlayback(control),
      `Could not ${control} playback.`,
    );
    if (result.ok) {
      const state = result.value;
      setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
    } else {
      setPlaybackMessage(result.message);
    }
    setPlaybackBusy(false);
  }

  async function removeWallpaper() {
    setPlaybackBusy(true);
    setPlaybackMessage("");
    const result = await runPlaybackAction(
      () => hostAdapter.removeWallpaper(),
      "Could not remove the wallpaper.",
    );
    if (result.ok) {
      const state = result.value;
      setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
    } else {
      setPlaybackMessage(result.message);
    }
    setPlaybackBusy(false);
    return result.ok;
  }

  async function commitSeek(positionMs: number) {
    if (!Number.isSafeInteger(positionMs) || positionMs < 0) return;
    const durationMs = playbackPanel?.state.duration100ns == null
      ? null
      : Math.floor(playbackPanel.state.duration100ns / 10_000);
    if (durationMs !== null && positionMs > durationMs) {
      setPlaybackMessage("Seek time cannot exceed the video duration.");
      return;
    }
    setPlaybackBusy(true);
    try {
      const state = await hostAdapter.seekPlayback(positionMs);
      setPlaybackPanel((panel) => panel ? { ...panel, state } : { state, settings: null });
      setSeekTimeDraft(formatTimecode(positionMs));
      setSeekTimeDirty(false);
      setPlaybackMessage("");
    } catch (error) {
      setPlaybackMessage(describeError(error, "The seek command failed."));
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function seekByFrame(direction: -1 | 1) {
    const state = playbackPanel?.state;
    if (!state || state.frameRateNumerator == null || state.frameRateDenominator == null) {
      setPlaybackMessage("Source frame rate is unavailable.");
      return;
    }
    const step = nominalFrameStep100ns(state.frameRateNumerator, state.frameRateDenominator);
    if (step === null) {
      setPlaybackMessage("Source frame rate is invalid.");
      return;
    }
    const duration = state.duration100ns ?? Number.MAX_SAFE_INTEGER;
    const target = Math.max(0, Math.min(duration, state.mediaTime100ns + direction * step));
    setPlaybackBusy(true);
    try {
      const next = await hostAdapter.seekPlayback100ns(target);
      setPlaybackPanel((panel) => panel ? { ...panel, state: next } : { state: next, settings: null });
      setPlaybackMessage("");
    } catch (error) {
      setPlaybackMessage(describeError(error, "Frame seek failed."));
    } finally {
      setPlaybackBusy(false);
    }
  }

  async function applyPlaybackSettingsDraft() {
    const active = playbackPanel?.state.wallpaperId;
    const draft = playbackSettingsDraft;
    if (active == null || !draft || draft.wallpaperId !== active) {
      setPlaybackMessage("Start a video before applying its playback settings.");
      return;
    }
    let segment = draft.segment;
    if (draft.loopMode === "segment") {
      const durationMs = playbackPanel?.state.duration100ns == null
        ? 0
        : Math.floor(playbackPanel.state.duration100ns / 10_000);
      const parsedSegment = parseSegmentDraft(segmentStartDraft, segmentEndDraft, durationMs);
      if (!parsedSegment) {
        setPlaybackMessage("Segment loop needs valid timecodes, at least 500 ms long, within the video duration.");
        return;
      }
      segment = parsedSegment;
    } else {
      segment = null;
    }
    const candidate = { ...draft, segment };
    setPlaybackBusy(true);
    setPlaybackMessage("");
    try {
      const panel = await hostAdapter.applyPlaybackSettings(active, candidate);
      setPlaybackPanel(panel);
      setPlaybackSettingsDraft(panel.settings);
      setPlaybackMessage("Playback settings applied and saved.");
    } catch (error) {
      setPlaybackMessage(describeError(error, "Playback settings could not be applied."));
    } finally {
      setPlaybackBusy(false);
    }
  }

  const playbackState = playbackPanel?.state ?? null;
  const playbackDurationMs = playbackState?.duration100ns == null
    ? 0
    : Math.max(0, Math.floor(playbackState.duration100ns / 10_000));
  const playbackPositionMs = playbackState
    ? Math.max(0, Math.floor(playbackState.mediaTime100ns / 10_000))
    : 0;
  const seekTimecode = async () => {
    const parsed = parseTimecode(seekTimeDraft);
    if (parsed === null) {
      setPlaybackMessage("Enter time as HH:MM:SS.mmm.");
      return;
    }
    await commitSeek(parsed);
  };


  return {
    playbackPanel,
    playbackSettingsDraft,
    playbackMessage,
    playbackBusy,
    seekTimeDraft,
    seekTimeDirty,
    segmentStartDraft,
    segmentEndDraft,
    timelineDraftMs,
    setPlaybackSettingsDraft,
    setPlaybackMessage,
    setSeekTimeDraft,
    setSeekTimeDirty,
    setSegmentStartDraft,
    setSegmentEndDraft,
    setTimelineDraftMs,
    playbackState,
    playbackDurationMs,
    playbackPositionMs,
    seekTimecode,
    startPlayback,
    sendPlaybackControl,
    removeWallpaper,
    commitSeek,
    seekByFrame,
    applyPlaybackSettingsDraft,
  };
}

export type PlaybackState = ReturnType<typeof usePlayback>;
