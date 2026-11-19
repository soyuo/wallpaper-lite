use std::{error::Error, fmt, io};
use wallpaper_core::error::{AppError, ErrorCode};

#[derive(Debug)]
pub enum IpcError {
    HostUnavailable,
    Timeout,
    Disconnected,
    MalformedMessage,
    MalformedRequest,
    MalformedResponse,
    MessageTooLarge,
    ProtocolVersionMismatch,
    UnsupportedPlatform,
    Transport(io::Error),
}

impl IpcError {
    pub fn code(&self) -> &'static str {
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
            Self::Transport(_) => "transport_error",
        }
    }

    /// Convert transport errors to safe UI data without exposing OS details.
    pub fn to_app_error(&self) -> AppError {
        let (code, message) = match self {
            Self::HostUnavailable => (ErrorCode::HostUnavailable, "Wallpaper host is not running."),
            Self::Timeout => (
                ErrorCode::Timeout,
                "Wallpaper host did not respond in time.",
            ),
            Self::Disconnected => (
                ErrorCode::Disconnected,
                "The connection to wallpaper host was interrupted.",
            ),
            Self::MalformedMessage => (
                ErrorCode::MalformedMessage,
                "Wallpaper host returned an invalid message.",
            ),
            Self::MalformedRequest => (
                ErrorCode::MalformedRequest,
                "Wallpaper host rejected the request.",
            ),
            Self::MalformedResponse => (
                ErrorCode::MalformedResponse,
                "Wallpaper host returned an invalid response.",
            ),
            Self::MessageTooLarge => (
                ErrorCode::MessageTooLarge,
                "Wallpaper request exceeded the supported size.",
            ),
            Self::ProtocolVersionMismatch => (
                ErrorCode::ProtocolVersionMismatch,
                "Wallpaper host uses an incompatible protocol.",
            ),
            Self::UnsupportedPlatform => (
                ErrorCode::UnsupportedPlatform,
                "This operation is only supported on Windows.",
            ),
            Self::Transport(_) => (
                ErrorCode::TransportError,
                "Could not communicate with wallpaper host.",
            ),
        };

        AppError::new(code, message)
    }

    /// Return diagnostic fields that do not include arbitrary OS error text.
    pub fn diagnostic_summary(&self) -> String {
        match self {
            Self::Transport(error) => format!(
                "code={} kind={:?} os_code={:?}",
                self.code(),
                error.kind(),
                error.raw_os_error()
            ),
            _ => format!("code={}", self.code()),
        }
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::IpcError;
    use wallpaper_core::error::ErrorCode;

    #[test]
    fn public_error_does_not_include_transport_details() {
        let error = IpcError::Transport(std::io::Error::other(
            "C:\\Users\\example\\private\\wallpaper.jpg",
        ));
        let app_error = error.to_app_error();
        let serialized = serde_json::to_string(&app_error).expect("serialize public error");

        assert_eq!(app_error.code, ErrorCode::TransportError);
        assert_eq!(
            app_error.message,
            "Could not communicate with wallpaper host."
        );
        assert!(!serialized.contains("C:\\Users\\example"));
        assert!(!error.diagnostic_summary().contains("C:\\Users\\example"));
    }
}

impl Error for IpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) fn from_io(error: io::Error) -> IpcError {
    #[cfg(windows)]
    if error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_PIPE_NOT_CONNECTED as i32)
    {
        return IpcError::Disconnected;
    }

    match error.kind() {
        io::ErrorKind::NotFound => IpcError::HostUnavailable,
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotConnected => IpcError::Disconnected,
        _ => IpcError::Transport(error),
    }
}
