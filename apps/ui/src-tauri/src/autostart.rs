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
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_BINARY, RRF_RT_REG_EXPAND_SZ,
            RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
        },
    },
    core::{HSTRING, PCWSTR},
};

pub const AUTOSTART_ARGUMENT: &str = "--autostart";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "wallpaper-lite";
/// Task Manager and Settings > Startup Apps record their on/off choice here.
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

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
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
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
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
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

/// First byte of a binary value, or `None` when it is absent or empty.
fn read_first_byte(subkey: &str, name: &str) -> Result<Option<u8>, AppError> {
    let subkey = HSTRING::from(subkey);
    let name = HSTRING::from(name);
    let mut buffer = [0_u8; 12];
    let mut capacity = buffer.len() as u32;
    // SAFETY: `buffer` holds `capacity` writable bytes and outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&mut capacity),
        )
    };
    // ERROR_MORE_DATA still fills the buffer; the first byte is all that matters.
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return Err(registry_error(status));
    }
    Ok((capacity > 0).then_some(buffer[0]))
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

/// Windows stores an even first byte for enabled items and an odd one for
/// items the user turned off in Task Manager or Settings.
fn is_enabled_at(run_key: &str, approved_key: &str, name: &str) -> Result<bool, AppError> {
    if read(run_key, name)?.is_none() {
        return Ok(false);
    }
    Ok(read_first_byte(approved_key, name)?.is_none_or(|byte| byte & 1 == 0))
}

fn set_enabled_at(
    run_key: &str,
    approved_key: &str,
    name: &str,
    command: &str,
    enabled: bool,
) -> Result<(), AppError> {
    if enabled {
        write(run_key, name, command)?;
        // Drop an earlier "disabled" choice so the item runs again.
        remove(approved_key, name)
    } else {
        remove(run_key, name)
    }
}

/// The executable named by a stored command, which is quoted when written by us.
fn stored_executable(command: &str) -> &str {
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => command.split(' ').next().unwrap_or(command),
    }
}

/// Repoint an entry whose executable no longer exists. An entry that still
/// points at a working install is left alone, so a development build or a copy
/// run from another folder never takes over the installed app's entry.
fn refresh_at(
    run_key: &str,
    name: &str,
    expected: &str,
    exists: impl Fn(&Path) -> bool,
) -> Result<(), AppError> {
    match read(run_key, name)? {
        Some(stored) if stored != expected && !exists(Path::new(stored_executable(&stored))) => {
            write(run_key, name, expected)
        }
        _ => Ok(()),
    }
}

/// True when launch at login is registered and not turned off in Windows.
pub fn is_enabled() -> Result<bool, AppError> {
    is_enabled_at(RUN_KEY, APPROVED_KEY, VALUE_NAME)
}

pub fn set_enabled(enabled: bool) -> Result<(), AppError> {
    set_enabled_at(
        RUN_KEY,
        APPROVED_KEY,
        VALUE_NAME,
        &current_command()?,
        enabled,
    )
}

/// Repoint an entry left behind by a moved or removed install. Does nothing
/// when launch at login is off, and never runs in debug builds.
pub fn refresh_registration() -> Result<(), AppError> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    refresh_at(RUN_KEY, VALUE_NAME, &current_command()?, Path::exists)
}

/// True when the process was started by the login entry.
pub fn launched_by_autostart(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter()
        .any(|argument| argument == AUTOSTART_ARGUMENT)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        command_line, is_enabled_at, launched_by_autostart, read, refresh_at, remove,
        set_enabled_at, stored_executable, write,
    };

    // A private key keeps the tests away from the real login entry.
    const TEST_ROOT: &str = r"Software\wallpaper-lite-test";
    const TEST_KEY: &str = r"Software\wallpaper-lite-test\Run";

    // Each test removes the shared test key, so the registry tests must not overlap.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    const TEST_APPROVED: &str = r"Software\wallpaper-lite-test\Approved";

    #[test]
    fn stored_executable_handles_quoted_and_plain_commands() {
        assert_eq!(
            stored_executable(r#""C:\Program Files\app.exe" --autostart"#),
            r"C:\Program Files\app.exe"
        );
        assert_eq!(stored_executable(r"C:\app.exe --autostart"), r"C:\app.exe");
    }

    #[test]
    fn refresh_repoints_only_an_entry_whose_executable_is_gone() {
        let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _cleanup = Cleanup;
        let new = r#""C:\New\app.exe" --autostart"#;
        let old = r#""C:\Old\app.exe" --autostart"#;

        // Off stays off: nothing is created.
        refresh_at(TEST_KEY, "refresh", new, |_| false).unwrap();
        assert_eq!(read(TEST_KEY, "refresh").unwrap(), None);

        // The old install still exists, so it keeps its entry.
        write(TEST_KEY, "refresh", old).unwrap();
        refresh_at(TEST_KEY, "refresh", new, |_| true).unwrap();
        assert_eq!(read(TEST_KEY, "refresh").unwrap().as_deref(), Some(old));

        // The old install is gone, so the entry follows the running app.
        refresh_at(TEST_KEY, "refresh", new, |_| false).unwrap();
        assert_eq!(read(TEST_KEY, "refresh").unwrap().as_deref(), Some(new));
    }

    #[test]
    fn a_startup_item_disabled_in_windows_reads_as_off_until_enabled_again() {
        use windows::{
            Win32::System::Registry::{HKEY_CURRENT_USER, REG_BINARY, RegSetKeyValueW},
            core::{HSTRING, PCWSTR},
        };

        let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _cleanup = Cleanup;
        let command = r#""C:\Apps\app.exe" --autostart"#;
        assert!(!is_enabled_at(TEST_KEY, TEST_APPROVED, "approval").unwrap());

        set_enabled_at(TEST_KEY, TEST_APPROVED, "approval", command, true).unwrap();
        assert!(is_enabled_at(TEST_KEY, TEST_APPROVED, "approval").unwrap());

        // Task Manager records a disabled item with an odd first byte.
        let subkey = HSTRING::from(TEST_APPROVED);
        let name = HSTRING::from("approval");
        let disabled = [3_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // SAFETY: names are NUL-terminated and the data slice outlives the call.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_BINARY.0,
                Some(disabled.as_ptr().cast()),
                disabled.len() as u32,
            )
        };
        assert_eq!(status.0, 0);
        assert!(!is_enabled_at(TEST_KEY, TEST_APPROVED, "approval").unwrap());

        // Turning it on again clears the disabled choice.
        set_enabled_at(TEST_KEY, TEST_APPROVED, "approval", command, true).unwrap();
        assert!(is_enabled_at(TEST_KEY, TEST_APPROVED, "approval").unwrap());
        set_enabled_at(TEST_KEY, TEST_APPROVED, "approval", command, false).unwrap();
        assert!(!is_enabled_at(TEST_KEY, TEST_APPROVED, "approval").unwrap());
    }

    #[test]
    fn registry_value_round_trips_and_removal_is_idempotent() {
        let _lock = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
