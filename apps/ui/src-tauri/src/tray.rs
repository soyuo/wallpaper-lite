//! System tray: keeps the wallpaper running while the settings window is closed.
//!
//! Closing the window only hides it. The tray's Quit item is the one exit path;
//! it stops the wallpaper host first so Windows restores the desktop background.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    AppHandle, Manager, Runtime,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use wallpaper_ipc::protocol::{Command, IpcPlaybackState, ResponseResult};

use crate::host_process;

static ACTIVE: AtomicBool = AtomicBool::new(false);

const MAIN_WINDOW: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_TOGGLE: &str = "toggle_playback";
const MENU_QUIT: &str = "quit";

/// Whether the tray icon exists, so a hidden window can still be reopened.
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Bring the settings window back, restoring it from hidden or minimized.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// The command a Play/Pause menu click should send for the current state.
fn toggle_command(state: IpcPlaybackState) -> Command {
    match state {
        IpcPlaybackState::Playing => Command::Pause,
        IpcPlaybackState::Stopped | IpcPlaybackState::Paused | IpcPlaybackState::Ended => {
            Command::Play
        }
    }
}

async fn toggle_playback() {
    let state = match wallpaper_ipc::client::request(Command::GetPlaybackState).await {
        Ok(ResponseResult::PlaybackState(event)) => event.state,
        Ok(_) | Err(_) => {
            tracing::warn!(
                component = "ui_backend",
                event = "tray_toggle_state_unavailable",
                "Tray play/pause could not read the playback state"
            );
            return;
        }
    };
    match wallpaper_ipc::client::request(toggle_command(state)).await {
        Ok(ResponseResult::PlaybackState(_)) => {}
        Ok(_) | Err(_) => tracing::warn!(
            component = "ui_backend",
            event = "tray_toggle_failed",
            "Tray play/pause was not accepted by the host"
        ),
    }
}

async fn quit<R: Runtime>(app: AppHandle<R>) {
    if let Err(error) = host_process::shutdown_host().await {
        // Exit anyway: the user asked to quit and keeping a UI with no way to
        // reach the host open would trap them. The failure is in the log.
        tracing::error!(
            component = "ui_backend",
            event = "tray_quit_host_shutdown_failed",
            error_code = error.code.as_str(),
            "The wallpaper host did not stop before the app exited"
        );
    }
    app.exit(0);
}

fn handle_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        MENU_OPEN => show_main_window(app),
        MENU_TOGGLE => {
            tauri::async_runtime::spawn(toggle_playback());
        }
        MENU_QUIT => {
            tauri::async_runtime::spawn(quit(app.clone()));
        }
        _ => {}
    }
}

/// Create the tray icon and its menu.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open settings", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, MENU_TOGGLE, "Play / Pause", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &toggle, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("wallpaper-lite")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    ACTIVE.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use wallpaper_ipc::protocol::{Command, IpcPlaybackState};

    use super::toggle_command;

    #[test]
    fn playing_pauses_and_every_other_state_plays() {
        assert_eq!(toggle_command(IpcPlaybackState::Playing), Command::Pause);
        for state in [
            IpcPlaybackState::Stopped,
            IpcPlaybackState::Paused,
            IpcPlaybackState::Ended,
        ] {
            assert_eq!(toggle_command(state), Command::Play);
        }
    }
}
