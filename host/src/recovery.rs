use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_desktop::{AttachedDesktopWindow, discover_desktop_host};
use wallpaper_media::DecodedImage;
use wallpaper_renderer::{D3d11Device, D3d11ImagePresenter, RendererError};

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(1);
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);

enum RecoveryCommand {
    ExplorerExited,
    ExplorerRestarted,
    PresentImage {
        image: DecodedImage,
        response: Sender<Result<(), RendererError>>,
    },
    Shutdown,
}

pub(crate) struct DesktopRecoveryWorker {
    commands: Sender<RecoveryCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DesktopRecoveryWorker {
    pub(crate) fn start(
        window: AttachedDesktopWindow,
        device: D3d11Device,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("wallpaper-desktop-recovery".to_owned())
            .spawn(move || recovery_loop(window, device, receiver))?;

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

    pub(crate) fn present_image(&self, image: DecodedImage) -> Result<(), RendererError> {
        let (response, result) = mpsc::channel();
        self.commands
            .send(RecoveryCommand::PresentImage { image, response })
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
) {
    let mut window = Some(initial_window);
    let mut presenter = None;
    let mut current_image: Option<DecodedImage> = None;
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
            Some(RecoveryCommand::PresentImage { image, response }) => {
                let result = if let Some(attached_window) = window.as_ref() {
                    match D3d11ImagePresenter::new(&device, attached_window.raw_handle(), &image) {
                        Ok(next_presenter) => {
                            presenter = Some(next_presenter);
                            current_image = Some(image);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    current_image = Some(image);
                    Ok(())
                };
                let _ = response.send(result);
            }
            Some(RecoveryCommand::Shutdown) => break,
            None if recovery_pending
                && next_retry.is_some_and(|deadline| deadline <= Instant::now()) =>
            {
                match discover_desktop_host().and_then(|host| {
                    let host_kind = host.kind();
                    host.attach_wallpaper_window()
                        .map(|window| (window, host_kind))
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
                        if let (Some(image), Some(attached_window)) =
                            (current_image.as_ref(), window.as_ref())
                        {
                            match D3d11ImagePresenter::new(
                                &device,
                                attached_window.raw_handle(),
                                image,
                            ) {
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

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DesktopRecoveryWorker, INITIAL_RETRY_DELAY, MAX_RETRY_DELAY, next_retry_delay};
    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_media::decode_image;
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
        let image = decode_image(fixture).expect("fixture should decode");
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");

        recovery
            .present_image(image)
            .expect("the recovery worker should present the image");
        assert!(recovery.stop());
    }
}
