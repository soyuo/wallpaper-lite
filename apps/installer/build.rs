use std::{env, fs, path::PathBuf};

/// Files the setup program carries. A missing file becomes an empty placeholder so a plain
/// `cargo build` works; `Payload::is_complete` then makes setup refuse to install.
const PAYLOAD_FILES: [&str; 2] = ["wallpaper-ui.exe", "wallpaper-host.exe"];

fn main() {
    println!("cargo:rerun-if-env-changed=WALLPAPER_PAYLOAD_DIR");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = env::var_os("WALLPAPER_PAYLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("payload"));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("payload");
    fs::create_dir_all(&out).unwrap();
    for name in PAYLOAD_FILES {
        let from = source.join(name);
        println!("cargo:rerun-if-changed={}", from.display());
        match fs::read(&from) {
            Ok(bytes) => fs::write(out.join(name), bytes).unwrap(),
            Err(_) => fs::write(out.join(name), []).unwrap(),
        }
    }
    tauri_build::build();
}
