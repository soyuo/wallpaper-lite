import { useEffect, useState } from "react";

import "./styles/tokens.css";
import "./styles/base.css";
import "./App.css";
import { hostAdapter } from "./host-adapter";
import { PerformancePanels } from "./features/performance/PerformancePanels";
import { usePerformance } from "./features/performance/usePerformance";
import { DisplayPanel } from "./features/displays/DisplayPanel";
import { useDisplays } from "./features/displays/useDisplays";
import { PlaybackPanelView } from "./features/playback/PlaybackPanelView";
import { usePlayback } from "./features/playback/usePlayback";
import { EditorPanel } from "./features/editor/EditorPanel";
import { useEditor } from "./features/editor/useEditor";
import { LibraryPage } from "./features/library/LibraryPage";
import { useLibrary } from "./features/library/useLibrary";

type ThemeChoice = "dark" | "light" | "system";
const THEME_LABELS: Record<ThemeChoice, string> = { dark: "Theme: Dark", light: "Theme: Light", system: "Theme: System" };
const NEXT_THEME: Record<ThemeChoice, ThemeChoice> = { dark: "light", light: "system", system: "dark" };

type AppPage = "library" | "editor" | "displays" | "playback" | "performance";

const APP_PAGES: { id: AppPage; label: string }[] = [
  { id: "library", label: "Library" },
  { id: "editor", label: "Editor" },
  { id: "displays", label: "Monitors" },
  { id: "playback", label: "Playback" },
  { id: "performance", label: "Performance" },
];

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");
  const displaysState = useDisplays();
  const { displaySnapshot } = displaysState;
  const performanceState = usePerformance(displaySnapshot);
  const playbackState = usePlayback();
  const libraryState = useLibrary();
  const { previews, setPreviews, setRecentIds } = libraryState;
  const editorState = useEditor({ previews, setPreviews, setRecentIds, onOpen: () => setPage("editor") });
  const [page, setPage] = useState<AppPage>("library");
  const [theme, setTheme] = useState<ThemeChoice>(() => {
    const stored = localStorage.getItem("wallpaper-lite.theme");
    return stored === "light" || stored === "system" ? stored : "dark";
  });

  useEffect(() => {
    localStorage.setItem("wallpaper-lite.theme", theme);
    if (theme !== "system") {
      document.documentElement.dataset.theme = theme;
      return;
    }
    const query = window.matchMedia("(prefers-color-scheme: light)");
    const apply = () => { document.documentElement.dataset.theme = query.matches ? "light" : "dark"; };
    apply();
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [theme]);

  const activeWallpaperId = playbackState.playbackPanel?.state.wallpaperId ?? null;
  const activeWallpaper = activeWallpaperId === null
    ? null
    : libraryState.library.find((entry) => entry.id === activeWallpaperId) ?? null;

  useEffect(() => {
    let cancelled = false;
    setHostStatus("Starting wallpaper host...");
    hostAdapter.startHost().then(
      () => { if (!cancelled) setHostStatus("Wallpaper host connected."); },
      (error: unknown) => {
        if (!cancelled) setHostStatus(error instanceof Error ? error.message : "Could not start the wallpaper host.");
      },
    );
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    let cancelled = false;
    hostAdapter.getAutostart().then(
      (enabled) => { if (!cancelled) setAutostart(enabled); },
      (error: unknown) => {
        if (cancelled) return;
        // Leave the checkbox usable so the user can still try to set it.
        setAutostart(false);
        setAutostartError(error instanceof Error ? error.message : "Could not read the launch-at-login setting.");
      },
    );
    return () => { cancelled = true; };
  }, []);

  async function changeAutostart(enabled: boolean) {
    setAutostartBusy(true);
    setAutostartError(null);
    try {
      await hostAdapter.setAutostart(enabled);
      setAutostart(enabled);
    } catch (error) {
      // Keep the checkbox at the real registry state when the change is refused.
      setAutostartError(error instanceof Error ? error.message : "Could not change the launch-at-login setting.");
    } finally {
      setAutostartBusy(false);
    }
  }

  async function checkHost() {
    setCheckingHost(true);
    setHostStatus("Checking host connection...");
    try {
      await hostAdapter.ping();
      setHostStatus("Wallpaper host connected.");
    } catch (error) {
      setHostStatus(error instanceof Error ? error.message : "Could not check the wallpaper host.");
    } finally {
      setCheckingHost(false);
    }
  }

  return (
    <div className="app-layout">
      <nav className="app-sidebar" aria-label="Main">
        <p className="app-brand">wallpaper-lite</p>
        <ul>
          {APP_PAGES.map((item) => (
            <li key={item.id}>
              <button className="app-nav-item" aria-current={page === item.id ? "page" : undefined} onClick={() => setPage(item.id)}>
                {item.label}
                {item.id === "editor" && editorState.editorDirty && <span className="app-nav-dirty" role="img" aria-label="Unapplied changes"> •</span>}
              </button>
            </li>
          ))}
        </ul>
        <div className="app-sidebar-footer">
          <label className="app-autostart">
            <input
              type="checkbox"
              checked={autostart ?? false}
              disabled={autostart === null || autostartBusy}
              onChange={(event) => void changeAutostart(event.target.checked)}
            />
            Launch at login
          </label>
          {autostartError && <p className="app-autostart-error" role="alert">{autostartError}</p>}
          <button className="app-theme-toggle" onClick={() => setTheme(NEXT_THEME[theme])}>
            {THEME_LABELS[theme]}
          </button>
        </div>
      </nav>
    <main className="app-shell">
      <header className="app-header">
        <div>
          <h1>{APP_PAGES.find((item) => item.id === page)?.label}</h1>
          <p>{activeWallpaper ? `Now playing: ${activeWallpaper.displayName}` : "No video wallpaper is playing"}</p>
        </div>
        <div className="app-header-actions">
          <p aria-live="polite" className="host-status" role="status">{hostStatus}</p>
          <button className="ui-button--primary" disabled={checkingHost} onClick={checkHost}>
            {checkingHost ? "Checking..." : "Check wallpaper host"}
          </button>
        </div>
      </header>

      <PerformancePanels hidden={page !== "performance"} state={performanceState} />

      <DisplayPanel hidden={page !== "displays"} state={displaysState} />

      <PlaybackPanelView hidden={page !== "playback"} state={playbackState} />
      {page === "editor" && !editorState.editorEntry && (<p className="app-empty">Choose a wallpaper in the Library and press Edit.</p>)}
      <EditorPanel hidden={page !== "editor"} state={editorState} previews={previews} />
      <LibraryPage hidden={page !== "library"} state={libraryState} onOpenEditor={(entry) => {
        const { editorDirty, editorEntry } = editorState;
        if (editorDirty && editorEntry && editorEntry.id !== entry.id
          && !window.confirm(`Discard unapplied changes to ${editorEntry.displayName} and open ${entry.displayName}?`)) return;
        void editorState.openEditorPreview(entry);
      }} onStartPlayback={(entry) => void playbackState.startPlayback(entry)} />
    </main>
    </div>
  );
}

export default App;
