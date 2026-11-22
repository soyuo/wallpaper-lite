import { useEffect, useState } from "react";

import { applyDisplayProfiles, buildDisplayProfiles, makeDisplayEdits } from "../../display-manager.mjs";
import { hostAdapter, type DisplayManagerSnapshot, type DisplayPresentationMode } from "../../host-adapter";

export function useDisplays() {
  const [displaySnapshot, setDisplaySnapshot] = useState<DisplayManagerSnapshot | null>(null);
  const [displayMode, setDisplayMode] = useState<DisplayPresentationMode>("independent");
  const [displayEdits, setDisplayEdits] = useState<Record<string, { wallpaperId: number | ""; fpsOverride: string }>>({});
  const [displayStatus, setDisplayStatus] = useState("Display assignments have not been loaded.");
  const [displayLoading, setDisplayLoading] = useState(false);
  const [displaySaving, setDisplaySaving] = useState(false);

  useEffect(() => {
    void refreshDisplayManager();
  }, []);

  async function refreshDisplayManager() {
    setDisplayLoading(true);
    try {
      const snapshot = await hostAdapter.displayManagerSnapshot();
      const state = makeDisplayEdits(snapshot);
      setDisplaySnapshot(snapshot);
      setDisplayMode(state.mode);
      setDisplayEdits(state.edits);
      const loadedMessage = `Loaded ${snapshot.displays.length} connected display${snapshot.displays.length === 1 ? "" : "s"}.`;
      setDisplayStatus(snapshot.targetFailures.length === 0
        ? loadedMessage
        : `${loadedMessage} ${snapshot.targetFailures.length} target${snapshot.targetFailures.length === 1 ? "" : "s"} could not apply their assignments.`);
    } catch (error) {
      setDisplayStatus(error instanceof Error ? error.message : "Display information could not be loaded.");
    } finally {
      setDisplayLoading(false);
    }
  }

  async function saveDisplayAssignments() {
    if (!displaySnapshot) return;
    let profiles;
    try {
      profiles = buildDisplayProfiles(displaySnapshot, displayMode, displayEdits);
    } catch (error) {
      setDisplayStatus(error instanceof Error ? error.message : "Display assignments are invalid.");
      return;
    }
    setDisplaySaving(true);
    const result = await applyDisplayProfiles(profiles, (candidate) => hostAdapter.applyDisplayProfiles(candidate));
    setDisplayStatus(result.message);
    if (result.ok) await refreshDisplayManager();
    setDisplaySaving(false);
  }

  return {
    displaySnapshot, displayMode, displayEdits, displayStatus, displayLoading, displaySaving,
    setDisplayMode, setDisplayEdits, refreshDisplayManager, saveDisplayAssignments,
  };
}

export type DisplaysState = ReturnType<typeof useDisplays>;
