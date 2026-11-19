use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    IpcError,
    protocol::{Command, PIPE_NAME, ResponseResult},
};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Send one bounded ping request to the wallpaper host.
pub async fn ping() -> Result<(), IpcError> {
    ping_at(PIPE_NAME).await
}

/// Send one typed command to the wallpaper host.
pub async fn request(command: Command) -> Result<ResponseResult, IpcError> {
    request_at(PIPE_NAME, command).await
}

/// Subscribe to coalesced playback-state events from the host.
#[cfg(windows)]
pub async fn subscribe_playback_state() -> Result<PlaybackStateSubscription, IpcError> {
    subscribe_playback_state_at(crate::protocol::PLAYBACK_EVENTS_PIPE_NAME).await
}

#[cfg(windows)]
pub struct PlaybackStateSubscription {
    pipe: tokio::net::windows::named_pipe::NamedPipeClient,
    request_id: u64,
}

#[cfg(windows)]
impl PlaybackStateSubscription {
    /// Wait for the next current-state event. Dropping the subscription closes
    /// its named-pipe connection and stops delivery.
    pub async fn next(&mut self) -> Result<crate::protocol::PlaybackStateEvent, IpcError> {
        use tokio::time;

        use crate::protocol::{IO_TIMEOUT, ResponseResult, decode_response, read_frame};

        let payload = time::timeout(IO_TIMEOUT, read_frame(&mut self.pipe))
            .await
            .map_err(|_| IpcError::Timeout)??;
        match decode_response(&payload, self.request_id)? {
            ResponseResult::PlaybackState(event) => Ok(event),
            _ => Err(IpcError::MalformedResponse),
        }
    }
}

#[cfg(windows)]
async fn subscribe_playback_state_at(
    pipe_name: &str,
) -> Result<PlaybackStateSubscription, IpcError> {
    use std::time::Duration;

    use tokio::{
        net::windows::named_pipe::ClientOptions,
        time::{self, Instant},
    };
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};

    use crate::{
        error::from_io,
        protocol::{Command, IO_TIMEOUT, Request, encode_message, write_frame},
    };

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut pipe = loop {
        match ClientOptions::new().open(pipe_name) {
            Ok(pipe) => break pipe,
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code) if code == ERROR_PIPE_BUSY as i32 || code == ERROR_FILE_NOT_FOUND as i32
                ) && Instant::now() < deadline =>
            {
                time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                return Err(IpcError::Timeout);
            }
            Err(error) => return Err(from_io(error)),
        }
    };

    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let request = encode_message(&Request::new(request_id, Command::SubscribePlaybackState))?;
    time::timeout(IO_TIMEOUT, write_frame(&mut pipe, &request))
        .await
        .map_err(|_| IpcError::Timeout)??;

    Ok(PlaybackStateSubscription { pipe, request_id })
}

#[cfg(windows)]
pub(crate) async fn ping_at(pipe_name: &str) -> Result<(), IpcError> {
    match request_at(pipe_name, Command::Ping).await? {
        ResponseResult::Pong => Ok(()),
        _ => Err(IpcError::MalformedResponse),
    }
}

#[cfg(windows)]
pub(crate) async fn request_at(
    pipe_name: &str,
    command: Command,
) -> Result<ResponseResult, IpcError> {
    use std::time::Duration;

    use tokio::{
        net::windows::named_pipe::{ClientOptions, NamedPipeClient},
        time::{self, Instant},
    };
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    use crate::{
        error::from_io,
        protocol::{IO_TIMEOUT, Request, decode_response, encode_message, read_frame, write_frame},
    };

    async fn open_pipe(pipe_name: &str) -> Result<NamedPipeClient, IpcError> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(client),
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                        && Instant::now() < deadline =>
                {
                    time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    return Err(IpcError::Timeout);
                }
                Err(error) => return Err(from_io(error)),
            }
        }
    }

    let mut pipe = open_pipe(pipe_name).await?;
    let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let request = encode_message(&Request::new(request_id, command))?;

    time::timeout(IO_TIMEOUT, async {
        write_frame(&mut pipe, &request).await?;
        let response = read_frame(&mut pipe).await.map_err(|error| match error {
            IpcError::MalformedMessage => IpcError::MalformedResponse,
            other => other,
        })?;
        decode_response(&response, request_id)
    })
    .await
    .map_err(|_| IpcError::Timeout)?
}

