use std::{os::windows::ffi::OsStrExt, path::Path};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{Interface, PCWSTR};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// Writes a `.lnk` that starts `target` in `working_dir`.
pub fn create(link: &Path, target: &Path, working_dir: &Path) -> Result<(), String> {
    let (target_w, dir_w, link_w) = (wide(target), wide(working_dir), wide(link));
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(PCWSTR(target_w.as_ptr()))?;
            shell_link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))?;
            shell_link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;
            let file: IPersistFile = shell_link.cast()?;
            file.Save(PCWSTR(link_w.as_ptr()), true)
        })();
        if initialized {
            CoUninitialize();
        }
        result.map_err(|error| error.to_string())
    }
}
