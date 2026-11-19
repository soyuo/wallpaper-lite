use std::{thread, time::Duration};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, FindWindowExW, GW_CHILD, GW_HWNDNEXT, GetClassNameW, GetClientRect,
            GetParent, GetShellWindow, GetWindow, GetWindowThreadProcessId, IsWindow,
            IsWindowVisible, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
        },
    },
    core::{BOOL, PCWSTR, w},
};

use crate::{DesktopError, DesktopHost, DesktopHostKind};

const MAX_DISCOVERY_ATTEMPTS: usize = 3;
const DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(50);
const SHELL_WORKER_CREATE_MESSAGE: u32 = 0x052C;
const SHELL_WORKER_CREATE_WPARAM: usize = 13;
const SHELL_WORKER_MESSAGE_TIMEOUT_MS: u32 = 1_000;

#[derive(Clone, Copy)]
struct WindowCandidate {
    raw_handle: isize,
    process_id: u32,
    kind: Option<DesktopHostKind>,
    has_shell_view: bool,
}

/// Find a shell-owned Progman or WorkerW that directly owns the desktop shell view.
///
/// Discovery is read-only and bounded: it does not send Explorer's private undocumented shell
/// messages or attach a wallpaper window.
pub fn discover_desktop_host() -> Result<DesktopHost, DesktopError> {
    let mut last_error = DesktopError::ShellUnavailable;
    for attempt in 0..MAX_DISCOVERY_ATTEMPTS {
        match discover_once() {
            Ok(host) => return Ok(host),
            Err(error) => {
                last_error = error;
                if attempt + 1 < MAX_DISCOVERY_ATTEMPTS {
                    thread::sleep(DISCOVERY_RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error)
}

fn discover_once() -> Result<DesktopHost, DesktopError> {
    let shell_window = unsafe { GetShellWindow() };
    if shell_window == HWND::default() {
        return Err(DesktopError::ShellUnavailable);
    }
    if !unsafe { IsWindow(Some(shell_window)) }.as_bool() {
        return Err(DesktopError::StaleDesktopHost);
    }

    let shell_process_id = process_id(shell_window).ok_or(DesktopError::StaleDesktopHost)?;
    let shell_candidate = WindowCandidate {
        raw_handle: shell_window.0 as isize,
        process_id: shell_process_id,
        kind: class_kind(shell_window),
        has_shell_view: has_direct_shell_view(shell_window),
    };

    if shell_candidate.kind.is_some() && shell_candidate.has_shell_view {
        let host = choose_candidate(shell_candidate, shell_process_id, &[])?;
        host.validate_current()?;
        return Ok(host);
    }

    let mut top_level_windows = Vec::new();
    let lparam = LPARAM((&mut top_level_windows as *mut Vec<HWND>) as isize);
    unsafe { EnumWindows(Some(collect_top_level_window), lparam) }.map_err(|error| {
        DesktopError::WindowEnumerationFailed {
            hresult: error.code().0,
        }
    })?;

    let candidates: Vec<_> = top_level_windows
        .into_iter()
        .filter_map(|window| {
            let process_id = process_id(window)?;
            Some(WindowCandidate {
                raw_handle: window.0 as isize,
                process_id,
                kind: class_kind(window),
                has_shell_view: has_direct_shell_view(window),
            })
        })
        .collect();

    let host = choose_candidate(shell_candidate, shell_process_id, &candidates)?;
    host.validate_current()?;
    Ok(host)
}

fn choose_candidate(
    shell_window: WindowCandidate,
    shell_process_id: u32,
    candidates: &[WindowCandidate],
) -> Result<DesktopHost, DesktopError> {
    let matching: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.process_id == shell_process_id
                && candidate.kind.is_some()
                && candidate.has_shell_view
        })
        .collect();

    match matching.as_slice() {
        [candidate] => desktop_host(*candidate),
        [] if shell_window.process_id == shell_process_id
            && shell_window.kind.is_some()
            && shell_window.has_shell_view =>
        {
            desktop_host(shell_window)
        }
        [] => Err(DesktopError::DesktopHostNotFound),
        _ => Err(DesktopError::AmbiguousDesktopHost {
            candidate_count: matching.len(),
        }),
    }
}

fn desktop_host(candidate: WindowCandidate) -> Result<DesktopHost, DesktopError> {
    let kind = candidate.kind.ok_or(DesktopError::DesktopHostNotFound)?;
    Ok(DesktopHost::new(
        candidate.raw_handle,
        candidate.process_id,
        kind,
    ))
}

fn process_id(window: HWND) -> Option<u32> {
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    (thread_id != 0 && process_id != 0).then_some(process_id)
}

fn class_kind(window: HWND) -> Option<DesktopHostKind> {
    let mut class_name = [0u16; 32];
    let length = unsafe { GetClassNameW(window, &mut class_name) };
    if length <= 0 {
        return None;
    }

    match String::from_utf16_lossy(&class_name[..length as usize]).as_str() {
        "Progman" => Some(DesktopHostKind::Progman),
        "WorkerW" => Some(DesktopHostKind::WorkerW),
        _ => None,
    }
}

fn has_direct_shell_view(window: HWND) -> bool {
    unsafe { FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), PCWSTR::null()) }
        .is_ok_and(|child| child != HWND::default())
}

