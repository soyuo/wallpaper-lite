import { failureLabels, resultLabel } from "./labels";
import { LibraryCard } from "./LibraryCard";
import type { LibraryEntry } from "../../host-adapter";
import type { LibraryState } from "./useLibrary";

export function LibraryPage({ hidden, state, onOpenEditor, onStartPlayback }: {
  hidden: boolean;
  state: LibraryState;
  onOpenEditor(entry: LibraryEntry): void;
  onStartPlayback(entry: LibraryEntry): void;
}) {
  const {
    importing,
    importResults,
    importStatus,
    library,
    recentLibrary,
    librarySearch,
    favoritesOnly,
    libraryBusy,
    libraryMessage,
    previews,
    previewErrors,
    previewLoading,
    busyEntryId,
    renamingId,
    renameDraft,
    keptIds,
    visibleCount,
    filteredLibrary,
    visibleLibrary,
    setLibrarySearch,
    setFavoritesOnly,
    setVisibleCount,
    setKeptIds,
    setRenamingId,
    setRenameDraft,
    refreshLibrary,
    loadPreview,
    importSelection,
    locateEntry,
    removeEntry,
    renameEntry,
    toggleFavorite,
  } = state;

  return (
    <>
      <section className="ui-panel import-panel" hidden={hidden} aria-labelledby="import-title">
        <h2 id="import-title">Import media</h2>
        <div className="import-actions">
          <button className="ui-button--primary" disabled={importing} onClick={() => void importSelection(false)}>Choose files</button>
          <button disabled={importing} onClick={() => void importSelection(true)}>Choose folder</button>
        </div>
        <p aria-live="polite" className="import-status" role="status">
          {importing ? "Validating selected media..." : importStatus}
        </p>
        {importResults.length > 0 && (
          <ul className="import-results" aria-label="Import results">
            {importResults.map((result, index) => (
              <li key={`${result.fileName}-${index}`}>
                <span>{result.fileName}</span>
                <span>{result.failure ? failureLabels[result.failure] : resultLabel(result)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="ui-panel library-panel" hidden={hidden} aria-labelledby="library-title">
        <div className="library-heading">
          <div>
            <h2 id="library-title">Wallpaper library</h2>
            <p>{library.length} item{library.length === 1 ? "" : "s"}</p>
          </div>
          <button disabled={libraryBusy} onClick={() => void refreshLibrary()}>
            {libraryBusy ? "Refreshing..." : "Refresh library"}
          </button>
        </div>
        {recentLibrary.length > 0 && (
          <section className="recent-wallpapers" aria-labelledby="recent-wallpapers-title">
            <h3 id="recent-wallpapers-title">Recently applied</h3>
            <ul>
              {recentLibrary.map((entry) => (
                <li key={entry.id}>
                  <span>{entry.displayName}</span>
                  <button disabled={entry.availability !== "available"} onClick={() => onOpenEditor(entry)}>Edit</button>
                </li>
              ))}
            </ul>
          </section>
        )}
        <label className="library-search">
          <span>Search by name or path</span>
          <input
            type="search"
            value={librarySearch}
            onChange={(event) => {
              setLibrarySearch(event.currentTarget.value);
              setVisibleCount(24);
            }}
            placeholder="Search wallpapers"
          />
        </label>
        <div className="seg library-filter" role="group" aria-label="Library filter">
          {([[false, "All"], [true, "Favorites"]] as const).map(([value, label]) => (
            <button
              key={label}
              type="button"
              aria-pressed={favoritesOnly === value}
              className={favoritesOnly === value ? "on" : undefined}
              onClick={() => { setFavoritesOnly(value); setVisibleCount(24); }}
            >{label}</button>
          ))}
        </div>
        <p aria-live="polite" className="library-message" role="status">{libraryMessage}</p>
        {filteredLibrary.length === 0 ? (
          libraryBusy ? <p className="library-empty">Loading library...</p> : library.length === 0 ? (
            <div className="state-card library-empty">
              <p className="state-title">Your library is empty</p>
              <p className="dashboard-detail">Add images or videos to start building your wallpaper collection. Your original files are never changed.</p>
              <div className="import-actions">
                <button className="ui-button--primary" disabled={importing} onClick={() => void importSelection(false)}>Choose files</button>
                <button disabled={importing} onClick={() => void importSelection(true)}>Choose folder</button>
              </div>
            </div>
          ) : (
            <div className="state-card library-empty">
              <p className="state-title">No matching wallpaper entries.</p>
              <p className="dashboard-detail">Try a different name or clear the search and filter.</p>
              <button onClick={() => { setLibrarySearch(""); setFavoritesOnly(false); }}>Clear filters</button>
            </div>
          )
        ) : (
          <ul className="library-list" aria-label="Wallpaper library entries">
            {visibleLibrary.map((entry) => (
              <LibraryCard
                key={entry.id}
                entry={entry}
                preview={previews[entry.id]}
                previewError={previewErrors[entry.id]}
                previewLoading={Boolean(previewLoading[entry.id])}
                keepAcknowledged={keptIds.has(entry.id)}
                actionBusy={busyEntryId === entry.id}
                renaming={renamingId === entry.id}
                renameDraft={renameDraft}
                onRequestPreview={(id) => void loadPreview(id)}
                onLocate={(item) => void locateEntry(item)}
                onRemove={(item) => void removeEntry(item)}
                onKeep={(id) => setKeptIds((current) => new Set(current).add(id))}
                onStartRename={(item) => {
                  setRenamingId(item.id);
                  setRenameDraft(item.displayName);
                }}
                onRenameDraftChange={setRenameDraft}
                onSubmitRename={(item) => void renameEntry(item)}
                onCancelRename={() => {
                  setRenamingId(null);
                  setRenameDraft("");
                }}
                onToggleFavorite={(item) => void toggleFavorite(item)}
                onOpenEditorPreview={(item) => onOpenEditor(item)}
                onStartPlayback={(item) => onStartPlayback(item)}
              />
            ))}
          </ul>
        )}
        {visibleCount < filteredLibrary.length && (
          <button className="library-more" onClick={() => setVisibleCount((count) => count + 24)}>
            Show more
          </button>
        )}
      </section>
    </>
  );
}
