use std::{sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};
use wallpaper_core::{
    error::{AppError, ErrorCode},
    playback_settings::PlaybackSettings,
};
use wallpaper_ipc::protocol::{
    Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackStateEvent, ResponseResult,
};
use wallpaper_media::{Availability, CatalogStore, MediaKind, PlaybackSettingsStore};

static SETTINGS_TRANSACTION: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStateView {
    sequence: u64,
    wallpaper_id: Option<u64>,
    state: IpcPlaybackState,
    media_time_100ns: i64,
    duration_100ns: Option<i64>,
    frame_rate_numerator: Option<u32>,
    frame_rate_denominator: Option<u32>,
    speed_percent: u16,
    fps_limit: u16,
    loading_video: bool,
    last_error: Option<PlaybackCommandErrorCode>,
}

impl From<PlaybackStateEvent> for PlaybackStateView {
    fn from(value: PlaybackStateEvent) -> Self {
        Self {
            sequence: value.sequence,
            wallpaper_id: value.wallpaper_id,
            state: value.state,
            media_time_100ns: value.media_time_100ns,
            duration_100ns: value.duration_100ns,
            frame_rate_numerator: value.frame_rate_numerator,
            frame_rate_denominator: value.frame_rate_denominator,
            speed_percent: value.speed_percent,
            fps_limit: value.fps_limit,
            loading_video: value.loading_video,
            last_error: value.last_error,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPanel {
    state: PlaybackStateView,
    settings: Option<PlaybackSettings>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackControl {
    Play,
    Pause,
    Stop,
}

#[tauri::command]
pub async fn get_playback_panel() -> Result<PlaybackPanel, AppError> {
    let mut state = request_state().await?;
    let settings = match state.wallpaper_id {
        Some(id) => {
            let settings = load_settings(id).await?;
            let catalog = CatalogStore::for_current_user()
                .map_err(|error| error.to_app_error())?
                .load()
                .map_err(|error| error.to_app_error())?;
            let entry = catalog.get_by_raw_id(id).ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidLibraryEntry,
                    "The active video is no longer in the library.",
                )
            })?;
            if entry.media_kind() != MediaKind::Video
                || entry.availability() != Availability::Available
            {
                return Err(AppError::new(
                    ErrorCode::InvalidLibraryEntry,
                    "The active video source is unavailable; saved playback settings were not restored.",
                ));
            }
            apply_host_settings(&settings, &entry.source_path().to_string_lossy()).await?;
            state.speed_percent = settings.speed_percent();
            state.fps_limit = settings.fps_limit();
            Some(settings)
        }
        None => None,
    };
    Ok(PlaybackPanel {
        state: state.into(),
        settings,
    })
}

#[tauri::command]
pub async fn playback_state() -> Result<PlaybackStateView, AppError> {
    Ok(request_state().await?.into())
}

#[tauri::command]
pub async fn load_video_for_playback(wallpaper_id: u64) -> Result<PlaybackPanel, AppError> {
    let catalog = CatalogStore::for_current_user()
        .map_err(|error| error.to_app_error())?
        .load()
        .map_err(|error| error.to_app_error())?;
    let entry = catalog.get_by_raw_id(wallpaper_id).ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidLibraryEntry,
            "The selected video is not in the library.",
        )
    })?;
    if entry.media_kind() != MediaKind::Video || entry.availability() != Availability::Available {
        return Err(AppError::new(
            ErrorCode::InvalidLibraryEntry,
            "The selected video source is unavailable.",
        ));
    }
    let source_path = entry.source_path().to_string_lossy().into_owned();
    let settings = load_settings(wallpaper_id).await?;
    match wallpaper_ipc::client::request(Command::LoadVideo {
        wallpaper_id,
        path: source_path.clone(),
    })
    .await
    .map_err(|error| error.to_app_error())?
    {
        ResponseResult::PlaybackState(_) => {}
        ResponseResult::Error { code } => return Err(playback_error(code)),
        _ => return Err(malformed_playback_response()),
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(AppError::new(
                ErrorCode::Timeout,
                "The video did not finish loading in time.",
            ));
        }
        match request_state().await {
            Ok(state) if !state.loading_video && state.wallpaper_id == Some(wallpaper_id) => {
                break;
            }
            Ok(state) if !state.loading_video && state.last_error.is_some() => {
                return Err(playback_error(state.last_error.expect("checked above")));
            }
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(error) if error.code == ErrorCode::VideoSourceOpenFailed => return Err(error),
            Err(error) => return Err(error),
        }
    }
    apply_host_settings(&settings, &source_path).await?;
    let state = match wallpaper_ipc::client::request(Command::Play)
        .await
        .map_err(|error| error.to_app_error())?
    {
        ResponseResult::PlaybackState(state) => state,
        ResponseResult::Error { code } => return Err(playback_error(code)),
        _ => return Err(malformed_playback_response()),
    };
    Ok(PlaybackPanel {
        state: state.into(),
        settings: Some(settings),
    })
}

