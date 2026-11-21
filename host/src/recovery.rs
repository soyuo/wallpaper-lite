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

pub(crate) enum RecoveryCommand {
    ExplorerExited,
    ExplorerRestarted,
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
    device: D3d11Device,
    commands: Receiver<RecoveryCommand>,
    mut display_bounds: Option<DisplayRect>,
    mut span_bounds: Option<SpanBounds>,
) {
    let mut window = Some(initial_window);
    let mut presenter = None;
    let mut current_image: Option<DecodedImage> = None;
    let mut current_video_frame: Option<DecodedVideoFrame> = None;
    let mut active_source_path: Option<PathBuf> = None;
    let mut active_editor_settings: Option<(PathBuf, WallpaperEditorSettings)> = None;
    let mut recovery_pending = false;
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

        match command {
            Some(RecoveryCommand::ExplorerExited) => {
                drop(presenter.take());
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
                let result = if let Some(attached_window) = window.as_ref() {
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
                            current_image = Some(image);
                            current_video_frame = None;
                            active_source_path = Some(source_path);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    current_image = Some(image);
                    current_video_frame = None;
                    active_source_path = Some(source_path);
                    Ok(())
                };
                if result.is_ok() && source_changed {
                    active_editor_settings = None;
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
                let result = if let Some(active_presenter) = presenter.as_mut() {
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
                } else if had_presenter {
                    presenter = None;
                    window = None;
                    recovery_pending = true;
                    next_retry = Some(Instant::now());
                    retry_delay = INITIAL_RETRY_DELAY;
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
                    if let Some(active_presenter) = presenter.as_mut() {
                        active_presenter
                            .set_editor_settings(&settings)
                            .map_err(ApplySettingsError::Renderer)
                    } else if window.is_none()
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
                    if let Some(attached_window) = window.as_ref() {
                        attached_window
                            .set_display_bounds(bounds)
                            .map_err(|_| RendererError::WindowUnavailable)?;
                    }
                    if let Some(active_presenter) = presenter.as_mut() {
                        active_presenter.refresh_if_resized()?;
                        active_presenter.set_span_bounds(span_bounds)?;
                    }
                    Ok(())
                })();
                if result.is_err() {
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
                        "The desktop wallpaper will retry attachment with the requested geometry"
                    );
                }
                let _ = response.send(result);
            }
            Some(RecoveryCommand::Shutdown) => break,
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
                                Err(error) => tracing::error!(
                                    component = "renderer",
                                    event = "wallpaper_image_restore_failed",
                                    error_code = error.to_app_error().code.as_str(),
                                    diagnostic = %error.diagnostic_summary(),
                                    "Wallpaper image could not be restored after shell restart"
                                ),
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
                if let Some(active_presenter) = presenter.as_mut() {
                    match active_presenter.refresh_if_resized() {
                        Ok(true) => tracing::info!(
                            component = "renderer",
                            event = "wallpaper_image_resized",
                            "Wallpaper image presented after desktop child resize"
                        ),
                        Ok(false) => {}
                        Err(error) => {
                            tracing::error!(
                                component = "renderer",
                                event = "wallpaper_image_resize_failed",
                                error_code = error.to_app_error().code.as_str(),
                                diagnostic = %error.diagnostic_summary(),
                                "Wallpaper image could not be presented after resize"
                            );
                            drop(presenter.take());
                        }
                    }
                }
            }
        }
    }

    drop(presenter);
    drop(window);
}

fn apply_active_settings(
    presenter: &mut D3d11ImagePresenter<'_>,
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
    presenter: &mut D3d11ImagePresenter<'_>,
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
        ApplySettingsError, DesktopRecoveryWorker, INITIAL_RETRY_DELAY, MAX_RETRY_DELAY,
        next_retry_delay, settings_for_source,
    };
    use wallpaper_core::editor_settings::WallpaperEditorSettings;
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
}
