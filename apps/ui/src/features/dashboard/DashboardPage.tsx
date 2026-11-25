import { useEffect } from "react";

import { friendlyDisplayName } from "../../format";
import { canPlayFromDashboard, hostSummary, nowPlayingSummary, toggleControl, toggleLabel } from "../../dashboard-view.mjs";
import type { HostState } from "../../dashboard-view.mjs";
import type { DisplayManagerSnapshot, LibraryEntry } from "../../host-adapter";
import type { LibraryState } from "../library/useLibrary";
import type { PlaybackState } from "../playback/usePlayback";

export function DashboardPage({ hidden, hostState, hostBusy, onStartHost, playback, library, displaySnapshot, onStartPlayback, onOpenEditor, onOpenLibrary }: {
  hidden: boolean;
  hostState: HostState;
  hostBusy: boolean;
  onStartHost(): void;
  playback: PlaybackState;
  library: LibraryState;
  displaySnapshot: DisplayManagerSnapshot | null;
  onStartPlayback(entry: LibraryEntry): void;
  onOpenEditor(entry: LibraryEntry): void;
  onOpenLibrary(): void;
}) {
  const { playbackState, playbackBusy, playbackMessage, sendPlaybackControl, playbackDurationMs, playbackPositionMs } = playback;
  const progress = playbackDurationMs > 0 ? Math.min(100, (playbackPositionMs / playbackDurationMs) * 100) : 0;
  const host = hostSummary(hostState);
  const playingEntry = playbackState?.wallpaperId == null
    ? null
    : library.library.find((entry) => entry.id === playbackState.wallpaperId) ?? null;
  const nowPlaying = nowPlayingSummary(playbackState, playingEntry);
  const canToggle = nowPlaying.active && !playbackBusy && !playbackState?.loadingVideo && hostState === "connected";

  const heroPreview = playingEntry ? library.previews[playingEntry.id] : undefined;

  // Previews are normally requested by the Library page; Home needs its own so
  // thumbnails show on a fresh start without visiting the Library first.
  const { loadPreview, recentLibrary } = library;
  const playingId = playingEntry?.id;
  const recentKey = recentLibrary.slice(0, 3).map((entry) => entry.id).join(",");
  useEffect(() => {
    const ids = recentKey ? recentKey.split(",").map(Number) : [];
    if (playingId !== undefined) ids.push(playingId);
    for (const id of ids) void loadPreview(id);
  }, [loadPreview, playingId, recentKey]);
  const stateLabel = playbackState?.state === "playing" ? "Playing" : playbackState?.state === "paused" ? "Paused" : "Idle";

  const missing = library.library.find((entry) => entry.availability === "file_missing") ?? null;
  const hostDown = hostState !== "connected";

  return (
    <section className="dashboard home" hidden={hidden} aria-label="Dashboard">
      {missing && !hostDown && (
        <p className="home-banner" role="alert">
          <span>“{missing.displayName}” can’t be found. Its file was moved or deleted.</span>
          <button onClick={onOpenLibrary}>Reconnect file</button>
        </p>
      )}
      {playbackState?.lastError && !hostDown && (
        <p className="home-banner" role="alert">
          <span>Couldn’t start playback. {playbackState.lastError}</span>
          {playingEntry && <button disabled={playbackBusy} onClick={() => onStartPlayback(playingEntry)}>Try again</button>}
          {playingEntry && <button onClick={() => onOpenEditor(playingEntry)}>Edit</button>}
        </p>
      )}
      {hostDown ? (
      <section className="home-hero glass-card home-state" data-host={hostBusy || hostState === "checking" ? "starting" : "stopped"} aria-labelledby="dashboard-playing-heading">
        <div className="home-hero-body">
          <span className="chip" data-state="disconnected"><span className="dot" />{hostBusy || hostState === "checking" ? "Starting" : "Stopped"}</span>
          <h2 id="dashboard-playing-heading" className="home-title">{hostBusy || hostState === "checking" ? "Starting the host…" : "The host is stopped"}</h2>
          <p className="dashboard-detail">{hostBusy || hostState === "checking" ? "This only takes a moment." : "Start the wallpaper host to show your wallpaper on the desktop."}</p>
          {(hostBusy || hostState === "checking") && <div className="dashboard-progress home-indeterminate" aria-hidden="true"><span /></div>}
          <div className="home-actions">
            <button className="ui-button--primary" disabled={hostBusy || hostState === "checking"} onClick={onStartHost}>{hostBusy ? "Starting…" : "Start host"}</button>
          </div>
        </div>
      </section>
      ) : (
      <section className="home-hero glass-card" aria-labelledby="dashboard-playing-heading">
        {heroPreview && <img className="home-hero-art" alt="" aria-hidden="true" src={heroPreview.thumbnailDataUrl} />}
        <div className="home-hero-body">
          <span className="chip" data-state={playbackState?.state ?? "stopped"}><span className="dot" />{stateLabel}</span>
          <h2 id="dashboard-playing-heading" className="home-eyebrow">Now playing</h2>
          <p className="home-title">{nowPlaying.title}</p>
          <p className="dashboard-detail">{nowPlaying.detail}</p>
          {nowPlaying.active && <div className="dashboard-progress" role="progressbar" aria-label="Playback position" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress)}><span style={{ width: `${progress}%` }} /></div>}
          <div className="home-actions">
            {nowPlaying.active ? (
              <button className="ui-button--primary" disabled={!canToggle} onClick={() => void sendPlaybackControl(toggleControl(playbackState?.state ?? "stopped"))}>
                {toggleLabel(playbackState?.state ?? "stopped")}
              </button>
            ) : (
              <button className="ui-button--primary" onClick={onOpenLibrary}>Open Library</button>
            )}
            {playingEntry && <button onClick={() => onOpenEditor(playingEntry)}>Edit</button>}
          </div>
          {playbackMessage && <p className="dashboard-detail" role="status">{playbackMessage}</p>}
        </div>
      </section>
      )}

      <aside className="home-aside glass-card" aria-labelledby="dashboard-host-heading">
        <h2 id="dashboard-host-heading" className="home-eyebrow">Wallpaper host</h2>
        <p className="chip dashboard-chip" data-state={hostState}><span className="dot" />{host.label}</p>
        <p className="dashboard-detail">{host.detail}</p>
        {(hostState === "disconnected" || hostBusy) && (
          <button className="ui-button--primary" disabled={hostBusy} onClick={onStartHost}>
            {hostBusy ? "Starting..." : "Start wallpaper host"}
          </button>
        )}
        {displaySnapshot && displaySnapshot.displays.length > 0 && (
          <>
            <h3 className="home-eyebrow">Displays</h3>
            <ul className="home-displays">
              {displaySnapshot.displays.map((display) => (
                <li key={display.id}><span>{friendlyDisplayName(display.name)}{display.isPrimary ? " · Primary" : ""}</span><span>{display.width}×{display.height}</span></li>
              ))}
            </ul>
          </>
        )}
      </aside>

      <section className="home-recent" aria-labelledby="dashboard-recent-heading">
        <h2 id="dashboard-recent-heading" className="home-eyebrow">Recently used</h2>
        {library.recentLibrary.length === 0 ? (
          <div className="state-card glass-card">
            <p className="state-title">Nothing used recently</p>
            <p className="dashboard-detail">Add images or videos in the Library to get started.</p>
            <button className="ui-button--primary" onClick={onOpenLibrary}>Open Library</button>
          </div>
        ) : (
          <ul className="home-tiles">
            {library.recentLibrary.slice(0, 3).map((entry) => {
              const preview = library.previews[entry.id];
              return (
                <li key={entry.id} className="home-tile">
                  {preview ? <img alt="" src={preview.thumbnailDataUrl} /> : <span className="home-tile-empty" aria-hidden="true" />}
                  <div className="home-tile-cap">
                    <span className="dashboard-entry-name">{entry.displayName}</span>
                    <span className="home-tile-actions">
                      {canPlayFromDashboard(entry) && (
                        <button aria-label={`Play ${entry.displayName}`} disabled={playbackBusy || playbackState?.loadingVideo === true || hostState !== "connected"} onClick={() => onStartPlayback(entry)}>Play</button>
                      )}
                      <button aria-label={`Edit ${entry.displayName}`} onClick={() => onOpenEditor(entry)}>Edit</button>
                    </span>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </section>
  );
}
