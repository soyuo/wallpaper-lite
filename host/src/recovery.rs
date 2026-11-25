use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_core::editor_settings::WallpaperEditorSettings;
use wallpaper_desktop::{AttachedDesktopWindow, discover_desktop_host};
use wallpaper_display::{DisplayRect, SpanBounds};
use wallpaper_media::{DecodedImage, DecodedVideoFrame};
use wallpaper_renderer::{D3d11Device, D3d11ImagePresenter, RendererError};

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(1);
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_DEVICE_RECOVERY_ATTEMPTS: u8 = 8;

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct RecoveryTestState {
    presenter_ready: bool,
    rendering_paused: bool,
    device_recovery_pending: bool,
    window_handle: Option<isize>,
    image_cached: bool,
    video_frame_cached: bool,
    active_source_path: Option<PathBuf>,
    settings_source_path: Option<PathBuf>,
    settings_brightness: Option<f32>,
}

pub(crate) enum RecoveryCommand {
    ExplorerExited,
    ExplorerRestarted,
    SetRenderingPaused(bool),
    Clear(Sender<()>),
    PresentImage {
        image: DecodedImage,
        source_path: PathBuf,
        response: Sender<Result<(), RendererError>>,
    },
    PresentVideoFrame {
        frame: DecodedVideoFrame,
        source_path: PathBuf,
        response: Sender<Result<(), RendererError>>,
    },
    ApplyEditorSettings {
        settings: WallpaperEditorSettings,
        source_path: PathBuf,
        response: Sender<Result<(), ApplySettingsError>>,
    },
    UpdateDisplayGeometry {
        bounds: DisplayRect,
        span_bounds: Option<SpanBounds>,
        response: Sender<Result<(), RendererError>>,
    },
    #[cfg(test)]
    SimulateDeviceLoss(Sender<()>),
    #[cfg(test)]
    InspectState(Sender<RecoveryTestState>),
    Shutdown,
}

pub(crate) struct DesktopRecoveryWorker {
    commands: Sender<RecoveryCommand>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct RecoveryFrameSender(Sender<RecoveryCommand>);

#[derive(Debug)]
pub(crate) enum ApplySettingsError {
    WallpaperNotActive,
    Renderer(RendererError),
}

impl RecoveryFrameSender {
    pub(crate) fn present_image(
        &self,
        image: DecodedImage,
        source_path: PathBuf,
    ) -> Result<(), RendererError> {
        let (response, result) = mpsc::channel();
        self.0
            .send(RecoveryCommand::PresentImage {
                image,
                source_path,
                response,
            })
            .map_err(|_| RendererError::WindowUnavailable)?;
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or(Err(RendererError::WindowUnavailable))
    }

    /// Release the wallpaper window so the Windows desktop shows again. A later
    /// present re-attaches it.
    pub(crate) fn clear(&self) -> bool {
        let (response, result) = mpsc::channel();
        self.0.send(RecoveryCommand::Clear(response)).is_ok()
            && result.recv_timeout(Duration::from_secs(2)).is_ok()
    }

    pub(crate) fn set_rendering_paused(&self, paused: bool) -> bool {
        self.0
            .send(RecoveryCommand::SetRenderingPaused(paused))
            .is_ok()
    }

    pub(crate) fn apply_editor_settings(
        &self,
        settings: WallpaperEditorSettings,
        source_path: PathBuf,
    ) -> Result<(), ApplySettingsError> {
        let (response, result) = mpsc::channel();
        self.0
            .send(RecoveryCommand::ApplyEditorSettings {
                settings,
                source_path,
                response,
            })
            .map_err(|_| ApplySettingsError::Renderer(RendererError::WindowUnavailable))?;
        result.recv().unwrap_or(Err(ApplySettingsError::Renderer(
            RendererError::WindowUnavailable,
        )))
    }

    pub(crate) fn present_video_frame(
        &self,
        frame: DecodedVideoFrame,
        source_path: PathBuf,
    ) -> Result<(), RendererError> {
        let (response, result) = mpsc::channel();
        self.0
            .send(RecoveryCommand::PresentVideoFrame {
                frame,
                source_path,
                response,
            })
            .map_err(|_| RendererError::WindowUnavailable)?;
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or(Err(RendererError::WindowUnavailable))
    }
}

impl DesktopRecoveryWorker {
    pub(crate) fn frame_sender(&self) -> RecoveryFrameSender {
        RecoveryFrameSender(self.commands.clone())
    }

