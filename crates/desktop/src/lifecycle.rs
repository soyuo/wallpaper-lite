use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use windows::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

use crate::{DesktopError, DesktopHost, ShellLifecycleEvent};

const PROCESS_POLL_MILLIS: u32 = 250;
const EVENT_RETRY_DELAY: Duration = Duration::from_millis(25);
const DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(100);
const EVENT_QUEUE_CAPACITY: usize = 4;

/// Monitors the Explorer process that owns a validated desktop host.
///
/// The monitor uses a waitable process handle plus bounded window validation polling. It emits
/// one exit event when the current shell host becomes unavailable, then one restart event after a
/// new validated desktop host appears. Dropping the monitor stops and joins its worker thread.
pub struct ShellLifecycleMonitor {
    stop: Arc<AtomicBool>,
    receiver: Receiver<ShellLifecycleEvent>,
    thread: Option<JoinHandle<()>>,
}

impl ShellLifecycleMonitor {
    pub fn start(initial_host: DesktopHost) -> Result<Self, DesktopError> {
        let (event_sender, receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);

        let thread = thread::Builder::new()
            .name("wallpaper-shell-lifecycle".into())
            .spawn(move || {
                let process_handle = match open_process(initial_host.shell_process_id) {
                    Ok(handle) => handle,
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        return;
                    }
                };
                if ready_sender.send(Ok(())).is_err() {
                    return;
                }

                monitor_loop(initial_host, process_handle, worker_stop, event_sender);
            })
            .map_err(|error| DesktopError::ShellLifecycleMonitorThreadFailed {
                os_error_code: error.raw_os_error(),
            })?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                receiver,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(DesktopError::ShellLifecycleMonitorThreadFailed {
                    os_error_code: None,
                })
            }
        }
    }

    pub fn try_recv(&self) -> Result<ShellLifecycleEvent, TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Drop for ShellLifecycleMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open_process(process_id: u32) -> Result<ProcessHandle, DesktopError> {
    unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) }
        .map(ProcessHandle)
        .map_err(
            |error| DesktopError::ShellLifecycleMonitorProcessOpenFailed {
                hresult: error.code().0,
            },
        )
}

fn monitor_loop(
    initial_host: DesktopHost,
    mut process_handle: ProcessHandle,
    stop: Arc<AtomicBool>,
    event_sender: SyncSender<ShellLifecycleEvent>,
) {
    let mut state = LifecycleState::active(initial_host);

    while !stop.load(Ordering::Acquire) {
        let current_host = state.current.expect("active monitor state has a host");
        let wait_result = unsafe { WaitForSingleObject(process_handle.0, PROCESS_POLL_MILLIS).0 };

        let shell_unavailable = if wait_result == WAIT_OBJECT_0.0 {
            true
        } else if wait_result == WAIT_TIMEOUT.0 {
            current_host.validate_current().is_err()
        } else {
            let error_code = if wait_result == WAIT_FAILED.0 {
                unsafe { GetLastError() }.0
            } else {
                wait_result
            };
            let _ = send_event(
                &event_sender,
                &stop,
                ShellLifecycleEvent::MonitorFailed {
                    stage: "wait_for_process",
                    error_code,
                },
            );
            return;
        };

        if !shell_unavailable {
            continue;
        }

        let Some(exit_event) = state.mark_exited() else {
            continue;
        };
        if !send_event(&event_sender, &stop, exit_event) {
            return;
        }
        drop(process_handle);

        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }

            match crate::discover_desktop_host() {
                Ok(next_host) if next_host.validate_current().is_ok() => {
                    let next_process = match open_process(next_host.shell_process_id) {
                        Ok(handle) => handle,
                        Err(error) if next_host.validate_current().is_err() => {
                            let _ = error;
                            thread::sleep(DISCOVERY_RETRY_DELAY);
                            continue;
                        }
                        Err(error) => {
                            let _ = send_event(
                                &event_sender,
                                &stop,
                                ShellLifecycleEvent::MonitorFailed {
                                    stage: "open_process",
                                    error_code: error_code_from_hresult(error),
                                },
                            );
                            return;
                        }
                    };

                    let Some(restart_event) = state.mark_restarted(next_host) else {
                        continue;
                    };
                    if !send_event(&event_sender, &stop, restart_event) {
                        return;
                    }
                    process_handle = next_process;
                    break;
                }
                _ => thread::sleep(DISCOVERY_RETRY_DELAY),
            }
        }
    }
}

