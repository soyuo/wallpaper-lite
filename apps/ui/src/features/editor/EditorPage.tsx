import { colorControls, editorDragTransform, editorGhostStyle } from "./editor-utils";
import type { LibraryPreview } from "../../host-adapter";
import type { EditorState } from "./useEditor";

export function EditorPage({ hidden, state, previews }: { hidden: boolean; state: EditorState; previews: Record<number, LibraryPreview> }) {
  const {
    beginCropPointer,
    beginEditorGesture,
    beginEditorPointer,
    cancelEditorChanges,
    changeEditorMonitor,
    createCropMode,
    cropPointer,
    editorBusy,
    editorEntry,
    editorError,
    editorHistory,
    editorMonitorId,
    editorMonitors,
    editorPointer,
    editorPresets,
    editorPreview,
    editorSaveBusy,
    editorSettingsDraft,
    endEditorGesture,
    loadPresetToDraft,
    moveCropPointer,
    moveEditorPointer,
    presetBusy,
    presetMessage,
    presetName,
    presetRename,
    previewCanvasRef,
    previewZoom,
    redoEditorChange,
    removePreset,
    renamePreset,
    resetEditorColor,
    resetEditorSettings,
    saveCurrentPreset,
    saveEditorSettingsDraft,
    selectPlacementMode,
    selectedEditorMonitor,
    setCreateCropMode,
    setCropPreset,
    setPresetName,
    setPresetRename,
    undoEditorChange,
    updateEditorColor,
    updateEditorRender,
    zoomPreview,
  } = state;

  return (
    <>
      {!editorEntry && !hidden && (<p className="app-empty">Choose a wallpaper in the Library and press Edit.</p>)}
      {editorEntry && (
        <section className="ui-panel editor-preview-panel editor-layout" hidden={hidden} aria-labelledby="editor-preview-title">
          <div className="editor-stage">
          <div className="editor-preview-heading">
            <div>
              <h2 id="editor-preview-title">Preview: {editorEntry.displayName}</h2>
              <p>Uses the shared physical-pixel placement and color rules. Apply updates the desktop; Cancel discards the draft.</p>
            </div>
            <button disabled={editorSaveBusy} onClick={cancelEditorChanges}>
              Close preview
            </button>
          </div>
          <div className="editor-toolbar">
            <div className="seg" role="group" aria-label="Preview display">
              {editorMonitors.map((monitor) => (
                <button key={monitor.id} type="button" aria-pressed={monitor.id === editorMonitorId} className={monitor.id === editorMonitorId ? "on" : undefined} onClick={() => void changeEditorMonitor(monitor.id)}>{monitor.label}</button>
              ))}
            </div>
            {editorSettingsDraft && editorPreview && !editorBusy && (
              <div className="editor-actions">
                <button type="button" aria-label="Undo" disabled={!editorHistory.current.canUndo} onClick={undoEditorChange}>Undo</button>
                <button type="button" aria-label="Redo" disabled={!editorHistory.current.canRedo} onClick={redoEditorChange}>Redo</button>
                <button type="button" onClick={resetEditorSettings}>Reset settings</button>
                <button disabled={editorSaveBusy} onClick={cancelEditorChanges}>Cancel</button>
                <button className="ui-button--primary" disabled={editorSaveBusy} onClick={() => void saveEditorSettingsDraft()}>
                  {editorSaveBusy ? "Applying and rendering..." : "Apply"}
                </button>
              </div>
            )}
          </div>
          {editorBusy ? (
            <p role="status">Rendering preview...</p>
          ) : editorError ? (
            <p role="status">{editorError}</p>
          ) : editorPreview ? (
            <div
              ref={previewCanvasRef}
              tabIndex={0}
              className="editor-preview-canvas"
              onKeyDown={(event) => {
                if (!(event.ctrlKey || event.metaKey)) return;
                if (event.key === "+" || event.key === "=") zoomPreview(0.1);
                else if (event.key === "-") zoomPreview(-0.1);
                else if (event.key === "0") zoomPreview("reset");
                else return;
                event.preventDefault();
                event.stopPropagation();
              }}
              aria-label={`Preview canvas ${editorPreview.monitorWidth} by ${editorPreview.monitorHeight}`}
              onPointerDown={(event) => beginEditorPointer(event, "move")}
              onPointerMove={moveEditorPointer}
              onPointerUp={() => { editorPointer.current = null; endEditorGesture(); }}
              onPointerCancel={() => { editorPointer.current = null; endEditorGesture(); }}
              style={{
                aspectRatio: `${editorPreview.width} / ${editorPreview.height}`,
                width: `min(100%, ${Math.min(56, 32 * editorPreview.width / editorPreview.height)}rem)`,
              }}
            >
              <div className="editor-preview-ghost" aria-hidden="true">
                <div className="editor-preview-zoom" style={{ transform: `scale(${previewZoom})` }}>
                  {previews[editorEntry.id] && editorSettingsDraft && (
                    <img draggable={false} src={previews[editorEntry.id].thumbnailDataUrl} alt="" style={editorGhostStyle(previews[editorEntry.id], editorSettingsDraft, editorPreview.monitorWidth, editorPreview.monitorHeight)} />
                  )}
                </div>
              </div>
              <div className="editor-preview-clip">
                <div className="editor-preview-zoom" style={{ transform: `scale(${previewZoom})` }}>
                <img draggable={false} src={editorPreview.dataUrl} alt={`Rendered ${editorEntry.displayName}`} style={editorDragTransform(editorPreview, editorSettingsDraft)} />
              </div>
              </div>
              <button
                className="editor-resize-handle"
                aria-label="Resize wallpaper; hold Shift to lock aspect ratio and Alt to resize from center"
                onPointerDown={(event) => { event.stopPropagation(); beginEditorPointer(event, "resize"); }}
              />
            </div>
          ) : null}
          <p className="editor-history-shortcuts">Ctrl/Cmd+Z undo · Ctrl/Cmd+Y redo · Delete resets settings · Drag the preview to position; drag the corner handle to resize (Shift locks aspect, Alt resizes around center).</p>
          <p className="editor-preview-note">Resizing this application window only scales the preview display; it does not change saved wallpaper settings.</p>
          </div>
          {editorSettingsDraft && editorPreview && !editorBusy && (
            <div className="editor-controls">
              <div className="seg" role="group" aria-label="Placement">
                {(["cover", "fit", "stretch", "center", "custom"] as const).map((mode) => (
                  <button key={mode} type="button" aria-pressed={editorSettingsDraft.render.placementMode === mode} className={editorSettingsDraft.render.placementMode === mode ? "on" : undefined} onClick={() => selectPlacementMode(mode)}>{mode[0].toUpperCase() + mode.slice(1)}</button>
                ))}
              </div>
              <label>Zoom
                <input
                  type="range" min="0.01" max="8" step="0.01" value={editorSettingsDraft.render.scale.x}
                  onPointerDown={() => beginEditorGesture("scale-slider")}
                  onPointerUp={() => endEditorGesture("scale-slider")}
                  onPointerCancel={() => endEditorGesture("scale-slider")}
                  onBlur={() => endEditorGesture("scale-slider")}
                  onKeyDown={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) beginEditorGesture("scale-slider"); }}
                  onKeyUp={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) endEditorGesture("scale-slider"); }}
                  onChange={(event) => { const value = Number(event.currentTarget.value); updateEditorRender({ placementMode: "custom", scale: { x: value, y: value } }); }}
                />
                <output>{editorSettingsDraft.render.scale.x.toFixed(2)}×</output>
              </label>
              <label>Rotation
                <input type="number" min="-3600" max="3600" step="0.5" value={editorSettingsDraft.render.rotationDegrees} onChange={(event) => updateEditorRender({ placementMode: "custom", rotationDegrees: Number(event.currentTarget.value) || 0 })} />
              </label>
              <label>Position X (physical px)
                <input type="number" min="-32768" max="32768" value={editorSettingsDraft.render.positionPx.x} onChange={(event) => updateEditorRender({ placementMode: "custom", positionPx: { ...editorSettingsDraft.render.positionPx, x: Number(event.currentTarget.value) || 0 } })} />
              </label>
              <label>Position Y (physical px)
                <input type="number" min="-32768" max="32768" value={editorSettingsDraft.render.positionPx.y} onChange={(event) => updateEditorRender({ placementMode: "custom", positionPx: { ...editorSettingsDraft.render.positionPx, y: Number(event.currentTarget.value) || 0 } })} />
              </label>
              {editorSettingsDraft.render.placementMode === "custom" && (
                <>
                  <label>Width (physical px)
                    <input type="number" min="1" max="32768" value={editorSettingsDraft.render.sizePx?.width ?? selectedEditorMonitor?.width ?? 1} onChange={(event) => updateEditorRender({ sizePx: { width: Math.max(1, Number(event.currentTarget.value) || 1), height: editorSettingsDraft.render.sizePx?.height ?? selectedEditorMonitor?.height ?? 1 } })} />
                  </label>
                  <label>Height (physical px)
                    <input type="number" min="1" max="32768" value={editorSettingsDraft.render.sizePx?.height ?? selectedEditorMonitor?.height ?? 1} onChange={(event) => updateEditorRender({ sizePx: { width: editorSettingsDraft.render.sizePx?.width ?? selectedEditorMonitor?.width ?? 1, height: Math.max(1, Number(event.currentTarget.value) || 1) } })} />
                  </label>
                </>
              )}
              <fieldset className="editor-color-controls">
                <legend>Color adjustments</legend>
                <div className="editor-color-grid">
                  {colorControls.map(({ key, label, min, max }) => (
                    <label key={key}>
                      <span>{label}</span>
                      <input
                        type="range"
                        min={min}
                        max={max}
                        step="0.01"
                        value={editorSettingsDraft.color[key]}
                        aria-label={label}
                        onPointerDown={() => beginEditorGesture(`color-${key}`)}
                        onPointerUp={() => endEditorGesture(`color-${key}`)}
                        onPointerCancel={() => endEditorGesture(`color-${key}`)}
                        onBlur={() => endEditorGesture(`color-${key}`)}
                        onKeyDown={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) beginEditorGesture(`color-${key}`); }}
                        onKeyUp={(event) => { if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End" || event.key.startsWith("Page")) endEditorGesture(`color-${key}`); }}
                        onChange={(event) => updateEditorColor({ [key]: Number(event.currentTarget.value) })}
                      />
                      <output aria-label={`${label} value`}>{editorSettingsDraft.color[key].toFixed(2)}</output>
                    </label>
                  ))}
                </div>
                <p className="editor-color-note">Temperature ranges from cool (-1) to warm (+1). Opacity scales adjusted color toward black; the desktop surface is not transparent.</p>
                <button type="button" onClick={resetEditorColor}>Reset color</button>
              </fieldset>
              <fieldset className="editor-crop-controls">
                <legend>Normalized crop</legend>
                {previews[editorEntry.id] && (
                  <div
                    className="editor-source-crop"
                    aria-label="Crop source image; drag to select a crop, drag inside the box to move it"
                    style={{ aspectRatio: `${previews[editorEntry.id].width} / ${previews[editorEntry.id].height}` }}
                    onPointerDown={(event) => { if (createCropMode) beginCropPointer(event, "create"); }}
                    onPointerMove={moveCropPointer}
                    onPointerUp={() => { cropPointer.current = null; setCreateCropMode(false); endEditorGesture(); }}
                    onPointerCancel={() => { cropPointer.current = null; setCreateCropMode(false); endEditorGesture(); }}
                  >
                    <img draggable={false} src={previews[editorEntry.id].thumbnailDataUrl} alt="Original source crop" />
                    <div
                      className={`editor-crop-selection${createCropMode ? " is-disabled" : ""}`}
                      style={{ left: `${editorSettingsDraft.render.crop.x * 100}%`, top: `${editorSettingsDraft.render.crop.y * 100}%`, width: `${editorSettingsDraft.render.crop.width * 100}%`, height: `${editorSettingsDraft.render.crop.height * 100}%` }}
                      onPointerDown={(event) => { if (!createCropMode) beginCropPointer(event, "move"); }}
                    >
                      <button aria-label="Resize crop selection" onPointerDown={(event) => { event.stopPropagation(); beginCropPointer(event, "resize"); }} />
                    </div>
                  </div>
                )}
                <label>Aspect preset
                  <select defaultValue="free" onChange={(event) => setCropPreset(event.currentTarget.value)}>
                    <option value="free">Free</option><option value="16:9">16:9</option><option value="4:3">4:3</option><option value="1:1">1:1</option><option value="9:16">9:16</option>
                  </select>
                </label>
                <button type="button" onClick={() => setCreateCropMode(true)}>Select crop</button>
                {(["x", "y", "width", "height"] as const).map((key) => (
                  <label key={key}>{key}
                    <input type="number" min="0" max="1" step="0.01" value={editorSettingsDraft.render.crop[key]} onChange={(event) => updateEditorRender({ crop: { ...editorSettingsDraft.render.crop, [key]: Number(event.currentTarget.value) || 0 } })} />
                  </label>
                ))}
                <button onClick={() => updateEditorRender({ crop: { x: 0, y: 0, width: 1, height: 1 } })}>Reset crop</button>
              </fieldset>
              <fieldset className="editor-presets">
                <legend>Presets</legend>
                <div className="preset-save-row">
                  <label>Save current draft
                    <input maxLength={64} value={presetName} onChange={(event) => setPresetName(event.currentTarget.value)} placeholder="Preset name" />
                  </label>
                  <button type="button" disabled={presetBusy || !presetName.trim()} onClick={() => void saveCurrentPreset()}>Save preset</button>
                </div>
                {editorPresets.length === 0 ? <p>No saved presets for this wallpaper.</p> : (
                  <ul className="preset-list">
                    {editorPresets.map((preset) => (
                      <li key={preset.name}>
                        {presetRename?.name === preset.name ? (
                          <>
                            <input aria-label={`Rename ${preset.name}`} maxLength={64} value={presetRename.draft} onChange={(event) => setPresetRename({ ...presetRename, draft: event.currentTarget.value })} />
                            <button type="button" disabled={presetBusy || !presetRename.draft.trim()} onClick={() => void renamePreset(preset)}>Save name</button>
                            <button type="button" disabled={presetBusy} onClick={() => setPresetRename(null)}>Cancel rename</button>
                          </>
                        ) : (
                          <>
                            <span>{preset.name}</span>
                            <button type="button" disabled={presetBusy} onClick={() => loadPresetToDraft(preset)}>Load to draft</button>
                            <button type="button" disabled={presetBusy} onClick={() => setPresetRename({ name: preset.name, draft: preset.name })}>Rename</button>
                            <button type="button" disabled={presetBusy} onClick={() => void removePreset(preset)}>Remove</button>
                          </>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
                <p role="status" aria-live="polite">{presetMessage}</p>
                <p>Loading a preset changes this draft only. Use Apply to update the desktop.</p>
              </fieldset>
              <p>Drag the preview to position. Drag the corner handle to resize; hold Shift to lock its aspect ratio or Alt to resize around its center.</p>
            </div>
          )}
        </section>
      )}

    </>
  );
}
