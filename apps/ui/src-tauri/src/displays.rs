use std::sync::OnceLock;

use serde::Serialize;
use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_display::{DisplayInfo, DisplayProfile, DisplayProfileSet, enumerate_displays};
use wallpaper_ipc::protocol::{
    Command, DisplayTargetFailure, PlaybackCommandErrorCode, ResponseResult,
};
use wallpaper_media::{Availability, CatalogStore, DisplayProfileStore, MediaKind, WallpaperEntry};

static DISPLAY_PROFILE_TRANSACTION: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayManagerSnapshot {
    displays: Vec<DisplayManagerDisplay>,
    profiles: Vec<DisplayProfile>,
    wallpapers: Vec<DisplayManagerWallpaper>,
    target_failures: Vec<DisplayTargetFailure>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayProfileApplyOutcome {
    applied: bool,
    failed_targets: Vec<DisplayTargetFailure>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DisplayManagerDisplay {
    id: String,
    name: String,
    device_name: String,
    width: u32,
    height: u32,
    dpi_scale_percent: u32,
    effective_dpi: f64,
    refresh_rate: Option<String>,
    is_primary: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DisplayManagerWallpaper {
    id: u64,
    name: String,
    media_kind: MediaKind,
    availability: Availability,
}

impl From<&DisplayInfo> for DisplayManagerDisplay {
    fn from(display: &DisplayInfo) -> Self {
        let refresh_rate = display
            .refresh_rate()
            .map(|rate| format!("{}/{} Hz", rate.numerator(), rate.denominator()));
        Self {
            id: display.id().as_str().to_owned(),
            name: display.name().to_owned(),
            device_name: display.device_name().to_owned(),
            width: display.resolution().width(),
            height: display.resolution().height(),
            dpi_scale_percent: display.dpi_scale_percent(),
            effective_dpi: display.effective_dpi(),
            refresh_rate,
            is_primary: display.is_primary(),
        }
    }
}

impl From<&WallpaperEntry> for DisplayManagerWallpaper {
    fn from(entry: &WallpaperEntry) -> Self {
        Self {
            id: entry.id().get(),
            name: entry.display_name().to_owned(),
            media_kind: entry.media_kind(),
            availability: entry.availability(),
        }
    }
}

#[tauri::command]
pub async fn display_manager_snapshot() -> Result<DisplayManagerSnapshot, AppError> {
    let mut snapshot = tauri::async_runtime::spawn_blocking(|| {
        let displays = enumerate_displays().map_err(|_| {
            AppError::new(
                ErrorCode::DisplayEnumerationFailed,
                "Connected displays could not be enumerated.",
            )
        })?;
        let profile_store = DisplayProfileStore::for_current_user().map_err(|_| {
            AppError::new(
                ErrorCode::DisplayProfilesStorageFailed,
                "Saved display assignments could not be loaded.",
            )
        })?;
        let profiles = profile_store.load().map_err(|_| {
            AppError::new(
                ErrorCode::DisplayProfilesStorageFailed,
                "Saved display assignments could not be loaded.",
            )
        })?;
        let catalog = CatalogStore::for_current_user()
            .map_err(|error| error.to_app_error())?
            .load()
            .map_err(|error| error.to_app_error())?;

        Ok(DisplayManagerSnapshot {
            displays: displays
                .displays()
                .iter()
                .map(DisplayManagerDisplay::from)
                .collect(),
            profiles: profiles.profiles().to_vec(),
            wallpapers: catalog
                .entries()
                .iter()
                .map(DisplayManagerWallpaper::from)
                .collect(),
            target_failures: Vec::new(),
        })
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "The display manager could not load its current state.",
        )
    })??;
    snapshot.target_failures = refresh_host_display_assignments()
        .await
        .unwrap_or_else(|_| {
            vec![DisplayTargetFailure {
                device_name: "wallpaper host".to_owned(),
                stage: "refresh_unavailable".to_owned(),
            }]
        });
    Ok(snapshot)
}

#[tauri::command]
pub async fn apply_display_profiles(
    profiles: Vec<DisplayProfile>,
) -> Result<DisplayProfileApplyOutcome, AppError> {
    let _transaction_guard = DISPLAY_PROFILE_TRANSACTION
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let candidate = DisplayProfileSet::new(profiles).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidDisplayProfiles,
            "Display assignments contain an invalid display, wallpaper, mode, or FPS value.",
        )
    })?;
    let store = DisplayProfileStore::for_current_user().map_err(|_| {
        AppError::new(
            ErrorCode::DisplayProfilesStorageFailed,
            "Display assignments could not be saved.",
        )
    })?;
    let previous = tauri::async_runtime::spawn_blocking({
        let store = store.clone();
        move || store.load()
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "Display assignments could not be loaded.",
        )
    })?
    .map_err(|_| {
        AppError::new(
            ErrorCode::DisplayProfilesStorageFailed,
            "Display assignments could not be loaded.",
        )
    })?;
    let save_store = store.clone();
    let save_candidate = candidate.clone();
    tauri::async_runtime::spawn_blocking(move || save_store.save(&save_candidate))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                "Display assignments could not be saved.",
            )
        })?
        .map_err(|_| {
            AppError::new(
                ErrorCode::DisplayProfilesStorageFailed,
                "Display assignments could not be saved.",
            )
        })?;

    match refresh_host_display_assignments().await {
        Ok(failed_targets) if failed_targets.is_empty() => {
            return Ok(DisplayProfileApplyOutcome {
                applied: true,
                failed_targets,
            });
        }
        Ok(failed_targets) => {
            restore_profiles_and_host(&store, previous).await?;
            return Ok(DisplayProfileApplyOutcome {
                applied: false,
                failed_targets,
            });
        }
        Err(_) => {}
    }

    restore_profiles_and_host(&store, previous).await?;
    Err(AppError::new(
        ErrorCode::DisplayProfilesApplyFailed,
        "The wallpaper host could not refresh the display assignments; the previous profile was restored.",
    ))
}