#[tauri::command]
pub async fn control_playback(control: PlaybackControl) -> Result<PlaybackStateView, AppError> {
    let command = match control {
        PlaybackControl::Play => Command::Play,
        PlaybackControl::Pause => Command::Pause,
        PlaybackControl::Stop => Command::Stop,
    };
    execute_state_command(command).await
}

#[tauri::command]
pub async fn seek_playback(position_ms: u64) -> Result<PlaybackStateView, AppError> {
    let position_100ns = position_ms
        .checked_mul(10_000)
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(invalid_playback_settings)?;
    seek_playback_100ns_inner(position_100ns).await
}

#[tauri::command]
pub async fn seek_playback_100ns(position_100ns: i64) -> Result<PlaybackStateView, AppError> {
    seek_playback_100ns_inner(position_100ns).await
}

async fn seek_playback_100ns_inner(position_100ns: i64) -> Result<PlaybackStateView, AppError> {
    if position_100ns < 0 {
        return Err(invalid_playback_settings());
    }
    execute_state_command(Command::Seek { position_100ns }).await
}

#[tauri::command]
pub async fn apply_playback_settings(
    wallpaper_id: u64,
    settings: PlaybackSettings,
) -> Result<PlaybackPanel, AppError> {
    let _guard = SETTINGS_TRANSACTION
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if settings.wallpaper_id() != wallpaper_id || settings.validate().is_err() {
        return Err(invalid_playback_settings());
    }
    let catalog = CatalogStore::for_current_user()
        .map_err(|error| error.to_app_error())?
        .load()
        .map_err(|error| error.to_app_error())?;
    let entry = catalog.get_by_raw_id(wallpaper_id).ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidLibraryEntry,
            "The selected video is no longer in the library.",
        )
    })?;
    if entry.media_kind() != MediaKind::Video || entry.availability() != Availability::Available {
        return Err(AppError::new(
            ErrorCode::InvalidLibraryEntry,
            "The selected video source is unavailable.",
        ));
    }
    let source_path = entry.source_path().to_string_lossy().into_owned();
    let mut state = request_state().await?;
    if state.wallpaper_id != Some(wallpaper_id) {
        return Err(AppError::new(
            ErrorCode::InvalidLibraryEntry,
            "Start this video on the desktop before changing its playback settings.",
        ));
    }
    let store = PlaybackSettingsStore::for_current_user().map_err(|error| error.to_app_error())?;
    let previous = tokio::task::spawn_blocking({
        let store = store.clone();
        move || {
            store
                .load(wallpaper_id)
                .map_err(|error| error.to_app_error())
        }
    })
    .await
    .map_err(|_| internal_playback_error())??;
    let apply_result = wallpaper_ipc::client::request(Command::ApplyPlaybackSettings {
        settings: settings.clone(),
        source_path: source_path.clone(),
    })
    .await;
    match apply_result {
        Ok(ResponseResult::PlaybackSettingsApplied) => {}
        Ok(ResponseResult::Error { code }) => return Err(playback_error(code)),
        Ok(_) => return Err(malformed_playback_response()),
        Err(error) => {
            let operation_error = error.to_app_error();
            if apply_host_settings(&previous, &source_path).await.is_err() {
                return Err(AppError::new(
                    ErrorCode::PlaybackSettingsRollbackFailed,
                    "The previous playback settings could not be restored after a timed out apply.",
                ));
            }
            return Err(operation_error);
        }
    }
    let write_store = store.clone();
    let candidate = settings.clone();
    if let Err(error) = tokio::task::spawn_blocking(move || write_store.save(&candidate))
        .await
        .map_err(|_| internal_playback_error())?
    {
        let operation_error = error.to_app_error();
        if apply_host_settings(&previous, &source_path).await.is_err() {
            return Err(AppError::new(
                ErrorCode::PlaybackSettingsRollbackFailed,
                "The previous playback settings could not be restored after saving failed.",
            ));
        }
        return Err(operation_error);
    }
    state.speed_percent = settings.speed_percent();
    state.fps_limit = settings.fps_limit();
    Ok(PlaybackPanel {
        state: state.into(),
        settings: Some(settings),
    })
}

