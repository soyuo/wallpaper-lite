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
    WindowsInitializationFailed,
    GraphicsInitializationFailed,
    ImageDecodeFailed,
    VideoSourceOpenFailed,
    VideoDecodeFailed,
    LibraryStorageFailed,
    DesktopHostUnavailable,
    Internal,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostUnavailable => "host_unavailable",
            Self::Timeout => "timeout",
            Self::Disconnected => "disconnected",
            Self::MalformedMessage => "malformed_message",
            Self::MalformedRequest => "malformed_request",
            Self::MalformedResponse => "malformed_response",
            Self::MessageTooLarge => "message_too_large",
            Self::ProtocolVersionMismatch => "protocol_version_mismatch",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::TransportError => "transport_error",
            Self::WindowsInitializationFailed => "windows_initialization_failed",
            Self::GraphicsInitializationFailed => "graphics_initialization_failed",
            Self::ImageDecodeFailed => "image_decode_failed",
            Self::VideoSourceOpenFailed => "video_source_open_failed",
            Self::VideoDecodeFailed => "video_decode_failed",
            Self::LibraryStorageFailed => "library_storage_failed",
            Self::DesktopHostUnavailable => "desktop_host_unavailable",
            Self::Internal => "internal",
        }
    }
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
