use std::{env, fs, path::PathBuf, process};

use rolling_file::{BasicRollingFileAppender, RollingConditionBasic};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use wallpaper_core::error::AppError;
use wallpaper_core::error::ErrorCode;
use wallpaper_media::{CatalogStore, ImportResult, import_local_media};

mod displays;
mod editor;
mod host_process;
mod library;
mod performance;
mod playback;

const APP_LOG_DIRECTORY: &str = "com.soyuo.wallpaper-lite";
const UI_LOG_FILE_COUNT: usize = 7;
const UI_LOG_MAX_SIZE_BYTES: u64 = 5 * 1024 * 1024;

fn install_stderr_logger() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .with_target(true)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn initialize_logging() -> Option<WorkerGuard> {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        install_stderr_logger();
        eprintln!("UI file logging is unavailable; using stderr.");
        return None;
    };

    let log_directory = PathBuf::from(local_app_data)
        .join(APP_LOG_DIRECTORY)
        .join("logs");
    if fs::create_dir_all(&log_directory).is_err() {
        install_stderr_logger();
        eprintln!("UI file logging is unavailable; using stderr.");
        return None;
    }

    let condition = RollingConditionBasic::new()
        .daily()
        .max_size(UI_LOG_MAX_SIZE_BYTES);
    let appender = match BasicRollingFileAppender::new(
        log_directory.join("ui.log"),
        condition,
        UI_LOG_FILE_COUNT - 1,
    ) {
        Ok(appender) => appender,
        Err(_) => {
            install_stderr_logger();
            eprintln!("UI file logging is unavailable; using stderr.");
            return None;
        }
    };

    let (writer, guard) = tracing_appender::non_blocking(appender);
    if tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .with_target(true)
        .with_ansi(false)
        .with_writer(writer)
        .try_init()
        .is_err()
    {
        install_stderr_logger();
        eprintln!("UI file logger could not be installed; continuing with stderr logging.");
        return None;
    }

    Some(guard)
}

fn tauri_error_code(error: &tauri::Error) -> &'static str {
    match error {
        tauri::Error::Runtime(_) => "runtime_error",
        tauri::Error::WindowLabelAlreadyExists(_) => "window_label_conflict",
        tauri::Error::WebviewLabelAlreadyExists(_) => "webview_label_conflict",
        tauri::Error::CannotReparentWebviewWindow => "cannot_reparent_webview",
        tauri::Error::AssetNotFound(_) => "asset_not_found",
        tauri::Error::Json(_) => "serialization_error",
        tauri::Error::Io(_) => "io_error",
        tauri::Error::InvalidIcon(_) => "invalid_icon",
        tauri::Error::InvalidArgs(..) => "invalid_arguments",
        tauri::Error::Setup(_) => "setup_error",
        tauri::Error::PluginInitialization(_, _) => "plugin_initialization_error",
        tauri::Error::InvalidUrl(_) => "invalid_url",
        tauri::Error::JoinError(_) => "task_join_error",
        tauri::Error::CurrentDir(_) => "current_directory_error",
        tauri::Error::UnknownPath => "unknown_path",
        tauri::Error::NoParent => "path_missing_parent",
        tauri::Error::NoExtension => "path_missing_extension",
        tauri::Error::NoBasename => "path_missing_basename",
        tauri::Error::WindowNotFound => "window_not_found",
        tauri::Error::WebviewNotFound => "webview_not_found",
        _ => "other_tauri_error",
    }
}

#[tauri::command]
async fn host_ping() -> Result<(), AppError> {
    match wallpaper_ipc::client::ping().await {
        Ok(()) => {
            tracing::info!(
                component = "ui_backend",
                event = "host_ping_succeeded",
                "Wallpaper host ping completed"
            );
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                component = "ui_backend",
                event = "host_ping_failed",
                error_code = error.code(),
                diagnostic = %error.diagnostic_summary(),
                "Wallpaper host ping failed"
            );
            Err(error.to_app_error())
        }
    }
}

/// Start the bundled wallpaper host unless one is already answering.
#[tauri::command]
async fn host_start() -> Result<host_process::HostStartOutcome, AppError> {
    host_process::ensure_host_running().await
}

/// Ask the wallpaper host to restore the desktop and exit.
#[tauri::command]
async fn host_shutdown() -> Result<(), AppError> {
    host_process::shutdown_host().await
}

#[tauri::command]
async fn import_media(paths: Vec<String>) -> Result<Vec<ImportResult>, AppError> {
    let store = CatalogStore::for_current_user().map_err(|error| error.to_app_error())?;
    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    tauri::async_runtime::spawn_blocking(move || {
        let _catalog_lock = library::lock_catalog_mutation()?;
        import_local_media(&store, &paths).map_err(|error| error.to_app_error())
    })
    .await
    .map_err(|_| {
        AppError::new(
            ErrorCode::Internal,
            "The media import task could not be completed.",
        )
    })?
}

pub fn run() -> process::ExitCode {
    let _logging_guard = initialize_logging();
    tracing::info!(
        component = "ui_backend",
        event = "startup",
        "Wallpaper UI starting"
    );

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            // Start the host without waiting for the webview, so the wallpaper is
            // restored even when the settings window never opens.
            tauri::async_runtime::spawn(async {
                if let Err(error) = host_process::ensure_host_running().await {
                    tracing::warn!(
                        component = "ui_backend",
                        event = "host_autostart_failed",
                        error_code = error.code.as_str(),
                        "The wallpaper host could not be started at launch"
                    );
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            host_ping,
            host_start,
            host_shutdown,
            import_media,
            editor::editor_monitors,
            editor::editor_preview,
            editor::apply_editor_settings,
            editor::list_editor_presets,
            editor::save_editor_preset,
            editor::rename_editor_preset,
            editor::remove_editor_preset,
            editor::recent_wallpaper_ids,
            displays::display_manager_snapshot,
            displays::apply_display_profiles,
            library::list_library,
            library::library_preview,
            library::remove_library_entry,
            library::rename_library_entry,
            library::set_library_favorite,
            library::relocate_library_entry,
            playback::get_playback_panel,
            performance::get_performance_policy_settings,
            performance::get_performance_metrics,
            performance::apply_performance_policy_settings,
            playback::playback_state,
            playback::load_video_for_playback,
            playback::control_playback,
            playback::seek_playback,
            playback::seek_playback_100ns,
            playback::apply_playback_settings,
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => {
            tracing::info!(
                component = "ui_backend",
                event = "shutdown",
                "Wallpaper UI stopped"
            );
            process::ExitCode::SUCCESS
        }
        Err(error) => {
            let error_code = tauri_error_code(&error);
            tracing::error!(
                component = "ui_backend",
                event = "tauri_runtime_failed",
                error_code,
                "Tauri runtime failed"
            );
            process::ExitCode::FAILURE
        }
    }
}
