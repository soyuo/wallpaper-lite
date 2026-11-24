# Errors and logging

Rust components use `tracing` for logs and `wallpaper_core::error::AppError` for
safe errors that cross into the UI. Keep the public error contract separate from
diagnostic details.

## Error boundary

`AppError` serializes as `{ "code": "...", "message": "..." }`. `code` is a
stable snake-case identifier for programmatic handling; `message` is safe to
show to a user. Add new codes in `wallpaper-core` and provide a plain-language
message. Do not include OS error text, paths, filenames, or other diagnostic
data in `message`.

`wallpaper-ipc::IpcError` keeps transport-specific causes inside Rust. Convert
it with `IpcError::to_app_error()` before returning it from a Tauri command.
Write `IpcError::diagnostic_summary()` to the log for internal troubleshooting;
do not return `Display`, `Debug`, or the diagnostic summary across the UI
boundary. The UI adapter validates the payload and displays only `message`.

## Log conventions

Use `tracing` with the component target and structured fields:

- `ERROR`: a component cannot continue or a required operation failed.
- `WARN`: a recoverable failure, rejected request, timeout, or disconnect.
- `INFO`: startup, shutdown, and successful user-visible operations.
- `DEBUG` and `TRACE`: development detail; keep high-volume events out of the
  default `INFO` output.
- Include `component`, `event`, and stable `error_code` fields when relevant.
  Keep messages factual and concise.
- Do not log wallpaper paths, filenames, user content, monitor identities,
  secrets, raw IPC frames, or arbitrary OS error strings. For transport errors,
  log the error code, `io::ErrorKind`, and numeric OS error code only.

Both processes write daily files under
`%LOCALAPPDATA%\kr.soyuo.wallpaper\logs`:

- UI backend: `ui.log` plus up to six numbered backups (`ui.log.1` through
  `ui.log.6`); at most seven files total, rotated daily or at 5 MiB.
  If file logging cannot initialize, the UI continues with stderr logging.
- Host: `host.*.log`; at most seven daily files. If the host cannot create its
  log file, it falls back to stderr without printing the failed path.

The UI file target follows Tauri's [logging directory guidance](https://v2.tauri.app/plugin/logging/).
The host uses [`tracing-appender`](https://docs.rs/tracing-appender/latest/tracing_appender/rolling/struct.Builder.html)
with the same directory and retention limit.

The host bootstrap logs `windows_initialization_failed` or
`graphics_initialization_failed` with the shared error code and a sanitized
diagnostic containing the stage and numeric HRESULT. It does not include raw
Windows error text, driver paths, or user paths.
