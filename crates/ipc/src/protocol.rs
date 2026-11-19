use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{IpcError, error::from_io};

pub const PIPE_NAME: &str = r"\\.\pipe\wallpaper-lite";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 4 * 1024;
pub(crate) const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Ping,
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
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseResult {
    Pong,
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

pub(crate) fn decode_response(payload: &[u8], request_id: u64) -> Result<(), IpcError> {
    let response: Response =
        serde_json::from_slice(payload).map_err(|_| IpcError::MalformedResponse)?;
    if response.version != PROTOCOL_VERSION {
        return Err(IpcError::ProtocolVersionMismatch);
    }
    if response.request_id != request_id || response.result != ResponseResult::Pong {
        return Err(IpcError::MalformedResponse);
    }
    Ok(())
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
    }
}