    #[cfg(test)]
    fn simulate_device_loss(&self) -> bool {
        let (response, result) = mpsc::channel();
        self.commands
            .send(RecoveryCommand::SimulateDeviceLoss(response))
            .is_ok()
            && result.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    #[cfg(test)]
    fn inspect_state(&self) -> Option<RecoveryTestState> {
        let (response, result) = mpsc::channel();
        self.commands
            .send(RecoveryCommand::InspectState(response))
            .ok()?;
        result.recv_timeout(Duration::from_secs(1)).ok()
    }

    pub(crate) fn request_display_geometry(
        &self,
        bounds: DisplayRect,
        span_bounds: Option<SpanBounds>,
    ) -> Result<Receiver<Result<(), RendererError>>, RendererError> {
        let (response, completion) = mpsc::channel();
        self.commands
            .send(RecoveryCommand::UpdateDisplayGeometry {
                bounds,
                span_bounds,
                response,
            })
            .map_err(|_| RendererError::WindowUnavailable)?;
        Ok(completion)
    }

    #[cfg(test)]
    pub(crate) fn start(
        window: AttachedDesktopWindow,
        device: D3d11Device,
    ) -> std::io::Result<Self> {
        Self::start_with_span(window, device, None)
    }

    #[cfg(test)]
    pub(crate) fn start_with_span(
        window: AttachedDesktopWindow,
        device: D3d11Device,
        span_bounds: Option<SpanBounds>,
    ) -> std::io::Result<Self> {
        Self::start_with_geometry(window, device, None, span_bounds)
    }

    pub(crate) fn start_with_geometry(
        window: AttachedDesktopWindow,
        device: D3d11Device,
        display_bounds: Option<DisplayRect>,
        span_bounds: Option<SpanBounds>,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("wallpaper-desktop-recovery".to_owned())
            .spawn(move || recovery_loop(window, device, receiver, display_bounds, span_bounds))?;

        Ok(Self {
            commands,
            thread: Some(thread),
        })
    }

    pub(crate) fn explorer_exited(&self) -> bool {
        self.commands.send(RecoveryCommand::ExplorerExited).is_ok()
    }

    pub(crate) fn explorer_restarted(&self) -> bool {
        self.commands
            .send(RecoveryCommand::ExplorerRestarted)
            .is_ok()
    }

    pub(crate) fn present_image(
        &self,
        image: DecodedImage,
        source_path: PathBuf,
    ) -> Result<(), RendererError> {
        let (response, result) = mpsc::channel();
        self.commands
            .send(RecoveryCommand::PresentImage {
                image,
                source_path,
                response,
            })
            .map_err(|_| RendererError::WindowUnavailable)?;
        result
            .recv()
            .unwrap_or(Err(RendererError::WindowUnavailable))
    }

    pub(crate) fn stop(&mut self) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };

        let _ = self.commands.send(RecoveryCommand::Shutdown);
        thread.join().is_ok()
    }
}

