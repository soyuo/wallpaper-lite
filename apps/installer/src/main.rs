#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install;
mod payload;
mod shortcut;

use std::{env, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupInfo {
    mode: &'static str,
    version: &'static str,
    install_dir: String,
    payload_ready: bool,
    existing_install: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptions {
    pub install_dir: String,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub desktop_shortcut: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallOptions {
    pub delete_data: bool,
}

fn uninstall_mode() -> bool {
    env::args().any(|arg| arg == "--uninstall")
}

fn silent_mode() -> bool {
    env::args().any(|arg| arg == "--silent" || arg.eq_ignore_ascii_case("/S"))
}

#[tauri::command]
fn setup_info() -> SetupInfo {
    let existing = install::registered_install_dir();
    SetupInfo {
        mode: if uninstall_mode() { "uninstall" } else { "install" },
        version: env!("CARGO_PKG_VERSION"),
        install_dir: existing.clone().unwrap_or_else(install::default_install_dir).display().to_string(),
        payload_ready: payload::is_complete(),
        existing_install: existing.is_some(),
    }
}

#[tauri::command]
async fn browse_folder(app: tauri::AppHandle) -> Option<String> {
    let picked = app.dialog().file().blocking_pick_folder()?;
    picked.into_path().ok().map(|path| path.display().to_string())
}

#[tauri::command]
async fn run_install(app: tauri::AppHandle, options: InstallOptions) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        install::install(&options, &|step, status| {
            let _ = app.emit("install-progress", serde_json::json!({ "step": step, "status": status }));
        })
    })
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn run_uninstall(options: UninstallOptions) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || install::uninstall(&options))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
fn running_apps() -> Vec<String> {
    install::running_apps()
}

#[tauri::command]
fn set_launch_at_login(install_dir: String, enabled: bool) -> Result<(), String> {
    install::set_launch_at_login(&PathBuf::from(install_dir), enabled)
}

#[tauri::command]
fn launch_app(install_dir: String) -> Result<(), String> {
    install::launch(&PathBuf::from(install_dir))
}

fn main() {
    if silent_mode() {
        let result = if uninstall_mode() {
            install::uninstall(&UninstallOptions { delete_data: env::args().any(|arg| arg == "--delete-data") })
        } else {
            install::install(&InstallOptions {
                install_dir: install::registered_install_dir()
                    .unwrap_or_else(install::default_install_dir)
                    .display()
                    .to_string(),
                launch_at_login: env::args().any(|arg| arg == "--launch-at-login"),
                desktop_shortcut: false,
            }, &|_, _| {})
        };
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![setup_info, browse_folder, run_install, run_uninstall, running_apps, set_launch_at_login, launch_app])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the setup window could not start");
}
