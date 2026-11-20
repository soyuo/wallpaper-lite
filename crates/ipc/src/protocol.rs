use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use wallpaper_core::editor_settings::WallpaperEditorSettings;

use crate::{IpcError, error::from_io};

pub const PIPE_NAME: &str = r"\\.\pipe\wallpaper-lite";
pub const PLAYBACK_EVENTS_PIPE_NAME: &str = r"\\.\pipe\wallpaper-lite-playback-events";
pub const PROTOCOL_VERSION: u16 = 4;
pub const MAX_FRAME_BYTES: usize = 4 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const COMMAND_QUEUE_CAPACITY: usize = 16;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub request_id: u64,
    pub command: Command,
}

impl Request {
    pub fn ping(request_id: u64) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            command: Command::Ping,
        }
    }

    pub fn new(request_id: u64, command: Command) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            command,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Ping,
    LoadVideo {
        path: String,
    },
    ApplyEditorSettings {
        settings: WallpaperEditorSettings,
        source_path: String,
    },
    Play,
    Pause,
    Stop,
    Seek {
        position_100ns: i64,
    },
    SetSpeed {
        percent: u16,
    },
    SetFps {
        frames_per_second: u16,
    },
    GetPlaybackState,
    SubscribePlaybackState,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub request_id: u64,
    pub result: ResponseResult,
}

impl Response {
    pub fn pong(request_id: u64) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            result: ResponseResult::Pong,
        }
    }

    pub fn new(request_id: u64, result: ResponseResult) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id,
            result,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseResult {
    Pong,
    EditorSettingsApplied,
    PlaybackState(PlaybackStateEvent),
    Error { code: PlaybackCommandErrorCode },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IpcPlaybackState {
    Stopped,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlaybackStateEvent {
    pub sequence: u64,
    pub state: IpcPlaybackState,
    pub media_time_100ns: i64,
    pub duration_100ns: Option<i64>,
    pub speed_percent: u16,
    pub fps_limit: u16,
    pub loading_video: bool,
    pub last_error: Option<PlaybackCommandErrorCode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackCommandErrorCode {
    PlaybackUnavailable,
    VideoLoadInProgress,
    VideoNotLoaded,
    VideoLoadFailed,
    InvalidSeek,
    SeekFailed,
    UnsupportedSpeed,
    UnsupportedFps,
    PlaybackFailed,
    DecodeFailed,
    EditorSettingsInvalid,
    EditorSettingsApplyFailed,
    EditorWallpaperNotActive,
}

#[cfg(test)]
mod editor_settings_tests {
    use super::{Command, PROTOCOL_VERSION, Request, ResponseResult};
    use wallpaper_core::editor_settings::WallpaperEditorSettings;

    #[test]
    fn apply_editor_settings_round_trips_as_a_versioned_command() {
        let settings = WallpaperEditorSettings::default_for(7).unwrap();
        let request = Request {
            version: PROTOCOL_VERSION,
            request_id: 42,
            command: Command::ApplyEditorSettings {
                settings: settings.clone(),
                source_path: r"C:\Wallpapers\active.png".to_owned(),
            },
        };
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: Request = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.version, 4);
        assert_eq!(decoded.request_id, 42);
        assert_eq!(
            decoded.command,
            Command::ApplyEditorSettings {
                settings,
                source_path: r"C:\Wallpapers\active.png".to_owned(),
            }
        );
        assert_eq!(
            serde_json::to_value(ResponseResult::EditorSettingsApplied).unwrap(),
            serde_json::json!("editor_settings_applied")
        );
    }
}

pub struct PlaybackCommandRequest {
    command: Command,
    response: oneshot::Sender<ResponseResult>,
}

impl PlaybackCommandRequest {
    pub fn command(&self) -> &Command {
        &self.command
    }

    pub fn response_is_closed(&self) -> bool {
        self.response.is_closed()
    }

    pub fn respond(self, result: ResponseResult) -> bool {
        self.response.send(result).is_ok()
    }
}

#[derive(Clone)]
pub struct PlaybackCommandSender(mpsc::Sender<PlaybackCommandRequest>);

pub struct PlaybackCommandReceiver(mpsc::Receiver<PlaybackCommandRequest>);

pub fn playback_command_channel() -> (PlaybackCommandSender, PlaybackCommandReceiver) {
    let (sender, receiver) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    (
        PlaybackCommandSender(sender),
        PlaybackCommandReceiver(receiver),
    )
}

impl PlaybackCommandSender {
    pub(crate) async fn dispatch(&self, command: Command) -> ResponseResult {
        let (response, result) = oneshot::channel();
        if self
            .0
            .send(PlaybackCommandRequest { command, response })
            .await
            .is_err()
        {
            return ResponseResult::Error {
                code: PlaybackCommandErrorCode::PlaybackUnavailable,
            };
        }
        result.await.unwrap_or(ResponseResult::Error {
            code: PlaybackCommandErrorCode::PlaybackUnavailable,
        })
    }
}

impl PlaybackCommandReceiver {
    pub fn try_recv(
        &mut self,
    ) -> Result<PlaybackCommandRequest, tokio::sync::mpsc::error::TryRecvError> {
        self.0.try_recv()
    }
}

pub(crate) fn encode_message<T: Serialize>(message: &T) -> Result<Vec<u8>, IpcError> {
    let payload = serde_json::to_vec(message).map_err(|_| IpcError::MalformedMessage)?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::MessageTooLarge);
    }
    Ok(payload)
}

