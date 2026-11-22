import type { WallpaperEditorSettings } from "../../host-adapter";

export const colorControls: Array<{
  key: keyof WallpaperEditorSettings["color"];
  label: string;
  min: number;
  max: number;
}> = [
  { key: "brightness", label: "Brightness", min: 0, max: 4 },
  { key: "contrast", label: "Contrast", min: 0, max: 4 },
  { key: "saturation", label: "Saturation", min: 0, max: 4 },
  { key: "gamma", label: "Gamma", min: 0.1, max: 4 },
  { key: "opacity", label: "Opacity", min: 0, max: 1 },
  { key: "temperature", label: "Color temperature", min: -1, max: 1 },
  { key: "red", label: "Red balance", min: 0, max: 4 },
  { key: "green", label: "Green balance", min: 0, max: 4 },
  { key: "blue", label: "Blue balance", min: 0, max: 4 },
];

export function defaultEditorSettings(wallpaperId: number): WallpaperEditorSettings {
  return {
    schemaVersion: 1,
    wallpaperId,
    render: {
      placementMode: "cover",
      positionPx: { x: 0, y: 0 },
      sizePx: null,
      scale: { x: 1, y: 1 },
      rotationDegrees: 0,
      anchor: { x: 0.5, y: 0.5 },
      crop: { x: 0, y: 0, width: 1, height: 1 },
    },
    color: {
      brightness: 1,
      contrast: 1,
      saturation: 1,
      gamma: 1,
      opacity: 1,
      temperature: 0,
      red: 1,
      green: 1,
      blue: 1,
    },
  };
}

export type EditorPointer = {
  kind: "move" | "resize";
  startX: number;
  startY: number;
  position: { x: number; y: number };
  size: { width: number; height: number };
  aspect: number;
};

export type CropPointer = {
  kind: "create" | "move" | "resize";
  startX: number; startY: number;
  origin: { x: number; y: number; width: number; height: number };
};

export type EditorPreviewLike = { monitorWidth: number; monitorHeight: number; settings: WallpaperEditorSettings };

// The preview bitmap is rendered once per load/apply; reflect pending move/resize drags on it live.
export function editorDragTransform(preview: EditorPreviewLike, draft: WallpaperEditorSettings | null): React.CSSProperties | undefined {
  if (!draft) return undefined;
  const { monitorWidth: mw, monitorHeight: mh } = preview;
  if (mw <= 0 || mh <= 0) return undefined;
  const base = preview.settings.render;
  const next = draft.render;
  const baseSize = base.sizePx ?? { width: mw, height: mh };
  const nextSize = next.sizePx ?? { width: mw, height: mh };
  const dx = ((next.positionPx.x - base.positionPx.x) / mw) * 100;
  const dy = ((next.positionPx.y - base.positionPx.y) / mh) * 100;
  const sx = nextSize.width / baseSize.width;
  const sy = nextSize.height / baseSize.height;
  if (dx === 0 && dy === 0 && sx === 1 && sy === 1) return undefined;
  return {
    transformOrigin: `${(base.positionPx.x / mw) * 100}% ${(base.positionPx.y / mh) * 100}%`,
    transform: `translate(${dx}%, ${dy}%) scale(${sx}, ${sy})`,
  };
}

// Approximates crates/renderer placement.rs on the full source so the part outside the display stays visible.
export function editorGhostStyle(source: { width: number; height: number }, draft: WallpaperEditorSettings, mw: number, mh: number): React.CSSProperties | undefined {
  const r = draft.render;
  const c = r.crop;
  const ew = source.width * c.width;
  const eh = source.height * c.height;
  if (mw <= 0 || mh <= 0 || ew <= 0 || eh <= 0 || c.width <= 0 || c.height <= 0) return undefined;
  let scale = Math.max(mw / ew, mh / eh);
  if (r.placementMode === "fit") scale = Math.min(mw / ew, mh / eh);
  else if (r.placementMode === "center") scale = 1;
  let boxW = ew * scale;
  let boxH = eh * scale;
  if (r.placementMode === "stretch") { boxW = mw; boxH = mh; }
  // Destination rectangle before sizePx: Cover and Stretch fill the display, Fit/Center use the box.
  const fills = r.placementMode === "cover" || r.placementMode === "custom" || r.placementMode === "stretch";
  const baseW = fills ? mw : boxW;
  const baseH = fills ? mh : boxH;
  const rectW = r.sizePx?.width ?? baseW;
  const rectH = r.sizePx?.height ?? baseH;
  const kx = rectW / baseW;
  const ky = rectH / baseH;
  const boxWs = boxW * kx;
  const boxHs = boxH * ky;
  const rectL = (mw - rectW) / 2;
  const rectT = (mh - rectH) / 2;
  const left = (mw - boxWs) / 2 - c.x * (boxWs / c.width);
  const top = (mh - boxHs) / 2 - c.y * (boxHs / c.height);
  const width = boxWs / c.width;
  const height = boxHs / c.height;
  const ax = rectL + rectW * r.anchor.x;
  const ay = rectT + rectH * r.anchor.y;
  return {
    position: "absolute",
    left: `${(left / mw) * 100}%`,
    top: `${(top / mh) * 100}%`,
    width: `${(width / mw) * 100}%`,
    height: `${(height / mh) * 100}%`,
    clipPath: `inset(${c.y * 100}% ${(1 - c.x - c.width) * 100}% ${(1 - c.y - c.height) * 100}% ${c.x * 100}%)`,
    transformOrigin: `${((ax - left) / width) * 100}% ${((ay - top) / height) * 100}%`,
    transform: `translate(${(r.positionPx.x / width) * 100}%, ${(r.positionPx.y / height) * 100}%) rotate(${r.rotationDegrees}deg) scale(${r.scale.x}, ${r.scale.y})`,
  };
}
