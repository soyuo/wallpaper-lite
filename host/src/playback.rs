use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_core::playback_settings::{LoopMode, PlaybackSettings, SegmentRange};
use wallpaper_ipc::protocol::{
    Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackCommandReceiver,
    PlaybackCommandRequest, PlaybackStateEvent, ResponseResult,
};
use wallpaper_media::{
    DecodedVideoFrame, MediaError, PlaybackScheduler, PlaybackState, VideoFpsLimit, VideoMetadata,
    VideoPlaybackSpeed, VideoReadEvent, VideoSource,
};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::independent_displays::DisplayRefreshRequest;
use crate::recovery::{ApplySettingsError, DesktopRecoveryWorker, RecoveryFrameSender};

const TICK_INTERVAL: Duration = Duration::from_millis(8);
const QUEUE_TARGET: usize = 3;
const WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const SOURCE_WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(1);

fn segment_bounds_100ns(segment: SegmentRange) -> Result<(i64, i64), PlaybackCommandErrorCode> {
    let convert = |milliseconds: u64| {
        milliseconds
            .checked_mul(10_000)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(PlaybackCommandErrorCode::PlaybackSettingsInvalid)
    };
    Ok((convert(segment.start_ms())?, convert(segment.end_ms())?))
}

pub(crate) struct PlaybackWorker {
    thread: Option<JoinHandle<()>>,
    stop_sender: Option<SyncSender<()>>,
}

impl PlaybackWorker {
    pub(crate) fn start(
        commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        Self::start_with_display_refresh(commands, device, recovery, None)
    }

    pub(crate) fn start_with_display_refresh(
        mut commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        let frame_sender = recovery.frame_sender();
        let (event_sender, event_receiver) = tokio::sync::watch::channel(PlaybackStateEvent {
            sequence: 0,
            wallpaper_id: None,
            state: IpcPlaybackState::Stopped,
            media_time_100ns: 0,
            duration_100ns: None,
            frame_rate_numerator: None,
            frame_rate_denominator: None,
            audio_available: false,
            audio_effective_muted: false,
            speed_percent: 100,
            fps_limit: 60,
            loading_video: false,
            last_error: None,
        });
        let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("wallpaper-video-playback".to_owned())
            .spawn(move || {
                let mut controller = PlaybackController::new_with_display_refresh(
                    device,
                    frame_sender,
                    event_sender,
                    display_refresh_sender,
                );
                loop {
                    match stop_receiver.try_recv() {
                        Ok(()) | Err(TryRecvError::Disconnected) => break,
                        Err(TryRecvError::Empty) => {}
                    }
                    match commands.try_recv() {
                        Ok(request) => {
                            if request.response_is_closed() {
                                continue;
                            }
                            if matches!(request.command(), Command::LoadVideo { .. }) {
                                controller.start_video_load(request);
                                continue;
                            }
                            let result = controller.execute(request.command().clone());
                            request.respond(result);
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                    }

                    controller.poll_video_load();
                    controller.tick();
                    thread::sleep(TICK_INTERVAL);
                }
            })?;

        Ok((
            Self {
                thread: Some(thread),
                stop_sender: Some(stop_sender),
            },
            event_receiver,
        ))
    }

    pub(crate) fn stop(&mut self) -> bool {
        if let Some(stop_sender) = self.stop_sender.take() {
            let _ = stop_sender.try_send(());
        }
        let Some(thread) = self.thread.as_ref() else {
            return true;
        };
        let deadline = Instant::now() + WORKER_STOP_TIMEOUT;
        while !thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            let thread = self.thread.take().expect("finished playback thread exists");
            if thread.join().is_err() {
                tracing::error!(
                    component = "media",
                    event = "playback_worker_panicked",
                    "Playback worker stopped after a panic"
                );
            }
            true
        } else {
            tracing::warn!(
                component = "media",
                event = "playback_worker_stop_timed_out",
                shutdown_deadline_seconds = WORKER_STOP_TIMEOUT.as_secs(),
                "Playback worker did not stop before the shutdown deadline"
            );
            false
        }
    }
}

impl Drop for PlaybackWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct PlaybackController {
    device: ID3D11Device,
    frame_sender: RecoveryFrameSender,
    source: Option<VideoSourceWorker>,
    pending_load: Option<PendingVideoLoad>,
    active_wallpaper_id: Option<u64>,
    deferred_frame: Option<VideoFrameMessage>,
    eos_pending: bool,
    scheduler: PlaybackScheduler,
    playback_settings: Option<PlaybackSettings>,
    clock_origin: Instant,
    sequence: u64,
    pending_error: Option<PlaybackCommandErrorCode>,
    loading_video: bool,
    last_error: Option<PlaybackCommandErrorCode>,
    event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
    last_event_sent: Instant,
    display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
}

struct PendingVideoLoad {
    wallpaper_id: u64,
    source: VideoSourceWorker,
}

struct VideoSourceWorker {
    source_path: PathBuf,
    command_sender: SyncSender<VideoSourceCommand>,
    stop_sender: Option<SyncSender<()>>,
    event_receiver: Receiver<VideoSourceEvent>,
    frame_receiver: Receiver<VideoFrameMessage>,
    thread: Option<JoinHandle<()>>,
    generation: u64,
    metadata: Option<VideoMetadata>,
    audio_available: bool,
    audio_clock: Arc<AtomicI64>,
}

enum VideoSourceCommand {
    Play,
    Pause,
    ConfigureAudio {
        volume_percent: u8,
        muted: bool,
        speed_percent: u16,
        playing: bool,
        response: SyncSender<Result<(), ()>>,
    },
    Seek {
        position_100ns: i64,
        generation: u64,
        response: SyncSender<Result<(), ()>>,
    },
}

enum VideoSourceEvent {
    Opened(VideoMetadata, bool),
    Ended { generation: u64 },
    Failed { generation: u64, error: MediaError },
}

struct VideoFrameMessage {
    generation: u64,
    frame: DecodedVideoFrame,
}

struct SourceDecodeState {
    playing: bool,
    prime_frame: bool,
    generation: u64,
    minimum_pts_100ns: i64,
    pending_frame: Option<VideoFrameMessage>,
    volume_percent: u8,
    muted: bool,
    speed_percent: u16,
}