unsafe extern "system" fn collect_top_level_window(window: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: EnumWindows invokes this callback synchronously while `top_level_windows` remains
    // alive and exclusively borrowed by the enumeration call.
    let windows = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    windows.push(window);
    BOOL(1)
}

pub(super) fn validate_current(host: DesktopHost) -> Result<(), DesktopError> {
    let window = HWND(host.raw_handle as *mut _);
    if !unsafe { IsWindow(Some(window)) }.as_bool()
        || process_id(window) != Some(host.shell_process_id)
        || class_kind(window) != Some(host.kind)
        || !has_direct_shell_view(window)
    {
        return Err(DesktopError::StaleDesktopHost);
    }
    Ok(())
}

/// Find the Explorer-owned full-screen WorkerW that follows the icon view in child z-order.
///
/// On current Windows desktop shells this is the background surface that can contain rendered
/// pixels without covering the shell's icon view. Failing closed avoids presenting over icons.
pub(super) fn wallpaper_layer(host: DesktopHost) -> Result<HWND, DesktopError> {
    validate_current(host)?;

    let host_window = HWND(host.raw_handle as *mut _);
    let shell_view = unsafe {
        FindWindowExW(
            Some(host_window),
            None,
            w!("SHELLDLL_DefView"),
            PCWSTR::null(),
        )
    }
    .map_err(|error| DesktopError::WindowEnumerationFailed {
        hresult: error.code().0,
    })?;
    if shell_view == HWND::default() {
        return Err(DesktopError::DesktopBackgroundLayerUnavailable);
    }

    let mut host_bounds = windows::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(host_window, &mut host_bounds) }.map_err(|error| {
        DesktopError::WindowPositionFailed {
            hresult: error.code().0,
        }
    })?;
    let host_width = host_bounds.right - host_bounds.left;
    let host_height = host_bounds.bottom - host_bounds.top;

    let (layer, has_shell_worker) =
        find_wallpaper_layer(host, host_window, shell_view, host_width, host_height);
    if let Some(layer) = layer {
        return Ok(layer);
    }

    // Explorer may restart without creating the background WorkerW. Request it only when no
    // same-process WorkerW exists; repeating this private message can toggle
    // the shell layer, so an existing but unsuitable worker must fail closed instead.
    if !has_shell_worker {
        let _ = unsafe {
            SendMessageTimeoutW(
                host_window,
                SHELL_WORKER_CREATE_MESSAGE,
                WPARAM(SHELL_WORKER_CREATE_WPARAM),
                LPARAM(1),
                SMTO_ABORTIFHUNG,
                SHELL_WORKER_MESSAGE_TIMEOUT_MS,
                None,
            )
        };

        for _ in 0..MAX_DISCOVERY_ATTEMPTS {
            thread::sleep(DISCOVERY_RETRY_DELAY);
            let (layer, _) =
                find_wallpaper_layer(host, host_window, shell_view, host_width, host_height);
            if let Some(layer) = layer {
                return Ok(layer);
            }
        }
    }

    Err(DesktopError::DesktopBackgroundLayerUnavailable)
}

