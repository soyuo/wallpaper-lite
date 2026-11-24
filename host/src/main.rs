use std::{env, ffi::OsStr, fs, path::PathBuf, process, sync::mpsc::TryRecvError, time::Duration};

use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::filter::LevelFilter;
use wallpaper_core::performance_policy::{PerformancePolicySettings, PolicyAction};
use wallpaper_desktop::{ShellLifecycleEvent, ShellLifecycleMonitor};
use wallpaper_ipc::protocol::{DisplayTargetFailure, PerformanceMediaKind};
use wallpaper_media::{DecodedImage, MediaError, PerformancePolicyStore, decode_image};

mod independent_displays;
mod performance_metrics;
mod performance_signals;
mod playback;
mod recovery;

use performance_metrics::PerformanceMetricsRegistry;
use performance_signals::{PerformanceSignalMonitor, PerformanceSignalSnapshot, policy_signals};
use playback::PlaybackWorker;
use recovery::DesktopRecoveryWorker;

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

/// Holds the per-session mutex that keeps a second host from attaching another
/// wallpaper window. Windows releases it when the process exits.
struct SingleInstanceGuard(windows::Win32::Foundation::HANDLE);

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateMutexW and is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

enum SingleInstance {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
    Unavailable(windows::core::Error),
}

fn acquire_single_instance() -> SingleInstance {
    use windows::{
        Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        },
        core::w,
    };

    // SAFETY: the name is a valid NUL-terminated wide string and no security
    // attributes are supplied, so the default per-user descriptor applies.
    match unsafe { CreateMutexW(None, false, w!("Local\\com.soyuo.wallpaper-lite.host")) } {
        Ok(handle) => {
            // SAFETY: reads the thread's last-error value set by CreateMutexW above.
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
                SingleInstance::AlreadyRunning
            } else {
                SingleInstance::Acquired(SingleInstanceGuard(handle))
            }
        }
        Err(error) => SingleInstance::Unavailable(error),
    }
}

