use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use wallpaper_ipc::protocol::{
    Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackCommandReceiver,
    PlaybackCommandRequest, PlaybackStateEvent, ResponseResult,
};
use wallpaper_media::{
    DecodedVideoFrame, MediaError, PlaybackScheduler, PlaybackState, VideoFpsLimit, VideoMetadata,
    VideoPlaybackSpeed, VideoReadEvent, VideoSource,
};
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use crate::recovery::{DesktopRecoveryWorker, RecoveryFrameSender};

const TICK_INTERVAL: Duration = Duration::from_millis(8);
const QUEUE_TARGET: usize = 3;
const WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const SOURCE_WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct PlaybackWorker {
    thread: Option<JoinHandle<()>>,
    stop_sender: Option<SyncSender<()>>,
}

impl PlaybackWorker {
    pub(crate) fn start(
        mut commands: PlaybackCommandReceiver,
        device: ID3D11Device,
        recovery: &DesktopRecoveryWorker,
    ) -> std::io::Result<(Self, tokio::sync::watch::Receiver<PlaybackStateEvent>)> {
        let frame_sender = recovery.frame_sender();
        let (event_sender, event_receiver) = tokio::sync::watch::channel(PlaybackStateEvent {
            sequence: 0,
            state: IpcPlaybackState::Stopped,
            media_time_100ns: 0,
            duration_100ns: None,
            speed_percent: 100,
            fps_limit: 60,
            loading_video: false,
            last_error: None,
        });
        let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("wallpaper-video-playback".to_owned())
            .spawn(move || {
                let mut controller = PlaybackController::new(device, frame_sender, event_sender);
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
    deferred_frame: Option<VideoFrameMessage>,
    eos_pending: bool,
    scheduler: PlaybackScheduler,
    clock_origin: Instant,
    sequence: u64,
    pending_error: Option<PlaybackCommandErrorCode>,
    loading_video: bool,
    last_error: Option<PlaybackCommandErrorCode>,
    event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
    last_event_sent: Instant,
}

struct PendingVideoLoad {
    source: VideoSourceWorker,
}

struct VideoSourceWorker {
    command_sender: SyncSender<VideoSourceCommand>,
    stop_sender: Option<SyncSender<()>>,
    event_receiver: Receiver<VideoSourceEvent>,
    frame_receiver: Receiver<VideoFrameMessage>,
    thread: Option<JoinHandle<()>>,
    generation: u64,
    metadata: Option<VideoMetadata>,
}

enum VideoSourceCommand {
    Play,
    Pause,
    Seek {
        position_100ns: i64,
        generation: u64,
        response: SyncSender<Result<(), ()>>,
    },
}

enum VideoSourceEvent {
    Opened(VideoMetadata),
    Ended { generation: u64 },
    Failed { generation: u64, error: MediaError },
}

struct VideoFrameMessage {
    generation: u64,
    frame: DecodedVideoFrame,
}

impl VideoSourceWorker {
    fn start(path: PathBuf, device: Option<ID3D11Device>) -> std::io::Result<Self> {
        let (command_sender, command_receiver) = mpsc::sync_channel(8);
        let (stop_sender, stop_receiver) = mpsc::sync_channel(1);
        let (event_sender, event_receiver) = mpsc::channel();
        let (frame_sender, frame_receiver) = mpsc::sync_channel(QUEUE_TARGET);
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
                if event_sender
                    .send(VideoSourceEvent::Opened(source.metadata()))
                    .is_err()
                {
                    return;
                }

                let mut playing = false;
                let mut prime_frame = true;
                let mut generation = 0;
                let mut pending_frame = None;
                loop {
                    match stop_receiver.try_recv() {
                        Ok(()) | Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => {}
                    }
                    if !playing && !prime_frame && pending_frame.is_none() {
                        match command_receiver.recv_timeout(TICK_INTERVAL) {
                            Ok(command) => {
                                if !Self::apply_command(
                                    command,
                                    &mut source,
                                    &mut generation,
                                    &mut playing,
                                    &mut prime_frame,
                                    &mut pending_frame,
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
                                &mut generation,
                                &mut playing,
                                &mut prime_frame,
                                &mut pending_frame,
                                &event_sender,
                            ) {
                                return;
                            }
                        }
                        Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => {}
                    }
                    if !playing && !prime_frame && pending_frame.is_none() {
                        continue;
                    }

                    if let Some(message) = pending_frame.take() {
                        match frame_sender.try_send(message) {
                            Ok(()) => {}
                            Err(TrySendError::Full(message)) => {
                                pending_frame = Some(message);
                                thread::sleep(TICK_INTERVAL);
                            }
                            Err(TrySendError::Disconnected(_)) => return,
                        }
                    } else {
                        match source.read_next() {
                            Ok(VideoReadEvent::Frame(frame)) => {
                                prime_frame = false;
                                pending_frame = Some(VideoFrameMessage { generation, frame });
                            }
                            Ok(VideoReadEvent::StreamTick { .. }) => {}
                            Ok(VideoReadEvent::EndOfStream) => {
                                let _ = event_sender.send(VideoSourceEvent::Ended { generation });
                                playing = false;
                                prime_frame = false;
                            }
                            Err(error) => {
                                let _ = event_sender
                                    .send(VideoSourceEvent::Failed { generation, error });
                                return;
                            }
                        }
                    }
                }
            })?;

        Ok(Self {
            command_sender,
            stop_sender: Some(stop_sender),
            event_receiver,
            frame_receiver,
            thread: Some(thread),
            generation: 0,
            metadata: None,
        })
    }

    fn apply_command(
        command: VideoSourceCommand,
        source: &mut VideoSource,
        generation: &mut u64,
        playing: &mut bool,
        prime_frame: &mut bool,
        pending_frame: &mut Option<VideoFrameMessage>,
        event_sender: &mpsc::Sender<VideoSourceEvent>,
    ) -> bool {
        match command {
            VideoSourceCommand::Play => *playing = true,
            VideoSourceCommand::Pause => *playing = false,
            VideoSourceCommand::Seek {
                position_100ns,
                generation: requested_generation,
                response,
            } => {
                pending_frame.take();
                *generation = requested_generation;
                match source.seek(position_100ns) {
                    Ok(()) => {
                        *prime_frame = true;
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
            Ok(VideoSourceEvent::Opened(metadata)) => {
                self.metadata = Some(metadata);
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
    fn new(
        device: ID3D11Device,
        frame_sender: RecoveryFrameSender,
        event_sender: tokio::sync::watch::Sender<PlaybackStateEvent>,
    ) -> Self {
        Self {
            device,
            frame_sender,
            source: None,
            pending_load: None,
            deferred_frame: None,
            eos_pending: false,
            scheduler: PlaybackScheduler::new(),
            clock_origin: Instant::now(),
            sequence: 0,
            pending_error: None,
            loading_video: false,
            last_error: None,
            event_sender,
            last_event_sent: Instant::now(),
        }
    }

    fn start_video_load(&mut self, request: PlaybackCommandRequest) {
        if self.pending_load.is_some() {
            request.respond(ResponseResult::Error {
                code: PlaybackCommandErrorCode::VideoLoadInProgress,
            });
            return;
        }
        let Command::LoadVideo { path } = request.command().clone() else {
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
                self.pending_load = Some(PendingVideoLoad { source });
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
                self.source = Some(pending.source);
                self.scheduler = PlaybackScheduler::new();
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
                let source = self.source.as_mut().expect("source checked above");
                if self.scheduler.state() == PlaybackState::Ended {
                    source
                        .seek(0)
                        .map_err(|_| PlaybackCommandErrorCode::SeekFailed)?;
                    self.eos_pending = false;
                    self.deferred_frame = None;
                }
                source
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
            && self.frame_sender.present_video_frame(frame).is_err()
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
                    VideoSourceEvent::Opened(_) => {}
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

        if self.scheduler.state() == PlaybackState::Playing {
            match self.scheduler.next_due_frame(self.now_100ns()) {
                Ok(Some(frame)) => {
                    if self.frame_sender.present_video_frame(frame).is_err() {
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
        self.scheduler = PlaybackScheduler::new();
        self.publish_event();
    }

    fn publish_event(&mut self) {
        if let ResponseResult::PlaybackState(event) = self.snapshot() {
            self.event_sender.send_replace(event);
            self.last_event_sent = Instant::now();
        }
    }

    fn snapshot(&mut self) -> ResponseResult {
        self.sequence = self.sequence.wrapping_add(1);
        let now = self.now_100ns();
        let media_time = self.scheduler.media_time_100ns(now).unwrap_or(0);
        let state = match self.scheduler.state() {
            PlaybackState::Stopped => IpcPlaybackState::Stopped,
            PlaybackState::Playing => IpcPlaybackState::Playing,
            PlaybackState::Paused => IpcPlaybackState::Paused,
            PlaybackState::Ended => IpcPlaybackState::Ended,
        };
        ResponseResult::PlaybackState(PlaybackStateEvent {
            sequence: self.sequence,
            state,
            media_time_100ns: media_time,
            duration_100ns: self
                .source
                .as_ref()
                .and_then(|source| source.metadata().map(VideoMetadata::duration_100ns)),
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

    use super::{PlaybackWorker, VideoSourceEvent, VideoSourceWorker};
    use crate::recovery::DesktopRecoveryWorker;
    use wallpaper_desktop::discover_desktop_host;
    use wallpaper_ipc::protocol::playback_command_channel;
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
                Ok(VideoSourceEvent::Opened(_)) | Err(TryRecvError::Empty) => {}
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
