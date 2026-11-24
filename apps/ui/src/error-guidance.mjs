// What to do next for each application error code.
//
// The backend already supplies a safe, specific message for every error. This table adds
// the next step and says whether trying again is likely to help. It never contains paths,
// system error codes or other diagnostics; those stay in the log files.

const STORAGE = "Check that the settings folder can be written to and the disk is not full, then try again.";
const INVALID = "Review the values you entered, or reset them to the defaults, then try again.";
const OUT_OF_STEP =
  "The app and the wallpaper host are out of step. Quit from the tray and start wallpaper-lite again. If it keeps happening, reinstall the latest version.";

export const ERROR_GUIDANCE = {
  host_unavailable: { next: "Make sure the wallpaper host is running, then try again.", retry: true },
  timeout: { next: "The host may be busy. Wait a moment and try again.", retry: true },
  disconnected: { next: "The connection to the wallpaper host was lost. Try again.", retry: true },
  malformed_message: { next: OUT_OF_STEP, retry: false },
  malformed_request: { next: OUT_OF_STEP, retry: false },
  malformed_response: { next: OUT_OF_STEP, retry: false },
  message_too_large: { next: OUT_OF_STEP, retry: false },
  protocol_version_mismatch: {
    next: "The app and the wallpaper host are different versions. Quit from the tray and start wallpaper-lite again. If it keeps happening, reinstall the latest version.",
    retry: false,
  },
  unsupported_platform: { next: "wallpaper-lite runs on Windows 10 and Windows 11 only.", retry: false },
  transport_error: { next: "Try again. If it keeps failing, quit from the tray and start wallpaper-lite again.", retry: true },
  host_launch_failed: {
    next: "If wallpaper-host.exe is missing, reinstall wallpaper-lite. Otherwise open the log folder in Settings to see why it did not start.",
    retry: true,
  },
  autostart_failed: { next: "Try again, or manage startup apps in Windows Settings.", retry: false },
  backup_failed: { next: "Choose a folder you can write to with enough free space, then try again.", retry: false },
  invalid_backup: {
    next: "Choose a folder made by Create backup in a compatible version of wallpaper-lite.",
    retry: false,
  },
  restore_failed: {
    next: "Your previous settings are kept in a restore-previous folder inside the settings folder. Open it from Settings, or try a different backup.",
    retry: false,
  },
  library_storage_failed: { next: STORAGE, retry: true },
  invalid_library_entry: {
    next: "Choose the wallpaper in the Library again. If its file is missing, locate it or import it again.",
    retry: false,
  },
  editor_settings_storage_failed: { next: STORAGE, retry: true },
  invalid_editor_settings: { next: INVALID, retry: false },
  editor_apply_failed: {
    next: "Your wallpaper was not changed. Make sure the wallpaper host is running, then apply again.",
    retry: true,
  },
  editor_apply_rollback_failed: {
    next: "The wallpaper may not match your saved settings. Apply it again from the Editor.",
    retry: false,
  },
  editor_wallpaper_not_active: {
    next: "Start this wallpaper from the Library first, then apply your edits.",
    retry: false,
  },
  playback_settings_storage_failed: { next: STORAGE, retry: true },
  invalid_playback_settings: { next: INVALID, retry: false },
  playback_control_failed: { next: "Make sure the wallpaper host is running, then try again.", retry: true },
  playback_settings_rollback_failed: {
    next: "Playback may not match your saved settings. Open Playback and apply them again.",
    retry: false,
  },
  display_enumeration_failed: { next: "Check that your monitors are connected, then reopen Monitors.", retry: true },
  display_profiles_storage_failed: { next: STORAGE, retry: true },
  invalid_display_profiles: { next: INVALID, retry: false },
  display_profiles_apply_failed: {
    next: "Your monitors were not changed. Make sure the wallpaper host is running, then apply again.",
    retry: true,
  },
  display_profiles_rollback_failed: {
    next: "Monitor assignments may not match your saved settings. Open Monitors and apply them again.",
    retry: false,
  },
  performance_policy_storage_failed: { next: STORAGE, retry: true },
  invalid_performance_policy: { next: INVALID, retry: false },
  performance_policy_apply_failed: { next: "The power-saving policy was not changed. Try again.", retry: true },
  performance_policy_rollback_failed: {
    next: "Open Performance and apply the policy again.",
    retry: false,
  },
  windows_initialization_failed: {
    next: "Quit from the tray and start wallpaper-lite again. If it keeps failing, restart Windows.",
    retry: false,
  },
  graphics_initialization_failed: {
    next: "Update your graphics driver, then start wallpaper-lite again.",
    retry: false,
  },
  graphics_device_lost: {
    next: "Your graphics driver was reset. The wallpaper normally recovers by itself; if it stays blank, quit from the tray and start wallpaper-lite again.",
    retry: false,
  },
  image_decode_failed: {
    next: "Check that the file is a supported image that opens in Windows, then import it again.",
    retry: false,
  },
  video_source_open_failed: {
    next: "Check that the file still exists and is a video Windows can play, then try again.",
    retry: true,
  },
  video_decode_failed: {
    next: "The video may be damaged or use a format Windows cannot decode. Try another file.",
    retry: false,
  },
  desktop_host_unavailable: {
    next: "Windows Explorer may have restarted. Wait a few seconds; if the wallpaper does not come back, quit from the tray and start wallpaper-lite again.",
    retry: true,
  },
  internal: { next: "Try again. If it keeps happening, open the log folder in Settings.", retry: true },
};

/** The guidance for a code, or null for a code this table does not know. */
export function guidanceFor(code) {
  return Object.hasOwn(ERROR_GUIDANCE, code) ? ERROR_GUIDANCE[code] : null;
}

/** The backend's message followed by the next step, or just the message for an unknown code. */
export function explainError(code, message) {
  const guidance = guidanceFor(code);
  return guidance === null ? message : `${message} ${guidance.next}`;
}

/** Whether trying the same action again is likely to help. */
export function canRetry(code) {
  return guidanceFor(code)?.retry ?? false;
}
