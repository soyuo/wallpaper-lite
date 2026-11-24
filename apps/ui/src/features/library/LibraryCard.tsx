import { useEffect, useRef } from "react";

import { formatBytes } from "../../format";
import type { LibraryEntry, LibraryPreview } from "../../host-adapter";
import { availabilityLabel, formatDuration } from "./labels";

export interface LibraryCardProps {
  entry: LibraryEntry;
  preview?: LibraryPreview;
  previewError?: string;
  previewLoading: boolean;
  keepAcknowledged: boolean;
  actionBusy: boolean;
  renaming: boolean;
  renameDraft: string;
  onRequestPreview(id: number): void;
  onLocate(entry: LibraryEntry): void;
  onRemove(entry: LibraryEntry): void;
  onKeep(id: number): void;
  onStartRename(entry: LibraryEntry): void;
  onRenameDraftChange(value: string): void;
  onSubmitRename(entry: LibraryEntry): void;
  onCancelRename(): void;
  onToggleFavorite(entry: LibraryEntry): void;
  onOpenEditorPreview(entry: LibraryEntry): void;
  onStartPlayback(entry: LibraryEntry): void;
}

export function LibraryCard({
  entry,
  preview,
  previewError,
  previewLoading,
  keepAcknowledged,
  actionBusy,
  renaming,
  renameDraft,
  onRequestPreview,
  onLocate,
  onRemove,
  onKeep,
  onStartRename,
  onRenameDraftChange,
  onSubmitRename,
  onCancelRename,
  onToggleFavorite,
  onOpenEditorPreview,
  onStartPlayback,
}: LibraryCardProps) {
  const cardRef = useRef<HTMLLIElement>(null);

  useEffect(() => {
    if (entry.availability !== "available" || preview || previewLoading || previewError) return;
    const element = cardRef.current;
    if (!element) return;
    if (!("IntersectionObserver" in window)) {
      onRequestPreview(entry.id);
      return;
    }

    const observer = new IntersectionObserver(
      (observations) => {
        if (observations.some((observation) => observation.isIntersecting)) {
          onRequestPreview(entry.id);
          observer.disconnect();
        }
      },
      { rootMargin: "180px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [entry.availability, entry.id, onRequestPreview, preview, previewError, previewLoading]);

  const duration = formatDuration(preview?.duration100ns ?? null);
  const unavailable = entry.availability !== "available";

  return (
    <li className="library-card" ref={cardRef}>
      <div className="library-thumbnail" aria-label={`${entry.displayName} thumbnail`}>
        {preview ? (
          <img src={preview.thumbnailDataUrl} alt={`${entry.displayName} preview`} />
        ) : unavailable ? (
          <span>{availabilityLabel(entry.availability)}</span>
        ) : previewError ? (
          <span>Preview unavailable</span>
        ) : previewLoading ? (
          <span className="skeleton">Loading preview...</span>
        ) : (
          <span>Preview pending</span>
        )}
      </div>
      <div className="library-card-content">
        {renaming && (
          <div className="dialog-backdrop" onKeyDown={(event) => { if (event.key === "Escape") onCancelRename(); }}>
            <form className="dialog glass-card" role="dialog" aria-modal="true" aria-labelledby={`rename-title-${entry.id}`} onSubmit={(event) => {
              event.preventDefault();
              onSubmitRename(entry);
            }}>
              <h2 id={`rename-title-${entry.id}`}>Rename wallpaper</h2>
              <p>The name changes in the library only. The source file is not renamed.</p>
              <label>
                <span className="visually-hidden">New name for {entry.displayName}</span>
                <input autoFocus maxLength={256} value={renameDraft} onChange={(event) => onRenameDraftChange(event.currentTarget.value)} />
              </label>
              <div className="dialog-actions">
                <button disabled={actionBusy} onClick={onCancelRename} type="button">Cancel</button>
                <button className="ui-button--primary" disabled={actionBusy || !renameDraft.trim()} type="submit">Save name</button>
              </div>
            </form>
          </div>
        )}
        <h3>{entry.displayName}</h3>
        <p className="library-card-path" title={entry.sourcePath}>{entry.sourcePath}</p>
        <p className="library-card-meta">
          {entry.mediaKind === "video" ? "Video" : "Image"} · {formatBytes(preview?.sizeBytes ?? entry.sizeBytes)}
          {preview ? ` · ${preview.width} × ${preview.height}` : ""}
          {duration ? ` · ${duration}` : ""}
        </p>
        <div className="library-card-actions">
          {entry.availability === "available" && (
            <button disabled={actionBusy} onClick={() => onOpenEditorPreview(entry)}>Preview on display</button>
          )}
          {entry.mediaKind === "video" && entry.availability === "available" && (
            <button disabled={actionBusy} onClick={() => onStartPlayback(entry)}>Play on desktop</button>
          )}
          <button
            aria-pressed={entry.isFavorite}
            disabled={actionBusy}
            onClick={() => onToggleFavorite(entry)}
          >
            {entry.isFavorite ? "★ Favorited" : "☆ Favorite"}
          </button>
          {!renaming && <button disabled={actionBusy} onClick={() => onStartRename(entry)}>Rename</button>}
          <button disabled={actionBusy} onClick={() => onRemove(entry)}>Remove entry</button>
          {unavailable && (
            <>
              <button disabled={actionBusy} onClick={() => onLocate(entry)}>Locate file</button>
              <button disabled={keepAcknowledged} onClick={() => onKeep(entry.id)}>
                {keepAcknowledged ? "Kept for later" : "Keep entry"}
              </button>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

