import { useCallback, useEffect, useState } from "react";

import { hostAdapter } from "../../host-adapter";
import type { AppInfo } from "../../host-adapter";

function messageOf(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}

export interface SettingsState {
  appInfo: AppInfo | null;
  autostart: boolean | null;
  autostartBusy: boolean;
  /** A failure to read the settings; stays until the next successful read. */
  loadError: string | null;
  /** The outcome of the last change the user made. */
  error: string | null;
  changeAutostart(enabled: boolean): Promise<void>;
  openFolder(folder: "data" | "logs"): Promise<void>;
}

/** `active` is true while the Settings page is shown, so its values are re-read on each visit. */
export function useSettings(active: boolean): SettingsState {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    setLoadError(null);
    hostAdapter.appInfo().then(
      (info) => { if (!cancelled) setAppInfo(info); },
      (reason: unknown) => { if (!cancelled) setLoadError(messageOf(reason, "Could not read the app information.")); },
    );
    hostAdapter.getAutostart().then(
      (enabled) => { if (!cancelled) setAutostart(enabled); },
      (reason: unknown) => {
        if (cancelled) return;
        // Leave the checkbox usable so the user can still try to set it.
        setAutostart((current) => current ?? false);
        setLoadError(messageOf(reason, "Could not read the launch-at-login setting. The checkbox may not match the real setting."));
      },
    );
    return () => { cancelled = true; };
  }, [active]);

  const changeAutostart = useCallback(async (enabled: boolean) => {
    setAutostartBusy(true);
    setError(null);
    try {
      await hostAdapter.setAutostart(enabled);
      setAutostart(enabled);
    } catch (reason) {
      // Keep the checkbox at the real registry state when the change is refused.
      setError(messageOf(reason, "Could not change the launch-at-login setting."));
    } finally {
      setAutostartBusy(false);
    }
  }, []);

  const openFolder = useCallback(async (folder: "data" | "logs") => {
    setError(null);
    try {
      await hostAdapter.openAppFolder(folder);
    } catch (reason) {
      setError(messageOf(reason, "Could not open the folder."));
    }
  }, []);

  return { appInfo, autostart, autostartBusy, loadError, error, changeAutostart, openFolder };
}
