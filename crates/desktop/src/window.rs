use std::{
    mem,
    sync::{OnceLock, mpsc},
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{
            ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT,
            WPARAM,
        },
        Graphics::Gdi::{HBRUSH, ValidateRect},
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{
            HiDpi::DPI_AWARENESS_CONTEXT,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
                GetMessageW, GetParent, GetWindowThreadProcessId, HWND_BOTTOM, IsWindow,
                MA_NOACTIVATE, MSG, PM_NOREMOVE, PeekMessageW, PostQuitMessage, PostThreadMessageW,
                RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, TranslateMessage,
                WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
                WM_MOUSEACTIVATE, WM_PAINT, WM_QUIT, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
                WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
            },
        },
    },
    core::{PCWSTR, s, w},
};

use crate::{DesktopError, DesktopHost};

const WINDOW_CLASS_NAME: PCWSTR = w!("WallpaperLiteDesktopChild");

type GetWindowDpiAwarenessContextFn = unsafe extern "system" fn(HWND) -> DPI_AWARENESS_CONTEXT;
type SetThreadDpiAwarenessContextFn =
    unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> DPI_AWARENESS_CONTEXT;

/// An owned wallpaper surface attached below the desktop icon view.
///
/// A dedicated message thread owns the HWND so it remains responsive while the host's Tokio
/// runtime serves IPC. Dropping this value closes the child window and joins that thread.
pub struct AttachedDesktopWindow {
    raw_handle: isize,
    parent: DesktopHost,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl AttachedDesktopWindow {
    pub(crate) fn attach(parent: DesktopHost) -> Result<Self, DesktopError> {
        parent.validate_current()?;

        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("wallpaper-desktop-window".into())
            .spawn(move || run_window_thread(parent, ready_tx))
            .map_err(|error| DesktopError::DesktopWindowThreadFailed {
                os_error_code: error.raw_os_error(),
            })?;

        match ready_rx.recv() {
            Ok(Ok((raw_handle, thread_id))) => Ok(Self {
                raw_handle,
                parent,
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(DesktopError::DesktopWindowThreadFailed {
                    os_error_code: None,
                })
            }
        }
    }

    pub const fn raw_handle(&self) -> isize {
        self.raw_handle
    }

    pub fn validate_current(&self) -> Result<(), DesktopError> {
        self.parent.validate_current()?;
        let hwnd = hwnd_from_raw(self.raw_handle);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(DesktopError::WallpaperWindowUnavailable);
        }

        let parent =
            unsafe { GetParent(hwnd) }.map_err(|error| DesktopError::WindowPositionFailed {
                hresult: error.code().0,
            })?;
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };

        if parent != hwnd_from_raw(self.parent.raw_handle())
            || process_id != unsafe { GetCurrentProcessId() }
        {
            return Err(DesktopError::WallpaperWindowUnavailable);
        }

        Ok(())
    }
}

impl Drop for AttachedDesktopWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_window_thread(
    parent: DesktopHost,
    ready_tx: mpsc::SyncSender<Result<(isize, u32), DesktopError>>,
) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let mut message = MSG::default();
    unsafe {
        // Force creation of this thread's message queue before the owner can request shutdown.
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }

    let hwnd = match create_and_attach_window(parent) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready_tx.send(Err(error));
            return;
        }
    };

    if ready_tx.send(Ok((hwnd.0 as isize, thread_id))).is_err() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return;
    }

    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn create_and_attach_window(parent: DesktopHost) -> Result<HWND, DesktopError> {
    parent.validate_current()?;
    register_window_class()?;

    let parent_hwnd = hwnd_from_raw(parent.raw_handle());
    let previous_dpi_context = set_parent_dpi_context(parent_hwnd)?;
    let result = create_positioned_window(parent, parent_hwnd);
    let restore_result = restore_dpi_context(previous_dpi_context);

    match (result, restore_result) {
        (Ok(hwnd), Ok(())) => Ok(hwnd),
        (Ok(hwnd), Err(error)) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Err(error)
        }
        (Err(error), _) => Err(error),
    }
}

