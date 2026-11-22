use std::sync::OnceLock;

use wallpaper_core::{
    error::{AppError, ErrorCode},
    performance_policy::PerformancePolicySettings,
};
use wallpaper_ipc::protocol::{Command, PlaybackCommandErrorCode, ResponseResult};
use wallpaper_media::PerformancePolicyStore;

static APPLY_POLICY_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn policy_error(code: ErrorCode, message: &'static str) -> AppError {
    AppError::new(code, message)
}

fn storage_error() -> AppError {
    policy_error(
        ErrorCode::PerformancePolicyStorageFailed,
        "Automatic playback policies could not be loaded or saved.",
    )
}

async fn apply_to_host(settings: PerformancePolicySettings) -> Result<(), AppError> {
    match wallpaper_ipc::client::request(Command::ApplyPerformancePolicySettings { settings })
        .await
        .map_err(|_| {
            policy_error(
                ErrorCode::PerformancePolicyApplyFailed,
                "The wallpaper host could not confirm the policy update.",
            )
        })? {
        ResponseResult::PerformancePolicyApplied => Ok(()),
        ResponseResult::Error {
            code: PlaybackCommandErrorCode::PerformancePolicyInvalid,
        } => Err(policy_error(
            ErrorCode::InvalidPerformancePolicy,
            "The automatic playback policy settings are invalid.",
        )),
        _ => Err(policy_error(
            ErrorCode::PerformancePolicyApplyFailed,
            "The wallpaper host could not apply the automatic playback policy.",
        )),
    }
}

#[tauri::command]
pub async fn get_performance_policy_settings() -> Result<PerformancePolicySettings, AppError> {
    tauri::async_runtime::spawn_blocking(|| {
        PerformancePolicyStore::for_current_user()
            .and_then(|store| store.load())
            .map_err(|_| storage_error())
    })
    .await
    .map_err(|_| {
        policy_error(
            ErrorCode::Internal,
            "The policy settings task could not be completed.",
        )
    })?
}

#[tauri::command]
pub async fn apply_performance_policy_settings(
    settings: PerformancePolicySettings,
) -> Result<(), AppError> {
    let _apply_guard = APPLY_POLICY_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let store = PerformancePolicyStore::for_current_user().map_err(|_| storage_error())?;
    let previous = tauri::async_runtime::spawn_blocking({
        let store = store.clone();
        move || store.load().map_err(|_| storage_error())
    })
    .await
    .map_err(|_| {
        policy_error(
            ErrorCode::Internal,
            "The policy settings task could not be completed.",
        )
    })??;

    if let Err(apply_error) = apply_to_host(settings.clone()).await {
        if apply_to_host(previous.clone()).await.is_err() {
            return Err(policy_error(
                ErrorCode::PerformancePolicyRollbackFailed,
                "The host could not confirm restoration of the previous automatic policy.",
            ));
        }
        return Err(apply_error);
    }

    let settings_to_save = settings.clone();
    let persisted = tauri::async_runtime::spawn_blocking({
        let store = store.clone();
        move || store.save(&settings_to_save).map_err(|_| storage_error())
    })
    .await
    .map_err(|_| {
        policy_error(
            ErrorCode::Internal,
            "The policy settings task could not be completed.",
        )
    })?;

    if let Err(storage_error) = persisted {
        if apply_to_host(previous).await.is_err() {
            return Err(policy_error(
                ErrorCode::PerformancePolicyRollbackFailed,
                "The policy save failed and the host could not confirm restoration of the previous policy.",
            ));
        }
        return Err(storage_error);
    }
    Ok(())
}