fn error_code_from_hresult(error: DesktopError) -> u32 {
    match error {
        DesktopError::ShellLifecycleMonitorProcessOpenFailed { hresult } => hresult as u32,
        _ => 0,
    }
}

fn send_event(
    sender: &SyncSender<ShellLifecycleEvent>,
    stop: &AtomicBool,
    mut event: ShellLifecycleEvent,
) -> bool {
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }

        match sender.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned_event)) => {
                event = returned_event;
                thread::sleep(EVENT_RETRY_DELAY);
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

#[derive(Clone, Copy)]
struct LifecycleState {
    current: Option<DesktopHost>,
}

impl LifecycleState {
    fn active(host: DesktopHost) -> Self {
        Self {
            current: Some(host),
        }
    }

    fn mark_exited(&mut self) -> Option<ShellLifecycleEvent> {
        let host = self.current.take()?;
        Some(ShellLifecycleEvent::ExplorerExited {
            shell_process_id: host.shell_process_id,
            host_kind: host.kind(),
        })
    }

    fn mark_restarted(&mut self, host: DesktopHost) -> Option<ShellLifecycleEvent> {
        if self.current.is_some() {
            return None;
        }

        self.current = Some(host);
        Some(ShellLifecycleEvent::ExplorerRestarted {
            shell_process_id: host.shell_process_id,
            host_kind: host.kind(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::{LifecycleState, ShellLifecycleMonitor};
    use crate::{DesktopHost, DesktopHostKind, ShellLifecycleEvent, discover_desktop_host};

    #[test]
    fn lifecycle_transitions_are_ordered_and_idempotent_across_fast_restarts() {
        let first = DesktopHost::new(10, 100, DesktopHostKind::Progman);
        let second = DesktopHost::new(20, 200, DesktopHostKind::WorkerW);
        let third = DesktopHost::new(30, 300, DesktopHostKind::Progman);
        let mut state = LifecycleState::active(first);

        assert_eq!(
            state.mark_exited(),
            Some(ShellLifecycleEvent::ExplorerExited {
                shell_process_id: 100,
                host_kind: DesktopHostKind::Progman,
            })
        );
        assert_eq!(state.mark_exited(), None);
        assert_eq!(
            state.mark_restarted(second),
            Some(ShellLifecycleEvent::ExplorerRestarted {
                shell_process_id: 200,
                host_kind: DesktopHostKind::WorkerW,
            })
        );
        assert_eq!(state.mark_restarted(second), None);
        assert_eq!(
            state.mark_exited(),
            Some(ShellLifecycleEvent::ExplorerExited {
                shell_process_id: 200,
                host_kind: DesktopHostKind::WorkerW,
            })
        );
        assert_eq!(
            state.mark_restarted(third),
            Some(ShellLifecycleEvent::ExplorerRestarted {
                shell_process_id: 300,
                host_kind: DesktopHostKind::Progman,
            })
        );
    }

    #[test]
    fn monitor_rejects_a_missing_process_and_does_not_leak_a_thread() {
        let missing_process = DesktopHost::new(0, u32::MAX, DesktopHostKind::Progman);
        assert!(matches!(
            ShellLifecycleMonitor::start(missing_process),
            Err(crate::DesktopError::ShellLifecycleMonitorProcessOpenFailed { .. })
        ));
    }

    #[test]
    fn monitor_stays_quiet_for_the_current_shell_and_stops_cleanly() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let monitor = ShellLifecycleMonitor::start(host).expect("Explorer should be monitorable");
        thread::sleep(Duration::from_millis(300));
        assert!(matches!(
            monitor.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(monitor);
    }
}