#[cfg(not(windows))]
pub(crate) async fn ping_at(_pipe_name: &str) -> Result<(), IpcError> {
    Err(IpcError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub(crate) async fn request_at(
    _pipe_name: &str,
    _command: Command,
) -> Result<ResponseResult, IpcError> {
    Err(IpcError::UnsupportedPlatform)
}

#[cfg(all(test, windows))]
mod tests {
    use std::{future::Future, time::Duration};

    use tokio::{
        io::AsyncReadExt,
        net::windows::named_pipe::ServerOptions,
        runtime::Builder,
        time::{self, Instant},
    };

    use crate::{
        IpcError,
        protocol::{IO_TIMEOUT, read_frame, write_frame},
    };

    fn test_pipe_name(label: &str) -> String {
        format!(
            r"\\.\pipe\wallpaper-lite-test-{}-{label}",
            std::process::id()
        )
    }

    fn run_test(future: impl Future<Output = ()>) {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create test runtime");
        runtime.block_on(future);
    }

    async fn wait_for_host(pipe_name: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match super::ping_at(pipe_name).await {
                Ok(()) => return,
                Err(IpcError::HostUnavailable) if Instant::now() < deadline => {
                    time::sleep(Duration::from_millis(25)).await;
                }
                result => panic!("expected ping response, got {result:?}"),
            }
        }
    }

    #[test]
    fn host_handles_reconnect_and_releases_pipe_on_shutdown() {
        run_test(async {
            let pipe_name = test_pipe_name("restart");
            let server_name = pipe_name.clone();
            let server =
                tokio::spawn(async move { crate::server::run_on_pipe(&server_name).await });

            wait_for_host(&pipe_name).await;
            super::ping_at(&pipe_name)
                .await
                .expect("reconnect after first ping");

            server.abort();
            let _ = server.await;
            assert!(matches!(
                super::ping_at(&pipe_name).await,
                Err(IpcError::HostUnavailable)
            ));

            let restarted_name = pipe_name.clone();
            let restarted_server =
                tokio::spawn(async move { crate::server::run_on_pipe(&restarted_name).await });
            wait_for_host(&pipe_name).await;
            restarted_server.abort();
            let _ = restarted_server.await;
        });
    }

    #[test]
    fn playback_commands_round_trip_over_named_pipe_with_typed_state_and_errors() {
        use crate::protocol::{
            Command, IpcPlaybackState, PlaybackCommandErrorCode, PlaybackStateEvent,
            ResponseResult, playback_command_channel,
        };

        run_test(async {
            let pipe_name = test_pipe_name("playback");
            let server_name = pipe_name.clone();
            let (sender, mut receiver) = playback_command_channel();
            let server = tokio::spawn(async move {
                crate::server::run_on_pipe_with_commands(&server_name, Some(sender)).await
            });
            let command_worker = tokio::spawn(async move {
                loop {
                    match receiver.try_recv() {
                        Ok(request) => {
                            let result = match request.command() {
                                Command::SetSpeed { percent } if *percent != 100 => {
                                    ResponseResult::Error {
                                        code: PlaybackCommandErrorCode::UnsupportedSpeed,
                                    }
                                }
                                _ => ResponseResult::PlaybackState(PlaybackStateEvent {
                                    sequence: 1,
                                    state: IpcPlaybackState::Playing,
                                    media_time_100ns: 12_345,
                                    duration_100ns: Some(10_000_000),
                                    speed_percent: 100,
                                    fps_limit: 60,
                                    loading_video: false,
                                    last_error: None,
                                }),
                            };
                            request.respond(result);
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                            tokio::task::yield_now().await;
                        }
                    }
                }
            });

            wait_for_host(&pipe_name).await;
            for command in [
                Command::LoadVideo {
                    path: "C:\\wallpaper\\sample.mp4".into(),
                },
                Command::Play,
                Command::Pause,
                Command::Stop,
                Command::Seek { position_100ns: 0 },
                Command::SetSpeed { percent: 100 },
                Command::SetFps {
                    frames_per_second: 60,
                },
                Command::GetPlaybackState,
            ] {
                let response = super::request_at(&pipe_name, command)
                    .await
                    .expect("playback command response");
                assert!(matches!(
                    response,
                    ResponseResult::PlaybackState(PlaybackStateEvent {
                        state: IpcPlaybackState::Playing,
                        media_time_100ns: 12_345,
                        ..
                    })
                ));
            }
            assert!(matches!(
                super::request_at(&pipe_name, Command::SetSpeed { percent: 33 }).await,
                Ok(ResponseResult::Error {
                    code: PlaybackCommandErrorCode::UnsupportedSpeed
                })
            ));

            server.abort();
            let _ = server.await;
            command_worker.abort();
            let _ = command_worker.await;
        });
    }

    #[test]
    fn disconnecting_during_load_video_cancels_the_response_wait() {
        use crate::protocol::{Command, Request, playback_command_channel, write_frame};

        run_test(async {
            let pipe_name = test_pipe_name("load-disconnect");
            let (command_sender, mut command_receiver) = playback_command_channel();
            let server_name = pipe_name.clone();
            let server = tokio::spawn(async move {
                crate::server::run_on_pipe_with_commands(&server_name, Some(command_sender)).await
            });

            wait_for_host(&pipe_name).await;
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut client = loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe_name) {
                    Ok(client) => break client,
                    Err(error)
                        if error.raw_os_error() == Some(231) && Instant::now() < deadline =>
                    {
                        time::sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => panic!("open command pipe: {error}"),
                }
            };
            let payload = crate::protocol::encode_message(&Request::new(
                77,
                Command::LoadVideo {
                    path: "C:/wallpaper/slow.mp4".to_owned(),
                },
            ))
            .expect("encode load request");
            write_frame(&mut client, &payload)
                .await
                .expect("write load request");

            let mut request = None;
            while request.is_none() && Instant::now() < deadline {
                request = command_receiver.try_recv().ok();
                if request.is_none() {
                    tokio::task::yield_now().await;
                }
            }
            let request = request.expect("host queued the playback command");
            assert!(!request.response_is_closed());

            drop(client);
            time::timeout(Duration::from_secs(1), async {
                while !request.response_is_closed() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("disconnect cancels command response wait");

            server.abort();
            let _ = server.await;
        });
    }

    #[test]
    fn playback_state_subscription_streams_named_pipe_events() {
        use crate::protocol::{IpcPlaybackState, PlaybackStateEvent};

        run_test(async {
            let pipe_name = test_pipe_name("events");
            let initial = PlaybackStateEvent {
                sequence: 0,
                state: IpcPlaybackState::Stopped,
                media_time_100ns: 0,
                duration_100ns: Some(10_000_000),
                speed_percent: 100,
                fps_limit: 60,
                loading_video: false,
                last_error: None,
            };
            let (event_sender, event_receiver) = tokio::sync::watch::channel(initial.clone());
            let server_name = pipe_name.clone();
            let server = tokio::spawn(async move {
                crate::server::run_on_event_pipe(&server_name, event_receiver).await
            });

            let mut subscription = super::subscribe_playback_state_at(&pipe_name)
                .await
                .expect("subscribe to state events");
            assert_eq!(subscription.next().await.expect("initial event"), initial);

            let playing = PlaybackStateEvent {
                sequence: 1,
                state: IpcPlaybackState::Playing,
                media_time_100ns: 250_000,
                ..initial
            };
            event_sender.send_replace(playing.clone());
            assert_eq!(subscription.next().await.expect("playing event"), playing);

            drop(subscription);
            server.abort();
            let _ = server.await;
        });
    }

    #[test]
    fn malformed_response_is_reported_separately_from_unavailable_host() {
        run_test(async {
            let pipe_name = test_pipe_name("malformed");
            assert!(matches!(
                super::ping_at(&pipe_name).await,
                Err(IpcError::HostUnavailable)
            ));

            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&pipe_name)
                .expect("create fake server");
            let fake_server = tokio::spawn(async move {
                server.connect().await.expect("accept client");
                let _ = read_frame(&mut server).await.expect("read request");
                write_frame(&mut server, b"not-json")
                    .await
                    .expect("write malformed response");
                let mut trailing_byte = [0_u8; 1];
                let _ = server.read(&mut trailing_byte).await;
                server.disconnect().expect("disconnect client");
            });

            assert!(matches!(
                super::ping_at(&pipe_name).await,
                Err(IpcError::MalformedResponse)
            ));
            fake_server.await.expect("finish fake server");
        });
    }

    #[test]
    fn response_wait_has_a_deadline() {
        run_test(async {
            let pipe_name = test_pipe_name("timeout");
            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&pipe_name)
                .expect("create slow server");
            let fake_server = tokio::spawn(async move {
                server.connect().await.expect("accept client");
                let _ = read_frame(&mut server).await.expect("read request");
                time::sleep(IO_TIMEOUT + Duration::from_secs(1)).await;
                let _ = server.disconnect();
            });

            let result = time::timeout(Duration::from_secs(5), super::ping_at(&pipe_name))
                .await
                .expect("client returns within test guard");
            assert!(matches!(result, Err(IpcError::Timeout)));
            fake_server.abort();
            let _ = fake_server.await;
        });
    }
}
