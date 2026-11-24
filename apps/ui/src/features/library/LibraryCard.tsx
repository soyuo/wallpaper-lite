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
  nameTaken: boolean;
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
  nameTaken,
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
        <span className="chip chip-glass library-kind">{entry.mediaKind === "video" ? "Video" : "Image"}</span>
        {renaming ? (
          <form
            className="tile-cap tile-cap--edit"
            onSubmit={(event) => {
              event.preventDefault();
              if (renameDraft.trim()) onSubmitRename(entry);
            }}
            onKeyDown={(event) => { if (event.key === "Escape") onCancelRename(); }}
          >
            <span className="tile-cap-row">
              <input
                autoFocus
                aria-invalid={!renameDraft.trim()}
                aria-label={`New name for ${entry.displayName}`}
                className="field"
                maxLength={256}
                placeholder="Enter a name"
                value={renameDraft}
                onChange={(event) => onRenameDraftChange(event.currentTarget.value)}
              />
              <span className="tile-cap-size">{formatBytes(preview?.sizeBytes ?? entry.sizeBytes)}</span>
            </span>
            <span className="tile-cap-hint" role="status">
              {!renameDraft.trim() ? (
                <><i aria-hidden="true">!</i>A name is required to save</>
              ) : nameTaken ? (
                <><i aria-hidden="true">!</i>Another item has this name. It will still be saved</>
              ) : (
                "Enter to save · Esc to cancel"
              )}
            </span>
          </form>
        ) : (
          <span className="tile-cap">
            <span className="tile-cap-name">
              <span title={entry.displayName}>{entry.displayName}</span>
              <button
                aria-label={`Rename ${entry.displayName}`}
                className="tile-cap-edit"
                disabled={actionBusy}
                onClick={() => onStartRename(entry)}
                type="button"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16Z" /></svg>
              </button>
            </span>
            <span className="tile-cap-size">{unavailable ? "—" : formatBytes(preview?.sizeBytes ?? entry.sizeBytes)}</span>
          </span>
        )}
      </div>
      <div className="library-card-content">
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