impl SourceDecodeState {
    fn initial() -> Self {
        Self {
            playing: false,
            prime_frame: true,
            generation: 0,
            minimum_pts_100ns: 0,
            pending_frame: None,
            volume_percent: 100,
            muted: false,
            speed_percent: 100,
        }
    }
}

impl VideoSourceWorker {
    fn start(path: PathBuf, device: Option<ID3D11Device>) -> std::io::Result<Self> {
        let source_path = path.clone();
        let (command_sender, command_receiver) = mpsc::sync_channel(8);
        let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
        let (event_sender, event_receiver) = mpsc::channel();
        let (frame_sender, frame_receiver) = mpsc::sync_channel(QUEUE_TARGET);
        let audio_clock = Arc::new(AtomicI64::new(-1));
        let worker_audio_clock = Arc::clone(&audio_clock);
        let thread = thread::Builder::new()
            .name("wallpaper-video-source".to_owned())
            .spawn(move || {
                let opened = match device.as_ref() {
                    Some(device) => VideoSource::open_with_d3d11_device(path, device),
                    None => VideoSource::open(path),
                };
                let mut source = match opened {
                    Ok(source) => source,
                    Err(error) => {
                        let _ = event_sender.send(VideoSourceEvent::Failed {
                            generation: 0,
                            error,
                        });
                        return;
                    }
                };
                let audio_available = source.has_audio();
                if let Err(error) = source.configure_audio(100, false, 100, false) {
                    let _ = event_sender.send(VideoSourceEvent::Failed {
                        generation: 0,
                        error,
                    });
                    return;
                }
                if event_sender
                    .send(VideoSourceEvent::Opened(source.metadata(), audio_available))
                    .is_err()
                {
                    return;
                }

                let mut decode = SourceDecodeState::initial();
                loop {
                    if let Some(media_time) = source.audio_time_100ns() {
                        worker_audio_clock.store(media_time, Ordering::Release);
                    }
                    match stop_receiver.try_recv() {
                        Ok(()) | Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => {}
                    }
                    if !decode.playing && !decode.prime_frame && decode.pending_frame.is_none() {
                        match command_receiver.recv_timeout(TICK_INTERVAL) {
                            Ok(command) => {
                                if !Self::apply_command(
                                    command,
                                    &mut source,
                                    &mut decode,
                                    &event_sender,
                                ) {
                                    return;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                        continue;
                    }

                    match command_receiver.try_recv() {
                        Ok(command) => {
                            if !Self::apply_command(
                                command,
                                &mut source,
                                &mut decode,
                                &event_sender,
                            ) {
                                return;
                            }
                        }
                        Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => {}
                    }
                    if !decode.playing && !decode.prime_frame && decode.pending_frame.is_none() {
                        continue;
                    }

                    if let Some(message) = decode.pending_frame.take() {
                        match frame_sender.try_send(message) {
                            Ok(()) => {}
                            Err(TrySendError::Full(message)) => {
                                decode.pending_frame = Some(message);
                                thread::sleep(TICK_INTERVAL);
                            }
                            Err(TrySendError::Disconnected(_)) => return,
                        }
                    } else {
                        match source.read_next() {
                            Ok(VideoReadEvent::Frame(frame)) => {
                                if frame.pts_100ns() >= decode.minimum_pts_100ns {
                                    decode.prime_frame = false;
                                    decode.pending_frame = Some(VideoFrameMessage {
                                        generation: decode.generation,
                                        frame,
                                    });
                                }
                            }
                            Ok(VideoReadEvent::StreamTick { .. }) => {}
                            Ok(VideoReadEvent::EndOfStream) => {
                                // The reader can reach EOS while the scheduler still has
                                // buffered frames to present. Keep the audio clock running
                                // until playback itself reaches its end or loop boundary.
                                let _ = event_sender.send(VideoSourceEvent::Ended {
                                    generation: decode.generation,
                                });
                                decode.playing = false;
                                decode.prime_frame = false;
                            }
                            Err(error) => {
                                let _ = event_sender.send(VideoSourceEvent::Failed {
                                    generation: decode.generation,
                                    error,
                                });
                                return;
                            }
                        }
                    }
                }
            })?;

        Ok(Self {
            source_path,
            command_sender,
            stop_sender: Some(stop_sender),
            event_receiver,
            frame_receiver,
            thread: Some(thread),
            generation: 0,
            metadata: None,
            audio_available: false,
            audio_clock,
        })
    }

    fn apply_command(
        command: VideoSourceCommand,
        source: &mut VideoSource,
        decode: &mut SourceDecodeState,
        event_sender: &mpsc::Sender<VideoSourceEvent>,
    ) -> bool {
        match command {
            VideoSourceCommand::Play => {
                if let Err(error) = source.configure_audio(
                    decode.volume_percent,
                    decode.muted,
                    decode.speed_percent,
                    true,
                ) {
                    let _ = event_sender.send(VideoSourceEvent::Failed {
                        generation: decode.generation,
                        error,
                    });
                    return false;
                }
                decode.playing = true;
            }
            VideoSourceCommand::Pause => {
                if let Err(error) = source.configure_audio(
                    decode.volume_percent,
                    decode.muted,
                    decode.speed_percent,
                    false,
                ) {
                    let _ = event_sender.send(VideoSourceEvent::Failed {
                        generation: decode.generation,
                        error,
                    });
                    return false;
                }
                decode.playing = false;
            }
            VideoSourceCommand::ConfigureAudio {
                volume_percent,
                muted,
                speed_percent,
                playing,
                response,
            } => {
                let result = source.configure_audio(volume_percent, muted, speed_percent, playing);
                if result.is_ok() {
                    decode.volume_percent = volume_percent;
                    decode.muted = muted;
                    decode.speed_percent = speed_percent;
                }
                let _ = response.send(result.as_ref().map(|_| ()).map_err(|_| ()));
                if let Err(error) = result {
                    let _ = event_sender.send(VideoSourceEvent::Failed {
                        generation: decode.generation,
                        error,
                    });
                    return false;
                }
            }
            VideoSourceCommand::Seek {
                position_100ns,
                generation: requested_generation,
                response,
            } => {
                decode.pending_frame.take();
                decode.generation = requested_generation;
                match source.seek(position_100ns) {
                    Ok(()) => {
                        decode.minimum_pts_100ns = position_100ns;
                        decode.prime_frame = true;
                        let _ = response.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = response.send(Err(()));
                        let _ = event_sender.send(VideoSourceEvent::Failed {
                            generation: requested_generation,
                            error,
                        });
                        return false;
                    }
                }
            }
        }
        true
    }

    fn poll_open(&mut self) -> Result<Option<VideoMetadata>, MediaError> {
        match self.event_receiver.try_recv() {
            Ok(VideoSourceEvent::Opened(metadata, audio_available)) => {
                self.metadata = Some(metadata);
                self.audio_available = audio_available;
                Ok(Some(metadata))
            }
            Ok(VideoSourceEvent::Failed { error, .. }) => Err(error),
            Ok(VideoSourceEvent::Ended { .. }) => Ok(None),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn try_event(&self) -> Result<VideoSourceEvent, TryRecvError> {
        self.event_receiver.try_recv()
    }

    fn try_frame(&self) -> Result<VideoFrameMessage, TryRecvError> {
        self.frame_receiver.try_recv()
    }

    fn play(&self) -> Result<(), ()> {
        self.command_sender
            .try_send(VideoSourceCommand::Play)
            .map_err(|_| ())
    }

    fn pause(&self) -> Result<(), ()> {
        self.command_sender
            .try_send(VideoSourceCommand::Pause)
            .map_err(|_| ())
    }

    fn configure_audio(
        &self,
        volume_percent: u8,
        muted: bool,
        speed_percent: u16,
        playing: bool,
    ) -> Result<(), ()> {
        let (response, result) = mpsc::sync_channel(1);
        self.command_sender
            .try_send(VideoSourceCommand::ConfigureAudio {
                volume_percent,
                muted,
                speed_percent,
                playing,
                response,
            })
            .map_err(|_| ())?;
        result
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| ())?
    }

    fn seek(&mut self, position_100ns: i64) -> Result<(), ()> {
        let generation = self.generation.wrapping_add(1);
        let (response, result) = mpsc::sync_channel(1);
        self.command_sender
            .try_send(VideoSourceCommand::Seek {
                position_100ns,
                generation,
                response,
            })
            .map_err(|_| ())?;
        self.generation = generation;
        result
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| ())?
    }

    fn metadata(&self) -> Option<VideoMetadata> {
        self.metadata
    }

    fn audio_time_100ns(&self) -> Option<i64> {
        let value = self.audio_clock.load(Ordering::Acquire);
        (self.audio_available && value >= 0).then_some(value)
    }
}

impl Drop for VideoSourceWorker {
    fn drop(&mut self) {
        if let Some(stop_sender) = self.stop_sender.take() {
            let _ = stop_sender.try_send(());
        }
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + SOURCE_WORKER_STOP_TIMEOUT;
        while !thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            if thread.join().is_err() {
                tracing::warn!(
                    component = "media",
                    event = "video_source_worker_panicked",
                    "Video source worker stopped after a panic"
                );
            }
        } else {
            tracing::warn!(
                component = "media",
                event = "video_source_worker_stop_timed_out",
                shutdown_deadline_seconds = SOURCE_WORKER_STOP_TIMEOUT.as_secs(),
                "Video source worker did not stop before the shutdown deadline"
            );
        }
    }
}

impl PlaybackController {
    #[cfg(test)]
    fn new(
        device: ID3D11Device,
        frame_sender: RecoveryFrameSender,
        event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
    ) -> Self {
        Self::new_with_display_refresh(device, frame_sender, event_sender, None)
    }

    fn new_with_display_refresh(
        device: ID3D11Device,
        frame_sender: RecoveryFrameSender,
        event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
    ) -> Self {
        Self {
            device,
            frame_sender,
            source: None,
            pending_load: None,
            active_wallpaper_id: None,
            deferred_frame: None,
            eos_pending: false,
            scheduler: PlaybackScheduler::new(),
            playback_settings: None,
            clock_origin: Instant::now(),
            sequence: 0,
            pending_error: None,
            loading_video: false,
            last_error: None,
            event_sender,
            last_event_sent: Instant::now(),
            display_refresh_sender,
        }
    }

    fn start_video_load(&mut self, request: PlaybackCommandRequest) {
        if self.pending_load.is_some() {
            request.respond(ResponseResult::Error {
                code: PlaybackCommandErrorCode::VideoLoadInProgress,
            });
            return;
        }
        let Command::LoadVideo { wallpaper_id, path } = request.command().clone() else {
            request.respond(ResponseResult::Error {
                code: PlaybackCommandErrorCode::VideoLoadFailed,
            });
            return;
        };
        match VideoSourceWorker::start(PathBuf::from(path), Some(self.device.clone())) {
            Ok(source) => {
                if request.response_is_closed() {
                    return;
                }
                self.pending_load = Some(PendingVideoLoad {
                    wallpaper_id,
                    source,
                });
                self.loading_video = true;
                self.last_error = None;
                let ResponseResult::PlaybackState(event) = self.snapshot() else {
                    unreachable!("snapshots always contain playback state")
                };
                if !request.respond(ResponseResult::PlaybackState(event.clone())) {
                    self.pending_load = None;
                    self.loading_video = false;
                    return;
                }
                self.event_sender.send_replace(event);
                self.last_event_sent = Instant::now();
            }
            Err(error) => {
                tracing::warn!(
                    component = "media",
                    event = "video_open_worker_start_failed",
                    error_kind = ?error.kind(),
                    "Could not start a Media Foundation open worker"
                );
                request.respond(ResponseResult::Error {
                    code: PlaybackCommandErrorCode::VideoLoadFailed,
                });
            }
        }
    }

    fn poll_video_load(&mut self) {
        let Some(pending) = &mut self.pending_load else {
            return;
        };
        match pending.source.poll_open() {
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    component = "media",
                    event = "video_load_failed",
                    diagnostic = %error.diagnostic_summary(),
                    "Video source could not be loaded"
                );
                self.pending_load = None;
                self.loading_video = false;
                self.last_error = Some(PlaybackCommandErrorCode::VideoLoadFailed);
                self.pending_error = Some(PlaybackCommandErrorCode::VideoLoadFailed);
                self.publish_event();
            }
            Ok(Some(_metadata)) => {
                let pending = self.pending_load.take().expect("pending source exists");
                self.active_wallpaper_id = Some(pending.wallpaper_id);
                self.source = Some(pending.source);
                self.scheduler = PlaybackScheduler::new();
                self.playback_settings = None;
                self.deferred_frame = None;
                self.eos_pending = false;
                self.loading_video = false;
                self.pending_error = None;
                self.last_error = None;
                self.publish_event();
            }
        }
    }

