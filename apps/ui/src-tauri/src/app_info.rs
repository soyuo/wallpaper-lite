//! Read-only facts shown on the Settings page, and opening the two folders it names.

use std::{env, fs, path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};
use wallpaper_core::error::{AppError, ErrorCode};

const SETTINGS_DIRECTORY: &str = "WallpaperLite";
const LOG_ROOT_DIRECTORY: &str = "kr.soyuo.wallpaper";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub data_directory: Option<String>,
    pub log_directory: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppFolder {
    Data,
    Logs,
}

pub(crate) fn data_directory_path() -> Option<PathBuf> {
    data_directory()
}

fn data_directory() -> Option<PathBuf> {
    Some(PathBuf::from(env::var_os("APPDATA")?).join(SETTINGS_DIRECTORY))
}

fn log_directory() -> Option<PathBuf> {
    Some(
        PathBuf::from(env::var_os("LOCALAPPDATA")?)
            .join(LOG_ROOT_DIRECTORY)
            .join("logs"),
    )
}

fn folder_path(folder: AppFolder) -> Option<PathBuf> {
    match folder {
        AppFolder::Data => data_directory(),
        AppFolder::Logs => log_directory(),
    }
}

fn display(path: Option<PathBuf>) -> Option<String> {
    path.map(|path| path.display().to_string())
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_directory: display(data_directory()),
        log_directory: display(log_directory()),
    }
}

/// Open one of the two fixed folders in Explorer. The UI never supplies a path.
#[tauri::command]
pub fn open_app_folder(folder: AppFolder) -> Result<(), AppError> {
    let failure = || {
        AppError::new(
            ErrorCode::Internal,
            "The folder could not be opened in Explorer.",
        )
    };
    let path = folder_path(folder).ok_or_else(failure)?;
    match folder {
        // Settings can be opened before anything was saved.
        AppFolder::Data => fs::create_dir_all(&path).map_err(|_| failure())?,
        // A missing log folder means logging is not working; say so instead of
        // showing an empty folder that looks healthy.
        AppFolder::Logs if !path.is_dir() => {
            return Err(AppError::new(
                ErrorCode::Internal,
                "No log folder exists yet. The app has not written any logs.",
            ));
        }
        AppFolder::Logs => {}
    }
    // Explorer reports a non-zero exit code even when it opens the folder, so only
    // a failure to start it is an error.
    Command::new("explorer.exe")
        .arg(&path)
        .spawn()
        .map(drop)
        .map_err(|_| failure())
}

#[cfg(test)]
mod tests {
    use super::{AppFolder, folder_path};

    #[test]
    fn folders_resolve_to_the_documented_locations() {
        let data = folder_path(AppFolder::Data).expect("APPDATA is set on Windows");
        assert!(data.ends_with("WallpaperLite"));
        let logs = folder_path(AppFolder::Logs).expect("LOCALAPPDATA is set on Windows");
        assert!(logs.ends_with(r"kr.soyuo.wallpaper\logs"));
    }

    #[test]
    fn folder_names_deserialize_from_the_ui_values() {
        assert_eq!(
            serde_json::from_str::<AppFolder>("\"data\"").unwrap(),
            AppFolder::Data
        );
        assert_eq!(
            serde_json::from_str::<AppFolder>("\"logs\"").unwrap(),
            AppFolder::Logs
        );
        assert!(serde_json::from_str::<AppFolder>("\"C:/Windows\"").is_err());
    }
}