fn find_wallpaper_layer(
    host: DesktopHost,
    host_window: HWND,
    shell_view: HWND,
    host_width: i32,
    host_height: i32,
) -> (Option<HWND>, bool) {
    let mut child = unsafe { GetWindow(host_window, GW_CHILD) }.unwrap_or_default();
    let mut shell_view_seen = false;
    let mut has_shell_worker = false;
    while child != HWND::default() {
        if child == shell_view {
            shell_view_seen = true;
        } else if class_kind(child) == Some(DesktopHostKind::WorkerW)
            && process_id(child) == Some(host.shell_process_id)
        {
            has_shell_worker = true;
            if shell_view_seen && is_fullscreen_worker(child, host, host_width, host_height) {
                return (Some(child), true);
            }
        }
        child = unsafe { GetWindow(child, GW_HWNDNEXT) }.unwrap_or_default();
    }
    (None, has_shell_worker)
}

/// Revalidate that an attached surface still belongs to the current shell background layer.
pub(super) fn validate_wallpaper_layer(
    host: DesktopHost,
    layer: isize,
) -> Result<(), DesktopError> {
    if wallpaper_layer(host)?.0 as isize != layer {
        return Err(DesktopError::StaleDesktopHost);
    }
    Ok(())
}

fn is_fullscreen_worker(child: HWND, host: DesktopHost, host_width: i32, host_height: i32) -> bool {
    if class_kind(child) != Some(DesktopHostKind::WorkerW)
        || process_id(child) != Some(host.shell_process_id)
        || !unsafe { GetParent(child) }
            .is_ok_and(|parent| parent == HWND(host.raw_handle as *mut _))
        || !unsafe { IsWindowVisible(child) }.as_bool()
    {
        return false;
    }

    let mut bounds = windows::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(child, &mut bounds) }.is_ok_and(|()| {
        bounds.right - bounds.left == host_width && bounds.bottom - bounds.top == host_height
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        raw_handle: isize,
        process_id: u32,
        kind: Option<DesktopHostKind>,
        has_shell_view: bool,
    ) -> WindowCandidate {
        WindowCandidate {
            raw_handle,
            process_id,
            kind,
            has_shell_view,
        }
    }

    #[test]
    fn ignores_unrelated_workerw_windows() {
        let shell = candidate(10, 100, Some(DesktopHostKind::Progman), false);
        let candidates = [
            candidate(20, 100, Some(DesktopHostKind::WorkerW), false),
            candidate(30, 200, Some(DesktopHostKind::WorkerW), true),
        ];

        assert_eq!(
            choose_candidate(shell, 100, &candidates),
            Err(DesktopError::DesktopHostNotFound)
        );
    }

    #[test]
    fn accepts_one_shell_owned_workerw_with_direct_shell_view() {
        let shell = candidate(10, 100, Some(DesktopHostKind::Progman), false);
        let candidates = [
            candidate(20, 100, Some(DesktopHostKind::WorkerW), false),
            candidate(30, 100, Some(DesktopHostKind::WorkerW), true),
        ];

        let result = choose_candidate(shell, 100, &candidates);
        assert_eq!(
            result,
            Ok(DesktopHost::new(30, 100, DesktopHostKind::WorkerW))
        );
    }

    #[test]
    fn rejects_multiple_shell_view_candidates_instead_of_choosing_arbitrarily() {
        let shell = candidate(10, 100, Some(DesktopHostKind::Progman), false);
        let candidates = [
            candidate(20, 100, Some(DesktopHostKind::WorkerW), true),
            candidate(30, 100, Some(DesktopHostKind::WorkerW), true),
        ];

        assert_eq!(
            choose_candidate(shell, 100, &candidates),
            Err(DesktopError::AmbiguousDesktopHost { candidate_count: 2 })
        );
    }

    #[test]
    fn rejects_a_stale_window_handle() {
        let stale = DesktopHost::new(0, 100, DesktopHostKind::Progman);
        assert_eq!(
            stale.validate_current(),
            Err(DesktopError::StaleDesktopHost)
        );
    }

    #[test]
    fn repeated_discovery_finds_the_same_current_shell_desktop() {
        let first = discover_desktop_host().expect("an interactive Windows shell is required");
        let second = discover_desktop_host().expect("the shell desktop should remain available");
        assert_eq!(first, second);
        assert_ne!(first.raw_handle(), 0);
        first
            .validate_current()
            .expect("the discovered shell window should still be current");
    }
}
