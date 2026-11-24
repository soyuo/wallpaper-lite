use std::{
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicI64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_core::performance_policy::{PerformancePolicySettings, PolicyAction};
use wallpaper_core::playback_settings::{LoopMode, PlaybackSettings, SegmentRange};
use wallpaper_ipc::protocol::{
    ActivePerformancePolicyCauses, Command, IpcPlaybackState, PerformanceMediaKind,
    PlaybackCommandErrorCode, PlaybackCommandReceiver, PlaybackCommandRequest, PlaybackStateEvent,
    ResponseResult,
};
use wallpaper_media::{
    DecodedVideoFrame, MediaError, PlaybackScheduler, PlaybackState, VideoFpsLimit, VideoMetadata,
    VideoPlaybackSpeed, VideoReadEvent, VideoSource,
};
use wallpaper_renderer::RendererError;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::independent_displays::DisplayRefreshRequest;
use crate::performance_metrics::{
    PerformanceMemorySampler, PerformanceMetricsRecorder, PerformanceMetricsRegistry,
    dropped_frames_since,
};
use crate::{
    performance_signals::{PerformanceSignalSnapshot, policy_signals},
    recovery::{ApplySettingsError, DesktopRecoveryWorker, RecoveryFrameSender},
};

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
    memory_sampler: Option<PerformanceMemorySampler>,
}

pub(crate) struct PerformanceMetricsBinding {
    pub(crate) display_id: String,
    pub(crate) media_kind: PerformanceMediaKind,
    pub(crate) registry: PerformanceMetricsRegistry,
    pub(crate) sample_memory: bool,
}

impl PlaybackWorker {
    #[cfg(test)]
    pub(crate) fn start(
        commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        Self::start_with_policy_and_display_refresh(
            commands,
            device,
            recovery,
            Arc::new(RwLock::new(PerformancePolicySettings::default())),
            None,
            None,
        )
    }

    pub(crate) fn start_with_policy_and_display_refresh(
        commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
        policy_settings: Arc<RwLock<PerformancePolicySettings>>,
        signal_receiver: Option<tokio::sync::watch::Receiver<PerformanceSignalSnapshot>>,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        Self::start_with_performance_metrics(
            commands,
            device,
            recovery,
            policy_settings,
            signal_receiver,
            display_refresh_sender,
            None,
        )
    }

