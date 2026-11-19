use std::{env, fs, path::PathBuf, process};

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::filter::LevelFilter;

const APP_LOG_DIRECTORY: &str = "com.soyuo.wallpaper-lite";

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
        eprintln!("Host file logging is unavailable; using stderr.");
        return None;
    };

    let log_directory = PathBuf::from(local_app_data)
        .join(APP_LOG_DIRECTORY)
        .join("logs");
    if fs::create_dir_all(&log_directory).is_err() {
        install_stderr_logger();
        eprintln!("Host file logging is unavailable; using stderr.");
        return None;
    }

    let appender = match RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("host")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&log_directory)
    {
        Ok(appender) => appender,
        Err(_) => {
            install_stderr_logger();
            eprintln!("Host file logging is unavailable; using stderr.");
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
        eprintln!("Host file logger could not be installed.");
        return None;
    }

    Some(guard)
}

fn main() -> process::ExitCode {
    let _logging_guard = initialize_logging();
    tracing::info!(
        component = "host",
        event = "startup",
        "Wallpaper host starting"
    );

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(
                component = "host",
                event = "runtime_initialization_failed",
                error_kind = ?error.kind(),
                os_error_code = ?error.raw_os_error(),
                "Wallpaper host runtime initialization failed"
            );
            return process::ExitCode::FAILURE;
        }
    };

    if let Err(error) = runtime.block_on(wallpaper_ipc::server::run()) {
        tracing::error!(
            component = "host",
            event = "ipc_server_failed",
            error_code = error.code(),
            diagnostic = %error.diagnostic_summary(),
            "Wallpaper host IPC server failed"
        );
        return process::ExitCode::FAILURE;
    }

    process::ExitCode::SUCCESS
}
