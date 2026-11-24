import { useId } from "react";

/** The wallpaper Lite mark: a sky tile with a sun and two waves, as drawn in the design. */
export function Logo({ size = 28 }: { size?: number }) {
  const gradient = useId();
  return (
    <svg viewBox="0 0 64 64" width={size} height={size} aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#7BA7FF" />
          <stop offset="1" stopColor="#3B68D8" />
        </linearGradient>
      </defs>
      <rect width="64" height="64" rx="16" fill={`url(#${gradient})`} />
      <circle cx="46" cy="21" r="6.5" fill="#fff" />
      <path d="M0 37 C12 30 22 30 32 36 S54 43 64 36 V64 H0Z" fill="#fff" opacity=".35" />
      <path d="M0 47 C14 39 24 41 34 47 S54 53 64 45 V48 a16 16 0 0 1 -16 16 H16 A16 16 0 0 1 0 48Z" fill="#fff" />
    </svg>
  );
}
