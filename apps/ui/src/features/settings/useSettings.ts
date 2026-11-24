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
  error: string | null;
  changeAutostart(enabled: boolean): Promise<void>;
  openFolder(folder: "data" | "logs"): Promise<void>;
}

export function useSettings(): SettingsState {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    hostAdapter.appInfo().then(
      (info) => { if (!cancelled) setAppInfo(info); },
      (reason: unknown) => { if (!cancelled) setError(messageOf(reason, "Could not read the app information.")); },
    );
    hostAdapter.getAutostart().then(
      (enabled) => { if (!cancelled) setAutostart(enabled); },
      (reason: unknown) => {
        if (cancelled) return;
        // Leave the checkbox usable so the user can still try to set it.
        setAutostart(false);
        setError(messageOf(reason, "Could not read the launch-at-login setting."));
      },
    );
    return () => { cancelled = true; };
  }, []);

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

  return { appInfo, autostart, autostartBusy, error, changeAutostart, openFolder };
}
