use serde::Serialize;

/// Stable error identifiers shared by native components and the UI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    HostUnavailable,
    Timeout,
    Disconnected,
    MalformedMessage,
    MalformedRequest,
    MalformedResponse,
    MessageTooLarge,
    ProtocolVersionMismatch,
    UnsupportedPlatform,
    TransportError,
    Internal,
}

/// Safe error data that may cross into the UI. Diagnostic details stay in logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: &'static str,
}

impl AppError {
    pub const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
}
