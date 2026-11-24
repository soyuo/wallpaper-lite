//! Per-user install, upgrade and uninstall. Paths and registry names follow the NSIS installer
//! this program replaces so an existing install is found and taken over.

use std::{
    env, fs, io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
};

use crate::{InstallOptions, UninstallOptions, payload, shortcut};

const PRODUCT: &str = "wallpaper Lite";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\wallpaper Lite";
const OLD_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\wallpaper-lite";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_VALUE: &str = "wallpaper-lite";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn default_install_dir() -> PathBuf {
    PathBuf::from(env::var_os("LOCALAPPDATA").unwrap_or_default()).join(PRODUCT)
}

pub fn registered_install_dir() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?;
    let location: String = key.get_value("InstallLocation").ok()?;
    let path = PathBuf::from(location.trim_matches('"'));
    path.join("wallpaper-ui.exe").exists().then_some(path)
}

fn start_menu_link() -> PathBuf {
    PathBuf::from(env::var_os("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(format!("{PRODUCT}.lnk"))
}

fn desktop_link() -> PathBuf {
    PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default()).join("Desktop").join(format!("{PRODUCT}.lnk"))
}

/// Stops the app first so it cannot start the host again, then the host.
fn stop_processes() {
    for image in ["wallpaper-ui.exe", "wallpaper-host.exe"] {
        let _ = Command::new("taskkill").args(["/F", "/IM", image]).creation_flags(CREATE_NO_WINDOW).status();
    }
    thread::sleep(Duration::from_millis(500));
}

/// Removes a copy installed under the earlier name `wallpaper-lite`; reports its launch-at-login choice.
fn remove_old_name_install() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let had_autostart = hkcu.open_subkey(RUN_KEY).and_then(|run| run.get_value::<String, _>(RUN_VALUE)).is_ok();
    if let Ok(old) = hkcu.open_subkey(OLD_UNINSTALL_KEY) {
        if let Ok(command) = old.get_value::<String, _>("UninstallString") {
            let _ = Command::new("cmd")
                .args(["/C", &format!("{command} /S")])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            for _ in 0..60 {
                thread::sleep(Duration::from_millis(500));
                if hkcu.open_subkey(OLD_UNINSTALL_KEY).is_err() {
                    break;
                }
            }
        }
    }
    had_autostart
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| format!("Could not write {}: {error}", path.display()))
}

/// Names of the app processes that are running and must be closed before files are replaced.
pub fn running_apps() -> Vec<String> {
    ["wallpaper-ui.exe", "wallpaper-host.exe"]
        .into_iter()
        .filter(|image| {
            Command::new("tasklist")
                .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).to_lowercase().contains(image))
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect()
}

/// Adds or removes the sign-in launch entry for the installed app.
pub fn set_launch_at_login(install_dir: &Path, enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if enabled {
        let app = install_dir.join("wallpaper-ui.exe");
        let (run, _) = hkcu.create_subkey(RUN_KEY).map_err(|error| error.to_string())?;
        run.set_value(RUN_VALUE, &format!("\"{}\" --autostart", app.display())).map_err(|error| error.to_string())
    } else {
        if let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            let _ = run.delete_value(RUN_VALUE);
        }
        Ok(())
    }
}

