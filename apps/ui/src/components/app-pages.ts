export type AppPage = "dashboard" | "library" | "editor" | "displays" | "playback" | "performance" | "settings";

export const APP_PAGES: { id: AppPage; label: string; icon: string }[] = [
  { id: "dashboard", label: "Dashboard", icon: "⌂" },
  { id: "library", label: "Library", icon: "▦" },
  { id: "editor", label: "Editor", icon: "✎" },
  { id: "displays", label: "Monitors", icon: "▭" },
  { id: "playback", label: "Playback", icon: "▶" },
  { id: "performance", label: "Performance", icon: "◔" },
  { id: "settings", label: "Settings", icon: "⚙" },
];
