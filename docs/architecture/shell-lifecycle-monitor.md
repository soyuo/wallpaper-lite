# Explorer shell lifecycle monitor

## Detection

The monitor watches the process ID recorded by the validated `DesktopHost` using a process handle
opened with `SYNCHRONIZE`. `WaitForSingleObject` waits up to 250 ms for process termination; a
separate validation after each timeout catches a stale Progman/WorkerW handle even when Explorer
keeps the process alive. Windows marks a process object signaled when the process terminates, and
`WaitForSingleObject` returns when its target is signaled or the timeout elapses
([process termination](https://learn.microsoft.com/en-us/windows/win32/procthread/terminating-a-process),
[WaitForSingleObject](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)).

On loss of the current shell host, the worker emits one `ExplorerExited` event and retries the
existing bounded desktop-host discovery every 100 ms. Once discovery and process-handle opening
succeed, it emits one `ExplorerRestarted` event with the new process ID and host kind. The state
transition prevents duplicate exit/start events during repeated or fast shell changes. Events use a
four-entry bounded channel; if the host has not drained it, the monitor retries without dropping
the pending state event or spinning.

The host polls the event queue while the named-pipe server continues running. It records separate
`shell_exited` and `shell_restarted` events. A monitor failure is logged and stops the host so it
cannot silently continue without shell lifecycle visibility. Dropping the monitor sets its stop
flag, joins its worker within the 250 ms wait bound, and closes the process handle through RAII.

## Recovery

`host/src/recovery.rs` owns the current `AttachedDesktopWindow` on a dedicated worker thread. On a
shell-exit event it drops the old child window before attempting rediscovery. On restart it uses
the existing validated discovery and attachment APIs; if the shell is not ready or attachment
races with another restart, it retries after 100 ms with exponential backoff capped at 1 second.
The worker waits on its command channel during backoff, so shutdown and newer lifecycle events
interrupt the delay. It keeps only one attached-window owner, which prevents duplicate wallpaper
children.

Recovery replaces the shell child window without restarting the host process, IPC server, or D3D11
device. The Phase 1 host has no media selection or playback state yet, so content continuity cannot
be exercised until the media and playback phases; recovery leaves those future host-owned states
outside the window lifecycle.

During host shutdown, `DesktopRecoveryWorker::stop()` queues `Shutdown`, wakes any retry wait, and
joins the worker before the lifecycle monitor is dropped. This stops new attachments and releases
the current child first; the monitor then sets its stop flag, joins its thread, and closes the
Explorer process handle. Calling `stop()` repeatedly is safe.

## Verification

The desktop tests cover ordered exit/restart transitions, duplicate suppression across rapid
restarts, startup failure for an unavailable process, and a quiet start/stop against the current
Windows 11 Explorer process. A live smoke check on Windows 11 Education 10.0.26300 terminated the
shell process (PID 11436) while the host monitor was running. The same host logged
`event="shell_exited" shell_process_id=11436`, then discovered the replacement shell (PID 6616) and
logged `event="shell_restarted" shell_process_id=6616`. The host remained running through both
events; stopping it left no wallpaper child window in the shell tree. Windows 10 runtime
verification is pending.

For recovery, a live Windows 11 Education 10.0.26300 smoke check terminated Explorer PID 8964
while host PID 30908 was active. The monitor reported replacement shell PID 28784, and the
recovery worker logged `event="wallpaper_window_recovered"`. UI Automation found exactly one
`WallpaperLiteDesktopChild` owned by PID 30908 with a `Progman` parent; the Start button Toggle
pattern worked and Escape dismissed the Start UI. The host remained alive until test cleanup; after
shutdown there was no host process or child window left.

For shutdown ordering, a Windows 11 Education 10.0.26300 smoke check started primary host PID 5072
and duplicate host PID 2704. The duplicate exited through the IPC server error path with Windows
error 231 (pipe busy). `EnumChildWindows` found exactly one wallpaper child, owned by the primary
host, after the duplicate exited, confirming its child was released before the process exit. The
primary was terminated after the measurement; no wallpaper host process or child window remained
after test cleanup. The host unit test also queues shell exit/restart events before calling
`stop()` twice, and both stops complete successfully. Windows 10 runtime verification and a live
Explorer exit during host shutdown remain pending.
