export type AppPage = "dashboard" | "library" | "editor" | "displays" | "playback" | "performance" | "settings";

export const APP_PAGES: { id: AppPage; label: string }[] = [
  { id: "dashboard", label: "Dashboard" },
  { id: "library", label: "Library" },
  { id: "editor", label: "Editor" },
  { id: "displays", label: "Monitors" },
  { id: "playback", label: "Playback" },
  { id: "performance", label: "Performance" },
  { id: "settings", label: "Settings" },
];
