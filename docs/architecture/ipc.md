# UI-to-host IPC

Phase 0 uses one Windows named pipe to prove the process boundary between the
Tauri UI and `wallpaper-host.exe`. The transport currently supports only a
request/response ping; media commands are outside this protocol version.

## Endpoint and framing

- Pipe: `\\.\pipe\wallpaper-lite`.
- One client request is handled at a time. The host disconnects the client and
  accepts another request on the same pipe instance.
- Each message is UTF-8 JSON preceded by a four-byte unsigned little-endian
  payload length. The length must be between 1 and 4096 bytes before the host
  or client allocates a payload buffer.
- The client retries a busy pipe every 25 ms for at most one second. No server
  produces the `host_unavailable` error. The request/response exchange and the
  host's wait for one request each have a two-second limit.
- The client closes its pipe handle when the request returns. The host calls
  `DisconnectNamedPipe` through Tokio's named-pipe server API before accepting
  the next client. Process shutdown closes the host's remaining handle.

## Version 1 ping

Request:

```json
{"version":1,"request_id":1,"command":"ping"}
```

Response:

```json
{"version":1,"request_id":1,"result":"pong"}
```

The response must use protocol version 1 and echo the request ID. Invalid JSON,
an unexpected result, or an ID mismatch returns `malformed_response`. A version
mismatch returns `protocol_version_mismatch`.

## Error codes

| Code | Meaning |
| --- | --- |
| `host_unavailable` | No host pipe exists. |
| `timeout` | Pipe remained busy or request/response I/O exceeded its deadline. |
| `disconnected` | The peer closed the pipe before completing the message. |
| `malformed_response` | Response JSON or correlation fields are invalid. |
| `message_too_large` | A frame exceeds 4096 bytes. |
| `protocol_version_mismatch` | The peer uses a different protocol version. |
| `transport_error` | Another Windows pipe I/O error occurred. |
| `unsupported_platform` | Named-pipe transport is only implemented on Windows. |

`wallpaper-ui` exposes the request as the Tauri command `host_ping`; the React
host adapter invokes it. `wallpaper-host` runs the server independently. See
[`apps/ui/README.md`](../../apps/ui/README.md) for the development round-trip
steps.