impl Drop for DesktopRecoveryWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn recovery_loop(
    initial_window: AttachedDesktopWindow,
    mut device: D3d11Device,
    commands: Receiver<RecoveryCommand>,
    mut display_bounds: Option<DisplayRect>,
    mut span_bounds: Option<SpanBounds>,
) {
    let mut window = Some(initial_window);
    let mut presenter: Option<D3d11ImagePresenter> = None;
    let mut current_image: Option<DecodedImage> = None;
    let mut current_video_frame: Option<DecodedVideoFrame> = None;
    let mut active_source_path: Option<PathBuf> = None;
    let mut active_editor_settings: Option<(PathBuf, WallpaperEditorSettings)> = None;
    let mut rendering_paused = false;
    let mut recovery_pending = false;
    let mut device_recovery_pending = false;
    let mut device_recovery_attempts = 0_u8;
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut next_retry: Option<Instant> = None;

    loop {
        let now = Instant::now();
        let wait = next_retry
            .map(|deadline| {
                deadline
                    .saturating_duration_since(now)
                    .min(RESIZE_POLL_INTERVAL)
            })
            .unwrap_or(RESIZE_POLL_INTERVAL);
        let command = match commands.recv_timeout(wait) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        if device_recovery_pending && next_retry.is_some_and(|deadline| deadline <= Instant::now())
        {
            attempt_device_recovery(
                &mut device,
                window.as_ref(),
                current_video_frame.as_ref(),
                current_image.as_ref(),
                rendering_paused,
                span_bounds,
                active_editor_settings.as_ref(),
                active_source_path.as_deref(),
                &mut presenter,
                &mut device_recovery_pending,
                &mut recovery_pending,
                &mut device_recovery_attempts,
                &mut retry_delay,
                &mut next_retry,
            );
        }

        match command {
            Some(RecoveryCommand::SetRenderingPaused(paused)) => {
                if rendering_paused != paused {
                    rendering_paused = paused;
                    if !paused && !device_recovery_pending {
                        let redraw_result = (|| {
                            if let (Some(attached_window), Some(bounds)) =
                                (window.as_ref(), display_bounds)
                            {
                                attached_window
                                    .set_display_bounds(bounds)
                                    .map_err(|_| RendererError::WindowUnavailable)?;
                            }
                            if let Some(active_presenter) = presenter.as_mut() {
                                active_presenter.refresh_if_resized()?;
                                active_presenter.set_span_bounds(span_bounds)?;
                            }

                            if let Some(active_presenter) = presenter.as_mut() {
                                let frame_result = if let Some(frame) = current_video_frame.as_ref()
                                {
                                    active_presenter.set_video_frame(frame)
                                } else if let Some(image) = current_image.as_ref() {
                                    active_presenter.set_image(image)
                                } else {
                                    Ok(())
                                };
                                frame_result.and_then(|()| {
                                    if let Some(settings) = settings_for_source(
                                        active_editor_settings.as_ref(),
                                        active_source_path.as_deref(),
                                    ) {
                                        active_presenter.set_editor_settings(settings)?;
                                    }
                                    Ok(())
                                })
                            } else if let (Some(attached_window), Some(frame)) =
                                (window.as_ref(), current_video_frame.as_ref())
                            {
                                D3d11ImagePresenter::new_with_video_frame(
                                    &device,
                                    attached_window.raw_handle(),
                                    frame,
                                )
                                .and_then(|mut next_presenter| {
                                    next_presenter.set_span_bounds(span_bounds)?;
                                    apply_active_settings(
                                        &mut next_presenter,
                                        active_editor_settings.as_ref(),
                                        active_source_path.as_deref(),
                                    )?;
                                    presenter = Some(next_presenter);
                                    Ok(())
                                })
                            } else if let (Some(attached_window), Some(image)) =
                                (window.as_ref(), current_image.as_ref())
                            {
                                D3d11ImagePresenter::new(
                                    &device,
                                    attached_window.raw_handle(),
                                    image,
                                )
                                .and_then(|mut next_presenter| {
                                    next_presenter.set_span_bounds(span_bounds)?;
                                    apply_active_settings(
                                        &mut next_presenter,
                                        active_editor_settings.as_ref(),
                                        active_source_path.as_deref(),
                                    )?;
                                    presenter = Some(next_presenter);
                                    Ok(())
                                })
                            } else {
                                Ok(())
                            }
                        })();
                        if let Err(error) = redraw_result {
                            let device_lost = begin_device_recovery(
                                error,
                                &mut presenter,
                                &mut device_recovery_pending,
                                &mut recovery_pending,
                                &mut device_recovery_attempts,
                                &mut retry_delay,
                                &mut next_retry,
                            );
                            if !device_lost {
                                drop(presenter.take());
                                drop(window.take());
                                recovery_pending = true;
                                next_retry = Some(Instant::now() + retry_delay);
                                retry_delay = next_retry_delay(retry_delay);
                            }
                            tracing::warn!(
                                component = "renderer",
                                event = "wallpaper_render_resume_failed",
                                error_code = error.to_app_error().code.as_str(),
                                diagnostic = %error.diagnostic_summary(),
                                "Latest wallpaper frame could not be presented after policy resume; scheduling recovery"
                            );
                        }
                    }
                }
            }
            Some(RecoveryCommand::Clear(response)) => {
                drop(presenter.take());
                drop(window.take());
                current_image = None;
                current_video_frame = None;
                active_source_path = None;
                active_editor_settings = None;
                device_recovery_pending = false;
                device_recovery_attempts = 0;
                recovery_pending = false;
                next_retry = None;
                retry_delay = INITIAL_RETRY_DELAY;
                tracing::info!(
                    component = "desktop",
                    event = "wallpaper_cleared",
                    "Wallpaper released; the Windows desktop is visible"
                );
                let _ = response.send(());
            }
            Some(RecoveryCommand::ExplorerExited) => {
                drop(presenter.take());
                device_recovery_pending = false;
                device_recovery_attempts = 0;
                if window.take().is_some() {
                    tracing::info!(
                        component = "desktop",
                        event = "wallpaper_window_detached",
                        "Wallpaper child released from the previous shell"
                    );
                }
                recovery_pending = true;
                retry_delay = INITIAL_RETRY_DELAY;
                next_retry = Some(Instant::now() + retry_delay);
            }
            Some(RecoveryCommand::ExplorerRestarted) => {
                if window.is_none() {
                    device_recovery_pending = false;
                    device_recovery_attempts = 0;
                    recovery_pending = true;
                    retry_delay = INITIAL_RETRY_DELAY;
                    next_retry = Some(Instant::now());
                }
            }
            Some(RecoveryCommand::PresentImage {
                image,
                source_path,
                response,
            }) => {
                let source_changed = !active_source_path
                    .as_deref()
                    .is_some_and(|active| same_source_path(active, &source_path));
                let result = if rendering_paused {
                    Ok(())
                } else if device_recovery_pending {
                    // Keep the newest requested content while the old device is being replaced.
                    Ok(())
                } else if let Some(attached_window) = window.as_ref() {
                    match D3d11ImagePresenter::new(&device, attached_window.raw_handle(), &image)
                        .and_then(|mut next_presenter| {
                            next_presenter.set_span_bounds(span_bounds)?;
                            apply_active_settings(
                                &mut next_presenter,
                                active_editor_settings.as_ref(),
                                Some(&source_path),
                            )?;
                            Ok(next_presenter)
                        }) {
                        Ok(next_presenter) => {
                            presenter = Some(next_presenter);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    if !recovery_pending {
                        // The wallpaper was cleared; re-attach for the new content.
                        recovery_pending = true;
                        next_retry = Some(Instant::now());
                    }
                    Ok(())
                };
                if result.is_ok() {
                    current_image = Some(image);
                    current_video_frame = None;
                    active_source_path = Some(source_path);
                    if source_changed {
                        active_editor_settings = None;
                    }
                } else if let Err(error) = result {
                    if begin_device_recovery(
                        error,
                        &mut presenter,
                        &mut device_recovery_pending,
                        &mut recovery_pending,
                        &mut device_recovery_attempts,
                        &mut retry_delay,
                        &mut next_retry,
                    ) {
                        tracing::error!(
                            component = "renderer",
                            event = "wallpaper_device_lost",
                            error_code = error.to_app_error().code.as_str(),
                            diagnostic = %error.diagnostic_summary(),
                            target_local = true,
                            "D3D device loss detected; recreating this wallpaper target"
                        );
                    }
                }
                let _ = response.send(result);
            }
            Some(RecoveryCommand::PresentVideoFrame {
                frame,
                source_path,
                response,
            }) => {
                let source_changed = !active_source_path
                    .as_deref()
                    .is_some_and(|active| same_source_path(active, &source_path));
                let had_presenter = presenter.is_some();
                if source_changed || !had_presenter {
                    tracing::info!(
                        component = "renderer",
                        event = "video_frame_received",
                        rendering_paused,
                        device_recovery_pending,
                        had_presenter,
                        has_window = window.is_some(),
                        "First video frame for this source reached the desktop presenter"
                    );
                }
                let result = if rendering_paused {
                    Ok(())
                } else if device_recovery_pending {
                    // Decode may continue during recovery; present only after the device returns.
                    Ok(())
                } else if let Some(active_presenter) = presenter.as_mut() {
                    let previous_settings = if source_changed {
                        settings_for_source(
                            active_editor_settings.as_ref(),
                            active_source_path.as_deref(),
                        )
                        .cloned()
                    } else {
                        None
                    };
                    let reset_result = previous_settings
                        .as_ref()
                        .map(|settings| {
                            reset_presenter_settings_for_source_change(active_presenter, settings)
                        })
                        .unwrap_or(Ok(()));
                    reset_result.and_then(|()| active_presenter.set_video_frame(&frame))
                } else if let Some(attached_window) = window.as_ref() {
                    D3d11ImagePresenter::new_with_video_frame(
                        &device,
                        attached_window.raw_handle(),
                        &frame,
                    )
                    .and_then(|mut next_presenter| {
                        next_presenter.set_span_bounds(span_bounds)?;
                        apply_active_settings(
                            &mut next_presenter,
                            active_editor_settings.as_ref(),
                            Some(&source_path),
                        )?;
                        Ok(next_presenter)
                    })
                    .map(|next_presenter| {
                        presenter = Some(next_presenter);
                    })
                } else if !recovery_pending {
                    // The wallpaper was cleared; re-attach for the new content.
                    recovery_pending = true;
                    next_retry = Some(Instant::now());
                    Ok(())
                } else {
                    Err(RendererError::WindowUnavailable)
                };
                if result.is_ok() {
                    current_image = None;
                    current_video_frame = Some(frame);
                    if source_changed {
                        active_editor_settings = None;
                    }
                    active_source_path = Some(source_path);
                } else if let Err(error) = result {
                    if !begin_device_recovery(
                        error,
                        &mut presenter,
                        &mut device_recovery_pending,
                        &mut recovery_pending,
                        &mut device_recovery_attempts,
                        &mut retry_delay,
                        &mut next_retry,
                    ) && had_presenter
                    {
                        presenter = None;
                        window = None;
                        recovery_pending = true;
                        next_retry = Some(Instant::now());
                        retry_delay = INITIAL_RETRY_DELAY;
                    }
                }
                let _ = response.send(result);
            }
            Some(RecoveryCommand::ApplyEditorSettings {
                settings,
                source_path,
                response,
            }) => {
                let result = if active_source_path
                    .as_deref()
                    .is_some_and(|active| same_source_path(active, &source_path))
                {
                    if rendering_paused {
                        Ok(())
                    } else if let Some(active_presenter) = presenter.as_mut() {
                        active_presenter
                            .set_editor_settings(&settings)
                            .map_err(ApplySettingsError::Renderer)
                    } else if (window.is_none() || device_recovery_pending)
                        && (current_image.is_some() || current_video_frame.is_some())
                    {
                        Ok(())
                    } else {
                        Err(ApplySettingsError::Renderer(
                            RendererError::WindowUnavailable,
                        ))
                    }
                } else {
                    Err(ApplySettingsError::WallpaperNotActive)
                };
                if let Err(ApplySettingsError::Renderer(error)) = &result {
                    if begin_device_recovery(
                        *error,
                        &mut presenter,
                        &mut device_recovery_pending,
                        &mut recovery_pending,
                        &mut device_recovery_attempts,
                        &mut retry_delay,
                        &mut next_retry,
                    ) {
                        tracing::error!(
                            component = "renderer",
                            event = "wallpaper_device_lost",
                            error_code = error.to_app_error().code.as_str(),
                            diagnostic = %error.diagnostic_summary(),
                            target_local = true,
                            "D3D device loss detected while applying settings; recreating this wallpaper target"
                        );
                    }
                }
                if result.is_ok() {
                    active_editor_settings = Some((source_path, settings));
                    tracing::info!(
                        component = "renderer",
                        event = "editor_settings_applied",
                        "Applied validated editor settings to the desktop presenter"
                    );
                } else if let Err(ApplySettingsError::Renderer(error)) = &result {
                    tracing::warn!(
                        component = "renderer",
                        event = "editor_settings_apply_failed",
                        error_code = error.to_app_error().code.as_str(),
                        diagnostic = %error.diagnostic_summary(),
                        "Could not apply editor settings to the desktop presenter"
                    );
                } else if matches!(&result, Err(ApplySettingsError::WallpaperNotActive)) {
                    tracing::info!(
                        component = "renderer",
                        event = "editor_settings_rejected_inactive_wallpaper",
                        "Editor settings did not match the active desktop source"
                    );
                }
                let _ = response.send(result);
            }
            Some(RecoveryCommand::UpdateDisplayGeometry {
                bounds,
                span_bounds: next_span_bounds,
                response,
            }) => {
                let result: Result<(), RendererError> = (|| {
                    // Retain the requested geometry even if the current HWND or swap
                    // chain rejects it so recovery retries the latest topology.
                    display_bounds = Some(bounds);
                    span_bounds = next_span_bounds;
                    if !rendering_paused {
                        if let Some(attached_window) = window.as_ref() {
                            attached_window
                                .set_display_bounds(bounds)
                                .map_err(|_| RendererError::WindowUnavailable)?;
                        }
                        if let Some(active_presenter) = presenter.as_mut() {
                            active_presenter.refresh_if_resized()?;
                            active_presenter.set_span_bounds(span_bounds)?;
                        }
                    }
                    Ok(())
                })();
                let device_lost = result.as_ref().is_err_and(|error| {
                    begin_device_recovery(
                        *error,
                        &mut presenter,
                        &mut device_recovery_pending,
                        &mut recovery_pending,
                        &mut device_recovery_attempts,
                        &mut retry_delay,
                        &mut next_retry,
                    )
                });
                if result.is_err() && !device_lost {
                    drop(presenter.take());
                    drop(window.take());
                    recovery_pending = true;
                    next_retry = Some(Instant::now() + retry_delay);
                    retry_delay = next_retry_delay(retry_delay);
                }
                if let Err(error) = &result {
                    tracing::warn!(
                        component = "display",
                        event = "display_geometry_update_failed",
                        error_code = error.to_app_error().code.as_str(),
                        "The desktop wallpaper will retry applying the requested geometry"
                    );
                }
                let _ = response.send(result);
            }
            #[cfg(test)]
            Some(RecoveryCommand::SimulateDeviceLoss(response)) => {
                let _ = begin_device_recovery(
                    RendererError::Present {
                        hresult: 0x887A0005_u32 as i32,
                    },
                    &mut presenter,
                    &mut device_recovery_pending,
                    &mut recovery_pending,
                    &mut device_recovery_attempts,
                    &mut retry_delay,
                    &mut next_retry,
                );
                let _ = response.send(());
            }
            #[cfg(test)]
            Some(RecoveryCommand::InspectState(response)) => {
                let (settings_source_path, settings_brightness) = active_editor_settings
                    .as_ref()
                    .map(|(path, settings)| {
                        (Some(path.clone()), Some(settings.color().brightness()))
                    })
                    .unwrap_or((None, None));
                let _ = response.send(RecoveryTestState {
                    presenter_ready: presenter.is_some()
                        && !device_recovery_pending
                        && !recovery_pending,
                    rendering_paused,
                    device_recovery_pending,
                    window_handle: window.as_ref().map(AttachedDesktopWindow::raw_handle),
                    image_cached: current_image.is_some(),
                    video_frame_cached: current_video_frame.is_some(),
                    active_source_path: active_source_path.clone(),
                    settings_source_path,
                    settings_brightness,
                });
            }
            Some(RecoveryCommand::Shutdown) => break,
            None if device_recovery_pending
                && next_retry.is_some_and(|deadline| deadline <= Instant::now()) =>
            {
                attempt_device_recovery(
                    &mut device,
                    window.as_ref(),
                    current_video_frame.as_ref(),
                    current_image.as_ref(),
                    rendering_paused,
                    span_bounds,
                    active_editor_settings.as_ref(),
                    active_source_path.as_deref(),
                    &mut presenter,
                    &mut device_recovery_pending,
                    &mut recovery_pending,
                    &mut device_recovery_attempts,
                    &mut retry_delay,
                    &mut next_retry,
                );
            }
            None if recovery_pending
                && next_retry.is_some_and(|deadline| deadline <= Instant::now()) =>
            {
                match discover_desktop_host().and_then(|host| {
                    let host_kind = host.kind();
                    let target_bounds =
                        display_bounds.or_else(|| span_bounds.map(SpanBounds::target));
                    let attached = match target_bounds {
                        Some(bounds) => host.attach_wallpaper_window_for_display(bounds),
                        None => host.attach_wallpaper_window(),
                    };
                    attached.map(|window| (window, host_kind))
                }) {
                    Ok((attached_window, host_kind)) => {
                        drop(presenter.take());
                        window = Some(attached_window);
                        recovery_pending = false;
                        next_retry = None;
                        retry_delay = INITIAL_RETRY_DELAY;
                        tracing::info!(
                            component = "desktop",
                            event = "wallpaper_window_recovered",
                            host_kind = host_kind.as_str(),
                            "Wallpaper window reattached to the current shell"
                        );
                        if rendering_paused {
                            continue;
                        }
                        if let Some(attached_window) = window.as_ref() {
                            let restored = if let Some(frame) = current_video_frame.as_ref() {
                                D3d11ImagePresenter::new_with_video_frame(
                                    &device,
                                    attached_window.raw_handle(),
                                    frame,
                                )
                                .and_then(|mut next_presenter| {
                                    next_presenter.set_span_bounds(span_bounds)?;
                                    apply_active_settings(
                                        &mut next_presenter,
                                        active_editor_settings.as_ref(),
                                        active_source_path.as_deref(),
                                    )?;
                                    Ok(next_presenter)
                                })
                            } else if let Some(image) = current_image.as_ref() {
                                D3d11ImagePresenter::new(
                                    &device,
                                    attached_window.raw_handle(),
                                    image,
                                )
                                .and_then(|mut next_presenter| {
                                    next_presenter.set_span_bounds(span_bounds)?;
                                    apply_active_settings(
                                        &mut next_presenter,
                                        active_editor_settings.as_ref(),
                                        active_source_path.as_deref(),
                                    )?;
                                    Ok(next_presenter)
                                })
                            } else {
                                continue;
                            };
                            match restored {
                                Ok(next_presenter) => {
                                    presenter = Some(next_presenter);
                                    tracing::info!(
                                        component = "renderer",
                                        event = "wallpaper_image_restored",
                                        "Wallpaper image presented after shell restart"
                                    );
                                }
                                Err(error) => {
                                    if begin_device_recovery(
                                        error,
                                        &mut presenter,
                                        &mut device_recovery_pending,
                                        &mut recovery_pending,
                                        &mut device_recovery_attempts,
                                        &mut retry_delay,
                                        &mut next_retry,
                                    ) {
                                        tracing::error!(
                                            component = "renderer",
                                            event = "wallpaper_device_lost",
                                            error_code = error.to_app_error().code.as_str(),
                                            diagnostic = %error.diagnostic_summary(),
                                            target_local = true,
                                            "D3D device loss detected while restoring the wallpaper; recreating this target"
                                        );
                                    } else {
                                        tracing::error!(
                                            component = "renderer",
                                            event = "wallpaper_image_restore_failed",
                                            error_code = error.to_app_error().code.as_str(),
                                            diagnostic = %error.diagnostic_summary(),
                                            "Wallpaper image could not be restored after shell restart"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            component = "desktop",
                            event = "wallpaper_window_recovery_retry",
                            error_code = error.to_app_error().code.as_str(),
                            diagnostic = %error.diagnostic_summary(),
                            retry_delay_ms = retry_delay.as_millis() as u64,
                            "Wallpaper window recovery will retry"
                        );
                        next_retry = Some(Instant::now() + retry_delay);
                        retry_delay = next_retry_delay(retry_delay);
                    }
                }
            }
            None => {
                if !rendering_paused && let Some(active_presenter) = presenter.as_mut() {
                    match active_presenter.refresh_if_resized() {
                        Ok(true) => tracing::info!(
                            component = "renderer",
                            event = "wallpaper_image_resized",
                            "Wallpaper image presented after desktop child resize"
                        ),
                        Ok(false) => {}
                        Err(error) => {
                            if !begin_device_recovery(
                                error,
                                &mut presenter,
                                &mut device_recovery_pending,
                                &mut recovery_pending,
                                &mut device_recovery_attempts,
                                &mut retry_delay,
                                &mut next_retry,
                            ) {
                                tracing::error!(
                                    component = "renderer",
                                    event = "wallpaper_image_resize_failed",
                                    error_code = error.to_app_error().code.as_str(),
                                    diagnostic = %error.diagnostic_summary(),
                                    "Wallpaper image could not be presented after resize"
                                );
                                drop(presenter.take());
                                recovery_pending = true;
                                next_retry = Some(Instant::now() + retry_delay);
                                retry_delay = next_retry_delay(retry_delay);
                            }
                        }
                    }
                }
            }
        }
    }

    drop(presenter);
    drop(window);
}

fn begin_device_recovery(
    error: RendererError,
    presenter: &mut Option<D3d11ImagePresenter>,
    device_recovery_pending: &mut bool,
    recovery_pending: &mut bool,
    attempts: &mut u8,
    retry_delay: &mut Duration,
    next_retry: &mut Option<Instant>,
) -> bool {
    if !error.is_device_lost() {
        return false;
    }

    drop(presenter.take());
    *device_recovery_pending = true;
    *recovery_pending = false;
    *attempts = 0;
    *retry_delay = INITIAL_RETRY_DELAY;
    *next_retry = Some(Instant::now() + *retry_delay);
    *retry_delay = next_retry_delay(*retry_delay);
    true
}

fn schedule_device_retry(
    attempts: u8,
    retry_delay: &mut Duration,
    next_retry: &mut Option<Instant>,
) -> bool {
    if attempts >= MAX_DEVICE_RECOVERY_ATTEMPTS {
        *next_retry = None;
        return false;
    }

    *next_retry = Some(Instant::now() + *retry_delay);
    *retry_delay = next_retry_delay(*retry_delay);
    true
}

fn finish_device_recovery_as_terminal(
    device_recovery_pending: &mut bool,
    recovery_pending: &mut bool,
    next_retry: &mut Option<Instant>,
) {
    // Keep the target in its failed state after the bounded retry budget is
    // exhausted so later video frames cannot restart recovery indefinitely.
    *device_recovery_pending = true;
    *recovery_pending = false;
    *next_retry = None;
}

#[allow(clippy::too_many_arguments)]
fn attempt_device_recovery(
    device: &mut D3d11Device,
    window: Option<&AttachedDesktopWindow>,
    current_video_frame: Option<&DecodedVideoFrame>,
    current_image: Option<&DecodedImage>,
    rendering_paused: bool,
    span_bounds: Option<SpanBounds>,
    active_editor_settings: Option<&(PathBuf, WallpaperEditorSettings)>,
    active_source_path: Option<&Path>,
    presenter: &mut Option<D3d11ImagePresenter>,
    device_recovery_pending: &mut bool,
    recovery_pending: &mut bool,
    attempts: &mut u8,
    retry_delay: &mut Duration,
    next_retry: &mut Option<Instant>,
) {
    *attempts = attempts.saturating_add(1);
    drop(presenter.take());

    let (result, device_creation_failed) = match D3d11Device::create_hardware() {
        Ok(replacement) => {
            *device = replacement;
            let result = if rendering_paused {
                Ok(None)
            } else {
                window
                    .ok_or(RendererError::WindowUnavailable)
                    .and_then(|attached_window| {
                        create_presenter_for_current(
                            device,
                            attached_window,
                            current_video_frame,
                            current_image,
                            span_bounds,
                            active_editor_settings,
                            active_source_path,
                        )
                    })
            };
            (result, false)
        }
        Err(error) => (Err(error), true),
    };

    match result {
        Ok(restored_presenter) => {
            *presenter = restored_presenter;
            *device_recovery_pending = false;
            *recovery_pending = false;
            *attempts = 0;
            *next_retry = None;
            *retry_delay = INITIAL_RETRY_DELAY;
            tracing::info!(
                component = "renderer",
                event = "wallpaper_device_recovered",
                target_local = true,
                "Recreated the D3D device and restored the last committed wallpaper"
            );
        }
        Err(error) => {
            let retryable = device_creation_failed || error.is_device_lost();
            let retry = retryable && schedule_device_retry(*attempts, retry_delay, next_retry);
            if !retry {
                finish_device_recovery_as_terminal(
                    device_recovery_pending,
                    recovery_pending,
                    next_retry,
                );
                tracing::error!(
                    component = "renderer",
                    event = "wallpaper_device_recovery_exhausted",
                    error_code = error.to_app_error().code.as_str(),
                    diagnostic = %error.diagnostic_summary(),
                    attempts = *attempts,
                    target_local = true,
                    "D3D device recovery stopped after a bounded number of attempts"
                );
            } else {
                tracing::warn!(
                    component = "renderer",
                    event = "wallpaper_device_recovery_retry",
                    error_code = error.to_app_error().code.as_str(),
                    diagnostic = %error.diagnostic_summary(),
                    attempts = *attempts,
                    retry_delay_ms = next_retry
                        .map(|deadline| deadline.saturating_duration_since(Instant::now()).as_millis() as u64)
                        .unwrap_or_default(),
                    target_local = true,
                    "D3D device recovery will retry for this wallpaper target"
                );
            }
        }
    }
}

fn create_presenter_for_current(
    device: &D3d11Device,
    window: &AttachedDesktopWindow,
    current_video_frame: Option<&DecodedVideoFrame>,
    current_image: Option<&DecodedImage>,
    span_bounds: Option<SpanBounds>,
    active_editor_settings: Option<&(PathBuf, WallpaperEditorSettings)>,
    active_source_path: Option<&Path>,
) -> Result<Option<D3d11ImagePresenter>, RendererError> {
    let mut presenter = if let Some(frame) = current_video_frame {
        Some(D3d11ImagePresenter::new_with_video_frame(
            device,
            window.raw_handle(),
            frame,
        )?)
    } else if let Some(image) = current_image {
        Some(D3d11ImagePresenter::new(
            device,
            window.raw_handle(),
            image,
        )?)
    } else {
        None
    };

    if let Some(presenter) = presenter.as_mut() {
        presenter.set_span_bounds(span_bounds)?;
        apply_active_settings(presenter, active_editor_settings, active_source_path)?;
    }
    Ok(presenter)
}

fn apply_active_settings(
    presenter: &mut D3d11ImagePresenter,
    settings: Option<&(PathBuf, WallpaperEditorSettings)>,
    source_path: Option<&Path>,
) -> Result<(), RendererError> {
    if let Some(settings) = settings_for_source(settings, source_path) {
        presenter.set_editor_settings(settings)?;
    }
    Ok(())
}

fn settings_for_source<'a>(
    settings: Option<&'a (PathBuf, WallpaperEditorSettings)>,
    source_path: Option<&Path>,
) -> Option<&'a WallpaperEditorSettings> {
    let (settings_path, settings) = settings?;
    source_path
        .is_some_and(|source_path| same_source_path(settings_path, source_path))
        .then_some(settings)
}

fn reset_presenter_settings_for_source_change(
    presenter: &mut D3d11ImagePresenter,
    previous_settings: &WallpaperEditorSettings,
) -> Result<(), RendererError> {
    let defaults = WallpaperEditorSettings::default_for(previous_settings.wallpaper_id())
        .map_err(|_| RendererError::InvalidEditorSettings)?;
    presenter.set_editor_settings(&defaults)
}

fn same_source_path(active: &Path, requested: &Path) -> bool {
    if active == requested {
        return true;
    }
    let active = active
        .canonicalize()
        .unwrap_or_else(|_| active.to_path_buf());
    let requested = requested
        .canonicalize()
        .unwrap_or_else(|_| requested.to_path_buf());
    active
        .to_string_lossy()
        .eq_ignore_ascii_case(&requested.to_string_lossy())
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        ApplySettingsError, DesktopRecoveryWorker, INITIAL_RETRY_DELAY,
        MAX_DEVICE_RECOVERY_ATTEMPTS, MAX_RETRY_DELAY, finish_device_recovery_as_terminal,
        next_retry_delay, schedule_device_retry, settings_for_source,
    };
    use wallpaper_core::editor_settings::{EditorColorSettings, WallpaperEditorSettings};
    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_media::{VideoReadEvent, VideoSource, decode_image};
    use wallpaper_renderer::D3d11Device;

    #[test]
    fn retry_backoff_doubles_to_a_one_second_cap() {
        let mut delay = INITIAL_RETRY_DELAY;
        let mut observed = vec![delay];
        for _ in 0..5 {
            delay = next_retry_delay(delay);
            observed.push(delay);
        }

        assert_eq!(
            observed,
            [
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(400),
                Duration::from_millis(800),
                Duration::from_secs(1),
                MAX_RETRY_DELAY,
            ]
        );
    }

    #[test]
    fn device_recovery_retry_count_and_terminal_state_are_bounded() {
        let mut delay = INITIAL_RETRY_DELAY;
        let mut next_retry = None;
        let mut device_recovery_pending = false;
        let mut recovery_pending = true;
        for attempt in 1..=MAX_DEVICE_RECOVERY_ATTEMPTS {
            assert_eq!(
                schedule_device_retry(attempt, &mut delay, &mut next_retry),
                attempt < MAX_DEVICE_RECOVERY_ATTEMPTS
            );
        }
        assert!(next_retry.is_none());

        finish_device_recovery_as_terminal(
            &mut device_recovery_pending,
            &mut recovery_pending,
            &mut next_retry,
        );
        assert!(device_recovery_pending);
        assert!(!recovery_pending);
        assert!(next_retry.is_none());
    }

    #[test]
    fn stopping_the_recovery_worker_is_idempotent() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");

        assert!(recovery.explorer_exited());
        assert!(recovery.explorer_restarted());
        assert!(recovery.stop());
        assert!(recovery.stop());
    }

    #[test]
    fn cached_editor_settings_only_match_their_source_path() {
        let source = PathBuf::from(r"C:\wallpapers\first.png");
        let other = PathBuf::from(r"C:\wallpapers\second.png");
        let settings = WallpaperEditorSettings::default_for(1)
            .expect("a valid settings snapshot should be available");
        let active_settings = Some((source.clone(), settings));

        assert!(settings_for_source(active_settings.as_ref(), Some(&source)).is_some());
        assert!(settings_for_source(active_settings.as_ref(), Some(&other)).is_none());
        assert!(settings_for_source(active_settings.as_ref(), None).is_none());
    }

    #[test]
    fn presented_image_resources_release_when_the_recovery_worker_stops() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crates")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join("pixels.png");
        let image = decode_image(&fixture).expect("fixture should decode");
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");

        recovery
            .present_image(image, fixture.clone())
            .expect("the recovery worker should present the image");
        let sender = recovery.frame_sender();
        let settings = WallpaperEditorSettings::default_for(1)
            .expect("a valid settings snapshot should be available");
        sender
            .apply_editor_settings(settings.clone(), fixture.clone())
            .expect("settings for the active source should apply");
        assert!(matches!(
            sender
                .apply_editor_settings(settings, fixture.with_file_name("different-wallpaper.png")),
            Err(ApplySettingsError::WallpaperNotActive)
        ));

        let video_path = fixture.with_file_name("short-h264.mp4");
        let mut video = VideoSource::open(&video_path).expect("video fixture should open");
        let frame = loop {
            match video.read_next().expect("video frame should decode") {
                VideoReadEvent::Frame(frame) => break frame,
                VideoReadEvent::StreamTick { .. } => continue,
                VideoReadEvent::EndOfStream => panic!("video fixture should contain a frame"),
            }
        };
        sender
            .present_video_frame(frame, video_path.clone())
            .expect("the video frame should replace the active presenter image");
        let video_settings = WallpaperEditorSettings::default_for(2)
            .expect("a valid video settings snapshot should be available");
        sender
            .apply_editor_settings(video_settings.clone(), video_path.clone())
            .expect("settings for the active video should apply");
        assert!(matches!(
            sender.apply_editor_settings(video_settings, fixture),
            Err(ApplySettingsError::WallpaperNotActive)
        ));
        assert!(recovery.stop());
    }

    #[test]
    fn device_recreation_restores_last_image_settings_and_same_child_window() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let original_window_handle = window.raw_handle();
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crates")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join("pixels.png");
        let image = decode_image(&fixture).expect("fixture should decode");
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");
        let sender = recovery.frame_sender();
        sender
            .present_image(image, fixture.clone())
            .expect("the recovery worker should present the image");

        let settings = WallpaperEditorSettings::default_for(1)
            .expect("default editor settings should be valid")
            .with_color(
                EditorColorSettings::new(1.5, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0)
                    .expect("non-default brightness should be valid"),
            )
            .expect("non-default editor settings should be valid");
        sender
            .apply_editor_settings(settings, fixture.clone())
            .expect("settings for the active source should apply");
        assert!(recovery.simulate_device_loss());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let restored = loop {
            let state = recovery
                .inspect_state()
                .expect("the recovery worker should answer state probes");
            if state.presenter_ready {
                break state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the target-local D3D recovery should complete within five seconds"
            );
            std::thread::sleep(Duration::from_millis(25));
        };

        assert_eq!(restored.window_handle, Some(original_window_handle));
        assert!(restored.image_cached);
        assert!(!restored.video_frame_cached);
        assert_eq!(restored.active_source_path, Some(fixture.clone()));
        assert_eq!(restored.settings_source_path, Some(fixture));
        assert_eq!(restored.settings_brightness, Some(1.5));
        assert!(recovery.stop());
    }

    #[test]
    fn device_recreation_restores_last_video_frame_and_same_child_window() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let original_window_handle = window.raw_handle();
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crates")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join("short-h264.mp4");
        let mut video = VideoSource::open(&fixture).expect("video fixture should open");
        let frame = loop {
            match video.read_next().expect("video frame should decode") {
                VideoReadEvent::Frame(frame) => break frame,
                VideoReadEvent::StreamTick { .. } => continue,
                VideoReadEvent::EndOfStream => panic!("video fixture should contain a frame"),
            }
        };
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");
        let sender = recovery.frame_sender();
        sender
            .present_video_frame(frame, fixture.clone())
            .expect("the recovery worker should present the video frame");

        let settings = WallpaperEditorSettings::default_for(2)
            .expect("default editor settings should be valid")
            .with_color(
                EditorColorSettings::new(1.5, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0)
                    .expect("non-default brightness should be valid"),
            )
            .expect("non-default editor settings should be valid");
        sender
            .apply_editor_settings(settings, fixture.clone())
            .expect("settings for the active video should apply");
        assert!(recovery.simulate_device_loss());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let restored = loop {
            let state = recovery
                .inspect_state()
                .expect("the recovery worker should answer state probes");
            if state.presenter_ready {
                break state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the target-local D3D recovery should complete within five seconds"
            );
            std::thread::sleep(Duration::from_millis(25));
        };

        assert_eq!(restored.window_handle, Some(original_window_handle));
        assert!(!restored.image_cached);
        assert!(restored.video_frame_cached);
        assert_eq!(restored.active_source_path, Some(fixture.clone()));
        assert_eq!(restored.settings_source_path, Some(fixture));
        assert_eq!(restored.settings_brightness, Some(1.5));
        assert!(recovery.stop());
    }

    #[test]
    fn device_recovery_defers_presenter_recreation_while_rendering_is_paused() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let original_window_handle = window.raw_handle();
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crates")
            .join("media")
            .join("tests")
            .join("fixtures")
            .join("pixels.png");
        let image = decode_image(&fixture).expect("fixture should decode");
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");
        let sender = recovery.frame_sender();
        sender
            .present_image(image, fixture.clone())
            .expect("the recovery worker should present the image");
        assert!(sender.set_rendering_paused(true));

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let state = recovery
                .inspect_state()
                .expect("the recovery worker should answer state probes");
            if state.rendering_paused {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the rendering pause command should be processed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(recovery.simulate_device_loss());
        let recovered_while_paused = loop {
            let state = recovery
                .inspect_state()
                .expect("the recovery worker should answer state probes");
            if !state.device_recovery_pending {
                break state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the replacement device should be created within two seconds"
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(recovered_while_paused.rendering_paused);
        assert!(!recovered_while_paused.presenter_ready);
        assert!(recovered_while_paused.image_cached);
        assert_eq!(
            recovered_while_paused.window_handle,
            Some(original_window_handle)
        );

        assert!(sender.set_rendering_paused(false));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let state = recovery
                .inspect_state()
                .expect("the recovery worker should answer state probes");
            if state.presenter_ready {
                assert!(!state.rendering_paused);
                assert_eq!(state.window_handle, Some(original_window_handle));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the cached image should be presented after rendering resumes"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(recovery.stop());
    }
}
