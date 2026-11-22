import type { DisplaysState } from "./useDisplays";
import type { DisplayPresentationMode } from "../../host-adapter";

export function DisplayPanel({ hidden, state }: { hidden: boolean; state: DisplaysState }) {
  const {
    displaySnapshot, displayMode, displayEdits, displayStatus, displayLoading, displaySaving,
    setDisplayMode, setDisplayEdits, refreshDisplayManager, saveDisplayAssignments,
  } = state;

  return (
      <section className="ui-panel display-manager" hidden={hidden} aria-labelledby="display-manager-title">
        <div className="display-manager-heading">
          <div>
            <h2 id="display-manager-title">Displays</h2>
            <p>Assign wallpapers and per-display frame-rate limits.</p>
          </div>
          <button disabled={displayLoading || displaySaving} onClick={() => void refreshDisplayManager()}>
            {displayLoading ? "Refreshing..." : "Refresh displays"}
          </button>
        </div>
        {displaySnapshot ? (
          <>
            <label className="display-mode-control">
              Presentation mode
              <select
                aria-label="Presentation mode"
                disabled={displaySaving}
                value={displayMode}
                onChange={(event) => setDisplayMode(event.currentTarget.value as DisplayPresentationMode)}
              >
                <option value="independent">Independent</option>
                <option value="duplicate">Duplicate</option>
                <option value="span">Span</option>
              </select>
            </label>
            <div className="display-manager-list">
              {displaySnapshot.displays.map((display) => {
                const edit = displayEdits[display.id] ?? { wallpaperId: "", fpsOverride: "" };
                const savedProfile = displaySnapshot.profiles.find((profile) => profile.displayId === display.id);
                const assignedWallpaper = savedProfile
                  ? displaySnapshot.wallpapers.find((wallpaper) => wallpaper.id === savedProfile.wallpaperId)
                  : null;
                const showWallpaper = displayMode === "independent" || display.isPrimary;
                return (
                  <article className="display-manager-card" key={display.id}>
                    <h3>{display.name}{display.isPrimary ? " · Primary" : ""}</h3>
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
                          aria-label={`${display.name} wallpaper`}
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
                      <input
                        aria-label={`${display.name} FPS override`}
                        type="number"
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
            <div className="display-manager-actions">
              <button disabled={displaySaving || displayLoading} onClick={() => void saveDisplayAssignments()}>
                {displaySaving ? "Applying..." : "Save and apply"}
              </button>
            </div>
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
