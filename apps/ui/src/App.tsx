import { useEffect, useState } from "react";

import "./styles/tokens.css";
import "./styles/base.css";
import "./App.css";
import type { HostState } from "./dashboard-view.mjs";
import { hostAdapter } from "./host-adapter";
import type { LibraryEntry } from "./host-adapter";
import { PerformancePanels } from "./features/performance/PerformancePanels";
import { usePerformance } from "./features/performance/usePerformance";
import { DisplayPanel } from "./features/displays/DisplayPanel";
import { useDisplays } from "./features/displays/useDisplays";
import { PlaybackPanelView } from "./features/playback/PlaybackPanelView";
import { usePlayback } from "./features/playback/usePlayback";
import { EditorPanel } from "./features/editor/EditorPanel";
import { useEditor } from "./features/editor/useEditor";
import { DashboardPage } from "./features/dashboard/DashboardPage";
import { LibraryPage } from "./features/library/LibraryPage";
import { useLibrary } from "./features/library/useLibrary";
import { SettingsPage } from "./features/settings/SettingsPage";
import type { ThemeChoice } from "./features/settings/SettingsPage";
import { useSettings } from "./features/settings/useSettings";
import { describeError, isRetryable } from "./features/errors/error-message";
import { ErrorNotice } from "./features/errors/ErrorNotice";

type AppPage = "dashboard" | "library" | "editor" | "displays" | "playback" | "performance" | "settings";

const APP_PAGES: { id: AppPage; label: string }[] = [
  { id: "dashboard", label: "Dashboard" },
  { id: "library", label: "Library" },
  { id: "editor", label: "Editor" },
  { id: "displays", label: "Monitors" },
  { id: "playback", label: "Playback" },
  { id: "performance", label: "Performance" },
  { id: "settings", label: "Settings" },
];

function App() {
  const [checkingHost, setCheckingHost] = useState(false);
  const [hostStatus, setHostStatus] = useState("Host connection has not been checked.");
  const [hostState, setHostState] = useState<HostState>("checking");
  const [hostProblem, setHostProblem] = useState<{ message: string; retryable: boolean } | null>(null);
  const displaysState = useDisplays();
  const { displaySnapshot } = displaysState;
  const performanceState = usePerformance(displaySnapshot);
  const playbackState = usePlayback();
  const libraryState = useLibrary();
  const [page, setPage] = useState<AppPage>("dashboard");
  const settingsState = useSettings(page === "settings");
  const { previews, setPreviews, setRecentIds } = libraryState;
  const editorState = useEditor({ previews, setPreviews, setRecentIds, onOpen: () => setPage("editor") });
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

  function hostConnected() {
    setHostState("connected");
    setHostStatus("Wallpaper host connected.");
    setHostProblem(null);
  }

  function hostFailed(error: unknown, fallback: string) {
    setHostState("disconnected");
    setHostStatus("Wallpaper host is not connected.");
    setHostProblem({ message: describeError(error, fallback), retryable: isRetryable(error) });
  }

  useEffect(() => {
    let cancelled = false;
    setHostStatus("Starting wallpaper host...");
    hostAdapter.startHost().then(
      () => { if (!cancelled) hostConnected(); },
      (error: unknown) => { if (!cancelled) hostFailed(error, "Could not start the wallpaper host."); },
    );
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    // The host can stop after startup (a crash, or Explorer restarting), so keep the
    // Dashboard's status honest. This only updates the status; failures are reported
    // when the user acts, so a single slow answer does not raise an alert.
    let stopped = false;
    const timer = window.setInterval(() => {
      if (checkingHost) return;
      hostAdapter.ping().then(
        () => { if (!stopped) setHostState("connected"); },
        () => { if (!stopped) setHostState("disconnected"); },
      );
    }, 10_000);
    return () => { stopped = true; window.clearInterval(timer); };
  }, [checkingHost]);

  async function checkHost() {
    setCheckingHost(true);
    setHostState("checking");
    setHostStatus("Checking host connection...");
    try {
      await hostAdapter.ping();
      hostConnected();
    } catch (error) {
      hostFailed(error, "Could not check the wallpaper host.");
    } finally {
      setCheckingHost(false);
    }
  }

  function openInEditor(entry: LibraryEntry) {
    const { editorDirty, editorEntry } = editorState;
    if (editorDirty && editorEntry && editorEntry.id !== entry.id
      && !window.confirm(`Discard unapplied changes to ${editorEntry.displayName} and open ${entry.displayName}?`)) return;
    void editorState.openEditorPreview(entry);
  }

  async function retryHost() {
    setCheckingHost(true);
    setHostState("checking");
    setHostStatus("Starting wallpaper host...");
    try {
      await hostAdapter.startHost();
      hostConnected();
    } catch (error) {
      hostFailed(error, "Could not start the wallpaper host.");
    } finally {
      setCheckingHost(false);
    }
  }

  return (
    <div className="app-layout">
      <nav className="app-sidebar" aria-label="Main">
        <p className="app-brand">wallpaper Lite</p>
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

      <ErrorNotice
        message={hostProblem?.message ?? null}
        onRetry={hostProblem?.retryable ? () => void retryHost() : undefined}
        retrying={checkingHost}
      />

      <PerformancePanels hidden={page !== "performance"} state={performanceState} />

      <SettingsPage hidden={page !== "settings"} state={settingsState} theme={theme} onThemeChange={setTheme} />

      <DisplayPanel hidden={page !== "displays"} state={displaysState} />

      <PlaybackPanelView hidden={page !== "playback"} state={playbackState} />
      {page === "editor" && !editorState.editorEntry && (<p className="app-empty">Choose a wallpaper in the Library and press Edit.</p>)}
      <EditorPanel hidden={page !== "editor"} state={editorState} previews={previews} />
      <DashboardPage
        hidden={page !== "dashboard"}
        hostState={hostState}
        hostBusy={checkingHost}
        onStartHost={() => void retryHost()}
        playback={playbackState}
        library={libraryState}
        onStartPlayback={(entry) => void playbackState.startPlayback(entry)}
        onOpenEditor={openInEditor}
        onOpenLibrary={() => setPage("library")}
      />
      <LibraryPage hidden={page !== "library"} state={libraryState} onOpenEditor={openInEditor} onStartPlayback={(entry) => void playbackState.startPlayback(entry)} />
    </main>
    </div>
  );
}

export default App;
