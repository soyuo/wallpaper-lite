import { ErrorNotice } from "../errors/ErrorNotice";
import type { SettingsState } from "./useSettings";

export type ThemeChoice = "dark" | "light" | "system";

const THEME_OPTIONS: { value: ThemeChoice; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

export function SettingsPage({ hidden, state, theme, onThemeChange }: {
  hidden: boolean;
  state: SettingsState;
  theme: ThemeChoice;
  onThemeChange(theme: ThemeChoice): void;
}) {
  const { appInfo, autostart, autostartBusy, loadError, error, backupBusy, backupNotice, changeAutostart, createBackup, chooseRestore, cancelRestore, restoreBackup, pendingRestore, openFolder } = state;

  return (
    <section className="settings-page" hidden={hidden} aria-label="Settings">
      <ErrorNotice message={loadError} />
      <ErrorNotice message={error} />

      <div className="glass-card settings-card">
        <label className="settings-line">
          <span className="settings-name">
            Launch at login
            <small>Starts hidden in the tray and restores your wallpaper.</small>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={autostart ?? false}
            disabled={autostart === null || autostartBusy}
            onChange={(event) => void changeAutostart(event.target.checked)}
          />
        </label>
        <hr className="settings-hr" />
        <div className="settings-line" role="group" aria-labelledby="appearance-title">
          <span className="settings-name" id="appearance-title">Theme</span>
          <div className="seg" role="group" aria-label="Theme">
            {THEME_OPTIONS.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={theme === option.value}
                className={theme === option.value ? "on" : undefined}
                onClick={() => onThemeChange(option.value)}
              >{option.label}</button>
            ))}
          </div>
        </div>
        <hr className="settings-hr" />
        <div className="settings-line">
          <span className="settings-name">App version</span>
          <span className="settings-mono">{appInfo?.version ?? "…"}</span>
        </div>
      </div>

      <div className="glass-card settings-card">
        <div className="settings-line">
          <span className="settings-name">
            Settings folder
            <small className="settings-mono">{appInfo?.dataDirectory ?? "Unavailable"}</small>
          </span>
          <button disabled={!appInfo?.dataDirectory} onClick={() => void openFolder("data")}>Open</button>
        </div>
        <hr className="settings-hr" />
        <div className="settings-line">
          <span className="settings-name">
            Log folder
            <small className="settings-mono">{appInfo?.logDirectory ?? "Unavailable"}</small>
          </span>
          <button disabled={!appInfo?.logDirectory} onClick={() => void openFolder("logs")}>Open</button>
        </div>
      </div>

      <div className="glass-card settings-card settings-backup">
        <span className="settings-name">
          Backup
          <small>Saves your library, editor, monitor and performance settings. Original images and videos are not copied or changed.</small>
        </span>
        <div className="settings-actions">
          <button disabled={backupBusy} onClick={() => void chooseRestore()}>Restore from backup…</button>
          <button className="ui-button--primary" disabled={backupBusy} onClick={() => void createBackup()}>Create backup…</button>
        </div>
        {pendingRestore !== null && (
          <div className="restore-confirm" role="alertdialog" aria-labelledby="restore-title">
            <h3 id="restore-title">Restore from this folder?</h3>
            <p className="restore-path">{pendingRestore}</p>
            <p>Your current settings are kept in the restore-previous folder, and the wallpaper host restarts. If the folder is not a valid backup, nothing is changed.</p>
            <div className="settings-actions">
              <button disabled={backupBusy} onClick={cancelRestore}>Cancel</button>
              <button className="ui-button--primary" disabled={backupBusy} onClick={() => void restoreBackup()}>{backupBusy ? "Restoring…" : "Restore"}</button>
            </div>
          </div>
        )}
        {backupNotice && <p className="settings-notice" role="status">{backupNotice}</p>}
      </div>
    </section>
  );
}
