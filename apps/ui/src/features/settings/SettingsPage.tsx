import { ErrorNotice } from "../errors/ErrorNotice";
import type { SettingsState } from "./useSettings";

export type ThemeChoice = "dark" | "light" | "system";

const THEME_OPTIONS: { value: ThemeChoice; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "Follow system" },
];

export function SettingsPage({ hidden, state, theme, onThemeChange }: {
  hidden: boolean;
  state: SettingsState;
  theme: ThemeChoice;
  onThemeChange(theme: ThemeChoice): void;
}) {
  const { appInfo, autostart, autostartBusy, loadError, error, backupBusy, backupNotice, changeAutostart, createBackup, restoreBackup, openFolder } = state;

  return (
    <section className="settings-page" hidden={hidden} aria-labelledby="settings-startup-heading">
      <ErrorNotice message={loadError} />
      <ErrorNotice message={error} />

      <div className="settings-group">
        <h2 id="settings-startup-heading">Startup</h2>
        <label className="settings-row">
          <input
            type="checkbox"
            checked={autostart ?? false}
            disabled={autostart === null || autostartBusy}
            onChange={(event) => void changeAutostart(event.target.checked)}
          />
          <span>
            Launch at login
            <small>Starts hidden in the tray and restores your wallpaper.</small>
          </span>
        </label>
      </div>

      <fieldset className="settings-group">
        <legend>Appearance</legend>
        {THEME_OPTIONS.map((option) => (
          <label className="settings-row" key={option.value}>
            <input
              type="radio"
              name="theme"
              value={option.value}
              checked={theme === option.value}
              onChange={() => onThemeChange(option.value)}
            />
            <span>{option.label}</span>
          </label>
        ))}
      </fieldset>

      <div className="settings-group">
        <h2>Backup</h2>
        <p className="settings-help">
          Saves your library, editor, monitor and performance settings to a folder you choose.
          Your original images and videos are not copied or changed.
        </p>
        <div className="settings-actions">
          <button disabled={backupBusy} onClick={() => void createBackup()}>Create backup…</button>
          <button disabled={backupBusy} onClick={() => void restoreBackup()}>Restore from backup…</button>
        </div>
        {backupNotice && <p className="settings-notice" role="status">{backupNotice}</p>}
      </div>

      <div className="settings-group">
        <h2>About</h2>
        <dl className="settings-facts">
          <dt>Version</dt>
          <dd>{appInfo?.version ?? "…"}</dd>
          <dt>Settings folder</dt>
          <dd>{appInfo?.dataDirectory ?? "Unavailable"}</dd>
          <dt>Log folder</dt>
          <dd>{appInfo?.logDirectory ?? "Unavailable"}</dd>
        </dl>
        <div className="settings-actions">
          <button disabled={!appInfo?.dataDirectory} onClick={() => void openFolder("data")}>Open settings folder</button>
          <button disabled={!appInfo?.logDirectory} onClick={() => void openFolder("logs")}>Open log folder</button>
        </div>
      </div>
    </section>
  );
}
