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

    if let Err(error) = wallpaper_desktop::initialize_windows() {
        tracing::error!(
            component = "desktop",
            event = "windows_initialization_failed",
            error_code = error.to_app_error().code.as_str(),
            diagnostic = %error.diagnostic_summary(),
            "Windows desktop initialization failed"
        );
        return process::ExitCode::FAILURE;
    }
    tracing::info!(
        component = "desktop",
        event = "windows_initialized",
        "Windows desktop process initialized"
    );

    let desktop_host = match wallpaper_desktop::discover_desktop_host() {
        Ok(desktop_host) => desktop_host,
        Err(error) => {
            tracing::error!(
                component = "desktop",
                event = "desktop_host_discovery_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Windows desktop host discovery failed"
            );
            return process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        component = "desktop",
        event = "desktop_host_discovered",
        host_kind = desktop_host.kind().as_str(),
        "Windows desktop host discovered"
    );

    let graphics_device = match wallpaper_renderer::D3d11Device::create_hardware() {
        Ok(device) => device,
        Err(error) => {
            tracing::error!(
                component = "renderer",
                event = "graphics_initialization_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Direct3D device initialization failed"
            );
            return process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        component = "renderer",
        event = "d3d11_device_initialized",
        feature_level = format!("0x{:04X}", graphics_device.feature_level()),
        "D3D11 device and immediate context initialized"
    );
    // Keep the device and immediate context alive for the host process lifetime.
    let _graphics_device = graphics_device;

    let wallpaper_window = match desktop_host.attach_wallpaper_window() {
        Ok(window) => window,
        Err(error) => {
            tracing::error!(
                component = "desktop",
                event = "wallpaper_window_attach_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Wallpaper window could not be attached"
            );
            return process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        component = "desktop",
        event = "wallpaper_window_attached",
        parent_kind = desktop_host.kind().as_str(),
        "Wallpaper window attached below desktop icons"
    );
    // Keep the native window alive until IPC exits; its Drop implementation destroys and joins.
    let _wallpaper_window = wallpaper_window;

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
