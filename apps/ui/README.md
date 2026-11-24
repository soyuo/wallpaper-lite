# wallpaper Lite UI

The desktop UI uses Tauri 2, React, TypeScript, and Vite. The React app lives in
`src`; the Rust/Tauri backend lives in `src-tauri` and is a member of the root
Cargo workspace. `src/host-adapter.ts` is the frontend boundary for host calls.
Issue #9 implements the `host_ping` Tauri command and its named-pipe round trip.

## Requirements

- Windows 10 or Windows 11, x64.
- Rust stable with the `x86_64-pc-windows-msvc` target.
- Visual Studio 2022 Build Tools with the Desktop development with C++ workload
  and a Windows SDK.
- Microsoft Edge WebView2 Runtime.
- Node.js LTS and npm.

See the repository's [Windows development setup](../../docs/development/setup.md)
for the Rust and MSVC toolchain setup. Tauri's Windows prerequisites and
WebView2 details are in the [official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

## Build and run

Run these commands from `apps/ui`:

```powershell
npm ci
npm run build
npm run tauri dev
```

`npm run build` checks the TypeScript app and builds its Vite assets. The
`npm run tauri dev` command starts the frontend development server, builds the
Tauri backend, and opens the desktop window. Close the window to exit the
development app. Installer bundling is disabled until the release phase.

## Verify UI-to-host IPC

Start the host in a separate PowerShell terminal from the repository root:

```powershell
cargo +stable run -p wallpaper-host
```

Then run `npm run tauri dev` from `apps/ui` and select **Check wallpaper host**.
The window reports a successful `pong` response. The host accepts another
connection after each request, so the check can be repeated. If the host is not
running, the UI reports `host_unavailable`; start the host and check again to
verify reconnection. Stop the host with Ctrl+C.

The wire format, message bound, timeouts, and error codes are documented in
[`docs/architecture/ipc.md`](../../docs/architecture/ipc.md).