fn create_positioned_window(parent: DesktopHost, parent_hwnd: HWND) -> Result<HWND, DesktopError> {
    let mut bounds = RECT::default();
    unsafe { GetClientRect(parent_hwnd, &mut bounds) }.map_err(|error| {
        DesktopError::WindowPositionFailed {
            hresult: error.code().0,
        }
    })?;
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    if width <= 0 || height <= 0 {
        return Err(DesktopError::DesktopWindowBoundsUnavailable);
    }

    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| {
        DesktopError::WindowCreationFailed {
            hresult: error.code().0,
        }
    })?;
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0),
            WINDOW_CLASS_NAME,
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_CLIPSIBLINGS.0),
            0,
            0,
            width,
            height,
            Some(parent_hwnd),
            None,
            Some(HINSTANCE(module.0)),
            None,
        )
    }
    .map_err(|error| DesktopError::WindowCreationFailed {
        hresult: error.code().0,
    })?;

    if let Err(error) = parent.validate_current() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }

    if let Err(error) = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    } {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(DesktopError::WindowPositionFailed {
            hresult: error.code().0,
        });
    }

    if let Err(error) = parent.validate_current() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    Ok(hwnd)
}

fn register_window_class() -> Result<(), DesktopError> {
    static REGISTERED: OnceLock<Result<(), u32>> = OnceLock::new();
    let result = REGISTERED.get_or_init(|| {
        let module =
            unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.code().0 as u32)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(module.0),
            hbrBackground: HBRUSH::default(),
            lpszClassName: WINDOW_CLASS_NAME,
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            let error = unsafe { GetLastError() }.0;
            if error == ERROR_CLASS_ALREADY_EXISTS.0 {
                Ok(())
            } else {
                Err(error)
            }
        } else {
            Ok(())
        }
    });

    result
        .as_ref()
        .map(|_| ())
        .map_err(|win32_error| DesktopError::WindowClassRegistrationFailed {
            win32_error: *win32_error,
        })
}

fn set_parent_dpi_context(parent: HWND) -> Result<Option<DPI_AWARENESS_CONTEXT>, DesktopError> {
    let user32 = unsafe { GetModuleHandleW(w!("user32.dll")) }
        .map_err(|_| DesktopError::DesktopWindowDpiContextUnavailable)?;
    let get_context = unsafe { GetProcAddress(user32, s!("GetWindowDpiAwarenessContext")) };
    let set_context = unsafe { GetProcAddress(user32, s!("SetThreadDpiAwarenessContext")) };

    let (Some(get_context), Some(set_context)) = (get_context, set_context) else {
        // These functions first shipped in Windows 10 version 1607. Earlier Windows 10 builds
        // use the documented mixed-awareness child-window behavior for cross-process creation.
        return Ok(None);
    };

    let get_context: GetWindowDpiAwarenessContextFn = unsafe { mem::transmute(get_context) };
    let set_context: SetThreadDpiAwarenessContextFn = unsafe { mem::transmute(set_context) };
    let parent_context = unsafe { get_context(parent) };
    if parent_context.0.is_null() {
        return Err(DesktopError::DesktopWindowDpiContextUnavailable);
    }
    let previous = unsafe { set_context(parent_context) };
    if previous.0.is_null() {
        return Err(DesktopError::DesktopWindowDpiContextFailed);
    }
    Ok(Some(previous))
}

fn restore_dpi_context(previous: Option<DPI_AWARENESS_CONTEXT>) -> Result<(), DesktopError> {
    let Some(previous) = previous else {
        return Ok(());
    };
    let user32 = unsafe { GetModuleHandleW(w!("user32.dll")) }
        .map_err(|_| DesktopError::DesktopWindowDpiContextUnavailable)?;
    let set_context = unsafe { GetProcAddress(user32, s!("SetThreadDpiAwarenessContext")) }
        .ok_or(DesktopError::DesktopWindowDpiContextUnavailable)?;
    let set_context: SetThreadDpiAwarenessContextFn = unsafe { mem::transmute(set_context) };
    if unsafe { set_context(previous) }.0.is_null() {
        return Err(DesktopError::DesktopWindowDpiContextFailed);
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn hwnd_from_raw(raw_handle: isize) -> HWND {
    HWND(raw_handle as *mut _)
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{
        GW_HWNDNEXT, GetForegroundWindow, GetWindow, IsWindow,
    };

    use super::hwnd_from_raw;
    use crate::discover_desktop_host;

    #[test]
    fn attached_window_stays_below_shell_children_and_drops_cleanly() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let foreground_before = unsafe { GetForegroundWindow() };

        for _ in 0..3 {
            let window = host
                .attach_wallpaper_window()
                .expect("the wallpaper child should attach to the desktop host");
            let hwnd = hwnd_from_raw(window.raw_handle());

            window
                .validate_current()
                .expect("the attached window should remain current");
            assert!(unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.is_err());
            assert_eq!(unsafe { GetForegroundWindow() }, foreground_before);

            drop(window);
            assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
        }
    }
}
