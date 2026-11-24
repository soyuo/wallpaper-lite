export function formatBytes(value: number | null): string {
  if (value === null) return "Size unavailable";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unit = units[0];
  for (let index = 1; size >= 1024 && index < units.length; index += 1) {
    size /= 1024;
    unit = units[index];
  }
  return `${size.toFixed(size >= 100 ? 0 : 1)} ${unit}`;
}

/** Windows names monitors `\.\DISPLAY1`; show "Display 1" instead and leave other names alone. */
export function friendlyDisplayName(name: string): string {
  const match = /^\\\.\DISPLAY(\d+)$/i.exec(name);
  return match ? `Display ${match[1]}` : name;
}