async fn load_settings(wallpaper_id: u64) -> Result<PlaybackSettings, AppError> {
    let store = PlaybackSettingsStore::for_current_user().map_err(|error| error.to_app_error())?;
    tokio::task::spawn_blocking(move || {
        store
            .load(wallpaper_id)
            .map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| internal_playback_error())?
}

async fn apply_host_settings(
    settings: &PlaybackSettings,
    source_path: &str,
) -> Result<(), AppError> {
    match wallpaper_ipc::client::request(Command::ApplyPlaybackSettings {
        settings: settings.clone(),
        source_path: source_path.to_owned(),
    })
    .await
    {
        Ok(ResponseResult::PlaybackSettingsApplied) => Ok(()),
        Ok(ResponseResult::Error { code }) => Err(playback_error(code)),
        Ok(_) => Err(malformed_playback_response()),
        Err(error) => Err(error.to_app_error()),
    }
}

async fn execute_state_command(command: Command) -> Result<PlaybackStateView, AppError> {
    match wallpaper_ipc::client::request(command)
        .await
        .map_err(|error| error.to_app_error())?
    {
        ResponseResult::PlaybackState(state) => Ok(state.into()),
        ResponseResult::Error { code } => Err(playback_error(code)),
        _ => Err(malformed_playback_response()),
    }
}

async fn request_state() -> Result<PlaybackStateEvent, AppError> {
    match wallpaper_ipc::client::request(Command::GetPlaybackState)
        .await
        .map_err(|error| error.to_app_error())?
    {
        ResponseResult::PlaybackState(state) => Ok(state),
        ResponseResult::Error { code } => Err(playback_error(code)),
        _ => Err(malformed_playback_response()),
    }
}

fn playback_error(code: PlaybackCommandErrorCode) -> AppError {
    match code {
        PlaybackCommandErrorCode::VideoLoadFailed => AppError::new(
            ErrorCode::VideoSourceOpenFailed,
            "The video could not be opened by the wallpaper host.",
        ),
        PlaybackCommandErrorCode::PlaybackSettingsInvalid => invalid_playback_settings(),
        _ => AppError::new(
            ErrorCode::PlaybackControlFailed,
            "The wallpaper host could not complete the playback command.",
        ),
    }
}

fn invalid_playback_settings() -> AppError {
    AppError::new(
        ErrorCode::InvalidPlaybackSettings,
        "The playback settings are invalid.",
    )
}

fn malformed_playback_response() -> AppError {
    AppError::new(
        ErrorCode::MalformedResponse,
        "The wallpaper host returned an unexpected response.",
    )
}

fn internal_playback_error() -> AppError {
    AppError::new(
        ErrorCode::Internal,
        "The playback operation could not be completed.",
    )
}
