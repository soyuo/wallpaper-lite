import { Logo } from "./Logo";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { HostState } from "../dashboard-view.mjs";
import { Icon } from "./icons";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function AppTopBar({ hostState, hostStatus, checking, onCheck }: {
  hostState: HostState;
  hostStatus: string;
  checking: boolean;
  onCheck(): void;
}) {
  return (
    <div className="app-topbar" data-tauri-drag-region={inTauri ? true : undefined}>
      <p className="app-brand" data-tauri-drag-region={inTauri ? true : undefined}><Logo size={26} />wallpaper Lite</p>
      <div className="app-topbar-status" data-tauri-drag-region={inTauri ? true : undefined}>
        <button
          type="button"
          className="host-status"
          data-state={hostState}
          data-checking={checking ? "true" : undefined}
          disabled={checking}
          title="Check the wallpaper host again"
          onClick={onCheck}
        >
          <span aria-live="polite" role="status">{checking ? "Checking..." : hostStatus}</span>
          <Icon name="refresh" size={14} />
        </button>
      </div>
      {inTauri && (
        <div className="app-window-controls">
          <button aria-label="Minimize window" onClick={() => void getCurrentWindow().minimize()}><Icon name="minimize" size={16} /></button>
          <button aria-label="Maximize or restore window" onClick={() => void getCurrentWindow().toggleMaximize()}><Icon name="maximize" size={16} /></button>
          <button aria-label="Close window" className="app-window-close" onClick={() => void getCurrentWindow().close()}><Icon name="close" size={16} /></button>
        </div>
      )}
    </div>
  );
}
