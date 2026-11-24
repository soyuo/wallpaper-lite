import { canPlayFromDashboard, hostSummary, nowPlayingSummary, toggleControl, toggleLabel } from "../../dashboard-view.mjs";
import type { HostState } from "../../dashboard-view.mjs";
import type { LibraryEntry } from "../../host-adapter";
import type { LibraryState } from "../library/useLibrary";
import type { PlaybackState } from "../playback/usePlayback";

export function DashboardPage({ hidden, hostState, hostBusy, onStartHost, playback, library, onStartPlayback, onOpenEditor, onOpenLibrary }: {
  hidden: boolean;
  hostState: HostState;
  hostBusy: boolean;
  onStartHost(): void;
  playback: PlaybackState;
  library: LibraryState;
  onStartPlayback(entry: LibraryEntry): void;
  onOpenEditor(entry: LibraryEntry): void;
  onOpenLibrary(): void;
}) {
  const { playbackState, playbackBusy, playbackMessage, sendPlaybackControl } = playback;
  const host = hostSummary(hostState);
  const playingEntry = playbackState?.wallpaperId == null
    ? null
    : library.library.find((entry) => entry.id === playbackState.wallpaperId) ?? null;
  const nowPlaying = nowPlayingSummary(playbackState, playingEntry);
  const canToggle = nowPlaying.active && !playbackBusy && !playbackState?.loadingVideo && hostState === "connected";

  return (
    <section className="dashboard" hidden={hidden} aria-labelledby="dashboard-host-heading">
      <div className="dashboard-card">
        <h2 id="dashboard-host-heading">Wallpaper host</h2>
        <p className="dashboard-headline" data-state={hostState}>{host.label}</p>
        <p className="dashboard-detail">{host.detail}</p>
        {hostState === "disconnected" && (
          <button className="ui-button--primary" disabled={hostBusy} onClick={onStartHost}>
            {hostBusy ? "Starting..." : "Start wallpaper host"}
          </button>
        )}
      </div>

      <div className="dashboard-card">
        <h2>Now playing</h2>
        <p className="dashboard-headline">{nowPlaying.title}</p>
        <p className="dashboard-detail">{nowPlaying.detail}</p>
        {nowPlaying.active ? (
          <button
            className="ui-button--primary"
            disabled={!canToggle}
            onClick={() => void sendPlaybackControl(toggleControl(playbackState?.state ?? "stopped"))}
          >
            {toggleLabel(playbackState?.state ?? "stopped")}
          </button>
        ) : (
          <button onClick={onOpenLibrary}>Open Library</button>
        )}
        {playbackState?.lastError && <p className="dashboard-detail" role="status">Playback error: {playbackState.lastError}</p>}
        {playbackMessage && <p className="dashboard-detail" role="status">{playbackMessage}</p>}
      </div>

      <div className="dashboard-card dashboard-recent">
        <h2>Recently used</h2>
        {library.recentLibrary.length === 0 ? (
          <>
            <p className="dashboard-detail">Nothing used recently. Add images or videos in the Library to get started.</p>
            <button onClick={onOpenLibrary}>Open Library</button>
          </>
        ) : (
          <ul className="dashboard-list">
            {library.recentLibrary.map((entry) => (
              <li key={entry.id}>
                <span className="dashboard-entry-name">{entry.displayName}</span>
                <span className="dashboard-entry-kind">{entry.mediaKind}</span>
                <span className="dashboard-entry-actions">
                  {canPlayFromDashboard(entry) && (
                    <button disabled={playbackBusy} onClick={() => onStartPlayback(entry)}>Play</button>
                  )}
                  <button onClick={() => onOpenEditor(entry)}>Edit</button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
