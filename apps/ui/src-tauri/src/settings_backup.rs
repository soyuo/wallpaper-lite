//! Tauri commands that back up and restore the settings folder.
//!
//! The UI passes only the folder the user picked in the native dialog. Which
//! files are read or written is decided by `wallpaper_media::backup`.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_media::{BackupError, create_backup, restore_backup};

use crate::{app_info, host_process, library};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    folder: String,
    file_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    file_count: usize,
    /// False when the host could not be restarted to read the restored settings.
    host_restarted: bool,
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Internal,
        "The settings task could not be completed.",
    )
}

fn data_directory() -> Result<PathBuf, AppError> {
    app_info::data_directory_path().ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "The settings folder could not be located.",
        )
    })
}

fn backup_error(error: BackupError) -> AppError {
    tracing::warn!(
        component = "ui_backend",
        event = "settings_backup_failed",
        reason = ?error,
        "A settings backup or restore did not complete"
    );
    error.to_app_error()
}

#[tauri::command]
pub async fn create_settings_backup(destination: String) -> Result<BackupSummary, AppError> {
    let data = data_directory()?;
    tauri::async_runtime::spawn_blocking(move || {
        // Hold the catalog lock so the library is not rewritten while it is copied.
        let _catalog_lock = library::lock_catalog_mutation()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        create_backup(
            &data,
            &PathBuf::from(destination),
            env!("CARGO_PKG_VERSION"),
            now,
        )
        .map(|outcome| BackupSummary {
            folder: outcome.folder.display().to_string(),
            file_count: outcome.file_count,
        })
        .map_err(backup_error)
    })
    .await
    .map_err(|_| busy())?
}

#[tauri::command]
pub async fn restore_settings_backup(source: String) -> Result<RestoreSummary, AppError> {
    let data = data_directory()?;

    // Stop the host first so it cannot write its old in-memory settings over the
    // restored files. If it cannot be stopped, change nothing.
    host_process::shutdown_host().await?;

    let restored = tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = library::lock_catalog_mutation()?;
        restore_backup(&data, &PathBuf::from(source)).map_err(backup_error)
    })
    .await
    .map_err(|_| busy())
    .and_then(|result| result);

    // Start the host again whether or not the restore worked, so the wallpaper keeps
    // playing and a successful restore is read from disk.
    let host_restarted = host_process::ensure_host_running().await.is_ok();
    let outcome = restored?;
    Ok(RestoreSummary {
        file_count: outcome.file_count,
        host_restarted,
    })
}
