import { useEffect } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

const KEY = "wallpaper-lite.zoom";
const MIN = 0.5;
const MAX = 2;
const STEP = 0.1;
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function clamp(value: number): number {
  return Math.min(MAX, Math.max(MIN, Math.round(value * 10) / 10));
}

function stored(): number {
  const value = Number(localStorage.getItem(KEY));
  return Number.isFinite(value) && value > 0 ? clamp(value) : 1;
}

/** Ctrl +/-/0 and Ctrl+wheel change the webview zoom; the level is restored on the next launch. */
export function useZoom() {
  useEffect(() => {
    if (!inTauri) return;
    let zoom = stored();
    const apply = (next: number) => {
      zoom = clamp(next);
      localStorage.setItem(KEY, String(zoom));
      void getCurrentWebview().setZoom(zoom).catch(() => undefined);
    };
    if (zoom !== 1) void getCurrentWebview().setZoom(zoom).catch(() => undefined);

    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey) return;
      if (event.key === "=" || event.key === "+") apply(zoom + STEP);
      else if (event.key === "-") apply(zoom - STEP);
      else if (event.key === "0") apply(1);
      else return;
      event.preventDefault();
    };
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      apply(zoom + (event.deltaY < 0 ? STEP : -STEP));
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("wheel", onWheel);
    };
  }, []);
}
