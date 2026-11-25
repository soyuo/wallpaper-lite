import { NumberField } from "../../components/NumberField";
import { friendlyDisplayName } from "../../format";
import { useState } from "react";

import { makeDisplayEdits } from "../../display-manager.mjs";
import type { DisplaysState } from "./useDisplays";
import type { DisplayPresentationMode } from "../../host-adapter";

export function DisplaysPage({ hidden, state }: { hidden: boolean; state: DisplaysState }) {
  const {
    displaySnapshot, displayMode, displayEdits, displayStatus, displayLoading, displaySaving,
    setDisplayMode, setDisplayEdits, refreshDisplayManager, saveDisplayAssignments,
  } = state;
  const saved = displaySnapshot ? makeDisplayEdits(displaySnapshot) : null;
  const dirty = saved !== null && (saved.mode !== displayMode || JSON.stringify(saved.edits) !== JSON.stringify(displayEdits));
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const MODES: { value: DisplayPresentationMode; label: string }[] = [
    { value: "independent", label: "Independent" },
    { value: "duplicate", label: "Duplicate" },
    { value: "span", label: "Span" },
  ];

  return (
      <section className="ui-panel display-manager" hidden={hidden} aria-labelledby="display-manager-title">
        <div className="display-manager-heading">
          <div>
            <h2 id="display-manager-title">Displays</h2>
            <p>Assign wallpapers and per-display frame-rate limits.</p>
          </div>
          <div className="display-toolbar">
            <div className="seg" role="group" aria-label="Presentation mode">
              {MODES.map((mode) => (
                <button key={mode.value} aria-pressed={displayMode === mode.value} className={displayMode === mode.value ? "on" : undefined} disabled={displaySaving || !displaySnapshot} onClick={() => setDisplayMode(mode.value)}>{mode.label}</button>
              ))}
            </div>
            <button disabled={displayLoading || displaySaving} onClick={() => void refreshDisplayManager()}>
              {displayLoading ? "Refreshing..." : "Refresh displays"}
            </button>
          </div>
        </div>
        {displaySnapshot ? (
          <>
            <div className="display-map" role="group" aria-label="Monitor map">
              {displaySnapshot.displays.map((display, index) => (
                <button
                  key={display.id}
                  className="display-tile"
                  aria-pressed={(selectedId ?? displaySnapshot.displays[0]?.id) === display.id}
                  style={{ flexGrow: display.width }}
                  onClick={() => setSelectedId(display.id)}
                >
                  <span className="display-tile-index">{index + 1}</span>
                  {saved && JSON.stringify(saved.edits[display.id]) !== JSON.stringify(displayEdits[display.id]) && <span className="chip display-tile-badge">Changed</span>}
                  <span className="display-tile-caption"><span>{friendlyDisplayName(display.name)}</span><span>{display.width}×{display.height}</span></span>
                </button>
              ))}
            </div>
            <div className="display-manager-list">
              {displaySnapshot.displays.filter((display) => display.id === (selectedId ?? displaySnapshot.displays[0]?.id)).map((display) => {
                const edit = displayEdits[display.id] ?? { wallpaperId: "", fpsOverride: "" };
                const savedProfile = displaySnapshot.profiles.find((profile) => profile.displayId === display.id);
                const assignedWallpaper = savedProfile
                  ? displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === savedProfile.wallpaperId)
                  : null;
                const showWallpaper = displayMode === "independent" || display.isPrimary;
                return (
                  <article className="display-manager-card" key={display.id}>
                    <h3>{friendlyDisplayName(display.name)}{display.isPrimary ? " · Primary" : ""}</h3>
                    <code className="display-stable-id" title={display.id}>Stable ID: {display.id}</code>
                    <p>{display.deviceName} · {display.width} × {display.height} · {display.refreshRate ?? "Refresh rate unavailable"} · {display.dpiScalePercent}% scale ({display.effectiveDpi.toFixed(1)} DPI)</p>
                    <p className={savedProfile && assignedWallpaper?.availability !== "available" ? "display-assignment-warning" : "display-assignment-state"}>
                      {savedProfile
                        ? assignedWallpaper?.availability === "available"
                          ? `Connected · ${assignedWallpaper.name} assigned`
                          : "Connected · assigned wallpaper is missing from the library or unavailable"
                        : "Connected · no saved assignment"}
                    </p>
                    {showWallpaper && (
                      <label>
                        {displayMode === "independent" ? "Wallpaper" : "Shared wallpaper"}
                        <select
                          aria-label={`${friendlyDisplayName(display.name)} wallpaper`}
                          disabled={displaySaving}
                          value={edit.wallpaperId}
                          onChange={(event) => {
                            const value = event.currentTarget.value;
                            setDisplayEdits((current) => ({
                              ...current,
                              [display.id]: {
                                ...(current[display.id] ?? edit),
                                wallpaperId: value === "" ? "" : Number(value),
                              },
                            }));
                          }}
                        >
                          <option value="">Choose wallpaper</option>
                          {displaySnapshot.wallpapers.filter((wallpaper) => wallpaper.availability === "available").map((wallpaper) => (
                            <option key={wallpaper.id} value={wallpaper.id}>{wallpaper.name} · {wallpaper.mediaKind}</option>
                          ))}
                        </select>
                      </label>
                    )}
                    <label>
                      FPS override
                      <NumberField
                        aria-label={`${friendlyDisplayName(display.name)} FPS override`}
                        min={1}
                        max={120}
                        step={1}
                        placeholder="Default"
                        disabled={displaySaving}
                        value={edit.fpsOverride}
                        onChange={(event) => {
                          const value = event.currentTarget.value;
                          setDisplayEdits((current) => ({
                            ...current,
                            [display.id]: {
                              ...(current[display.id] ?? edit),
                              fpsOverride: value,
                            },
                          }));
                        }}
                      />
                    </label>
                  </article>
                );
              })}
            </div>
            {displaySnapshot.profiles.filter((profile) => !displaySnapshot.displays.some((display) => display.id === profile.displayId)).length > 0 && (
              <div className="display-disconnected">
                <h3>Disconnected display assignments</h3>
                <p>These stable identities are retained and will be restored when their displays reconnect.</p>
                <ul>
                  {displaySnapshot.profiles.filter((profile) => !displaySnapshot.displays.some((display) => display.id === profile.displayId)).map((profile) => (
                    <li key={profile.displayId}>
                      {profile.displayId} · {displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === profile.wallpaperId)?.name ?? `wallpaper ${profile.wallpaperId}`}
                      {displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === profile.wallpaperId)?.availability !== "available" ? " (media missing/unavailable)" : ""}
                      {` · ${profile.presentationMode}`}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {dirty && (
              <p className="home-banner display-dirty" role="status">
                <span>You have unsaved changes.</span>
                <button disabled={displaySaving || displayLoading} onClick={() => void refreshDisplayManager()}>Revert</button>
                <button className="ui-button--primary" disabled={displaySaving || displayLoading} onClick={() => void saveDisplayAssignments()}>Save and apply</button>
              </p>
            )}
          </>
        ) : (
          <p>No display state is available yet.</p>
        )}
        {displaySnapshot?.targetFailures.map((failure) => (
          <p className="display-manager-error" role="alert" key={`${failure.deviceName}:${failure.stage}`}>
            {failure.deviceName}: {failure.stage}
          </p>
        ))}
        <p className="display-manager-status" aria-live="polite" role="status">{displayStatus}</p>
      </section>
  );
}
