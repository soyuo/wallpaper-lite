import { formatTimecode, isTimelineCommitKey, moveSegmentHandle, parseTimecode } from "../../playback-time.mjs";
import type { PlaybackSettings } from "../../host-adapter";
import type { PlaybackState } from "./usePlayback";

export function PlaybackPanelView({ hidden, state }: { hidden: boolean; state: PlaybackState }) {
  const {
    playbackSettingsDraft,
    playbackMessage,
    playbackBusy,
    seekTimeDraft,
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
    sendPlaybackControl,
    commitSeek,
    seekByFrame,
    applyPlaybackSettingsDraft,
  } = state;

  return (
      <section className="ui-panel playback-panel" hidden={hidden} aria-labelledby="playback-title">
        <h2 id="playback-title">Video playback</h2>
        {playbackState?.wallpaperId == null ? (
          <p>No video is active. Start a video from its library card.</p>
        ) : (
          <>
            <p className="playback-state">{playbackState.loadingVideo ? "Loading" : playbackState.state} · {playbackState.speedPercent}% · {playbackState.fpsLimit} FPS</p>
            {playbackState.lastError && <p className="playback-message" role="status">Host playback error: {playbackState.lastError}</p>}
            <div className="playback-timeline">
              <label htmlFor="playback-seek">Timeline</label>
              <input
                id="playback-seek"
                type="range"
                min={0}
                max={Math.max(1, playbackDurationMs)}
                value={Math.min(playbackDurationMs || playbackPositionMs, timelineDraftMs ?? playbackPositionMs)}
                disabled={playbackBusy || playbackDurationMs <= 0 || playbackState.loadingVideo}
                onChange={(event) => setTimelineDraftMs(Number(event.currentTarget.value))}
                onPointerUp={(event) => {
                  const value = Number(event.currentTarget.value);
                  setTimelineDraftMs(null);
                  void commitSeek(value);
                }}
                onKeyUp={(event) => {
                  if (isTimelineCommitKey(event.key)) {
                    const value = Number(event.currentTarget.value);
                    setTimelineDraftMs(null);
                    void commitSeek(value);
                  }
                }}
              />
              <span>{formatTimecode(playbackPositionMs)} / {formatTimecode(playbackDurationMs)}</span>
            </div>
            <div className="playback-controls" aria-label="Video transport controls">
              <button disabled={playbackBusy || playbackState.loadingVideo || !playbackState.frameRateNumerator} onClick={() => void seekByFrame(-1)}>Previous frame</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("play")}>Play</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("pause")}>Pause</button>
              <button disabled={playbackBusy || playbackState.loadingVideo} onClick={() => void sendPlaybackControl("stop")}>Stop</button>
              <button disabled={playbackBusy || playbackState.loadingVideo || !playbackState.frameRateNumerator} onClick={() => void seekByFrame(1)}>Next frame</button>
            </div>
            <label className="playback-time-entry">
              <span>Seek time (HH:MM:SS.mmm)</span>
              <input value={seekTimeDraft} onChange={(event) => { setSeekTimeDirty(true); setSeekTimeDraft(event.currentTarget.value); }} onKeyDown={(event) => { if (event.key === "Enter") void seekTimecode(); }} onBlur={() => { if (parseTimecode(seekTimeDraft) === null) setPlaybackMessage("Enter time as HH:MM:SS.mmm."); }} />
              <button disabled={playbackBusy} onClick={() => void seekTimecode()}>Seek</button>
            </label>
            {playbackSettingsDraft?.wallpaperId === playbackState.wallpaperId && (
              <fieldset className="playback-settings">
                <legend>Loop and playback settings</legend>
                <label>
                  <span>Loop mode</span>
                  <select
                    value={playbackSettingsDraft.loopMode}
                    onChange={(event) => {
                      const loopMode = event.currentTarget.value as PlaybackSettings["loopMode"];
                      const segment = loopMode === "segment"
                        ? playbackSettingsDraft.segment ?? { startMs: 0, endMs: playbackDurationMs }
                        : null;
                      setPlaybackSettingsDraft({ ...playbackSettingsDraft, loopMode, segment });
                      if (loopMode === "segment" && segment) {
                        setSegmentStartDraft(formatTimecode(segment.startMs));
                        setSegmentEndDraft(formatTimecode(segment.endMs));
                      }
                    }}
                  >
                    <option value="disabled">Disabled</option>
                    <option value="full">Full video</option>
                    <option value="segment">Segment</option>
                  </select>
                </label>
                {playbackSettingsDraft.loopMode === "segment" && (
                  <div className="playback-segment-fields">
                    <label>
                      <span>Segment start handle</span>
                      <input
                        type="range"
                        min={0}
                        max={Math.max(0, playbackDurationMs - 500)}
                        value={Math.min(Math.max(0, playbackDurationMs - 500), parseTimecode(segmentStartDraft) ?? 0)}
                        onChange={(event) => {
                          const segment = moveSegmentHandle(
                            "start",
                            Number(event.currentTarget.value),
                            parseTimecode(segmentStartDraft) ?? 0,
                            parseTimecode(segmentEndDraft) ?? playbackDurationMs,
                            playbackDurationMs,
                          );
                          if (!segment) return;
                          setSegmentStartDraft(formatTimecode(segment.startMs));
                          setSegmentEndDraft(formatTimecode(segment.endMs));
                        }}
                      />
                    </label>
                    <label>
                      <span>Segment end handle</span>
                      <input
                        type="range"
                        min={Math.min(playbackDurationMs, (parseTimecode(segmentStartDraft) ?? 0) + 500)}
                        max={playbackDurationMs}
                        value={Math.max(Math.min(playbackDurationMs, (parseTimecode(segmentStartDraft) ?? 0) + 500), Math.min(playbackDurationMs, parseTimecode(segmentEndDraft) ?? playbackDurationMs))}
                        onChange={(event) => {
                          const segment = moveSegmentHandle(
                            "end",
                            Number(event.currentTarget.value),
                            parseTimecode(segmentStartDraft) ?? 0,
                            parseTimecode(segmentEndDraft) ?? playbackDurationMs,
                            playbackDurationMs,
                          );
                          if (!segment) return;
                          setSegmentStartDraft(formatTimecode(segment.startMs));
                          setSegmentEndDraft(formatTimecode(segment.endMs));
                        }}
                      />
                    </label>
                    <label><span>Segment start</span><input value={segmentStartDraft} onChange={(event) => setSegmentStartDraft(event.currentTarget.value)} placeholder="00:00:00.000" /></label>
                    <label><span>Segment end</span><input value={segmentEndDraft} onChange={(event) => setSegmentEndDraft(event.currentTarget.value)} placeholder="00:00:00.000" /></label>
                    <p>Segment must be at least 500 ms and fit within the video duration.</p>
                  </div>
                )}
                <label><span>Speed</span><select value={playbackSettingsDraft.speedPercent} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, speedPercent: Number(event.currentTarget.value) })}>{[25, 50, 75, 100, 125, 150, 200].map((value) => <option key={value} value={value}>{value}%</option>)}</select></label>
                <label><span>Frame rate limit</span><select value={playbackSettingsDraft.fpsLimit} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, fpsLimit: Number(event.currentTarget.value) })}>{[15, 24, 30, 60, 120].map((value) => <option key={value} value={value}>{value} FPS</option>)}</select></label>
                <label><span>Volume ({playbackSettingsDraft.volumePercent}%)</span><input aria-label="Playback volume" type="range" min="0" max="100" step="1" value={playbackSettingsDraft.volumePercent} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, volumePercent: Number(event.currentTarget.value) })} /></label>
                <label className="playback-mute"><input type="checkbox" checked={playbackSettingsDraft.muted} onChange={(event) => setPlaybackSettingsDraft({ ...playbackSettingsDraft, muted: event.currentTarget.checked })} /><span>Mute audio</span></label>
                {!playbackState.audioAvailable && <p>This video has no audio track.</p>}
                {playbackState.audioAvailable && playbackState.audioEffectiveMuted && playbackSettingsDraft.muted && <p>Audio is muted.</p>}
                {playbackState.audioAvailable && playbackState.audioEffectiveMuted && !playbackSettingsDraft.muted && playbackState.speedPercent !== 100 && <p>Audio is muted while video runs at non-1x speed.</p>}
                <button disabled={playbackBusy} onClick={() => void applyPlaybackSettingsDraft()}>Apply playback settings</button>
              </fieldset>
            )}
          </>
        )}
        <p className="playback-message" aria-live="polite" role="status">{playbackMessage}</p>
      </section>
  );
}
