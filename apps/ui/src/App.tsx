import { useEffect, useState } from "react";

import { Backdrop } from "./components/Backdrop";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/shell.css";
import "./styles/pages.css";
import "./App.css";
import { APP_PAGES } from "./components/app-pages";
import type { AppPage } from "./components/app-pages";
import { AppDock } from "./components/AppDock";
import { AppTopBar } from "./components/AppTopBar";
import { FirstRun } from "./components/FirstRun";
import { Splash } from "./components/Splash";
import type { HostState } from "./dashboard-view.mjs";
import { hostAdapter } from "./host-adapter";
import type { LibraryEntry } from "./host-adapter";
import { PerformancePage } from "./features/performance/PerformancePage";
import { usePerformance } from "./features/performance/usePerformance";
import { DisplaysPage } from "./features/displays/DisplaysPage";
import { useDisplays } from "./features/displays/useDisplays";
import { PlaybackPage } from "./features/playback/PlaybackPage";
import { usePlayback } from "./features/playback/usePlayback";
import { EditorPage } from "./features/editor/EditorPage";
import { useEditor } from "./features/editor/useEditor";
import { DashboardPage } from "./features/dashboard/DashboardPage";
import { LibraryPage } from "./features/library/LibraryPage";
import { useLibrary } from "./features/library/useLibrary";
import { SettingsPage } from "./features/settings/SettingsPage";
import type { ThemeChoice } from "./features/settings/SettingsPage";
import { useSettings } from "./features/settings/useSettings";
import { describeError, isRetryable } from "./features/errors/error-message";
import { ErrorNotice } from "./features/errors/ErrorNotice";

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
  const [splashOpen, setSplashOpen] = useState(true);
  const [firstRunOpen, setFirstRunOpen] = useState(() => localStorage.getItem("wallpaper-lite.first-run-done") === null);
  const settingsState = useSettings(page === "settings" || firstRunOpen);
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
    <div className="app-shell">
      <Backdrop />
      <AppTopBar hostState={hostState} hostStatus={hostStatus} checking={checkingHost} onCheck={checkHost} />
    <main className="app-main">
      <header className="app-header">
        <h1>{APP_PAGES.find((item) => item.id === page)?.label}</h1>
        <p>{activeWallpaper ? `Now playing: ${activeWallpaper.displayName}` : "No video wallpaper is playing"}</p>
      </header>

      <ErrorNotice
        message={hostProblem?.message ?? null}
        onRetry={hostProblem?.retryable ? () => void retryHost() : undefined}
        retrying={checkingHost}
      />

      <PerformancePage hidden={page !== "performance"} state={performanceState} />

      <SettingsPage hidden={page !== "settings"} state={settingsState} theme={theme} onThemeChange={setTheme} />

      <DisplaysPage hidden={page !== "displays"} state={displaysState} />

      <PlaybackPage hidden={page !== "playback"} state={playbackState} onOpenLibrary={() => setPage("library")} />
      <EditorPage hidden={page !== "editor"} state={editorState} previews={previews} />
      <DashboardPage
        hidden={page !== "dashboard"}
        hostState={hostState}
        hostBusy={checkingHost}
        onStartHost={() => void retryHost()}
        playback={playbackState}
        library={libraryState}
        displaySnapshot={displaySnapshot}
        onStartPlayback={(entry) => void playbackState.startPlayback(entry)}
        onOpenEditor={openInEditor}
        onOpenLibrary={() => setPage("library")}
      />
      <LibraryPage hidden={page !== "library"} state={libraryState} onOpenEditor={openInEditor} onStartPlayback={(entry) => void playbackState.startPlayback(entry)} />
    </main>
      {splashOpen && <Splash ready={hostState !== "checking"} onDone={() => setSplashOpen(false)} />}
      {!splashOpen && firstRunOpen && (
        <FirstRun
          library={libraryState}
          settings={settingsState}
          theme={theme}
          onThemeChange={setTheme}
          onFinish={() => { localStorage.setItem("wallpaper-lite.first-run-done", "1"); setFirstRunOpen(false); }}
        />
      )}
      <AppDock page={page} editorDirty={editorState.editorDirty} onNavigate={setPage} />
    </div>
  );
}

export default App;