/// Copies the app files and registers the install. `progress(step, status)` reports each step
/// with status `"working"` or `"done"`.
pub fn install(options: &InstallOptions, progress: &dyn Fn(&str, &str)) -> Result<(), String> {
    if !payload::is_complete() {
        return Err("This setup program was built without the app files.".into());
    }
    let dir = PathBuf::from(&options.install_dir);
    if !dir.is_absolute() {
        return Err("Choose a full folder path.".into());
    }
    stop_processes();
    let had_autostart = remove_old_name_install();
    fs::create_dir_all(&dir).map_err(|error| format!("Could not create {}: {error}", dir.display()))?;

    let app = dir.join("wallpaper-ui.exe");
    progress("wallpaper-ui.exe", "working");
    write_file(&app, payload::APP_EXE)?;
    progress("wallpaper-ui.exe", "done");
    progress("wallpaper-host.exe", "working");
    write_file(&dir.join("wallpaper-host.exe"), payload::HOST_EXE)?;
    progress("wallpaper-host.exe", "done");
    let uninstaller = dir.join("uninstall.exe");
    progress("uninstall.exe", "working");
    let current = env::current_exe().map_err(|error| error.to_string())?;
    if current != uninstaller {
        fs::copy(&current, &uninstaller).map_err(|error| format!("Could not write uninstall.exe: {error}"))?;
    }
    progress("uninstall.exe", "done");
    progress("Shortcuts and registration", "working");

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(UNINSTALL_KEY).map_err(|error| error.to_string())?;
    let size_kb = (payload::APP_EXE.len() + payload::HOST_EXE.len()) as u32 / 1024;
    let uninstall_command = format!("\"{}\" --uninstall", uninstaller.display());
    for (name, value) in [
        ("DisplayName", PRODUCT.to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "soyuo".to_string()),
        ("InstallLocation", dir.display().to_string()),
        ("DisplayIcon", app.display().to_string()),
        ("UninstallString", uninstall_command.clone()),
        ("QuietUninstallString", format!("{uninstall_command} --silent")),
    ] {
        key.set_value(name, &value).map_err(|error| error.to_string())?;
    }
    let _ = key.set_value("NoModify", &1u32);
    let _ = key.set_value("NoRepair", &1u32);
    let _ = key.set_value("EstimatedSize", &size_kb);

    let start_menu = start_menu_link();
    if let Some(parent) = start_menu.parent() {
        let _ = fs::create_dir_all(parent);
    }
    shortcut::create(&start_menu, &app, &dir)?;
    if options.desktop_shortcut {
        shortcut::create(&desktop_link(), &app, &dir)?;
    }
    if options.launch_at_login || had_autostart {
        set_launch_at_login(&dir, true)?;
    }
    progress("Shortcuts and registration", "done");
    Ok(())
}

pub fn uninstall(options: &UninstallOptions) -> Result<(), String> {
    stop_processes();
    let dir = registered_install_dir()
        .or_else(|| env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)))
        .ok_or("The install folder was not found.")?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
        let _ = run.delete_value(RUN_VALUE);
    }
    if let Ok(approved) = hkcu.open_subkey_with_flags(APPROVED_KEY, KEY_SET_VALUE) {
        let _ = approved.delete_value(RUN_VALUE);
    }
    for link in [start_menu_link(), desktop_link()] {
        let _ = fs::remove_file(link);
    }
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);

    for name in ["wallpaper-ui.exe", "wallpaper-host.exe"] {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not remove {name}: {error}")),
        }
    }
    if options.delete_data {
        // Library, editor settings and display profiles. Source images and videos are never touched.
        for base in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(root) = env::var_os(base) {
                for folder in ["WallpaperLite", "kr.soyuo.wallpaper"] {
                    let _ = fs::remove_dir_all(PathBuf::from(&root).join(folder));
                }
            }
        }
    }
    // This program runs from the install folder, so a detached command removes the folder after it
    // exits. It retries for about a minute because the wizard window can stay open for a while, and
    // it runs from the temp folder so it does not hold the install folder as its own directory.
    // `raw_arg` is needed: `args` would escape the quotes as `\"`, which `cmd` reads wrongly and
    // which breaks on the space in "wallpaper Lite".
    let dir = dir.display();
    let _ = Command::new("cmd")
        .raw_arg(format!(
            "/D /C \"for /L %i in (1,1,60) do (ping -n 2 127.0.0.1 >nul & rmdir /s /q \"{dir}\" 2>nul & if not exist \"{dir}\" exit)\""
        ))
        .current_dir(env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    Ok(())
}

pub fn launch(dir: &Path) -> Result<(), String> {
    Command::new(dir.join("wallpaper-ui.exe"))
        .current_dir(dir)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not start the app: {error}"))
}
