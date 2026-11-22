use tokio::sync::watch;
use wallpaper_core::performance_policy::PerformanceSignals;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemPowerState {
    #[default]
    Running,
    Suspending,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SignalDetectorHealth {
    pub fullscreen_error_code: Option<i32>,
    pub session_error_code: Option<i32>,
    pub power_error_code: Option<i32>,
    pub power_notification_error_code: Option<i32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PerformanceSignalSnapshot {
    pub revision: u64,
    pub fullscreen: Option<bool>,
    pub session_locked: Option<bool>,
    pub on_battery: Option<bool>,
    pub power_state: SystemPowerState,
    pub health: SignalDetectorHealth,
}

/// Build the signals currently owned by Phase 7's automatic playback policies.
/// An unknown initial power state fails open; the monitor retains the last known value.
pub fn policy_signals(snapshot: &PerformanceSignalSnapshot) -> PerformanceSignals {
    PerformanceSignals {
        fullscreen: snapshot.fullscreen.unwrap_or(false),
        session_locked: snapshot.session_locked.unwrap_or(false),
        on_battery: snapshot.on_battery.unwrap_or(false),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalUpdate {
    Fullscreen(Option<bool>, Option<i32>),
    SessionLocked(Option<bool>, Option<i32>),
    OnBattery(Option<bool>, Option<i32>),
    PowerNotificationError(Option<i32>),
    PowerState(SystemPowerState),
}

impl PerformanceSignalSnapshot {
    fn apply(&mut self, update: SignalUpdate) -> bool {
        let previous = self.clone();
        match update {
            SignalUpdate::Fullscreen(value, error) => {
                if let Some(value) = value {
                    self.fullscreen = Some(value);
                }
                self.health.fullscreen_error_code = error;
            }
            SignalUpdate::SessionLocked(value, error) => {
                if let Some(value) = value {
                    self.session_locked = Some(value);
                }
                self.health.session_error_code = error;
            }
            SignalUpdate::OnBattery(value, error) => {
                if let Some(value) = value {
                    self.on_battery = Some(value);
                }
                self.health.power_error_code = error;
            }
            SignalUpdate::PowerNotificationError(error) => {
                self.health.power_notification_error_code = error;
            }
            SignalUpdate::PowerState(value) => self.power_state = value,
        }
        if *self != previous {
            self.revision = previous.revision.saturating_add(1);
            true
        } else {
            false
        }
    }
}

fn publish_snapshot(
    updates: &watch::Sender<PerformanceSignalSnapshot>,
    current: &mut PerformanceSignalSnapshot,
    update: SignalUpdate,
) {
    if current.apply(update) {
        updates.send_replace(current.clone());
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SignalMonitorError {
    pub stage: &'static str,
    pub error_code: Option<i32>,
}

#[cfg(windows)]
mod platform {
    use std::{
        mem::{ManuallyDrop, offset_of, size_of},
        ptr::{self, addr_of},
        sync::mpsc::{self, RecvTimeoutError},
        thread::{self, JoinHandle},
        time::Duration,
    };

    use tokio::sync::watch;
    use windows::{
        Win32::{
            Foundation::{GetLastError, HANDLE, HWND, LPARAM, RECT, WPARAM},
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
            },
            System::{
                LibraryLoader::GetModuleHandleW,
                Power::{
                    GetSystemPowerStatus, RegisterPowerSettingNotification, SYSTEM_POWER_STATUS,
                    UnregisterPowerSettingNotification,
                },
                RemoteDesktop::{
                    WTS_CURRENT_SESSION, WTSFreeMemory, WTSQuerySessionInformationW,
                    WTSRegisterSessionNotification, WTSSessionInfoEx,
                    WTSUnRegisterSessionNotification,
                },
                Threading::{GetCurrentProcessId, GetCurrentThreadId},
            },
            UI::{
                Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState},
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                    GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowThreadProcessId,
                    IsIconic, IsWindowVisible, KillTimer, MSG, PBT_APMPOWERSTATUSCHANGE,
                    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND,
                    PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, PostThreadMessageW, RegisterClassW,
                    SetTimer, UnregisterClassW, WM_DISPLAYCHANGE, WM_DPICHANGED,
                    WM_DPICHANGED_AFTERPARENT, WM_DPICHANGED_BEFOREPARENT, WM_POWERBROADCAST,
                    WM_QUIT, WM_SETTINGCHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW,
                    WS_OVERLAPPED, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
                },
            },
        },
        core::w,
    };

    use super::{
        PerformanceSignalSnapshot, SignalMonitorError, SignalUpdate, SystemPowerState,
        publish_snapshot as publish,
    };

    const TIMER_ID: usize = 1;
    const SAMPLE_INTERVAL_MS: u32 = 1_000;
    const DISPLAY_DEBOUNCE_TIMER_ID: usize = 2;
    const DISPLAY_DEBOUNCE_MS: u32 = 300;
    const WTS_SESSION_NOTIFICATION_FOR_THIS_SESSION: u32 = 0;
    const WTS_INFO_LEVEL1: u32 = 1;
    const WTS_SESSION_FLAG_LOCK: i32 = 0;
    const WTS_SESSION_FLAG_UNLOCK: i32 = 1;
    const GUID_ACDC_POWER_SOURCE: windows::core::GUID =
        windows::core::GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WtsInfoExLevel1Prefix {
        session_id: u32,
        session_state: i32,
        session_flags: i32,
    }

    #[repr(C)]
    union WtsInfoExData {
        level1: ManuallyDrop<WtsInfoExLevel1Prefix>,
        alignment: u64,
    }

    #[repr(C)]
    struct WtsInfoEx {
        level: u32,
        data: WtsInfoExData,
    }

    pub struct PerformanceSignalMonitor {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
        receiver: watch::Receiver<PerformanceSignalSnapshot>,
        display_receiver: watch::Receiver<u64>,
    }

    impl PerformanceSignalMonitor {
        pub fn start() -> Result<Self, SignalMonitorError> {
            let (updates, receiver) = watch::channel(PerformanceSignalSnapshot::default());
            let (display_updates, display_receiver) = watch::channel(0_u64);
            let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
            let (thread_id_sender, thread_id_receiver) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("wallpaper-performance-signals".to_owned())
                .spawn(move || {
                    let _ = thread_id_sender.send(unsafe { GetCurrentThreadId() });
                    if let Err(error) = run_monitor(updates, display_updates, ready_sender) {
                        tracing::error!(
                            component = "performance",
                            event = "signal_monitor_failed",
                            stage = error.stage,
                            error_code = ?error.error_code,
                            "Performance signal monitor stopped"
                        );
                    }
                })
                .map_err(|_| SignalMonitorError {
                    stage: "thread_create",
                    error_code: None,
                })?;
            let thread_id = thread_id_receiver
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| SignalMonitorError {
                    stage: "thread_start",
                    error_code: None,
                })?;
            match ready_receiver.recv_timeout(Duration::from_secs(3)) {
                Ok(Ok(())) => Ok(Self {
                    thread_id,
                    thread: Some(thread),
                    receiver,
                    display_receiver,
                }),
                Ok(Err(error)) => {
                    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                    let _ = thread.join();
                    Err(error)
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                    let _ = thread.join();
                    Err(SignalMonitorError {
                        stage: "window_start_timeout",
                        error_code: None,
                    })
                }
            }
        }

        pub fn subscribe(&self) -> watch::Receiver<PerformanceSignalSnapshot> {
            self.receiver.clone()
        }

        pub fn subscribe_display_changes(&self) -> watch::Receiver<u64> {
            self.display_receiver.clone()
        }

        pub fn stop(mut self) -> bool {
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            self.thread
                .take()
                .is_none_or(|thread| thread.join().is_ok())
        }
    }

    impl Drop for PerformanceSignalMonitor {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                let _ =
                    unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                let _ = thread.join();
            }
        }
    }

    fn run_monitor(
        updates: watch::Sender<PerformanceSignalSnapshot>,
        display_updates: watch::Sender<u64>,
        ready: mpsc::SyncSender<Result<(), SignalMonitorError>>,
    ) -> Result<(), SignalMonitorError> {
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| SignalMonitorError {
            stage: "module_handle",
            error_code: Some(error.code().0),
        })?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(signal_window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("WallpaperLitePerformanceSignalWindow"),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            let error_code = unsafe { GetLastError().0 as i32 };
            let error = SignalMonitorError {
                stage: "window_class_register",
                error_code: Some(error_code),
            };
            let _ = ready.send(Err(error));
            return Err(error);
        }
        let window = match unsafe {
            CreateWindowExW(
                Default::default(),
                class.lpszClassName,
                w!(""),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
        } {
            Ok(window) => window,
            Err(error) => {
                let _ = unsafe { UnregisterClassW(class.lpszClassName, Some(instance.into())) };
                let failure = SignalMonitorError {
                    stage: "window_create",
                    error_code: Some(error.code().0),
                };
                let _ = ready.send(Err(failure));
                return Err(failure);
            }
        };
        if unsafe { SetTimer(Some(window), TIMER_ID, SAMPLE_INTERVAL_MS, None) } == 0 {
            unsafe {
                let _ = DestroyWindow(window);
                let _ = UnregisterClassW(class.lpszClassName, Some(instance.into()));
            }
            let failure = SignalMonitorError {
                stage: "sample_timer_create",
                error_code: Some(unsafe { GetLastError().0 as i32 }),
            };
            let _ = ready.send(Err(failure));
            return Err(failure);
        }

        let mut snapshot = PerformanceSignalSnapshot::default();
        let session_registration = unsafe {
            WTSRegisterSessionNotification(window, WTS_SESSION_NOTIFICATION_FOR_THIS_SESSION)
        };
        let session_error = session_registration
            .as_ref()
            .err()
            .map(|error| error.code().0);
        let power_registration = unsafe {
            RegisterPowerSettingNotification(
                HANDLE(window.0),
                &GUID_ACDC_POWER_SOURCE,
                windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        };
        let power_notification_error = power_registration
            .as_ref()
            .err()
            .map(|error| error.code().0);

        let (fullscreen, fullscreen_error) = sample_fullscreen();
        snapshot.apply(SignalUpdate::Fullscreen(fullscreen, fullscreen_error));
        let (session_locked, session_query_error) = query_session_locked();
        snapshot.apply(SignalUpdate::SessionLocked(
            session_locked,
            session_query_error.or(session_error),
        ));
        let (on_battery, battery_error) = sample_battery();
        snapshot.apply(SignalUpdate::OnBattery(on_battery, battery_error));
        snapshot.apply(SignalUpdate::PowerNotificationError(
            power_notification_error,
        ));
        updates.send_replace(snapshot.clone());
        let _ = ready.send(Ok(()));

        let mut message = MSG::default();
        let mut message_loop_error = None;
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if result == 0 {
                break;
            }
            if result < 0 {
                message_loop_error = Some(SignalMonitorError {
                    stage: "message_loop",
                    error_code: Some(unsafe { GetLastError().0 as i32 }),
                });
                break;
            }
            if message.message == WM_TIMER && message.wParam.0 == TIMER_ID {
                let (fullscreen, fullscreen_error) = sample_fullscreen();
                publish(
                    &updates,
                    &mut snapshot,
                    SignalUpdate::Fullscreen(fullscreen, fullscreen_error),
                );
                let (session_locked, session_query_error) = query_session_locked();
                publish(
                    &updates,
                    &mut snapshot,
                    SignalUpdate::SessionLocked(
                        session_locked,
                        session_query_error.or(session_error),
                    ),
                );
                let (on_battery, battery_error) = sample_battery();
                publish(
                    &updates,
                    &mut snapshot,
                    SignalUpdate::OnBattery(on_battery, battery_error),
                );
            } else if message.message == WM_TIMER && message.wParam.0 == DISPLAY_DEBOUNCE_TIMER_ID {
                unsafe {
                    let _ = KillTimer(Some(window), DISPLAY_DEBOUNCE_TIMER_ID);
                }
                display_updates.send_modify(|revision| {
                    *revision = revision.saturating_add(1);
                });
            } else if message.message == WM_WTSSESSION_CHANGE {
                let locked = match message.wParam.0 as u32 {
                    WTS_SESSION_LOCK => Some(true),
                    WTS_SESSION_UNLOCK => Some(false),
                    _ => None,
                };
                if let Some(locked) = locked {
                    publish(
                        &updates,
                        &mut snapshot,
                        SignalUpdate::SessionLocked(Some(locked), None),
                    );
                }
            } else if message.message == WM_POWERBROADCAST {
                match message.wParam.0 as u32 {
                    PBT_APMSUSPEND => publish(
                        &updates,
                        &mut snapshot,
                        SignalUpdate::PowerState(SystemPowerState::Suspending),
                    ),
                    PBT_APMRESUMECRITICAL | PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        publish(
                            &updates,
                            &mut snapshot,
                            SignalUpdate::PowerState(SystemPowerState::Running),
                        );
                        let (on_battery, error) = sample_battery();
                        publish(
                            &updates,
                            &mut snapshot,
                            SignalUpdate::OnBattery(on_battery, error),
                        );
                    }
                    PBT_APMPOWERSTATUSCHANGE | PBT_POWERSETTINGCHANGE => {
                        let (on_battery, error) = sample_battery();
                        publish(
                            &updates,
                            &mut snapshot,
                            SignalUpdate::OnBattery(on_battery, error),
                        );
                    }
                    _ => {}
                }
            } else if is_display_topology_message(message.message)
                && unsafe {
                    SetTimer(
                        Some(window),
                        DISPLAY_DEBOUNCE_TIMER_ID,
                        DISPLAY_DEBOUNCE_MS,
                        None,
                    )
                } == 0
            {
                tracing::warn!(
                    component = "display",
                    event = "display_topology_debounce_failed",
                    error_code = unsafe { GetLastError().0 as i32 },
                    "Display change will be reconciled without debounce"
                );
                display_updates.send_modify(|revision| {
                    *revision = revision.saturating_add(1);
                });
            }
            unsafe { DispatchMessageW(&message) };
        }

        unsafe {
            let _ = KillTimer(Some(window), TIMER_ID);
            let _ = KillTimer(Some(window), DISPLAY_DEBOUNCE_TIMER_ID);
            if session_registration.is_ok() {
                let _ = WTSUnRegisterSessionNotification(window);
            }
            if let Ok(notification) = power_registration {
                let _ = UnregisterPowerSettingNotification(notification);
            }
            let _ = DestroyWindow(window);
            let _ = UnregisterClassW(class.lpszClassName, Some(instance.into()));
        }
        message_loop_error.map_or(Ok(()), Err)
    }

    fn is_display_topology_message(message: u32) -> bool {
        matches!(
            message,
            WM_DISPLAYCHANGE
                | WM_SETTINGCHANGE
                | WM_DPICHANGED
                | WM_DPICHANGED_BEFOREPARENT
                | WM_DPICHANGED_AFTERPARENT
        )
    }

    unsafe extern "system" fn signal_window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn sample_fullscreen() -> (Option<bool>, Option<i32>) {
        let shell = unsafe { SHQueryUserNotificationState() }
            .map(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN)
            .map_err(|error| error.code().0);
        let geometry = foreground_covers_monitor().map_err(|error| error.code().0);
        let fullscreen = match (shell, geometry) {
            (Ok(shell), Ok(geometry)) => Some(shell || geometry),
            (Ok(shell), Err(_)) => Some(shell),
            (Err(_), Ok(geometry)) => Some(geometry),
            (Err(shell_error), Err(_)) => return (None, Some(shell_error)),
        };
        let error = if shell.is_err() && geometry.is_err() {
            shell.err().or_else(|| geometry.err())
        } else {
            None
        };
        (fullscreen, error)
    }

    fn foreground_covers_monitor() -> windows::core::Result<bool> {
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground.0.is_null()
                || !IsWindowVisible(foreground).as_bool()
                || IsIconic(foreground).as_bool()
            {
                return Ok(false);
            }
            let mut process_id = 0;
            let _ = GetWindowThreadProcessId(foreground, Some(&mut process_id));
            if process_id == GetCurrentProcessId() {
                return Ok(false);
            }
            let mut window_rect = RECT::default();
            GetWindowRect(foreground, &mut window_rect)?;
            let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
            if monitor.0.is_null() {
                return Ok(false);
            }
            let mut monitor_info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                return Err(windows::core::Error::from_win32());
            }
            let bounds = monitor_info.rcMonitor;
            Ok(window_rect.left <= bounds.left
                && window_rect.top <= bounds.top
                && window_rect.right >= bounds.right
                && window_rect.bottom >= bounds.bottom)
        }
    }

    fn query_session_locked() -> (Option<bool>, Option<i32>) {
        let mut buffer = windows::core::PWSTR::null();
        let mut bytes = 0;
        let result = unsafe {
            WTSQuerySessionInformationW(
                None,
                WTS_CURRENT_SESSION,
                WTSSessionInfoEx,
                &mut buffer,
                &mut bytes,
            )
        };
        if let Err(error) = result {
            return (None, Some(error.code().0));
        }
        if buffer.is_null() || (bytes as usize) < size_of::<WtsInfoEx>() {
            if !buffer.is_null() {
                unsafe { WTSFreeMemory(buffer.0.cast()) };
            }
            return (None, Some(-1));
        }

        let level = unsafe { ptr::read_unaligned(buffer.0.cast::<u32>()) };
        let flags = if level == WTS_INFO_LEVEL1 {
            let data = unsafe {
                buffer
                    .0
                    .cast::<u8>()
                    .add(offset_of!(WtsInfoEx, data))
                    .cast::<WtsInfoExData>()
            };
            Some(unsafe {
                ManuallyDrop::into_inner(ptr::read_unaligned(addr_of!((*data).level1)))
                    .session_flags
            })
        } else {
            None
        };
        unsafe { WTSFreeMemory(buffer.0.cast()) };
        match flags {
            Some(WTS_SESSION_FLAG_LOCK) => (Some(true), None),
            Some(WTS_SESSION_FLAG_UNLOCK) => (Some(false), None),
            _ => (None, Some(-2)),
        }
    }

    fn sample_battery() -> (Option<bool>, Option<i32>) {
        let mut status = SYSTEM_POWER_STATUS::default();
        match unsafe { GetSystemPowerStatus(&mut status) } {
            Ok(()) => match status.ACLineStatus {
                0 => (Some(true), None),
                1 => (Some(false), None),
                _ => (None, Some(-3)),
            },
            Err(error) => (None, Some(error.code().0)),
        }
    }

    #[cfg(test)]
    mod topology_message_tests {
        use super::*;

        #[test]
        fn recognizes_display_and_dpi_notifications_for_debounce() {
            for message in [
                WM_DISPLAYCHANGE,
                WM_SETTINGCHANGE,
                WM_DPICHANGED,
                WM_DPICHANGED_BEFOREPARENT,
                WM_DPICHANGED_AFTERPARENT,
            ] {
                assert!(is_display_topology_message(message));
            }
            assert!(!is_display_topology_message(WM_TIMER));
            assert!(!is_display_topology_message(WM_WTSSESSION_CHANGE));
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use tokio::sync::watch;

    use super::{PerformanceSignalSnapshot, SignalMonitorError};

    pub struct PerformanceSignalMonitor;

    impl PerformanceSignalMonitor {
        pub fn start() -> Result<Self, SignalMonitorError> {
            Err(SignalMonitorError {
                stage: "unsupported_platform",
                error_code: None,
            })
        }

        pub fn subscribe(&self) -> watch::Receiver<PerformanceSignalSnapshot> {
            watch::channel(PerformanceSignalSnapshot::default()).1
        }

        pub fn subscribe_display_changes(&self) -> watch::Receiver<u64> {
            watch::channel(0_u64).1
        }

        pub fn stop(self) -> bool {
            true
        }
    }
}

pub use platform::PerformanceSignalMonitor;

#[cfg(test)]
mod tests {
    use super::{
        PerformanceSignalSnapshot, SignalUpdate, SystemPowerState, policy_signals, publish_snapshot,
    };
    use tokio::sync::watch;
    use wallpaper_core::performance_policy::PerformanceSignals;

    #[test]
    fn unavailable_signals_fail_open_and_known_battery_state_is_used() {
        assert_eq!(
            policy_signals(&PerformanceSignalSnapshot::default()),
            PerformanceSignals::default()
        );
        let mut snapshot = PerformanceSignalSnapshot::default();
        assert!(snapshot.apply(SignalUpdate::OnBattery(Some(true), None)));
        assert!(policy_signals(&snapshot).on_battery);
        assert!(snapshot.apply(SignalUpdate::OnBattery(None, Some(5))));
        assert!(policy_signals(&snapshot).on_battery);
    }

    #[test]
    fn duplicate_signal_updates_are_coalesced_and_transitions_are_retained() {
        let mut snapshot = PerformanceSignalSnapshot::default();
        assert!(snapshot.apply(SignalUpdate::Fullscreen(Some(false), None)));
        assert!(!snapshot.apply(SignalUpdate::Fullscreen(Some(false), None)));
        assert!(snapshot.apply(SignalUpdate::Fullscreen(Some(true), None)));
        assert!(snapshot.apply(SignalUpdate::SessionLocked(Some(true), None)));
        assert!(snapshot.apply(SignalUpdate::SessionLocked(Some(false), None)));
        assert!(snapshot.apply(SignalUpdate::OnBattery(Some(true), None)));
        assert!(snapshot.apply(SignalUpdate::OnBattery(Some(false), None)));
        assert!(snapshot.apply(SignalUpdate::PowerState(SystemPowerState::Suspending)));
        assert!(snapshot.apply(SignalUpdate::PowerState(SystemPowerState::Running)));
        assert_eq!(snapshot.fullscreen, Some(true));
        assert_eq!(snapshot.session_locked, Some(false));
        assert_eq!(snapshot.on_battery, Some(false));
        assert_eq!(snapshot.power_state, SystemPowerState::Running);
    }

    #[test]
    fn detector_failure_is_reported_without_erasing_last_known_signal() {
        let mut snapshot = PerformanceSignalSnapshot::default();
        snapshot.apply(SignalUpdate::SessionLocked(Some(false), None));
        assert!(snapshot.apply(SignalUpdate::SessionLocked(None, Some(1234))));
        assert_eq!(snapshot.session_locked, Some(false));
        assert_eq!(snapshot.health.session_error_code, Some(1234));
    }

    #[test]
    fn watch_channel_publishes_only_distinct_signal_snapshots() {
        let initial = PerformanceSignalSnapshot::default();
        let (updates, mut receiver) = watch::channel(initial.clone());
        let mut current = initial;

        publish_snapshot(
            &updates,
            &mut current,
            SignalUpdate::Fullscreen(Some(false), None),
        );
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow_and_update().fullscreen, Some(false));

        publish_snapshot(
            &updates,
            &mut current,
            SignalUpdate::Fullscreen(Some(false), None),
        );
        assert!(!receiver.has_changed().unwrap());

        publish_snapshot(
            &updates,
            &mut current,
            SignalUpdate::Fullscreen(Some(true), None),
        );
        assert!(receiver.has_changed().unwrap());
        assert_eq!(receiver.borrow_and_update().fullscreen, Some(true));
    }
}