async fn restore_profiles_and_host(
    store: &DisplayProfileStore,
    previous: DisplayProfileSet,
) -> Result<(), AppError> {
    let rollback_store = store.clone();
    let rollback = tauri::async_runtime::spawn_blocking(move || rollback_store.save(&previous))
        .await
        .is_ok_and(|result| result.is_ok());
    if !rollback {
        return Err(AppError::new(
            ErrorCode::DisplayProfilesRollbackFailed,
            "The display apply failed and saved assignments could not be restored.",
        ));
    }
    match refresh_host_display_assignments().await {
        Ok(failed_targets) if failed_targets.is_empty() => Ok(()),
        _ => Err(AppError::new(
            ErrorCode::DisplayProfilesRollbackFailed,
            "Saved assignments were restored, but the wallpaper host could not confirm its previous display state.",
        )),
    }
}

async fn refresh_host_display_assignments() -> Result<Vec<DisplayTargetFailure>, AppError> {
    match wallpaper_ipc::client::request(Command::RefreshDisplayAssignments).await {
        Ok(ResponseResult::DisplayAssignmentsRefreshed { failed_targets }) => Ok(failed_targets),
        Ok(ResponseResult::Error {
            code: PlaybackCommandErrorCode::DisplayAssignmentsApplyFailed,
        }) => Err(AppError::new(
            ErrorCode::DisplayProfilesApplyFailed,
            "The wallpaper host could not apply the display assignments.",
        )),
        Ok(_) => Err(AppError::new(
            ErrorCode::MalformedResponse,
            "Wallpaper host returned an invalid display apply response.",
        )),
        Err(error) => Err(error.to_app_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayManagerSnapshot, DisplayTargetFailure};

    #[test]
    fn display_snapshot_serializes_target_failures_for_the_ui() {
        let snapshot = DisplayManagerSnapshot {
            displays: Vec::new(),
            profiles: Vec::new(),
            wallpapers: Vec::new(),
            target_failures: vec![DisplayTargetFailure {
                device_name: r"\\.\DISPLAY2".to_owned(),
                stage: "video_load".to_owned(),
            }],
        };

        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({
                "displays": [],
                "profiles": [],
                "wallpapers": [],
                "targetFailures": [{
                    "deviceName": r"\\.\DISPLAY2",
                    "stage": "video_load",
                }],
            })
        );
    }
}
