//! Launches, waits for, and stops the bundled `wallpaper-host.exe`.
//!
//! The host owns the desktop wallpaper windows, so it must outlive the settings
//! window. This module therefore never kills the host with the UI process: it
//! starts the host detached and stops it only through the IPC `Shutdown` command.

use std::{
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use wallpaper_core::error::{AppError, ErrorCode};
use wallpaper_ipc::{
    IpcError,
    protocol::{HOST_ALREADY_RUNNING_EXIT_CODE, ResponseResult},
};

const HOST_EXECUTABLE: &str = "wallpaper-host.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const RESPAWN_BACKOFF: Duration = Duration::from_millis(500);

/// Serializes start and stop so two callers never race to spawn two hosts.
static LIFECYCLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStartOutcome {
    AlreadyRunning,
    Started,
}

/// The host is installed next to the UI executable.
fn host_executable_path(ui_executable: &Path) -> Option<PathBuf> {
    Some(ui_executable.parent()?.join(HOST_EXECUTABLE))
}

fn spawn_host(path: &Path) -> io::Result<Child> {
    Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

fn launch_failed(message: &'static str) -> AppError {
    AppError::new(ErrorCode::HostLaunchFailed, message)
}

/// Make sure a host is answering on the control pipe, starting one if needed.
pub async fn ensure_host_running() -> Result<HostStartOutcome, AppError> {
    let _lifecycle = LIFECYCLE_LOCK.lock().await;
    match wallpaper_ipc::client::ping().await {
        Ok(()) => return Ok(HostStartOutcome::AlreadyRunning),
        Err(IpcError::HostUnavailable) => {}
        // A host that is busy or speaks another protocol version is still running.
        // Starting a second one would fight it for the pipe and the wallpaper window.
        Err(error) => {
            tracing::warn!(
                component = "ui_backend",
                event = "host_unhealthy",
                error_code = error.code(),
                "A wallpaper host exists but did not answer the liveness check"
            );
            return Err(error.to_app_error());
        }
    }

    let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|exe| host_executable_path(&exe))
    else {
        tracing::error!(
            component = "ui_backend",
            event = "host_path_unresolved",
            "The UI executable location could not be resolved"
        );
        return Err(launch_failed(
            "The wallpaper host location could not be determined.",
        ));
    };
    if !path.is_file() {
        tracing::error!(
            component = "ui_backend",
            event = "host_executable_missing",
            "The wallpaper host executable is missing next to the UI"
        );
        return Err(launch_failed(
            "The wallpaper host executable was not found next to the app.",
        ));
    }

    let spawn = || {
        spawn_host(&path).map_err(|error| {
            tracing::error!(
                component = "ui_backend",
                event = "host_spawn_failed",
                error_kind = ?error.kind(),
                os_error_code = ?error.raw_os_error(),
                "The wallpaper host process could not be created"
            );
            launch_failed("The wallpaper host could not be started.")
        })
    };
    let mut child = spawn()?;

    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if wallpaper_ipc::client::ping().await.is_ok() {
            tracing::info!(
                component = "ui_backend",
                event = "host_started",
                "Wallpaper host answered after launch"
            );
            return Ok(HostStartOutcome::Started);
        }
        match child.try_wait() {
            // The new host found the previous one still holding the single-instance
            // mutex, for example while that host finishes shutting down. Try again
            // until the old host is gone or one answers.
            Ok(Some(status))
                if status.code() == Some(i32::from(HOST_ALREADY_RUNNING_EXIT_CODE)) =>
            {
                // Back off so a host that is slow to release its mutex is not met with
                // a new process on every poll.
                tokio::time::sleep(RESPAWN_BACKOFF).await;
                if wallpaper_ipc::client::ping().await.is_err() {
                    child = spawn()?;
                }
            }
            Ok(Some(status)) if !status.success() => {
                tracing::error!(
                    component = "ui_backend",
                    event = "host_exited_during_startup",
                    exit_code = ?status.code(),
                    "The wallpaper host exited before it became ready"
                );
                return Err(launch_failed(
                    "The wallpaper host stopped while starting. Check the host log.",
                ));
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            tracing::error!(
                component = "ui_backend",
                event = "host_start_timed_out",
                "The wallpaper host did not answer in time"
            );
            return Err(launch_failed(
                "The wallpaper host did not respond after starting.",
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Ask the host to restore the desktop and exit, then wait for its pipe to close.
/// Returns without error when no host is running.
pub async fn shutdown_host() -> Result<(), AppError> {
    let _lifecycle = LIFECYCLE_LOCK.lock().await;
    match wallpaper_ipc::client::request(wallpaper_ipc::protocol::Command::Shutdown).await {
        Ok(ResponseResult::ShuttingDown) => {}
        Err(IpcError::HostUnavailable) => return Ok(()),
        Ok(_) => {
            return Err(AppError::new(
                ErrorCode::MalformedResponse,
                "The wallpaper host answered the stop request unexpectedly.",
            ));
        }
        Err(error) => return Err(error.to_app_error()),
    }

    let deadline = Instant::now() + STOP_TIMEOUT;
    while wallpaper_ipc::client::ping().await.is_ok() {
        if Instant::now() >= deadline {
            tracing::error!(
                component = "ui_backend",
                event = "host_stop_timed_out",
                "The wallpaper host acknowledged shutdown but is still answering"
            );
            return Err(AppError::new(
                ErrorCode::Timeout,
                "The wallpaper host did not stop in time.",
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    tracing::info!(
        component = "ui_backend",
        event = "host_stopped",
        "Wallpaper host stopped"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::host_executable_path;

    #[test]
    fn host_is_resolved_next_to_the_ui_executable() {
        assert_eq!(
            host_executable_path(Path::new(r"C:\Apps\wallpaper-lite\wallpaper-ui.exe")),
            Some(Path::new(r"C:\Apps\wallpaper-lite\wallpaper-host.exe").to_path_buf())
        );
    }

    #[test]
    fn a_path_without_a_parent_has_no_host_location() {
        assert_eq!(host_executable_path(Path::new("")), None);
    }
}