fn main() -> process::ExitCode {
    let _logging_guard = initialize_logging();
    tracing::info!(
        component = "host",
        event = "startup",
        "Wallpaper host starting"
    );
    let _single_instance = match acquire_single_instance() {
        SingleInstance::Acquired(guard) => Some(guard),
        SingleInstance::AlreadyRunning => {
            tracing::info!(
                component = "host",
                event = "already_running",
                "Another wallpaper host is already running; exiting"
            );
            return process::ExitCode::from(
                wallpaper_ipc::protocol::HOST_ALREADY_RUNNING_EXIT_CODE,
            );
        }
        SingleInstance::Unavailable(error) => {
            tracing::warn!(
                component = "host",
                event = "single_instance_unavailable",
                os_error_code = error.code().0,
                "Single-instance guard is unavailable; continuing without it"
            );
            None
        }
    };

    let initial_image = match load_startup_image() {
        Ok(image) => image,
        Err(StartupImageError::Arguments) => {
            tracing::error!(
                component = "host",
                event = "startup_arguments_invalid",
                "Expected optional arguments in the form --image <path>"
            );
            return process::ExitCode::FAILURE;
        }
        Err(StartupImageError::Decode(error)) => {
            tracing::error!(
                component = "media",
                event = "startup_image_decode_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Startup image could not be decoded"
            );
            return process::ExitCode::FAILURE;
        }
    };
    let has_explicit_startup_image = initial_image.is_some();

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
    // Monitor rectangles must be queried after the process enters per-monitor DPI awareness;
    // otherwise Windows can virtualize them for an unaware caller.
    let display_assignments = independent_displays::load_display_assignments();
    let performance_metrics = PerformanceMetricsRegistry::default();
    let primary_display_id = display_assignments
        .primary_display_id
        .as_deref()
        .unwrap_or("primary");
    performance_metrics.register_target(&primary_display_id, PerformanceMediaKind::Unknown);
    let has_secondary_assignment = display_assignments
        .profiles
        .iter()
        .any(|assignment| !assignment.display.is_primary());
    let has_display_assignment = !display_assignments.profiles.is_empty();
    let primary_span_bounds = display_assignments
        .profiles
        .iter()
        .find(|assignment| assignment.display.is_primary())
        .and_then(|assignment| assignment.span_bounds);

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
    let wallpaper_window_result = if has_display_assignment {
        match display_assignments.primary_bounds {
            Some(bounds) => desktop_host.attach_wallpaper_window_for_display(bounds),
            None => {
                tracing::error!(
                    component = "display",
                    event = "primary_display_bounds_unavailable",
                    "Display assignments cannot start without primary display bounds"
                );
                desktop_host.attach_wallpaper_window()
            }
        }
    } else {
        desktop_host.attach_wallpaper_window()
    };
    let wallpaper_window = match wallpaper_window_result {
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
        host_kind = desktop_host.kind().as_str(),
        wallpaper_layer_kind = "workerw",
        "Wallpaper window attached below desktop icons"
    );
    let shell_lifecycle_monitor = match ShellLifecycleMonitor::start(desktop_host) {
        Ok(monitor) => monitor,
        Err(error) => {
            tracing::error!(
                component = "desktop",
                event = "shell_lifecycle_monitor_start_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Explorer lifecycle monitoring could not start"
            );
            return process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        component = "desktop",
        event = "shell_lifecycle_monitor_started",
        "Explorer lifecycle monitoring started"
    );

    let media_device = graphics_device.device().clone();
    let mut desktop_recovery = match DesktopRecoveryWorker::start_with_geometry(
        wallpaper_window,
        graphics_device,
        display_assignments.primary_bounds,
        primary_span_bounds,
    ) {
        Ok(recovery) => recovery,
        Err(error) => {
            tracing::error!(
                component = "desktop",
                event = "desktop_recovery_start_failed",
                error_kind = ?error.kind(),
                os_error_code = ?error.raw_os_error(),
                "Desktop recovery worker could not start"
            );
            return process::ExitCode::FAILURE;
        }
    };
    tracing::info!(
        component = "desktop",
        event = "desktop_recovery_started",
        "Desktop host recovery worker started"
    );
    let policy_settings =
        match PerformancePolicyStore::for_current_user().and_then(|store| store.load()) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    component = "performance",
                    event = "performance_policy_load_failed",
                    error = ?error,
                    "Using default automatic pause policies"
                );
                PerformancePolicySettings::default()
            }
        };
    let policy_settings = std::sync::Arc::new(std::sync::RwLock::new(policy_settings));
    let mut signal_monitor = match PerformanceSignalMonitor::start() {
        Ok(monitor) => {
            tracing::info!(
                component = "performance",
                event = "signal_monitor_started",
                "Fullscreen, session, and power signal monitor started"
            );
            Some(monitor)
        }
        Err(error) => {
            tracing::warn!(
                component = "performance",
                event = "signal_monitor_start_failed",
                stage = error.stage,
                error_code = ?error.error_code,
                "Performance signal monitoring is unavailable; wallpaper rendering will continue"
            );
            None
        }
    };
    let mut signal_receiver = signal_monitor
        .as_ref()
        .map(PerformanceSignalMonitor::subscribe);
    let playback_signal_receiver = signal_receiver.as_ref().cloned();
    let initial_signals = signal_receiver
        .as_mut()
        .map(|receiver| policy_signals(&receiver.borrow_and_update()))
        .unwrap_or_default();
    let initial_action = policy_settings
        .read()
        .map(|settings| settings.evaluate(initial_signals, false).action())
        .unwrap_or(PolicyAction::Continue);
    let initial_rendering_paused = initial_action != PolicyAction::Continue;
    if initial_rendering_paused && !desktop_recovery.frame_sender().set_rendering_paused(true) {
        tracing::warn!(
            component = "renderer",
            event = "initial_render_policy_update_failed",
            "Desktop recovery worker did not accept the initial rendering policy"
        );
    }

    let (command_sender, command_receiver) = wallpaper_ipc::protocol::playback_command_channel();
    let (display_refresh_sender, mut display_refresh_receiver) =
        tokio::sync::mpsc::unbounded_channel();
    let (mut playback_worker, mut playback_events) =
        match PlaybackWorker::start_with_performance_metrics(
            command_receiver,
            media_device,
            &desktop_recovery,
            policy_settings.clone(),
            playback_signal_receiver,
            Some(display_refresh_sender),
            Some(playback::PerformanceMetricsBinding {
                display_id: primary_display_id.to_owned(),
                media_kind: PerformanceMediaKind::Unknown,
                registry: performance_metrics.clone(),
                sample_memory: true,
            }),
        ) {
            Ok(worker) => worker,
            Err(error) => {
                tracing::error!(
                    component = "media",
                    event = "playback_worker_start_failed",
                    error_kind = ?error.kind(),
                    os_error_code = ?error.raw_os_error(),
                    "Video playback worker could not start"
                );
                stop_desktop_recovery(&mut desktop_recovery);
                drop(desktop_recovery);
                drop(shell_lifecycle_monitor);
                if let Some(monitor) = signal_monitor.take() {
                    let _ = monitor.stop();
                }
                return process::ExitCode::FAILURE;
            }
        };
    if let Some((source_path, image)) = initial_image {
        match desktop_recovery.present_image(image, source_path) {
            Ok(()) => {
                performance_metrics.set_media_kind(primary_display_id, PerformanceMediaKind::Image);
                tracing::info!(
                    component = "renderer",
                    event = "wallpaper_image_presented",
                    "Startup image presented in the desktop child"
                )
            }
            Err(error) => tracing::error!(
                component = "renderer",
                event = "wallpaper_image_presentation_failed",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Startup image could not be presented"
            ),
        }
    }

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
            drop(command_sender);
            if !playback_worker.stop() {
                tracing::error!(
                    component = "media",
                    event = "playback_worker_join_failed",
                    "Playback worker is still using recovery resources; retaining them until process exit"
                );
                std::mem::forget(playback_worker);
                std::mem::forget(desktop_recovery);
            } else {
                stop_desktop_recovery(&mut desktop_recovery);
                drop(desktop_recovery);
            }
            if let Some(monitor) = signal_monitor.take() {
                let _ = monitor.stop();
            }
            drop(shell_lifecycle_monitor);
            return process::ExitCode::FAILURE;
        }
    };

    let mut display_change_receiver = signal_monitor
        .as_ref()
        .map(PerformanceSignalMonitor::subscribe_display_changes);

    let mut initial_primary_assignment = None;
    if !has_explicit_startup_image {
        if let Some(primary) = display_assignments
            .profiles
            .iter()
            .find(|assignment| assignment.display.is_primary())
        {
            match primary.wallpaper.media_kind() {
                wallpaper_media::MediaKind::Image => {
                    if let Err(stage) = independent_displays::present_assigned_primary_image(
                        primary,
                        &desktop_recovery,
                    ) {
                        tracing::warn!(
                            component = "display",
                            event = "primary_assignment_start_failed",
                            stage,
                            "Primary display profile could not be started"
                        );
                    } else {
                        performance_metrics.set_media_kind(
                            primary.display.id().as_str(),
                            PerformanceMediaKind::Image,
                        );
                        initial_primary_assignment = Some(primary.clone());
                    }
                }
                wallpaper_media::MediaKind::Video => {
                    if let Err(stage) = independent_displays::start_assigned_primary_video(
                        &runtime,
                        &command_sender,
                        &mut playback_events,
                        &desktop_recovery,
                        primary,
                    ) {
                        tracing::warn!(
                            component = "display",
                            event = "primary_assignment_start_failed",
                            stage,
                            "Primary display profile could not be started"
                        );
                    } else {
                        initial_primary_assignment = Some(primary.clone());
                    }
                }
            }
        }
    }
    let primary_assignment_reconciler = runtime.block_on(async {
        independent_displays::PrimaryAssignmentReconciler::start(
            initial_primary_assignment.as_ref(),
            command_sender.clone(),
            playback_events.clone(),
            desktop_recovery.frame_sender(),
            performance_metrics.clone(),
        )
    });
    let mut display_targets = independent_displays::DisplayTargetManager::start(
        if has_secondary_assignment && display_assignments.primary_bounds.is_some() {
            &display_assignments.profiles
        } else {
            &[]
        },
        display_assignments.primary_bounds,
        policy_settings.clone(),
        signal_receiver.as_ref().cloned(),
        performance_metrics.clone(),
    );

    let run_result = runtime.block_on(async {
        let server = wallpaper_ipc::server::run_with_commands_and_events(
            command_sender,
            playback_events,
        );
        tokio::pin!(server);
        let mut lifecycle_poll = tokio::time::interval(Duration::from_millis(100));
        let mut last_signal_snapshot: Option<PerformanceSignalSnapshot> = None;

        loop {
            tokio::select! {
                result = &mut server => break Ok(result),
                request = display_refresh_receiver.recv() => {
                    if let Some(request) = request {
                        match independent_displays::try_load_display_assignments() {
                            Ok(assignments) => {
                                let mut failures = Vec::new();
                                let geometry_completion = if let Some(bounds) = assignments.primary_bounds {
                                    let span_bounds = assignments.profiles.iter()
                                        .find(|assignment| assignment.display.is_primary())
                                        .and_then(|assignment| assignment.span_bounds);
                                    match desktop_recovery.request_display_geometry(bounds, span_bounds) {
                                        Ok(completion) => Some(completion),
                                        Err(_) => {
                                            failures.push(DisplayTargetFailure {
                                                device_name: "primary display".to_owned(),
                                                stage: "geometry_queue".to_owned(),
                                            });
                                            None
                                        }
                                    }
                                } else {
                                    None
                                };
                                let primary_completion = if let Some(primary) = assignments.profiles.iter()
                                    .find(|assignment| assignment.display.is_primary())
                                {
                                    let device_name = primary.display.device_name().to_owned();
                                    match primary_assignment_reconciler.reconcile_with_result(primary.clone()) {
                                        Ok(receiver) => Some((device_name, receiver)),
                                        Err(()) => {
                                            failures.push(DisplayTargetFailure {
                                                device_name,
                                                stage: "primary_worker_unavailable".to_owned(),
                                            });
                                            None
                                        }
                                    }
                                } else {
                                    None
                                };
                                let secondary_completion = match display_targets.reconcile_with_result(&assignments) {
                                    Ok(receiver) => Some(receiver),
                                    Err(()) => {
                                        failures.push(DisplayTargetFailure {
                                            device_name: "secondary displays".to_owned(),
                                            stage: "display_manager_unavailable".to_owned(),
                                        });
                                        None
                                    }
                                };
                                let response = request.response;
                                tokio::spawn(async move {
                                    let geometry = async move {
                                        if let Some(completion) = geometry_completion {
                                            tokio::task::spawn_blocking(move || {
                                                completion.recv_timeout(Duration::from_millis(1_250))
                                            })
                                            .await
                                            .ok()
                                            .and_then(Result::ok)
                                            .and_then(Result::ok)
                                            .is_none()
                                        } else {
                                            false
                                        }
                                    };
                                    let primary = async move {
                                        match primary_completion {
                                            Some((device_name, completion)) => {
                                                let result = completion.await;
                                                Some((device_name, result))
                                            }
                                            None => None,
                                        }
                                    };
                                    let secondary = async move {
                                        match secondary_completion {
                                            Some(completion) => Some(completion.await),
                                            None => None,
                                        }
                                    };
                                    let (geometry_failed, primary_result, secondary_result) =
                                        tokio::join!(geometry, primary, secondary);
                                    if geometry_failed {
                                        failures.push(DisplayTargetFailure {
                                            device_name: "primary display".to_owned(),
                                            stage: "geometry_apply_failed".to_owned(),
                                        });
                                    }
                                    if let Some((device_name, result)) = primary_result {
                                        match result {
                                            Ok(Ok(())) => {}
                                            Ok(Err(stage)) => failures.push(DisplayTargetFailure {
                                                device_name,
                                                stage: stage.to_owned(),
                                            }),
                                            Err(_) => failures.push(DisplayTargetFailure {
                                                device_name,
                                                stage: "primary_worker_stopped".to_owned(),
                                            }),
                                        }
                                    }
                                    if let Some(result) = secondary_result {
                                        match result {
                                            Ok(target_failures) => failures.extend(target_failures),
                                            Err(_) => failures.push(DisplayTargetFailure {
                                                device_name: "secondary displays".to_owned(),
                                                stage: "display_manager_stopped".to_owned(),
                                            }),
                                        }
                                    }
                                    failures.truncate(16);
                                    let _ = response.try_send(failures);
                                });
                            }
                            Err(stage) => {
                                tracing::warn!(
                                    component = "display",
                                    event = "display_profile_apply_failed",
                                    stage,
                                    "Could not load display profiles for the requested apply"
                                );
                                let _ = request.response.try_send(vec![DisplayTargetFailure {
                                    device_name: "display profiles".to_owned(),
                                    stage: stage.to_owned(),
                                }]);
                            }
                        }
                    }
                }
                _ = lifecycle_poll.tick() => {
                    if let Some(receiver) = display_change_receiver.as_mut() {
                        match receiver.has_changed() {
                            Ok(true) => {
                                let revision = *receiver.borrow_and_update();
                                match independent_displays::try_load_display_assignments() {
                                    Ok(assignments) => {
                                        tracing::info!(
                                            component = "display",
                                            event = "display_topology_reconciled",
                                            revision,
                                            active_profile_count = assignments.profiles.len(),
                                            primary_bounds = ?assignments.primary_bounds,
                                            "Applying the debounced display topology"
                                        );
                                        if let Some(bounds) = assignments.primary_bounds {
                                            let span_bounds = assignments.profiles.iter()
                                                .find(|assignment| assignment.display.is_primary())
                                                .and_then(|assignment| assignment.span_bounds);
                                            if let Err(error) = desktop_recovery.request_display_geometry(bounds, span_bounds) {
                                                tracing::warn!(
                                                    component = "display",
                                                    event = "primary_display_geometry_update_failed",
                                                    error_code = error.to_app_error().code.as_str(),
                                                    "The primary wallpaper worker did not accept the new geometry"
                                                );
                                            }
                                        }
                                        if let Some(primary) = assignments
                                            .profiles
                                            .iter()
                                            .find(|assignment| assignment.display.is_primary())
                                        {
                                            primary_assignment_reconciler.reconcile(primary.clone());
                                        }
                                        display_targets.reconcile(&assignments);
                                    }
                                    Err(stage) => tracing::warn!(
                                        component = "display",
                                        event = "display_topology_reconcile_skipped",
                                        revision,
                                        stage,
                                        "Keeping the active outputs because fresh display state could not be read"
                                    ),
                                }
                            }
                            Ok(false) => {}
                            Err(_) => {
                                tracing::warn!(
                                    component = "display",
                                    event = "display_topology_monitor_stopped",
                                    "Display changes will not be reconciled until restart"
                                );
                                display_change_receiver = None;
                            }
                        }
                    }
                    if let Some(receiver) = signal_receiver.as_mut() {
                        match receiver.has_changed() {
                            Ok(true) => {
                                let snapshot = receiver.borrow_and_update().clone();
                                if last_signal_snapshot.as_ref() != Some(&snapshot) {
                                    tracing::info!(
                                        component = "performance",
                                        event = "performance_signals_changed",
                                        revision = snapshot.revision,
                                        fullscreen = ?snapshot.fullscreen,
                                        session_locked = ?snapshot.session_locked,
                                        on_battery = ?snapshot.on_battery,
                                        power_state = ?snapshot.power_state,
                                        fullscreen_error_code = ?snapshot.health.fullscreen_error_code,
                                        session_error_code = ?snapshot.health.session_error_code,
                                        power_error_code = ?snapshot.health.power_error_code,
                                        power_notification_error_code = ?snapshot.health.power_notification_error_code,
                                        "Performance signal snapshot changed"
                                    );
                                    last_signal_snapshot = Some(snapshot);
                                }
                            }
                            Ok(false) => {}
                            Err(_) => {
                                tracing::warn!(
                                    component = "performance",
                                    event = "signal_monitor_stopped",
                                    "Performance signal monitor stopped; wallpaper rendering will continue"
                                );
                                signal_receiver = None;
                            }
                        }
                    }
                    loop {
                    match shell_lifecycle_monitor.try_recv() {
                        Ok(ShellLifecycleEvent::ExplorerExited { shell_process_id, host_kind }) => {
                            tracing::warn!(
                                component = "desktop",
                                event = "shell_exited",
                                shell_process_id,
                                host_kind = host_kind.as_str(),
                                "Explorer desktop host exited"
                            );
                            if !desktop_recovery.explorer_exited() {
                                tracing::error!(
                                    component = "desktop",
                                    event = "desktop_recovery_worker_stopped",
                                    "Primary display recovery stopped; other display targets will continue"
                                );
                            }
                            display_targets.explorer_exited();
                        }
                        Ok(ShellLifecycleEvent::ExplorerRestarted { shell_process_id, host_kind }) => {
                            tracing::info!(
                                component = "desktop",
                                event = "shell_restarted",
                                shell_process_id,
                                host_kind = host_kind.as_str(),
                                "Explorer desktop host became available again"
                            );
                            if !desktop_recovery.explorer_restarted() {
                                tracing::error!(
                                    component = "desktop",
                                    event = "desktop_recovery_worker_stopped",
                                    "Primary display recovery remains unavailable; other display targets continue"
                                );
                            }
                            display_targets.explorer_restarted();
                        }
                        Ok(ShellLifecycleEvent::MonitorFailed { stage, error_code }) => {
                            tracing::error!(
                                component = "desktop",
                                event = "shell_lifecycle_monitor_failed",
                                stage,
                                error_code,
                                "Explorer lifecycle monitoring failed"
                            );
                            return Err(());
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            tracing::error!(
                                component = "desktop",
                                event = "shell_lifecycle_monitor_stopped",
                                "Explorer lifecycle monitor stopped unexpectedly"
                            );
                            return Err(());
                        }
                    }
                    }
                }
            }
        }
    });
    let primary_assignment_stopped = runtime.block_on(primary_assignment_reconciler.stop());
    let playback_stopped = playback_worker.stop();
    let targets_stopped = display_targets.stop();
    if !primary_assignment_stopped {
        tracing::error!(
            component = "display",
            event = "primary_assignment_worker_join_failed",
            "Primary assignment worker may still use playback and recovery channels"
        );
    }
    if !playback_stopped || !primary_assignment_stopped {
        tracing::error!(
            component = "media",
            event = "playback_worker_join_failed",
            "Playback worker is still using recovery resources; retaining them until process exit"
        );
        // A timed-out worker may still send frames, so keep its recovery channel and window alive
        // until process termination rather than closing resources underneath it.
        std::mem::forget(playback_worker);
        std::mem::forget(desktop_recovery);
    } else {
        stop_desktop_recovery(&mut desktop_recovery);
        drop(desktop_recovery);
    }
    if !targets_stopped {
        tracing::error!(
            component = "display",
            event = "display_target_shutdown_incomplete",
            "At least one display target worker did not stop cleanly"
        );
    }
    if let Some(monitor) = signal_monitor {
        if !monitor.stop() {
            tracing::warn!(
                component = "performance",
                event = "signal_monitor_join_failed",
                "Performance signal monitor did not stop cleanly"
            );
        }
    }
    drop(shell_lifecycle_monitor);

    match run_result {
        Ok(Ok(())) => process::ExitCode::SUCCESS,
        Ok(Err(error)) => {
            tracing::error!(
                component = "host",
                event = "ipc_server_failed",
                error_code = error.code(),
                diagnostic = %error.diagnostic_summary(),
                "Wallpaper host IPC server failed"
            );
            process::ExitCode::FAILURE
        }
        Err(()) => process::ExitCode::FAILURE,
    }
}

fn stop_desktop_recovery(recovery: &mut DesktopRecoveryWorker) {
    if !recovery.stop() {
        tracing::error!(
            component = "desktop",
            event = "desktop_recovery_join_failed",
            "Desktop recovery worker did not stop cleanly"
        );
    }
}

enum StartupImageError {
    Arguments,
    Decode(MediaError),
}

fn load_startup_image() -> Result<Option<(PathBuf, DecodedImage)>, StartupImageError> {
    let mut arguments = env::args_os().skip(1);
    let Some(option) = arguments.next() else {
        return Ok(None);
    };
    if option != OsStr::new("--image") {
        return Err(StartupImageError::Arguments);
    }
    let Some(path) = arguments.next() else {
        return Err(StartupImageError::Arguments);
    };
    if arguments.next().is_some() {
        return Err(StartupImageError::Arguments);
    }

    let source_path = PathBuf::from(path);
    decode_image(&source_path)
        .map(|image| Some((source_path, image)))
        .map_err(StartupImageError::Decode)
}
