import { useCallback, useEffect, useState } from "react";

import { open } from "@tauri-apps/plugin-dialog";

import { hostAdapter } from "../../host-adapter";
import type { AppInfo } from "../../host-adapter";

const RESTORE_NOTICE_KEY = "wallpaper-lite.restore-notice";

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
  backupBusy: boolean;
  /** The result of the last backup or of a restore that reloaded the page. */
  backupNotice: string | null;
  changeAutostart(enabled: boolean): Promise<void>;
  createBackup(): Promise<void>;
  restoreBackup(): Promise<void>;
  openFolder(folder: "data" | "logs"): Promise<void>;
}

/** `active` is true while the Settings page is shown, so its values are re-read on each visit. */
export function useSettings(active: boolean): SettingsState {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupNotice, setBackupNotice] = useState<string | null>(null);

  useEffect(() => {
    // A restore reloads the page so every view reads the restored files; show its result afterwards.
    // Read it in an effect, and only overwrite with a real value, so a repeated effect run
    // (React StrictMode) cannot discard it.
    const notice = sessionStorage.getItem(RESTORE_NOTICE_KEY);
    if (notice === null) return;
    sessionStorage.removeItem(RESTORE_NOTICE_KEY);
    setBackupNotice(notice);
  }, []);

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

  const createBackup = useCallback(async () => {
    setError(null);
    setBackupNotice(null);
    setBackupBusy(true);
    try {
      const destination = await open({ directory: true, multiple: false, title: "Choose where to save the backup" });
      if (typeof destination !== "string") return;
      const summary = await hostAdapter.createSettingsBackup(destination);
      setBackupNotice(`Backed up ${summary.fileCount} settings files to ${summary.folder}.`);
    } catch (reason) {
      setError(messageOf(reason, "Could not create the backup."));
    } finally {
      setBackupBusy(false);
    }
  }, []);

  const restoreBackup = useCallback(async () => {
    setError(null);
    setBackupNotice(null);
    setBackupBusy(true);
    try {
      const source = await open({ directory: true, multiple: false, title: "Choose a wallpaper-lite backup folder" });
      if (typeof source !== "string") { setBackupBusy(false); return; }
      if (!window.confirm("Restore settings from this backup? Your current settings are kept in the restore-previous folder, and the wallpaper host restarts.")) { setBackupBusy(false); return; }
      const summary = await hostAdapter.restoreSettingsBackup(source);
      sessionStorage.setItem(RESTORE_NOTICE_KEY, summary.hostRestarted
        ? `Restored ${summary.fileCount} settings files.`
        : `Restored ${summary.fileCount} settings files, but the wallpaper host could not be restarted. Quit from the tray and start the app again.`);
      window.location.reload();
    } catch (reason) {
      setError(messageOf(reason, "Could not restore the backup."));
      setBackupBusy(false);
    }
  }, []);

  return { appInfo, autostart, autostartBusy, loadError, error, backupBusy, backupNotice, changeAutostart, createBackup, restoreBackup, openFolder };
}
