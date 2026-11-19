use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_desktop::{AttachedDesktopWindow, discover_desktop_host};

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(1);

enum RecoveryCommand {
    ExplorerExited,
    ExplorerRestarted,
    Shutdown,
}

pub(crate) struct DesktopRecoveryWorker {
    commands: Sender<RecoveryCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DesktopRecoveryWorker {
    pub(crate) fn start(window: AttachedDesktopWindow) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("wallpaper-desktop-recovery".to_owned())
            .spawn(move || recovery_loop(window, receiver))?;

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
}

impl Drop for DesktopRecoveryWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(RecoveryCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn recovery_loop(initial_window: AttachedDesktopWindow, commands: Receiver<RecoveryCommand>) {
    let mut window = Some(initial_window);
    let mut recovery_pending = false;
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut next_retry: Option<Instant> = None;

    loop {
        let command = match next_retry {
            Some(deadline) => {
                match commands.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            },
        };

        match command {
            Some(RecoveryCommand::ExplorerExited) => {
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
            Some(RecoveryCommand::Shutdown) => break,
            None if recovery_pending => {
                match discover_desktop_host().and_then(|host| {
                    let host_kind = host.kind();
                    host.attach_wallpaper_window()
                        .map(|window| (window, host_kind))
                }) {
                    Ok((attached_window, host_kind)) => {
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
            None => {}
        }
    }

    drop(window);
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{INITIAL_RETRY_DELAY, MAX_RETRY_DELAY, next_retry_delay};

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
}
