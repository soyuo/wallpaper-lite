import { useState } from "react";

import type { ThemeChoice } from "../features/settings/SettingsPage";
import type { LibraryState } from "../features/library/useLibrary";
import type { SettingsState } from "../features/settings/useSettings";

const STEPS = ["Welcome", "Import", "Setup"] as const;
const THEMES: { value: ThemeChoice; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "Follow system" },
];

export function FirstRun({ library, settings, theme, onThemeChange, onFinish }: {
  library: LibraryState;
  settings: SettingsState;
  theme: ThemeChoice;
  onThemeChange(theme: ThemeChoice): void;
  onFinish(): void;
}) {
  const [step, setStep] = useState(0);
  const { importing, importStatus, importSelection, library: entries } = library;
  const { autostart, autostartBusy, changeAutostart } = settings;

  return (
    <div className="first-run" role="dialog" aria-modal="true" aria-labelledby="first-run-title">
      <div className="first-run-card glass-card">
        <ol className="first-run-steps" aria-label="Setup progress">
          {STEPS.map((label, index) => (
            <li key={label} aria-current={index === step ? "step" : undefined} data-done={index < step}>{index + 1}. {label}</li>
          ))}
        </ol>

        {step === 0 && (
          <div className="first-run-body">
            <h2 id="first-run-title">Welcome to wallpaper Lite</h2>
            <p className="dashboard-detail">Use your own images and videos as a live desktop wallpaper. Your original files are never moved, changed or deleted.</p>
          </div>
        )}
        {step === 1 && (
          <div className="first-run-body">
            <h2 id="first-run-title">Add your first wallpaper</h2>
            <p className="dashboard-detail">Choose images or videos now, or skip and add them later from the Library.</p>
            <div className="import-actions">
              <button className="ui-button--primary" disabled={importing} onClick={() => void importSelection(false)}>Choose files</button>
              <button disabled={importing} onClick={() => void importSelection(true)}>Choose folder</button>
            </div>
            <p className="dashboard-detail" role="status" aria-live="polite">
              {importing ? "Validating selected media..." : importStatus || (entries.length > 0 ? `${entries.length} in your library.` : "")}
            </p>
          </div>
        )}
        {step === 2 && (
          <div className="first-run-body">
            <h2 id="first-run-title">Make it yours</h2>
            <label className="settings-row">
              <input type="checkbox" checked={autostart ?? false} disabled={autostart === null || autostartBusy} onChange={(event) => void changeAutostart(event.target.checked)} />
              <span>Launch at login<small>Starts hidden in the tray and restores your wallpaper.</small></span>
            </label>
            <div className="seg" role="group" aria-label="Theme">
              {THEMES.map((option) => (
                <button key={option.value} type="button" aria-pressed={theme === option.value} className={theme === option.value ? "on" : undefined} onClick={() => onThemeChange(option.value)}>{option.label}</button>
              ))}
            </div>
          </div>
        )}

        <div className="first-run-actions">
          {step === 0 ? <button onClick={onFinish}>Skip setup</button> : <button onClick={() => setStep(step - 1)}>Back</button>}
          {step < STEPS.length - 1
            ? <button className="ui-button--primary" onClick={() => setStep(step + 1)}>{step === 1 && entries.length === 0 ? "Skip" : "Next"}</button>
            : <button className="ui-button--primary" onClick={onFinish}>Finish</button>}
        </div>
      </div>
    </div>
  );
}
