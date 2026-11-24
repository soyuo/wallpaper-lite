// Display rules for the Dashboard page, kept free of React so they can be tested.

/** The command a Play/Pause button sends for the current playback state. */
export function toggleControl(state) {
  return state === "playing" ? "pause" : "play";
}

/** The label of the Play/Pause button. */
export function toggleLabel(state) {
  return state === "playing" ? "Pause" : "Play";
}

/** Short, plain-language status for the host card. */
export function hostSummary(hostState) {
  switch (hostState) {
    case "connected":
      return { label: "Running", detail: "The wallpaper host is running and answering." };
    case "disconnected":
      return { label: "Not running", detail: "The wallpaper host is not answering, so your wallpaper is not shown." };
    default:
      return { label: "Checking…", detail: "Checking the wallpaper host." };
  }
}

/**
 * What the "now playing" card should say.
 * `entry` is the library entry for the playing wallpaper, or null if it is not in the library.
 */
export function nowPlayingSummary(playback, entry) {
  if (!playback || playback.wallpaperId == null) {
    return { active: false, title: "Nothing is playing", detail: "Pick a video in the Library to play it on your desktop." };
  }
  const title = entry ? entry.displayName : "A wallpaper that is no longer in the Library";
  if (playback.loadingVideo) {
    return { active: true, title, detail: "Loading…" };
  }
  const stateLabel = { playing: "Playing", paused: "Paused", stopped: "Stopped", ended: "Finished" }[playback.state] ?? "Stopped";
  return { active: true, title, detail: `${stateLabel} · ${playback.speedPercent}% speed · ${playback.fpsLimit} FPS limit` };
}

/** Whether a library entry can be started from the Dashboard (only available videos can). */
export function canPlayFromDashboard(entry) {
  return entry.mediaKind === "video" && entry.availability === "available";
}