    pub(crate) fn start_with_performance_metrics(
        mut commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
        policy_settings: Arc<RwLock<PerformancePolicySettings>>,
        signal_receiver: Option<tokio::sync::watch::Receiver<PerformanceSignalSnapshot>>,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
        metrics: Option<PerformanceMetricsBinding>,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        let memory_sampler = metrics
            .as_ref()
            .filter(|metrics| metrics.sample_memory)
            .and_then(|metrics| {
                match PerformanceMemorySampler::start(device.clone(), metrics.registry.clone()) {
                    Ok(sampler) => Some(sampler),
                    Err(error) => {
                        tracing::warn!(
                            component = "performance",
                            event = "memory_sampler_start_failed",
                            error_kind = ?error.kind(),
                            "Process and GPU memory metrics will remain unavailable"
                        );
                        None
                    }
                }
            });
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
                let mut controller = PlaybackController::new_with_metrics(
                    device,
                    frame_sender,
                    event_sender,
                    policy_settings,
                    signal_receiver,
                    display_refresh_sender,
                    metrics,
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

                    controller.poll_policy();
                    controller.poll_video_load();
                    controller.tick();
                    thread::sleep(TICK_INTERVAL);
                }
            })?;

        Ok((
            Self {
                thread: Some(thread),
                stop_sender: Some(stop_sender),
                memory_sampler,
            },
            event_receiver,
        ))
    }

    pub(crate) fn stop(&mut self) -> bool {
        if let Some(stop_sender) = self.stop_sender.take() {
            let _ = stop_sender.try_send(());
        }
        let Some(thread) = self.thread.as_ref() else {
            if let Some(mut sampler) = self.memory_sampler.take() {
                sampler.stop();
            }
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
            if let Some(mut sampler) = self.memory_sampler.take() {
                sampler.stop();
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
    configured_fps_limit: u16,
    playback_settings: Option<PlaybackSettings>,
    clock_origin: Instant,
    sequence: u64,
    pending_error: Option<PlaybackCommandErrorCode>,
    loading_video: bool,
    last_error: Option<PlaybackCommandErrorCode>,
    event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
    last_event_sent: Instant,
    last_diagnostic: Instant,
    display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
    policy_settings: Arc<RwLock<PerformancePolicySettings>>,
    signal_receiver: Option<tokio::sync::watch::Receiver<PerformanceSignalSnapshot>>,
    policy_state: PlaybackPolicyState,
    metrics_registry: PerformanceMetricsRegistry,
    metrics_recorder: Option<PerformanceMetricsRecorder>,
    last_metrics_policy_causes: Option<ActivePerformancePolicyCauses>,
    last_metrics_dropped_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum UserPlaybackIntent {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Copy, Debug, Default)]
struct PlaybackPolicyState {
    action: PolicyAction,
    user_intent: UserPlaybackIntent,
    paused_by_policy: bool,
    play_requested_while_blocked: bool,
    battery_fps_limit: Option<u16>,
}

impl PlaybackPolicyState {
    fn update_battery_fps_limit(&mut self, next: Option<u16>) -> bool {
        let changed = self.battery_fps_limit != next;
        self.battery_fps_limit = next;
        changed
    }

    const fn effective_fps_limit(self, configured: u16) -> u16 {
        match self.battery_fps_limit {
            Some(cap) if cap < configured => cap,
            _ => configured,
        }
    }

    fn user_play(&mut self) -> bool {
        self.user_intent = UserPlaybackIntent::Playing;
        if self.action == PolicyAction::PausePlayback {
            self.play_requested_while_blocked = true;
            false
        } else {
            self.play_requested_while_blocked = false;
            true
        }
    }

    fn user_pause(&mut self) {
        self.user_intent = UserPlaybackIntent::Paused;
        self.paused_by_policy = false;
        self.play_requested_while_blocked = false;
    }

    fn user_stop(&mut self) {
        self.user_intent = UserPlaybackIntent::Stopped;
        self.paused_by_policy = false;
        self.play_requested_while_blocked = false;
    }

    fn transition(&mut self, next: PolicyAction, is_playing: bool) -> PolicyTransition {
        let previous = self.action;
        self.action = next;
        let pause_playback = next == PolicyAction::PausePlayback && is_playing;
        if pause_playback {
            self.paused_by_policy = true;
        }
        let resume_playback = next != PolicyAction::PausePlayback
            && self.user_intent == UserPlaybackIntent::Playing
            && (self.paused_by_policy || self.play_requested_while_blocked);
        if resume_playback {
            self.paused_by_policy = false;
            self.play_requested_while_blocked = false;
        }
        PolicyTransition {
            changed: previous != next,
            pause_rendering: next != PolicyAction::Continue,
            pause_playback,
            resume_playback,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PolicyTransition {
    changed: bool,
    pause_rendering: bool,
    pause_playback: bool,
    resume_playback: bool,
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
    decode_latency: Duration,
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
                        let decode_started = Instant::now();
                        match source.read_next() {
                            Ok(VideoReadEvent::Frame(frame)) => {
                                if frame.pts_100ns() >= decode.minimum_pts_100ns {
                                    decode.prime_frame = false;
                                    decode.pending_frame = Some(VideoFrameMessage {
                                        generation: decode.generation,
                                        frame,
                                        decode_latency: decode_started.elapsed(),
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
        Self::new_with_policy_and_display_refresh(
            device,
            frame_sender,
            event_sender,
            Arc::new(RwLock::new(PerformancePolicySettings::default())),
            None,
            None,
        )
    }

    fn new_with_policy_and_display_refresh(
        device: ID3D11Device,
        frame_sender: RecoveryFrameSender,
        event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
        policy_settings: Arc<RwLock<PerformancePolicySettings>>,
        signal_receiver: Option<tokio::sync::watch::Receiver<PerformanceSignalSnapshot>>,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
    ) -> Self {
        Self::new_with_metrics(
            device,
            frame_sender,
            event_sender,
            policy_settings,
            signal_receiver,
            display_refresh_sender,
            None,
        )
    }

    fn new_with_metrics(
        device: ID3D11Device,
        frame_sender: RecoveryFrameSender,
        event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
        policy_settings: Arc<RwLock<PerformancePolicySettings>>,
        signal_receiver: Option<tokio::sync::watch::Receiver<PerformanceSignalSnapshot>>,
        display_refresh_sender: Option<tokio::sync::mpsc::UnboundedSender<DisplayRefreshRequest>>,
        metrics: Option<PerformanceMetricsBinding>,
    ) -> Self {
        let metrics_registry = metrics
            .as_ref()
            .map(|metrics| metrics.registry.clone())
            .unwrap_or_default();
        let metrics_recorder = metrics.map(|metrics| {
            PerformanceMetricsRecorder::new(
                metrics.display_id,
                metrics.media_kind,
                metrics.registry,
            )
        });
        Self {
            device,
            frame_sender,
            source: None,
            pending_load: None,
            active_wallpaper_id: None,
            deferred_frame: None,
            eos_pending: false,
            scheduler: PlaybackScheduler::new(),
            configured_fps_limit: 60,
            playback_settings: None,
            clock_origin: Instant::now(),
            sequence: 0,
            pending_error: None,
            loading_video: false,
            last_error: None,
            event_sender,
            last_event_sent: Instant::now(),
            last_diagnostic: Instant::now(),
            display_refresh_sender,
            policy_settings,
            signal_receiver,
            policy_state: PlaybackPolicyState::default(),
            metrics_registry,
            metrics_recorder,
            last_metrics_policy_causes: None,
            last_metrics_dropped_count: 0,
        }
    }

    fn poll_policy(&mut self) {
        let signals = if let Some(receiver) = self.signal_receiver.as_mut() {
            if receiver.has_changed().is_err() {
                tracing::warn!(
                    component = "performance",
                    event = "signal_monitor_stopped",
                    "Performance signal monitor stopped; automatic pauses are being cleared"
                );
                self.signal_receiver = None;
                Default::default()
            } else {
                policy_signals(&receiver.borrow_and_update())
            }
        } else {
            Default::default()
        };
        let settings = self
            .policy_settings
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_default();
        let decision = settings.evaluate(signals, false);
        let fps_limit_changed = self
            .policy_state
            .update_battery_fps_limit(decision.battery_fps_limit());
        if fps_limit_changed {
            self.apply_effective_fps_limit();
            tracing::info!(
                component = "performance",
                event = "battery_fps_limit_changed",
                on_battery = signals.on_battery,
                configured_fps = self.configured_fps_limit,
                battery_fps_limit = ?decision.battery_fps_limit(),
                effective_fps = self
                    .policy_state
                    .effective_fps_limit(self.configured_fps_limit),
                "Applied temporary battery frame-rate policy"
            );
        }
        let action = decision.action();
        let metrics_policy_causes = ActivePerformancePolicyCauses {
            fullscreen: signals.fullscreen,
            session_locked: signals.session_locked,
            on_battery: signals.on_battery,
            effective_action: action,
            battery_fps_limit: decision.battery_fps_limit(),
        };
        if self.last_metrics_policy_causes.as_ref() != Some(&metrics_policy_causes) {
            self.metrics_registry
                .set_active_policy_causes(metrics_policy_causes.clone());
            self.last_metrics_policy_causes = Some(metrics_policy_causes);
        }
        let transition = self
            .policy_state
            .transition(action, self.scheduler.state() == PlaybackState::Playing);
        if transition.changed {
            if !self
                .frame_sender
                .set_rendering_paused(transition.pause_rendering)
            {
                tracing::warn!(
                    component = "renderer",
                    event = "render_policy_update_failed",
                    "Desktop recovery worker did not accept the rendering policy"
                );
            }
            tracing::info!(
                component = "performance",
                event = "automatic_playback_policy_changed",
                action = ?action,
                "Automatic playback policy changed"
            );
        }
        let now = self.now_100ns();
        if transition.pause_playback {
            let paused = self
                .source
                .as_ref()
                .is_some_and(|source| source.pause().is_ok())
                && self.scheduler.pause(now).is_ok();
            if !paused {
                self.fail_playback(
                    "automatic_policy_pause_failed",
                    PlaybackCommandErrorCode::PlaybackFailed,
                );
                return;
            }
            self.publish_event();
        } else if transition.resume_playback {
            if let Err(code) = self.start_source_playback(now) {
                self.fail_playback("automatic_policy_resume_failed", code);
                return;
            }
            self.publish_event();
        } else if fps_limit_changed {
            self.publish_event();
        }
    }

    fn apply_effective_fps_limit(&mut self) {
        let frames_per_second = self
            .policy_state
            .effective_fps_limit(self.configured_fps_limit);
        if let Ok(limit) = VideoFpsLimit::custom(frames_per_second) {
            self.scheduler.set_fps_limit(limit);
        }
    }

    fn request_play(&mut self, now: i64) -> Result<(), PlaybackCommandErrorCode> {
        self.require_source()?;
        if self.policy_state.user_play() {
            if let Err(error) = self.start_source_playback(now) {
                self.policy_state.user_pause();
                return Err(error);
            }
        }
        Ok(())
    }

    fn start_source_playback(&mut self, now: i64) -> Result<(), PlaybackCommandErrorCode> {
        self.require_source()?;
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
                self.reset_scheduler();
                self.playback_settings = None;
                self.deferred_frame = None;
                self.eos_pending = false;
                self.loading_video = false;
                self.pending_error = None;
                self.last_error = None;
                self.policy_state.user_stop();
                if let Some(metrics) = self.metrics_recorder.as_mut() {
                    metrics.set_media_kind(PerformanceMediaKind::Video);
                }
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
            Command::ApplyPerformancePolicySettings { settings } => {
                let Ok(mut current) = self.policy_settings.write() else {
                    return ResponseResult::Error {
                        code: PlaybackCommandErrorCode::PerformancePolicyApplyFailed,
                    };
                };
                wallpaper_renderer::set_sharp_upscaling(settings.sharp_upscaling());
                *current = settings;
                drop(current);
                self.poll_policy();
                return ResponseResult::PerformancePolicyApplied;
            }
            Command::ApplyPlaybackSettings {
                settings,
                source_path,
            } => return self.apply_playback_settings(settings, PathBuf::from(source_path)),
            Command::GetPerformanceMetrics => {
                return ResponseResult::PerformanceMetrics(self.metrics_registry.snapshot());
            }
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
        let publish_event = !matches!(
            &command,
            Command::GetPlaybackState | Command::GetPerformanceMetrics | Command::Ping
        );
        if matches!(&command, Command::GetPlaybackState) {
            if let Some(code) = self.pending_error.take() {
                return self.error(code);
            }
        }
        let now = self.now_100ns();
        let result = match command {
            Command::LoadVideo { .. } => Err(PlaybackCommandErrorCode::PlaybackUnavailable),
            Command::GetPerformanceMetrics => Ok(()),
            Command::Play => self.request_play(now),
            Command::Pause => self.require_source().and_then(|()| {
                self.source
                    .as_ref()
                    .expect("source checked above")
                    .pause()
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                self.scheduler
                    .pause(now)
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                self.policy_state.user_pause();
                Ok(())
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
                    .map_err(|_| PlaybackCommandErrorCode::PlaybackFailed)?;
                self.policy_state.user_stop();
                Ok(())
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
                VideoFpsLimit::custom(frames_per_second)
                    .map_err(|_| PlaybackCommandErrorCode::UnsupportedFps)?;
                self.configured_fps_limit = frames_per_second;
                self.apply_effective_fps_limit();
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
            Command::ApplyPerformancePolicySettings { .. } => {
                Err(PlaybackCommandErrorCode::PerformancePolicyApplyFailed)
            }
            Command::Ping => return ResponseResult::Pong,
            // The IPC server answers Shutdown itself and never dispatches it.
            Command::Shutdown => return self.error(PlaybackCommandErrorCode::PlaybackUnavailable),
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
        if VideoFpsLimit::custom(settings.fps_limit()).is_err() {
            return self.error(PlaybackCommandErrorCode::PlaybackSettingsInvalid);
        }
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
        self.configured_fps_limit = settings.fps_limit();
        self.scheduler.set_fps_limit(
            VideoFpsLimit::custom(
                self.policy_state
                    .effective_fps_limit(self.configured_fps_limit),
            )
            .expect("validated configured and battery FPS limits"),
        );
        self.playback_settings = Some(settings);
        self.publish_event();
        ResponseResult::PlaybackSettingsApplied
    }

    fn present_video_frame(&mut self, frame: DecodedVideoFrame) -> Result<(), RendererError> {
        let source_path = self
            .source
            .as_ref()
            .map(|source| source.source_path.clone())
            .ok_or(wallpaper_renderer::RendererError::WindowUnavailable)?;
        let started = Instant::now();
        self.frame_sender.present_video_frame(frame, source_path)?;
        if let Some(metrics) = self.metrics_recorder.as_mut() {
            metrics.record_present(started.elapsed());
        }
        Ok(())
    }

    fn handle_video_presentation_error(&mut self, error: RendererError) -> bool {
        if error.is_device_lost() {
            tracing::warn!(
                component = "renderer",
                event = "video_frame_waiting_for_device_recovery",
                error_code = error.to_app_error().code.as_str(),
                diagnostic = %error.diagnostic_summary(),
                "Video playback is retained while the target graphics device recovers"
            );
            false
        } else {
            self.fail_playback(
                "video_frame_presentation_failed",
                PlaybackCommandErrorCode::PlaybackFailed,
            );
            true
        }
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
                if let Some(metrics) = self.metrics_recorder.as_mut() {
                    metrics.record_source_frame(message.frame.pts_100ns(), message.decode_latency);
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
                if let Some(metrics) = self.metrics_recorder.as_mut() {
                    metrics.record_source_frame(message.frame.pts_100ns(), message.decode_latency);
                }
                paused_frame = Some(message.frame);
            }
        };
        if let Some(frame) = paused_frame {
            if let Err(error) = self.present_video_frame(frame)
                && self.handle_video_presentation_error(error)
            {
                return;
            }
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
                    if let Err(error) = self.present_video_frame(frame)
                        && self.handle_video_presentation_error(error)
                    {
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

        if self.source.is_some() && self.last_diagnostic.elapsed() >= Duration::from_secs(2) {
            self.last_diagnostic = Instant::now();
            let now = self.now_100ns();
            tracing::info!(
                component = "media",
                event = "video_playback_diagnostic",
                state = ?self.scheduler.state(),
                queued_frames = self.scheduler.queued_frames(),
                dropped_frames = self.scheduler.dropped_frames(),
                media_time_100ns = self.scheduler.media_time_100ns(now).unwrap_or(-1),
                audio_time_100ns = self
                    .source
                    .as_ref()
                    .and_then(VideoSourceWorker::audio_time_100ns)
                    .unwrap_or(-1),
                "Video playback progress"
            );
        }
        self.publish_performance_metrics();
        if self.scheduler.state() == PlaybackState::Playing
            && self.last_event_sent.elapsed() >= Duration::from_millis(250)
        {
            self.publish_event();
        }
    }

    fn publish_performance_metrics(&mut self) {
        self.collect_scheduler_drops();
        if let Some(metrics) = self.metrics_recorder.as_mut() {
            metrics.publish_if_due(Instant::now());
        }
    }

    fn collect_scheduler_drops(&mut self) {
        let newly_dropped = dropped_frames_since(
            self.scheduler.dropped_frames(),
            &mut self.last_metrics_dropped_count,
        );
        if let Some(metrics) = self.metrics_recorder.as_mut() {
            metrics.record_dropped_frames(newly_dropped);
        }
    }

    fn reset_scheduler(&mut self) {
        self.collect_scheduler_drops();
        self.scheduler = PlaybackScheduler::new();
        self.last_metrics_dropped_count = 0;
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
        self.reset_scheduler();
        self.policy_state.user_stop();
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

    use super::{
        PlaybackController, PlaybackPolicyState, PlaybackWorker, VideoSourceEvent,
        VideoSourceWorker,
    };
    use crate::recovery::DesktopRecoveryWorker;
    use wallpaper_core::performance_policy::PolicyAction;
    use wallpaper_core::playback_settings::{LoopMode, PlaybackSettings, SegmentRange};
    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_ipc::protocol::{
        Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackStateEvent, ResponseResult,
        playback_command_channel,
    };
    use wallpaper_media::PlaybackState;
    use wallpaper_renderer::{D3d11Device, RendererError};

    #[test]
    fn overlapping_automatic_causes_resume_only_after_the_last_cause_clears() {
        let mut state = PlaybackPolicyState {
            user_intent: super::UserPlaybackIntent::Playing,
            ..PlaybackPolicyState::default()
        };
        let first = state.transition(PolicyAction::PausePlayback, true);
        assert!(first.pause_playback);
        assert!(first.pause_rendering);

        let overlap = state.transition(PolicyAction::PausePlayback, false);
        assert!(!overlap.resume_playback);

        let cleared = state.transition(PolicyAction::Continue, false);
        assert!(cleared.resume_playback);
        assert!(!cleared.pause_rendering);
    }

    #[test]
    fn manual_pause_during_an_automatic_pause_prevents_resume() {
        let mut state = PlaybackPolicyState {
            user_intent: super::UserPlaybackIntent::Playing,
            ..PlaybackPolicyState::default()
        };
        assert!(
            state
                .transition(PolicyAction::PausePlayback, true)
                .pause_playback
        );
        state.user_pause();
        let cleared = state.transition(PolicyAction::Continue, false);
        assert!(!cleared.resume_playback);
        assert_eq!(state.user_intent, super::UserPlaybackIntent::Paused);
    }

    #[test]
    fn manual_stop_during_an_automatic_pause_prevents_resume() {
        let mut state = PlaybackPolicyState {
            user_intent: super::UserPlaybackIntent::Playing,
            ..PlaybackPolicyState::default()
        };
        assert!(
            state
                .transition(PolicyAction::PausePlayback, true)
                .pause_playback
        );
        state.user_stop();
        let cleared = state.transition(PolicyAction::Continue, false);
        assert!(!cleared.resume_playback);
        assert_eq!(state.user_intent, super::UserPlaybackIntent::Stopped);
    }

    #[test]
    fn policy_state_is_independent_per_playback_worker() {
        let mut first = PlaybackPolicyState {
            user_intent: super::UserPlaybackIntent::Playing,
            ..PlaybackPolicyState::default()
        };
        let mut second = PlaybackPolicyState {
            user_intent: super::UserPlaybackIntent::Playing,
            ..PlaybackPolicyState::default()
        };

        assert!(
            first
                .transition(PolicyAction::PausePlayback, true)
                .pause_playback
        );
        assert!(
            !second
                .transition(PolicyAction::Continue, true)
                .pause_playback
        );
        assert!(
            first
                .transition(PolicyAction::Continue, false)
                .resume_playback
        );
        assert!(
            !second
                .transition(PolicyAction::Continue, true)
                .resume_playback
        );
    }

    #[test]
    fn battery_fps_cap_is_temporary_and_restores_each_targets_base_limit() {
        let mut first_target = PlaybackPolicyState::default();
        let mut second_target = PlaybackPolicyState::default();
        assert!(first_target.update_battery_fps_limit(Some(30)));
        assert!(second_target.update_battery_fps_limit(Some(30)));
        assert_eq!(first_target.effective_fps_limit(45), 30);
        assert_eq!(second_target.effective_fps_limit(60), 30);

        assert!(first_target.update_battery_fps_limit(None));
        assert!(second_target.update_battery_fps_limit(None));
        assert_eq!(first_target.effective_fps_limit(45), 45);
        assert_eq!(second_target.effective_fps_limit(60), 60);
        assert!(!first_target.update_battery_fps_limit(None));
    }

    #[test]
    fn play_requested_while_blocked_is_deferred_until_policy_clears() {
        let mut state = PlaybackPolicyState {
            action: PolicyAction::PausePlayback,
            ..PlaybackPolicyState::default()
        };
        assert!(!state.user_play());
        assert!(
            state
                .transition(PolicyAction::Continue, false)
                .resume_playback
        );
    }

    #[test]
    fn rendering_only_policy_does_not_pause_playback() {
        let mut state = PlaybackPolicyState::default();
        let transition = state.transition(PolicyAction::PauseRendering, true);
        assert!(transition.pause_rendering);
        assert!(!transition.pause_playback);
        assert!(!transition.resume_playback);
    }

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
    fn transient_device_loss_does_not_discard_video_playback_state() {
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
        let mut source = VideoSourceWorker::start(path, Some(media_device))
            .expect("the test video source should start");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if source
                .poll_open()
                .expect("test video source should open")
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline, "video source open timed out");
            thread::sleep(Duration::from_millis(10));
        }
        source.play().expect("test video source should play");
        controller.source = Some(source);
        controller
            .scheduler
            .play(controller.now_100ns())
            .expect("playback clock should start");

        assert!(
            !controller.handle_video_presentation_error(RendererError::Present {
                hresult: 0x887A0005_u32 as i32,
            })
        );
        assert!(controller.source.is_some());
        assert_eq!(controller.scheduler.state(), PlaybackState::Playing);
        assert_eq!(controller.last_error, None);

        assert!(controller.handle_video_presentation_error(RendererError::Present { hresult: -1 }));
        assert!(controller.source.is_none());
        assert_eq!(controller.scheduler.state(), PlaybackState::Stopped);
        assert_eq!(
            controller.last_error,
            Some(PlaybackCommandErrorCode::PlaybackFailed)
        );
        assert!(recovery.stop());
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
