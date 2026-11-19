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

For the Tauri UI's Node.js, WebView2, and development commands, see
[`apps/ui/README.md`](../../apps/ui/README.md).


See [Phase 0 Windows acceptance](phase-0-acceptance.md) for the integrated GUI, IPC, and D3D11 runtime results and reproduction steps.

See [Phase 0 Windows acceptance](phase-0-acceptance.md) for the integrated GUI, IPC, and D3D11 runtime results and reproduction steps.
