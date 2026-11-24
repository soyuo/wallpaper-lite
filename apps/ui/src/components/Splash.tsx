import { useEffect, useRef } from "react";
import { Logo } from "./Logo";

/** Brand screen shown while the wallpaper host starts. It leaves on its own after `maxMs`. */
export function Splash({ ready, onDone, maxMs = 2500 }: { ready: boolean; onDone: () => void; maxMs?: number }) {
  // Keep the latest callback in a ref: the parent passes a new function on every render, and
  // listing it as a dependency restarted the timer whenever the host status polled.
  const done = useRef(onDone);
  done.current = onDone;
  useEffect(() => {
    const timer = window.setTimeout(() => done.current(), ready ? 600 : maxMs);
    return () => window.clearTimeout(timer);
  }, [ready, maxMs]);
  return (
    <div className="splash" role="status" aria-live="polite">
      <div className="splash-card glass-card">
        <Logo size={84} />
        <h1>wallpaper Lite</h1>
        <div className="splash-bar" aria-hidden="true"><i /></div>
        <p>{ready ? "Ready" : "Starting the wallpaper host..."}</p>
      </div>
    </div>
  );
}
