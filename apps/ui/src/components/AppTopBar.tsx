import type { HostState } from "../dashboard-view.mjs";

export function AppTopBar({ hostState, hostStatus, checking, onCheck }: {
  hostState: HostState;
  hostStatus: string;
  checking: boolean;
  onCheck(): void;
}) {
  return (
    <div className="app-topbar">
      <p className="app-brand"><span className="app-brand-mark" aria-hidden="true" />wallpaper Lite</p>
      <div className="app-topbar-status">
        <p aria-live="polite" className="host-status" data-state={hostState} role="status"><span>{hostStatus}</span></p>
        <button className="ui-button--primary" disabled={checking} onClick={onCheck}>
          {checking ? "Checking..." : "Check wallpaper host"}
        </button>
      </div>
    </div>
  );
}
