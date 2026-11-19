# Desktop host discovery

The desktop crate discovers a fresh, validated Explorer shell window before `wallpaper-host.exe` initializes D3D11 or starts its IPC server. This establishes the trusted shell-owned handle for the attachment work in issue #2; it does not create the wallpaper window or choose its z-order. Explorer restart detection and recovery remain in issues #3 and #4.

## Candidate validation

Discovery uses `GetShellWindow` as the authoritative shell reference and reads its owning process with `GetWindowThreadProcessId`. It accepts only a top-level window whose class is `Progman` or `WorkerW`, whose process matches the shell process, and which directly owns a `SHELLDLL_DefView` child. If the shell window itself owns that view, it is used directly. Otherwise, `EnumWindows` enumerates top-level windows and discovery accepts exactly one matching candidate. Class name alone is never enough to select a WorkerW.

This read-only path sends no undocumented Explorer messages. The `DesktopHost` value contains a borrowed HWND, not an owned window. Call `validate_current` immediately before using it; validation checks that the window still exists and still has the same process, class, and direct shell-view child. Windows can recycle HWND values after validation, so later attachment code must rediscover or revalidate at the point of use.

The Win32 APIs used are documented by Microsoft: [GetShellWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getshellwindow), [GetWindowThreadProcessId](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid), [GetClassNameW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclassnamew), [FindWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-findwindowexw), [EnumWindows](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enumwindows), and [IsWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindow). `EnumWindows` is used for top-level enumeration instead of walking windows with `GetWindow`.

## Failure and retry behavior

Discovery makes at most three attempts and waits 50 ms between attempts, for no more than 100 ms of deliberate delay. A missing shell, missing desktop view, ambiguous candidates, stale handle, or Win32 enumeration failure has a distinct internal `DesktopError`. The host reports the safe shared code `desktop_host_unavailable` and a sanitized reason; enumeration diagnostics contain only an HRESULT. No retry loop runs indefinitely or spins without a delay.

## Verification

On Windows 11 Education 10.0.26300 x64, the shell topology probe found `GetShellWindow` returning a `Progman` window with a direct `SHELLDLL_DefView` child. Other top-level WorkerW windows were present without that child, confirming that a class-only lookup would select unrelated windows. Starting `wallpaper-host.exe` logged `event="desktop_host_discovered" host_kind="progman"` and continued through D3D11 initialization.

The Windows-target desktop tests cover rejection of unrelated WorkerW windows, a different process, ambiguous matching candidates, stale handles, and discovery/revalidation of the current shell. Verification commands:

```powershell
cargo +1.85.0 check --workspace --target x86_64-pc-windows-msvc
cargo +1.85.0 test -p wallpaper-desktop --target x86_64-pc-windows-msvc -- --test-threads=1
cargo +1.85.0 build -p wallpaper-host --target x86_64-pc-windows-msvc
cargo +stable clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings
cargo +stable fmt --all -- --check
```

Windows 10 runtime verification was not available in this environment. Discovery uses documented shell/window APIs and does not attach windows; Windows 10 runtime behavior remains to be confirmed on that OS. The current implementation does not monitor Explorer restarts; that recovery is tracked separately.
