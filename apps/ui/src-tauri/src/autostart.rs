//! Launch-at-login through the current user's `Run` registry key.
//!
//! The entry lives under `HKEY_CURRENT_USER`, so it needs no administrator
//! rights and uninstalling for one user never affects another. The registered
//! command passes `--autostart`, which starts the app hidden in the tray; the
//! wallpaper host then restores the saved display profiles on its own.

use std::{ffi::c_void, path::Path};

use wallpaper_core::error::{AppError, ErrorCode};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    },
    core::{HSTRING, PCWSTR},
};

pub const AUTOSTART_ARGUMENT: &str = "--autostart";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "wallpaper-lite";

/// The command stored in the registry for `executable`.
fn command_line(executable: &Path) -> String {
    format!("\"{}\" {AUTOSTART_ARGUMENT}", executable.display())
}

fn registry_error(code: WIN32_ERROR) -> AppError {
    tracing::warn!(
        component = "ui_backend",
        event = "autostart_registry_failed",
        os_error_code = code.0,
        "The launch-at-login registry entry could not be changed"
    );
    AppError::new(
        ErrorCode::AutostartFailed,
        "Launch at login could not be changed. Check the log for details.",
    )
}

fn read(subkey: &str, name: &str) -> Result<Option<String>, AppError> {
    let subkey = HSTRING::from(subkey);
    let name = HSTRING::from(name);
    let mut byte_len = 0_u32;
    // SAFETY: the key and value names are NUL-terminated and outlive the call; passing
    // no data buffer asks only for the required size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut byte_len),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(registry_error(status));
    }

    let mut buffer = vec![0_u16; (byte_len as usize).div_ceil(2)];
    let mut capacity = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `capacity` writable bytes and outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&mut capacity),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(registry_error(status));
    }
    let length = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..length])))
}

fn write(subkey: &str, name: &str, value: &str) -> Result<(), AppError> {
    let subkey = HSTRING::from(subkey);
    let name = HSTRING::from(name);
    let data = HSTRING::from(value);
    // REG_SZ data includes the terminating NUL, which HSTRING guarantees in memory.
    let byte_len = ((data.len() + 1) * 2) as u32;
    // SAFETY: the names and data are NUL-terminated and outlive the call; `byte_len`
    // covers the data plus its terminator.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast::<c_void>()),
            byte_len,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(registry_error(status))
    }
}

fn remove(subkey: &str, name: &str) -> Result<(), AppError> {
    let subkey = HSTRING::from(subkey);
    let name = HSTRING::from(name);
    // SAFETY: the names are NUL-terminated and outlive the call.
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
        )
    };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(registry_error(status))
    }
}

fn current_command() -> Result<String, AppError> {
    let executable = std::env::current_exe().map_err(|_| {
        AppError::new(
            ErrorCode::AutostartFailed,
            "The app location could not be determined.",
        )
    })?;
    Ok(command_line(&executable))
}

/// True when launch at login is registered for this user, even for another
/// install location. Use [`refresh_registration`] to repoint a stale entry.
pub fn is_enabled() -> Result<bool, AppError> {
    Ok(read(RUN_KEY, VALUE_NAME)?.is_some())
}

pub fn set_enabled(enabled: bool) -> Result<(), AppError> {
    if enabled {
        write(RUN_KEY, VALUE_NAME, &current_command()?)
    } else {
        remove(RUN_KEY, VALUE_NAME)
    }
}

/// Repoint an existing entry at the running executable, for example after the
/// app was moved or reinstalled. Does nothing when launch at login is off.
pub fn refresh_registration() -> Result<(), AppError> {
    let expected = current_command()?;
    match read(RUN_KEY, VALUE_NAME)? {
        Some(stored) if stored != expected => write(RUN_KEY, VALUE_NAME, &expected),
        _ => Ok(()),
    }
}

/// True when the process was started by the login entry.
pub fn launched_by_autostart(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter()
        .any(|argument| argument == AUTOSTART_ARGUMENT)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{command_line, launched_by_autostart, read, remove, write};

    // A private key keeps the tests away from the real login entry.
    const TEST_ROOT: &str = r"Software\wallpaper-lite-test";
    const TEST_KEY: &str = r"Software\wallpaper-lite-test\Run";

    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            use windows::{
                Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW},
                core::{HSTRING, PCWSTR},
            };
            let root = HSTRING::from(TEST_ROOT);
            // SAFETY: the key name is NUL-terminated and outlives the call.
            let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(root.as_ptr())) };
        }
    }

    #[test]
    fn command_quotes_a_path_with_spaces_and_adds_the_autostart_flag() {
        assert_eq!(
            command_line(Path::new(
                r"C:\Program Files\wallpaper-lite\wallpaper-ui.exe"
            )),
            r#""C:\Program Files\wallpaper-lite\wallpaper-ui.exe" --autostart"#
        );
    }

    #[test]
    fn autostart_flag_is_detected_only_when_present() {
        assert!(launched_by_autostart([
            "app.exe".to_owned(),
            "--autostart".to_owned()
        ]));
        assert!(!launched_by_autostart(["app.exe".to_owned()]));
    }

    #[test]
    fn registry_value_round_trips_and_removal_is_idempotent() {
        let _cleanup = Cleanup;
        let value = r#""C:\Apps\한글 폴더\wallpaper-ui.exe" --autostart"#;
        assert_eq!(read(TEST_KEY, "entry").unwrap(), None);
        write(TEST_KEY, "entry", value).unwrap();
        assert_eq!(read(TEST_KEY, "entry").unwrap().as_deref(), Some(value));
        remove(TEST_KEY, "entry").unwrap();
        assert_eq!(read(TEST_KEY, "entry").unwrap(), None);
        remove(TEST_KEY, "entry").unwrap();
    }
}
