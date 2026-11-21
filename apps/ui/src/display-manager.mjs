export function makeDisplayEdits(snapshot) {
  const stored = new Map(snapshot.profiles.map((profile) => [profile.displayId, profile]));
  const firstAvailable = snapshot.wallpapers.find((wallpaper) => wallpaper.availability === "available")?.id;
  const edits = {};
  for (const display of snapshot.displays) {
    const profile = stored.get(display.id);
    edits[display.id] = {
      wallpaperId: profile?.wallpaperId ?? firstAvailable ?? "",
      fpsOverride: profile?.fpsOverride == null ? "" : String(profile.fpsOverride),
    };
  }
  const primary = snapshot.displays.find((display) => display.isPrimary) ?? snapshot.displays[0];
  const mode = (primary ? stored.get(primary.id)?.presentationMode : undefined)
    ?? snapshot.displays.map((display) => stored.get(display.id)?.presentationMode).find(Boolean)
    ?? "independent";
  return { mode, edits };
}

export function buildDisplayProfiles(snapshot, mode, edits) {
  if (!["independent", "duplicate", "span"].includes(mode)) {
    throw new Error("Choose a supported display mode.");
  }
  if (snapshot.displays.length === 0) throw new Error("No connected displays are available.");
  const previous = new Map(snapshot.profiles.map((profile) => [profile.displayId, profile]));
  const availableWallpapers = new Set(snapshot.wallpapers
    .filter((wallpaper) => wallpaper.availability === "available")
    .map((wallpaper) => wallpaper.id));
  const connected = [...snapshot.displays].sort((a, b) => Number(b.isPrimary) - Number(a.isPrimary));
  const primary = connected.find((display) => display.isPrimary) ?? connected[0];
  const readChoice = (display, requireWallpaper = true) => {
    const edit = edits[display.id];
    const previousProfile = previous.get(display.id);
    let wallpaperId = previousProfile?.wallpaperId ?? 0;
    if (requireWallpaper) {
      const rawWallpaper = edit?.wallpaperId ?? previousProfile?.wallpaperId ?? "";
      wallpaperId = Number(rawWallpaper);
      if (!Number.isSafeInteger(wallpaperId) || wallpaperId <= 0 || !availableWallpapers.has(wallpaperId)) {
        throw new Error(`Choose an available wallpaper for ${display.name}.`);
      }
    }
    const rawFps = edit?.fpsOverride ?? (previousProfile?.fpsOverride == null ? "" : String(previousProfile.fpsOverride));
    const fpsOverride = rawFps === "" ? null : Number(rawFps);
    if (fpsOverride !== null && (!Number.isInteger(fpsOverride) || fpsOverride < 1 || fpsOverride > 120)) {
      throw new Error(`FPS override for ${display.name} must be 1–120 or blank.`);
    }
    return { wallpaperId, fpsOverride };
  };
  const commonWallpaper = mode === "independent" ? null : readChoice(primary).wallpaperId;
  const next = new Map(snapshot.profiles.map((profile) => [profile.displayId, profile]));
  for (const display of connected) {
    const choice = readChoice(display, mode === "independent");
    next.set(display.id, {
      displayId: display.id,
      wallpaperId: commonWallpaper ?? choice.wallpaperId,
      presentationMode: mode,
      fpsOverride: choice.fpsOverride,
    });
  }
  return [...next.values()].sort((a, b) => a.displayId.localeCompare(b.displayId));
}

export async function applyDisplayProfiles(profiles, apply) {
  try {
    const outcome = await apply(profiles);
    if (!outcome.applied) {
      const targets = outcome.failedTargets
        .map((target) => `${target.deviceName} (${target.stage})`)
        .join(", ");
      return {
        ok: false,
        message: targets
          ? `Assignments were restored because these displays could not apply them: ${targets}.`
          : "Assignments were restored because the wallpaper host could not confirm them.",
      };
    }
    return { ok: true, message: "Display assignments saved and applied." };
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : "Display assignments could not be applied.",
    };
  }
}
