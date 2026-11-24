//! Files embedded by `build.rs`. Empty placeholders mean the payload was not staged.

pub const APP_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload/wallpaper-ui.exe"));
pub const HOST_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload/wallpaper-host.exe"));

pub fn is_complete() -> bool {
    !APP_EXE.is_empty() && !HOST_EXE.is_empty()
}