    fn execute(&mut self, command: Command) -> ResponseResult {
        if matches!(command, Command::RefreshDisplayAssignments) {
            let Some(sender) = &self.display_refresh_sender else {
                return ResponseResult::Error {
                    code: PlaybackCommandErrorCode::DisplayAssignmentsApplyFailed,
                };
            };
            let (response, receiver) = std::sync::mpsc::sync_channel(1);
            if sender.send(DisplayRefreshRequest { response }).is_err() {
                return ResponseResult::Error {
                    code: PlaybackCommandErrorCode::DisplayAssignmentsApplyFailed,
                };
            }
            return match receiver.recv_timeout(Duration::from_millis(1_750)) {
                Ok(failed_targets) => {
                    ResponseResult::DisplayAssignmentsRefreshed { failed_targets }
                }
                Err(_) => ResponseResult::Error {
                    code: PlaybackCommandErrorCode::DisplayAssignmentsApplyFailed,
                },
            };
        }
        let command = match command {
            Command::ApplyPlaybackSettings {
                settings,
                source_path,
            } => return self.apply_playback_settings(settings, PathBuf::from(source_path)),
            Command::ApplyEditorSettings {
                settings,
                source_path,
            } => {
                if settings.validate().is_err() {
                    return ResponseResult::Error {
                        code: PlaybackCommandErrorCode::EditorSettingsInvalid,
                    };
                }
                return match self
                    .frame_sender
                    .apply_editor_settings(settings, PathBuf::from(source_path))
                {
                    Ok(()) => ResponseResult::EditorSettingsApplied,
                    Err(ApplySettingsError::WallpaperNotActive) => ResponseResult::Error {
                        code: PlaybackCommandErrorCode::EditorWallpaperNotActive,
                    },
                    Err(ApplySettingsError::Renderer(error)) => {
                        tracing::warn!(
                            component = "renderer",
                            event = "editor_settings_apply_failed",
                            error_code = error.to_app_error().code.as_str(),
                            diagnostic = %error.diagnostic_summary(),
                            "Editor settings could not be applied to the desktop"
                        );
                        ResponseResult::Error {
                            code: PlaybackCommandErrorCode::EditorSettingsApplyFailed,
                        }
                    }
                };
            }
            command => command,
        };
        let publish_event = !matches!(&command, Command::GetPlaybackState | Command::Ping);
        if matches!(&command, Command::GetPlaybackState) {
            if let Some(code) = self.pending_error.take() {
                return self.error(code);
            }
        }
        let now = self.now_100ns();
        let result = match command {
            Command::LoadVideo { .. } => Err(PlaybackCommandErrorCode::PlaybackUnavailable),
            Command::Play => self.require_source().and_then(|()| {
                let restart_position = self.restart_position_for_play(now)?;
                if let Some(position) = restart_position {
                    self.source
                        .as_mut()
                        .expect("source checked above")
                        .seek(position)
                        .map_err(|_| PlaybackCommandErrorCode::SeekFailed)?;
                    self.eos_pending = false;
                    self.deferred_frame = None;
                    self.scheduler
                        .seek(position, now)
                        .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                }
                self.source
                    .as_mut()
                    .expect("source checked above")
                    .play()
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                self.scheduler
                    .play(now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)
            }),
            Command::Pause => self.require_source().and_then(|()| {
                self.source
                    .as_ref()
                    .expect("source checked above")
                    .pause()
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                self.scheduler
                    .pause(now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)
            }),
            Command::Stop => self.require_source().and_then(|()| {
                let source = self.source.as_mut().expect("source checked above");
                source
                    .pause()
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                source
                    .seek(0)
                    .map_err(|_| PlaybackCommandErrorCode::SeekFailed)?;
                self.eos_pending = false;
                self.deferred_frame = None;
                self.scheduler
                    .stop(now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)
            }),
            Command::Seek { position_100ns } => self.require_source().and_then(|()| {
                let source = self.source.as_mut().expect("source checked above");
                let duration = source
                    .metadata()
                    .ok_or(PlaybackCommandErrorCode::VideoNotLoaded)?
                    .duration_100ns();
                if !(0..=duration).contains(&position_100ns) {
                    return Err(PlaybackCommandErrorCode::InvalidSeek);
                }
                source
                    .seek(position_100ns)
                    .map_err(|_| PlaybackCommandErrorCode::SeekFailed)?;
                self.eos_pending = false;
                self.deferred_frame = None;
                self.scheduler
                    .seek(position_100ns, now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)
            }),
            Command::SetSpeed { percent } => self.require_source().and_then(|()| {
                let speed = VideoPlaybackSpeed::from_percent(percent)
                    .map_err(|_| PlaybackCommandErrorCode::UnsupportedSpeed)?;
                self.scheduler
                    .set_speed(speed, now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                let (volume, muted) = self
                    .playback_settings
                    .as_ref()
                    .map_or((100, false), |s| (s.volume_percent(), s.muted()));
                self.source
                    .as_ref()
                    .expect("source checked above")
                    .configure_audio(
                        volume,
                        muted,
                        percent,
                        self.scheduler.state() == PlaybackState::Playing,
                    )
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)
            }),
            Command::SetFps { frames_per_second } => self.require_source().and_then(|()| {
                let limit = VideoFpsLimit::custom(frames_per_second)
                    .map_err(|_| PlaybackCommandErrorCode::UnsupportedFps)?;
                self.scheduler.set_fps_limit(limit);
                Ok(())
            }),
            Command::GetPlaybackState => Ok(()),
            Command::SubscribePlaybackState => {
                return self.error(PlaybackCommandErrorCode::PlaybackUnavailable);
            }
            Command::ApplyEditorSettings { .. } => {
                Err(PlaybackCommandErrorCode::EditorSettingsApplyFailed)
            }
            Command::ApplyPlaybackSettings { .. } => {
                Err(PlaybackCommandErrorCode::PlaybackSettingsApplyFailed)
            }
            Command::RefreshDisplayAssignments => {
                Err(PlaybackCommandErrorCode::DisplayAssignmentsApplyFailed)
            }
            Command::Ping => return ResponseResult::Pong,
        };

        match result {
            Ok(()) => {
                let snapshot = self.snapshot();
                if publish_event {
                    if let ResponseResult::PlaybackState(event) = &snapshot {
                        self.event_sender.send_replace(event.clone());
                        self.last_event_sent = Instant::now();
                    }
                }
                snapshot
            }
            Err(code) => self.error(code),
        }
    }

