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
    GraphicsDeviceLost,
    ImageDecodeFailed,
    VideoSourceOpenFailed,
    VideoDecodeFailed,
    LibraryStorageFailed,
    InvalidLibraryEntry,
    EditorSettingsStorageFailed,
    InvalidEditorSettings,
    EditorApplyFailed,
    EditorApplyRollbackFailed,
    EditorWallpaperNotActive,
    PlaybackSettingsStorageFailed,
    InvalidPlaybackSettings,
    PlaybackControlFailed,
    PlaybackSettingsRollbackFailed,
    DisplayEnumerationFailed,
    DisplayProfilesStorageFailed,
    InvalidDisplayProfiles,
    DisplayProfilesApplyFailed,
    DisplayProfilesRollbackFailed,
    PerformancePolicyStorageFailed,
    InvalidPerformancePolicy,
    PerformancePolicyApplyFailed,
    PerformancePolicyRollbackFailed,
    DesktopHostUnavailable,
    HostLaunchFailed,
    AutostartFailed,
    BackupFailed,
    InvalidBackup,
    RestoreFailed,
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
            Self::GraphicsDeviceLost => "graphics_device_lost",
            Self::ImageDecodeFailed => "image_decode_failed",
            Self::VideoSourceOpenFailed => "video_source_open_failed",
            Self::VideoDecodeFailed => "video_decode_failed",
            Self::LibraryStorageFailed => "library_storage_failed",
            Self::InvalidLibraryEntry => "invalid_library_entry",
            Self::EditorSettingsStorageFailed => "editor_settings_storage_failed",
            Self::InvalidEditorSettings => "invalid_editor_settings",
            Self::EditorApplyFailed => "editor_apply_failed",
            Self::EditorApplyRollbackFailed => "editor_apply_rollback_failed",
            Self::EditorWallpaperNotActive => "editor_wallpaper_not_active",
            Self::PlaybackSettingsStorageFailed => "playback_settings_storage_failed",
            Self::InvalidPlaybackSettings => "invalid_playback_settings",
            Self::PlaybackControlFailed => "playback_control_failed",
            Self::PlaybackSettingsRollbackFailed => "playback_settings_rollback_failed",
            Self::DisplayEnumerationFailed => "display_enumeration_failed",
            Self::DisplayProfilesStorageFailed => "display_profiles_storage_failed",
            Self::InvalidDisplayProfiles => "invalid_display_profiles",
            Self::DisplayProfilesApplyFailed => "display_profiles_apply_failed",
            Self::DisplayProfilesRollbackFailed => "display_profiles_rollback_failed",
            Self::PerformancePolicyStorageFailed => "performance_policy_storage_failed",
            Self::InvalidPerformancePolicy => "invalid_performance_policy",
            Self::PerformancePolicyApplyFailed => "performance_policy_apply_failed",
            Self::PerformancePolicyRollbackFailed => "performance_policy_rollback_failed",
            Self::DesktopHostUnavailable => "desktop_host_unavailable",
            Self::HostLaunchFailed => "host_launch_failed",
            Self::AutostartFailed => "autostart_failed",
            Self::BackupFailed => "backup_failed",
            Self::InvalidBackup => "invalid_backup",
            Self::RestoreFailed => "restore_failed",
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
