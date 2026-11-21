#![cfg(target_os = "windows")]

use wallpaper_desktop::{discover_desktop_host, initialize_windows};
use wallpaper_display::DisplayRect;
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::GetWindowRect,
};

#[test]
#[ignore = "temporarily creates a child window on the live Explorer desktop"]
fn display_child_uses_requested_screen_origin_and_extent() {
    initialize_windows().expect("the test process should use per-monitor DPI awareness");
    let bounds = DisplayRect::new(120, 180, 520, 460).unwrap();
    let child = discover_desktop_host()
        .expect("Explorer desktop host should be available")
        .attach_wallpaper_window_for_display(bounds)
        .expect("display child should attach");

    let mut actual = RECT::default();
    unsafe { GetWindowRect(HWND(child.raw_handle() as _), &mut actual) }
        .expect("the child window should have screen coordinates");

    assert_eq!(actual.left, bounds.left());
    assert_eq!(actual.top, bounds.top());
    assert_eq!(actual.right, bounds.right());
    assert_eq!(actual.bottom, bounds.bottom());
    drop(child);
}