    fn require_source(&self) -> Result<(), PlaybackCommandErrorCode> {
        self.source
            .as_ref()
            .map(|_| ())
            .ok_or(PlaybackCommandErrorCode::VideoNotLoaded)
    }

    fn apply_playback_settings(
        &mut self,
        settings: PlaybackSettings,
        source_path: PathBuf,
    ) -> ResponseResult {
        if settings.validate().is_err() {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid);
        }
        let Some(source) = self.source.as_ref() else {
            return self.error(PlaybackCommandErrorCode::VideoNotLoaded);
        };
        if source.source_path != source_path {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsSourceMismatch);
        }
        let Some(metadata) = source.metadata() else {
            return self.error(PlaybackCommandErrorCode::VideoNotLoaded);
        };
        let duration_ms = match u64::try_from(metadata.duration_100ns() / 10_000) {
            Ok(duration_ms) => duration_ms,
            Err(_) => return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid),
        };
        if settings.validate_for_duration(duration_ms).is_err() {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid);
        }
        let speed = match VideoPlaybackSpeed::from_percent(settings.speed_percent()) {
            Ok(speed) => speed,
            Err(_) => return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid),
        };
        let fps_limit = match VideoFpsLimit::custom(settings.fps_limit()) {
            Ok(fps_limit) => fps_limit,
            Err(_) => return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid),
        };
        let now = self.now_100ns();
        if self.scheduler.state() == PlaybackState::Playing
            && settings.loop_mode() == LoopMode::Segment
        {
            let Some(segment) = settings.segment() else {
                return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid);
            };
            let (start, end) = match segment_bounds_100ns(segment) {
                Ok(bounds) => bounds,
                Err(code) => return self.error(code),
            };
            let current = match self.scheduler.media_time_100ns(now) {
                Ok(position) => position,
                Err(_) => return self.error(PlaybackCommandErrorCode::PlaybackSettingsApplyFailed),
            };
            if current < start || current >= end {
                let Some(source) = self.source.as_mut() else {
                    return self.error(PlaybackCommandErrorCode::VideoNotLoaded);
                };
                if source.seek(start).is_err() || self.scheduler.seek(start, now).is_err() {
                    return self.error(PlaybackCommandErrorCode::PlaybackSettingsApplyFailed);
                }
                self.deferred_frame = None;
                self.eos_pending = false;
            }
        }
        if self.scheduler.set_speed(speed, now).is_err() {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsApplyFailed);
        }
        if self.source.as_ref().is_none_or(|source| {
            source
                .configure_audio(
                    settings.volume_percent(),
                    settings.muted(),
                    settings.speed_percent(),
                    self.scheduler.state() == PlaybackState::Playing,
                )
                .is_err()
        }) {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsApplyFailed);
        }
        self.scheduler.set_fps_limit(fps_limit);
        self.playback_settings = Some(settings);
        self.publish_event();
        ResponseResult::PlaybackSettingsApplied
    }

    fn present_video_frame(
        &self,
        frame: DecodedVideoFrame,
    ) -> Result<(), wallpaper_renderer::RendererError> {
        let source_path = self
            .source
            .as_ref()
            .map(|source| source.source_path.clone())
            .ok_or(wallpaper_renderer::RendererError::WindowUnavailable)?;
        self.frame_sender.present_video_frame(frame, source_path)
    }

    fn tick(&mut self) {
        let is_playing = self.scheduler.state() == PlaybackState::Playing;
        let mut paused_frame = None;
        let frames_empty = loop {
            let message = if let Some(message) = self.deferred_frame.take() {
                Ok(message)
            } else if let Some(source) = self.source.as_ref() {
                source.try_frame()
            } else {
                Err(TryRecvError::Empty)
            };
            let message = match message {
                Ok(message) => message,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                    break true;
                }
            };
            if self
                .source
                .as_ref()
                .is_none_or(|source| message.generation != source.generation)
            {
                continue;
            }
            if is_playing {
                if self.scheduler.queued_frames() >= QUEUE_TARGET {
                    self.deferred_frame = Some(message);
                    break false;
                }
                if self
                    .scheduler
                    .enqueue_frame(message.frame, self.now_100ns())
                    .is_err()
                {
                    self.fail_playback(
                        "scheduler_frame_rejected",
                        PlaybackCommandErrorCode::PlaybackFailed,
                    );
                    return;
                }
            } else {
                paused_frame = Some(message.frame);
            }
        };
        if let Some(frame) = paused_frame
            && self.present_video_frame(frame).is_err()
        {
            self.fail_playback(
                "video_frame_presentation_failed",
                PlaybackCommandErrorCode::PlaybackFailed,
            );
            return;
        }

        let mut decode_error = None;
        if let Some(source) = self.source.as_mut() {
            while let Ok(event) = source.try_event() {
                match event {
                    VideoSourceEvent::Opened(_, _) => {}
                    VideoSourceEvent::Ended { generation } => {
                        if generation == source.generation {
                            self.eos_pending = true;
                        }
                    }
                    VideoSourceEvent::Failed { generation, error } => {
                        if generation == source.generation {
                            decode_error = Some(error.diagnostic_summary());
                        }
                    }
                }
            }
        }
        if let Some(diagnostic) = decode_error {
            tracing::warn!(
                component = "media",
                event = "video_decode_failed",
                diagnostic,
                "Video decoding stopped after a source error"
            );
            self.fail_playback(
                "video_decode_failed",
                PlaybackCommandErrorCode::DecodeFailed,
            );
            return;
        }
        if self.eos_pending && frames_empty && self.deferred_frame.is_none() {
            self.eos_pending = false;
            if self.scheduler.state() == PlaybackState::Playing {
                if self.scheduler.end_of_stream(self.now_100ns()).is_err() {
                    self.fail_playback(
                        "scheduler_eos_rejected",
                        PlaybackCommandErrorCode::PlaybackFailed,
                    );
                    return;
                }
                self.publish_event();
            }
        }

        let now = self.now_100ns();
        if self.scheduler.state() == PlaybackState::Playing
            && self.scheduler.speed().percent() == 100
            && let Some(audio_time) = self
                .source
                .as_ref()
                .and_then(VideoSourceWorker::audio_time_100ns)
            && self
                .scheduler
                .synchronize_media_time(audio_time, now)
                .is_err()
        {
            self.fail_playback(
                "audio_clock_rejected",
                PlaybackCommandErrorCode::PlaybackFailed,
            );
            return;
        }
        if self.scheduler.state() == PlaybackState::Playing {
            match self.current_loop_target(now) {
                Ok(Some(target)) => {
                    if self.wrap_video_loop(target, now).is_err() {
                        self.fail_playback(
                            "video_segment_loop_seek_failed",
                            PlaybackCommandErrorCode::SeekFailed,
                        );
                        return;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    self.fail_playback(
                        "scheduler_loop_boundary_failed",
                        PlaybackCommandErrorCode::PlaybackFailed,
                    );
                    return;
                }
            }
        }

        if self.scheduler.state() == PlaybackState::Playing {
            match self.scheduler.next_due_frame(self.now_100ns()) {
                Ok(Some(frame)) => {
                    if self.present_video_frame(frame).is_err() {
                        self.fail_playback(
                            "video_frame_presentation_failed",
                            PlaybackCommandErrorCode::PlaybackFailed,
                        );
                        return;
                    }
                }
                Ok(None) => {}
                Err(_) => self.fail_playback(
                    "scheduler_clock_failed",
                    PlaybackCommandErrorCode::PlaybackFailed,
                ),
            }
        }

        if self.scheduler.state() == PlaybackState::Ended {
            match self.current_loop_target(self.now_100ns()) {
                Ok(Some(target)) => {
                    if self.wrap_video_loop(target, self.now_100ns()).is_err() {
                        self.fail_playback(
                            "video_full_loop_seek_failed",
                            PlaybackCommandErrorCode::SeekFailed,
                        );
                        return;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    self.fail_playback(
                        "scheduler_loop_boundary_failed",
                        PlaybackCommandErrorCode::PlaybackFailed,
                    );
                    return;
                }
            }
        }

        if self.scheduler.state() == PlaybackState::Playing
            && self.last_event_sent.elapsed() >= Duration::from_millis(250)
        {
            self.publish_event();
        }
    }

    fn fail_playback(&mut self, reason: &'static str, code: PlaybackCommandErrorCode) {
        tracing::warn!(
            component = "media",
            event = reason,
            "Video playback stopped"
        );
        self.pending_error = Some(code);
        self.last_error = Some(code);
        self.source = None;
        self.playback_settings = None;
        self.scheduler = PlaybackScheduler::new();
        self.publish_event();
    }

    fn publish_event(&mut self) {
        if let ResponseResult::PlaybackState(event) = self.snapshot() {
            self.event_sender.send_replace(event);
            self.last_event_sent = Instant::now();
        }
    }

    fn current_loop_target(&mut self, now_100ns: i64) -> Result<Option<i64>, ()> {
        let Some(settings) = self.playback_settings.as_ref() else {
            return Ok(None);
        };
        self.scheduler
            .loop_seek_target(settings.loop_mode(), settings.segment(), now_100ns)
            .map_err(|_| ())
    }

    fn restart_position_for_play(
        &mut self,
        now_100ns: i64,
    ) -> Result<Option<i64>, PlaybackCommandErrorCode> {
        let state = self.scheduler.state();
        if state == PlaybackState::Playing {
            return Ok(None);
        }
        let Some(settings) = self.playback_settings.as_ref() else {
            return Ok((state == PlaybackState::Ended).then_some(0));
        };
        match settings.loop_mode() {
            LoopMode::Segment => {
                let Some(segment) = settings.segment() else {
                    return Err(PlaybackCommandErrorCode::PlaybackSettingsInvalid);
                };
                let (start, end) = segment_bounds_100ns(segment)?;
                let current = self
                    .scheduler
                    .media_time_100ns(now_100ns)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                Ok((current < start || current >= end).then_some(start))
            }
            LoopMode::Full | LoopMode::Disabled => Ok((state == PlaybackState::Ended).then_some(0)),
        }
    }

    fn wrap_video_loop(
        &mut self,
        position_100ns: i64,
        now_100ns: i64,
    ) -> Result<(), PlaybackCommandErrorCode> {
        let source = self
            .source
            .as_mut()
            .ok_or(PlaybackCommandErrorCode::VideoNotLoaded)?;
        source
            .seek(position_100ns)
            .map_err(|_| PlaybackCommandErrorCode::SeekFailed)?;
        self.scheduler
            .seek(position_100ns, now_100ns)
            .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
        self.deferred_frame = None;
        self.eos_pending = false;
        source
            .play()
            .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
        self.scheduler
            .play(now_100ns)
            .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
        self.publish_event();
        Ok(())
    }

    fn snapshot(&mut self) -> ResponseResult {
        self.sequence = self.sequence.wrapping_add(1);
        let now = self.now_100ns();
        if self.scheduler.state() == PlaybackState::Playing
            && self.scheduler.speed().percent() == 100
            && let Some(audio_time) = self
                .source
                .as_ref()
                .and_then(VideoSourceWorker::audio_time_100ns)
        {
            let _ = self.scheduler.synchronize_media_time(audio_time, now);
        }
        let media_time = self.scheduler.media_time_100ns(now).unwrap_or(0);
        let state = match self.scheduler.state() {
            PlaybackState::Stopped => IpcPlaybackState::Stopped,
            PlaybackState::Playing => IpcPlaybackState::Playing,
            PlaybackState::Paused => IpcPlaybackState::Paused,
            PlaybackState::Ended => IpcPlaybackState::Ended,
        };
        ResponseResult::PlaybackState(PlaybackStateEvent {
            sequence: self.sequence,
            wallpaper_id: self.active_wallpaper_id,
            state,
            media_time_100ns: media_time,
            duration_100ns: self
                .source
                .as_ref()
                .and_then(|source| source.metadata().map(VideoMetadata::duration_100ns)),
            frame_rate_numerator: self
                .source
                .as_ref()
                .and_then(|source| source.metadata().map(VideoMetadata::frame_rate_numerator)),
            frame_rate_denominator: self
                .source
                .as_ref()
                .and_then(|source| source.metadata().map(VideoMetadata::frame_rate_denominator)),
            audio_available: self
                .source
                .as_ref()
                .is_some_and(|source| source.audio_available),
            audio_effective_muted: self.source.as_ref().is_some_and(|source| {
                source.audio_available
                    && (self.scheduler.speed().percent() != 100
                        || self
                            .playback_settings
                            .as_ref()
                            .is_some_and(PlaybackSettings::muted))
            }),
            speed_percent: self.scheduler.speed().percent(),
            fps_limit: self.scheduler.fps_limit().frames_per_second(),
            loading_video: self.loading_video,
            last_error: self.last_error,
        })
    }

    fn error(&mut self, code: PlaybackCommandErrorCode) -> ResponseResult {
        ResponseResult::Error { code }
    }

    fn now_100ns(&self) -> i64 {
        i64::try_from(self.clock_origin.elapsed().as_nanos() / 100).unwrap_or(i64::MAX)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{
        path::PathBuf,
        sync::mpsc::TryRecvError,
        thread,
        time::{Duration, Instant},
    };

    use super::{PlaybackController, PlaybackWorker, VideoSourceEvent, VideoSourceWorker};
    use crate::recovery::DesktopRecoveryWorker;
    use wallpaper_core::playback_settings::{LoopMode, PlaybackSettings, SegmentRange};
    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_ipc::protocol::{
        Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackStateEvent, ResponseResult,
        playback_command_channel,
    };
    use wallpaper_media::PlaybackState;
    use wallpaper_renderer::D3d11Device;

    #[test]
    fn stopping_playback_worker_does_not_require_ipc_sender_disconnect() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let media_device = device.device().clone();
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");
        let (command_sender, command_receiver) = playback_command_channel();
        let (mut playback, _events) =
            PlaybackWorker::start(command_receiver, media_device, &recovery)
                .expect("the playback worker should start");

        assert!(playback.stop(), "explicit shutdown should stop the worker");
        assert!(std::mem::size_of_val(&command_sender) > 0);
        assert!(
            recovery.stop(),
            "recovery resources should stop after playback"
        );
    }

    #[test]
    fn paused_source_worker_primes_a_frame_after_seek() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/short-h264.mp4");
        let mut source = VideoSourceWorker::start(path, None).expect("start source worker");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if source.poll_open().expect("open fixture").is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "source open timed out");
            thread::sleep(Duration::from_millis(10));
        }

        let initial = wait_for_frame(&source, 0, deadline);
        assert_eq!(initial.frame.pts_100ns(), 0);

        source.seek(0).expect("seek while source is paused");
        let seeked = wait_for_frame(&source, source.generation, deadline);
        assert_eq!(seeked.frame.pts_100ns(), 0);

        source.play().expect("resume source worker");
        thread::sleep(Duration::from_millis(100));
        source
            .seek(0)
            .expect("seek while decoded-frame queue is full");
        let seeked_while_playing = wait_for_frame(&source, source.generation, deadline);
        assert_eq!(seeked_while_playing.frame.pts_100ns(), 0);
    }

    #[test]
    fn source_worker_waits_for_frame_queue_capacity_before_reading_to_eos() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/short-h264.mp4");
        let mut source = VideoSourceWorker::start(path, None).expect("start source worker");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if source.poll_open().expect("open fixture").is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "source open timed out");
            thread::sleep(Duration::from_millis(10));
        }

        source.play().expect("start source worker playback");
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match source.try_event() {
                Ok(VideoSourceEvent::Ended { .. }) => {
                    panic!("the source must not reach EOS while its bounded frame queue is full")
                }
                Ok(VideoSourceEvent::Failed { error, .. }) => {
                    panic!("source decoding failed unexpectedly: {error:?}")
                }
                Ok(VideoSourceEvent::Opened(_, _)) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panic!("source worker stopped before a stop request")
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            source.try_frame().is_ok(),
            "the source should retain decoded frames while applying backpressure"
        );
    }

    #[test]
    fn source_worker_repeats_segment_seeks_without_old_or_pre_start_frames() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/loop-boundary-h264.mp4");
        let mut source = VideoSourceWorker::start(path, None).expect("start source worker");
        let deadline = Instant::now() + Duration::from_secs(10);
        let metadata = loop {
            if let Some(metadata) = source.poll_open().expect("open fixture") {
                break metadata;
            }
            assert!(Instant::now() < deadline, "source open timed out");
            thread::sleep(Duration::from_millis(10));
        };
        let start_100ns = 5_000_000;
        let end_100ns = 10_000_000;
        assert!(start_100ns > 0 && start_100ns < metadata.duration_100ns());
        assert!(end_100ns <= metadata.duration_100ns());

        source.play().expect("start full source iteration");
        wait_for_end_of_stream(&source, source.generation, deadline);
        for _ in 0..3 {
            let old_generation = source.generation;
            source.seek(start_100ns).expect("seek to segment start");
            let generation = source.generation;
            assert_ne!(generation, old_generation);
            let first = wait_for_frame(&source, generation, deadline);
            assert_eq!(first.generation, generation);
            assert!(
                first.frame.pts_100ns() >= start_100ns,
                "a keyframe seek must discard decoded samples before the requested segment start"
            );
            assert!(
                first.frame.pts_100ns() < end_100ns,
                "the first eligible frame must be inside the segment"
            );
            let center_index = first.frame.stride() as usize * (first.frame.height() as usize / 2)
                + (first.frame.width() as usize / 2) * 4;
            let center = &first.frame.rgba8()[center_index..center_index + 4];
            assert!(
                center[0] as u16 > center[1] as u16 * 2 && center[0] as u16 > center[2] as u16 * 2,
                "the first eligible frame should be red segment content, got {center:?}"
            );
            source.play().expect("resume segment iteration");
            wait_for_end_of_stream(&source, generation, deadline);
        }
    }

    #[test]
    fn controller_applies_typed_loop_settings_and_repeats_full_and_segment() {
        let host = discover_desktop_host().expect("an interactive Windows shell is required");
        let window = host
            .attach_wallpaper_window()
            .expect("the wallpaper child should attach to the desktop host");
        let device = D3d11Device::create_hardware().expect("hardware D3D11 is required");
        let media_device = device.device().clone();
        let mut recovery =
            DesktopRecoveryWorker::start(window, device).expect("the recovery worker should start");
        let (event_sender, _event_receiver) = tokio::sync::watch::channel(PlaybackStateEvent {
            sequence: 0,
            wallpaper_id: None,
            state: IpcPlaybackState::Stopped,
            media_time_100ns: 0,
            duration_100ns: None,
            frame_rate_numerator: None,
            frame_rate_denominator: None,
            audio_available: false,
            audio_effective_muted: false,
            speed_percent: 100,
            fps_limit: 60,
            loading_video: false,
            last_error: None,
        });
        let mut controller =
            PlaybackController::new(media_device.clone(), recovery.frame_sender(), event_sender);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/media/tests/fixtures/short-h264.mp4");
        let mut source = VideoSourceWorker::start(path.clone(), Some(media_device))
            .expect("start source worker");
        let deadline = Instant::now() + Duration::from_secs(15);
        let metadata = loop {
            if let Some(metadata) = source.poll_open().expect("open fixture") {
                break metadata;
            }
            assert!(Instant::now() < deadline, "source open timed out");
            thread::sleep(Duration::from_millis(10));
        };
        let duration_ms = u64::try_from(metadata.duration_100ns() / 10_000)
            .expect("fixture duration is nonnegative");
        assert!(
            duration_ms >= 1_000,
            "fixture must be long enough for a segment"
        );
        controller.source = Some(source);

        let invalid = PlaybackSettings::new(
            1,
            LoopMode::Segment,
            Some(SegmentRange::new(duration_ms, duration_ms + 500).unwrap()),
            200,
            60,
            100,
            false,
        )
        .unwrap();
        assert_eq!(
            controller.execute(Command::ApplyPlaybackSettings {
                settings: invalid,
                source_path: path.to_string_lossy().into_owned(),
            }),
            ResponseResult::Error {
                code: PlaybackCommandErrorCode::PlaybackSettingsInvalid
            }
        );
        assert_eq!(controller.scheduler.speed().percent(), 100);

        let full = PlaybackSettings::new(1, LoopMode::Full, None, 200, 30, 100, false).unwrap();
        assert_eq!(
            controller.execute(Command::ApplyPlaybackSettings {
                settings: full.clone(),
                source_path: path
                    .with_file_name("other.mp4")
                    .to_string_lossy()
                    .into_owned(),
            }),
            ResponseResult::Error {
                code: PlaybackCommandErrorCode::PlaybackSettingsSourceMismatch
            }
        );
        assert_eq!(controller.scheduler.speed().percent(), 100);
        let generation_before_full = controller.source.as_ref().unwrap().generation;
        assert_eq!(
            controller.execute(Command::ApplyPlaybackSettings {
                settings: full,
                source_path: path.to_string_lossy().into_owned(),
            }),
            ResponseResult::PlaybackSettingsApplied
        );
        assert!(matches!(
            controller.execute(Command::Play),
            ResponseResult::PlaybackState(PlaybackStateEvent {
                speed_percent: 200,
                fps_limit: 30,
                ..
            })
        ));
        tick_until(&mut controller, deadline, |controller| {
            controller
                .source
                .as_ref()
                .is_some_and(|source| source.generation >= generation_before_full + 3)
        });
        assert_eq!(controller.scheduler.state(), PlaybackState::Playing);

        let generation_before_segment = controller.source.as_ref().unwrap().generation;
        let segment = PlaybackSettings::new(
            1,
            LoopMode::Segment,
            Some(SegmentRange::new(500, 1_000).unwrap()),
            200,
            60,
            100,
            false,
        )
        .unwrap();
        assert_eq!(
            controller.execute(Command::ApplyPlaybackSettings {
                settings: segment,
                source_path: path.to_string_lossy().into_owned(),
            }),
            ResponseResult::PlaybackSettingsApplied
        );
        tick_until(&mut controller, deadline, |controller| {
            controller
                .source
                .as_ref()
                .is_some_and(|source| source.generation >= generation_before_segment + 2)
        });
        assert_eq!(controller.scheduler.state(), PlaybackState::Playing);

        let generation_before_disabled = controller.source.as_ref().unwrap().generation;
        let disabled =
            PlaybackSettings::new(1, LoopMode::Disabled, None, 200, 60, 100, false).unwrap();
        assert_eq!(
            controller.execute(Command::ApplyPlaybackSettings {
                settings: disabled,
                source_path: path.to_string_lossy().into_owned(),
            }),
            ResponseResult::PlaybackSettingsApplied
        );
        tick_until(&mut controller, deadline, |controller| {
            controller.scheduler.state() == PlaybackState::Ended
        });
        assert_eq!(
            controller.source.as_ref().unwrap().generation,
            generation_before_disabled,
            "disabled playback reaches EOS without another seek"
        );

        controller.source = None;
        assert!(
            recovery.stop(),
            "recovery resources should stop after playback"
        );
    }

    fn tick_until(
        controller: &mut PlaybackController,
        deadline: Instant,
        mut predicate: impl FnMut(&PlaybackController) -> bool,
    ) {
        while !predicate(controller) {
            assert!(Instant::now() < deadline, "loop transition timed out");
            controller.tick();
            thread::sleep(Duration::from_millis(8));
        }
    }

    fn wait_for_end_of_stream(source: &VideoSourceWorker, generation: u64, deadline: Instant) {
        loop {
            while source.try_frame().is_ok() {}
            match source.try_event() {
                Ok(VideoSourceEvent::Ended {
                    generation: ended_generation,
                }) if ended_generation == generation => return,
                Ok(VideoSourceEvent::Failed { error, .. }) => {
                    panic!("source decoding failed unexpectedly: {error:?}")
                }
                Ok(VideoSourceEvent::Opened(_, _) | VideoSourceEvent::Ended { .. })
                | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panic!("source worker stopped before reporting end-of-stream")
                }
            }
            assert!(
                Instant::now() < deadline,
                "source did not reach end-of-stream"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_for_frame(
        source: &VideoSourceWorker,
        generation: u64,
        deadline: Instant,
    ) -> super::VideoFrameMessage {
        loop {
            match source.try_frame() {
                Ok(frame) if frame.generation == generation => return frame,
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("source worker stopped before producing a frame")
                }
            }
            assert!(Instant::now() < deadline, "frame decode timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
