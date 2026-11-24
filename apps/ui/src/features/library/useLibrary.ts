import { uniqueDisplayName } from "./labels";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { hostAdapter, type ImportResult, type LibraryEntry, type LibraryPreview } from "../../host-adapter";

export function useLibrary() {
  const [importing, setImporting] = useState(false);
  const [importResults, setImportResults] = useState<ImportResult[]>([]);
  const [importStatus, setImportStatus] = useState("");
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [recentIds, setRecentIds] = useState<number[]>([]);
  const [librarySearch, setLibrarySearch] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [kindFilter, setKindFilter] = useState<"all" | "image" | "video" | "missing">("all");
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [libraryMessage, setLibraryMessage] = useState("");
  const [previews, setPreviews] = useState<Record<number, LibraryPreview>>({});
  const [previewErrors, setPreviewErrors] = useState<Record<number, string>>({});
  const [previewLoading, setPreviewLoading] = useState<Record<number, boolean>>({});
  const [busyEntryId, setBusyEntryId] = useState<number | null>(null);
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [keptIds, setKeptIds] = useState<Set<number>>(() => new Set());
  const [visibleCount, setVisibleCount] = useState(24);
  const previewRequests = useRef(new Set<number>());

  const refreshLibrary = useCallback(async () => {
    setLibraryBusy(true);
    setLibraryMessage("");
    try {
      const entries = await hostAdapter.listLibrary();
      setLibrary(entries);
      try {
        setRecentIds(await hostAdapter.recentWallpaperIds());
      } catch {
        setRecentIds([]);
      }
    } catch {
      setLibraryMessage("The library could not be loaded. Other application functions remain available.");
    } finally {
      setLibraryBusy(false);
    }
  }, []);

  useEffect(() => {
    void refreshLibrary();
  }, [refreshLibrary]);

  const loadPreview = useCallback(async (id: number) => {
    if (previewRequests.current.has(id)) return;
    previewRequests.current.add(id);
    setPreviewLoading((current) => ({ ...current, [id]: true }));
    try {
      const preview = await hostAdapter.libraryPreview(id);
      setPreviews((current) => ({ ...current, [id]: preview }));
    } catch {
      setPreviewErrors((current) => ({ ...current, [id]: "Preview unavailable" }));
    } finally {
      setPreviewLoading((current) => ({ ...current, [id]: false }));
    }
  }, []);

  const filteredLibrary = useMemo(() => {
    const query = librarySearch.trim().toLocaleLowerCase();
    return library.filter((entry) =>
      (!favoritesOnly || entry.isFavorite) &&
      (kindFilter === "all" || (kindFilter === "missing" ? entry.availability !== "available" : entry.mediaKind === kindFilter)) &&
      (!query || `${entry.displayName}\n${entry.sourcePath}`.toLocaleLowerCase().includes(query)),
    );
  }, [favoritesOnly, kindFilter, library, librarySearch]);
  const visibleLibrary = filteredLibrary.slice(0, visibleCount);
  const recentLibrary = useMemo(() => recentIds
    .map((id) => library.find((entry) => entry.id === id))
    .filter((entry): entry is LibraryEntry => entry !== undefined)
    .slice(0, 10), [library, recentIds]);

  async function importSelection(directory: boolean) {
    setImporting(true);
    setImportStatus("");
    setImportResults([]);
    try {
      const selection = await open(
        directory
          ? { directory: true, multiple: false, title: "Import wallpaper folder" }
          : { multiple: true, title: "Import wallpaper files" },
      );
      if (!selection) return;
      const paths = Array.isArray(selection) ? selection : [selection];
      const results = await hostAdapter.importMedia(paths);
      setImportResults(results);
      const imported = results.filter((result) => result.status === "imported").length;
      setImportStatus(`${imported} item${imported === 1 ? "" : "s"} added; ${results.length} result${results.length === 1 ? "" : "s"}.`);
      await refreshLibrary();
    } catch {
      setImportStatus("Import could not be completed. Check the library and try again.");
    } finally {
      setImporting(false);
    }
  }

  async function locateEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      const path = await open({ multiple: false, title: "Locate wallpaper file" });
      if (typeof path !== "string") return;
      await hostAdapter.relocateLibraryEntry(entry.id, path);
      previewRequests.current.delete(entry.id);
      setPreviews((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setPreviewErrors((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setKeptIds((current) => {
        const next = new Set(current);
        next.delete(entry.id);
        return next;
      });
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not locate "${entry.displayName}". Choose a supported file of the same media type.`);
    } finally {
      setBusyEntryId(null);
    }
  }

  async function removeEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.removeLibraryEntry(entry.id);
      setPreviews((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      setPreviewErrors((current) => {
        const next = { ...current };
        delete next[entry.id];
        return next;
      });
      previewRequests.current.delete(entry.id);
      setKeptIds((current) => {
        const next = new Set(current);
        next.delete(entry.id);
        return next;
      });
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not remove "${entry.displayName}" from the library. Its source file remains untouched.`);
    } finally {
      setBusyEntryId(null);
    }
  }

  async function renameEntry(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.renameLibraryEntry(entry.id, renameDraft);
      setRenamingId(null);
      setRenameDraft("");
      setLibraryMessage(`Renamed to "${uniqueDisplayName(library, entry.id, renameDraft)}".`);
      await refreshLibrary();
    } catch {
      setLibraryMessage("Could not rename this entry. Use a name of 1 to 256 characters.");
    } finally {
      setBusyEntryId(null);
    }
  }

  async function toggleFavorite(entry: LibraryEntry) {
    setBusyEntryId(entry.id);
    setLibraryMessage("");
    try {
      await hostAdapter.setLibraryFavorite(entry.id, !entry.isFavorite);
      await refreshLibrary();
    } catch {
      setLibraryMessage(`Could not update the favorite state for "${entry.displayName}".`);
    } finally {
      setBusyEntryId(null);
    }
  }


  return {
    importing,
    importResults,
    importStatus,
    library,
    recentLibrary,
    librarySearch,
    favoritesOnly,
    kindFilter,
    setKindFilter,
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
    setPreviews,
    setRecentIds,
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
  };
}

export type LibraryState = ReturnType<typeof useLibrary>;
