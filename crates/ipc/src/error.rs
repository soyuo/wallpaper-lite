use std::{error::Error, fmt, io};

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
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "{}: {error}", self.code()),
            _ => f.write_str(self.code()),
        }
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
