# Windows development setup

## Requirements

- Windows 10 or Windows 11, x64.
- Rust stable with the `x86_64-pc-windows-msvc` target.
- Visual Studio 2022 Build Tools with the Desktop development with C++ workload
  and a Windows SDK, so Cargo can link Windows binaries.
- PowerShell for the commands below.

Install the Rust target from PowerShell:

```powershell
rustup toolchain install stable
rustup target add --toolchain stable x86_64-pc-windows-msvc
```

## Validate the workspace

Run from the repository root:

```powershell
cargo +stable metadata --no-deps --format-version 1
cargo +stable check --workspace --target x86_64-pc-windows-msvc
```

The Tauri UI's Node.js and package-manager requirements will be documented with
the UI scaffold in issue #8.