pub(crate) fn decode_request(payload: &[u8]) -> Result<Request, IpcError> {
    let request: Request =
        serde_json::from_slice(payload).map_err(|_| IpcError::MalformedRequest)?;
    if request.version != PROTOCOL_VERSION {
        return Err(IpcError::ProtocolVersionMismatch);
    }
    Ok(request)
}

pub(crate) fn decode_response(payload: &[u8], request_id: u64) -> Result<ResponseResult, IpcError> {
    let response: Response =
        serde_json::from_slice(payload).map_err(|_| IpcError::MalformedResponse)?;
    if response.version != PROTOCOL_VERSION {
        return Err(IpcError::ProtocolVersionMismatch);
    }
    if response.request_id != request_id {
        return Err(IpcError::MalformedResponse);
    }
    Ok(response.result)
}

fn checked_frame_len(length: u32) -> Result<usize, IpcError> {
    let length = length as usize;
    if length == 0 {
        return Err(IpcError::MalformedMessage);
    }
    if length > MAX_FRAME_BYTES {
        return Err(IpcError::MessageTooLarge);
    }
    Ok(length)
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, IpcError> {
    let mut length_prefix = [0_u8; 4];
    reader
        .read_exact(&mut length_prefix)
        .await
        .map_err(from_io)?;
    let length = checked_frame_len(u32::from_le_bytes(length_prefix))?;
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload).await.map_err(from_io)?;
    Ok(payload)
}

pub(crate) async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), IpcError> {
    if payload.is_empty() {
        return Err(IpcError::MalformedMessage);
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::MessageTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| IpcError::MessageTooLarge)?;
    writer
        .write_all(&length.to_le_bytes())
        .await
        .map_err(from_io)?;
    writer.write_all(payload).await.map_err(from_io)?;
    writer.flush().await.map_err(from_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_request_observes_client_cancellation() {
        let (response, receiver) = oneshot::channel();
        let request = PlaybackCommandRequest {
            command: Command::LoadVideo {
                path: "clip.mp4".to_owned(),
            },
            response,
        };

        assert!(!request.response_is_closed());
        drop(receiver);
        assert!(request.response_is_closed());
    }

    #[test]
    fn ping_protocol_round_trips_and_matches_request_id() {
        let request = Request::ping(42);
        let request_payload = encode_message(&request).expect("encode request");
        let decoded_request = decode_request(&request_payload).expect("decode request");
        assert_eq!(decoded_request.request_id, 42);
        assert_eq!(decoded_request.command, Command::Ping);

        let response = Response::pong(42);
        let response_payload = encode_message(&response).expect("encode response");
        decode_response(&response_payload, 42).expect("decode response");
        assert!(matches!(
            decode_response(&response_payload, 43),
            Err(IpcError::MalformedResponse)
        ));
    }

    #[test]
    fn playback_state_round_trips_async_load_status_and_error() {
        let response = Response::new(
            7,
            ResponseResult::PlaybackState(PlaybackStateEvent {
                sequence: 3,
                state: IpcPlaybackState::Playing,
                media_time_100ns: 250_000,
                duration_100ns: Some(10_000_000),
                speed_percent: 100,
                fps_limit: 60,
                loading_video: true,
                last_error: Some(PlaybackCommandErrorCode::VideoLoadFailed),
            }),
        );
        let payload = encode_message(&response).expect("encode playback snapshot");
        assert_eq!(
            decode_response(&payload, 7).expect("decode playback snapshot"),
            response.result
        );
    }

    #[test]
    fn malformed_or_oversized_messages_are_rejected() {
        assert!(matches!(
            decode_response(b"not-json", 1),
            Err(IpcError::MalformedResponse)
        ));
        assert!(matches!(
            checked_frame_len(0),
            Err(IpcError::MalformedMessage)
        ));
        assert!(matches!(
            checked_frame_len((MAX_FRAME_BYTES + 1) as u32),
            Err(IpcError::MessageTooLarge)
        ));
        assert!(matches!(
            encode_message(&"x".repeat(MAX_FRAME_BYTES)),
            Err(IpcError::MessageTooLarge)
        ));
    }

    #[test]
    fn unsupported_protocol_versions_are_distinct() {
        let payload = br#"{"version":2,"request_id":1,"result":"pong"}"#;
        assert!(matches!(
            decode_response(payload, 1),
            Err(IpcError::ProtocolVersionMismatch)
        ));

        let old_request = br#"{"version":1,"request_id":1,"command":"ping"}"#;
        assert!(matches!(
            decode_request(old_request),
            Err(IpcError::ProtocolVersionMismatch)
        ));
    }
}
