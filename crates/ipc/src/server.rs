use crate::IpcError;

/// Run the wallpaper host's named-pipe request loop until the process exits.
pub async fn run() -> Result<(), IpcError> {
    run_on_pipe(crate::protocol::PIPE_NAME).await
}

/// Run the host pipe server with a dedicated playback-command worker.
pub async fn run_with_commands(
    commands: crate::protocol::PlaybackCommandSender,
) -> Result<(), IpcError> {
    run_on_pipe_with_commands(crate::protocol::PIPE_NAME, Some(commands)).await
}

/// Run the control pipe and the independent playback event stream.
pub async fn run_with_commands_and_events(
    commands: crate::protocol::PlaybackCommandSender,
    events: tokio::sync::watch::Receiver<crate::protocol::PlaybackStateEvent>,
) -> Result<(), IpcError> {
    #[cfg(windows)]
    {
        tokio::try_join!(
            run_on_pipe_with_commands(crate::protocol::PIPE_NAME, Some(commands)),
            run_on_event_pipe(crate::protocol::PLAYBACK_EVENTS_PIPE_NAME, events),
        )?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (commands, events);
        Err(IpcError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
pub(crate) async fn run_on_pipe(pipe_name: &str) -> Result<(), IpcError> {
    run_on_pipe_with_commands(pipe_name, None).await
}

#[cfg(windows)]
pub(crate) async fn run_on_pipe_with_commands(
    pipe_name: &str,
    commands: Option<crate::protocol::PlaybackCommandSender>,
) -> Result<(), IpcError> {
    use tokio::{io::AsyncReadExt, net::windows::named_pipe::ServerOptions, time};

    use crate::{
        error::from_io,
        protocol::{
            Command, IO_TIMEOUT, PlaybackCommandErrorCode, Response, ResponseResult,
            decode_request, encode_message, read_frame, write_frame,
        },
    };

    let mut options = ServerOptions::new();
    options.first_pipe_instance(true).max_instances(1);
    let mut pipe = options.create(pipe_name).map_err(from_io)?;

    loop {
        pipe.connect().await.map_err(from_io)?;

        let read_result = time::timeout(IO_TIMEOUT, async {
            let payload = read_frame(&mut pipe).await?;
            decode_request(&payload)
        })
        .await;

        let request_result = match read_result {
            Err(_) => Err(IpcError::Timeout),
            Ok(Err(error)) => Err(error),
            Ok(Ok(request)) => {
                match time::timeout(IO_TIMEOUT, async {
                    let request_id = request.request_id;
                    let result = match request.command {
                        Command::Ping => ResponseResult::Pong,
                        command => match &commands {
                            Some(dispatcher) => {
                                let mut trailing_byte = [0_u8; 1];
                                tokio::select! {
                                    biased;
                                    read = pipe.read(&mut trailing_byte) => match read {
                                        Ok(0) => return Err(IpcError::Disconnected),
                                        Ok(_) => return Err(IpcError::MalformedRequest),
                                        Err(error) => return Err(from_io(error)),
                                    },
                                    result = dispatcher.dispatch(command) => result,
                                }
                            }
                            None => ResponseResult::Error {
                                code: PlaybackCommandErrorCode::PlaybackUnavailable,
                            },
                        },
                    };
                    let response = encode_message(&Response::new(request_id, result))?;
                    write_frame(&mut pipe, &response).await?;

                    // Keep the response available until the client has read it and
                    // closed its handle. Disconnecting immediately can discard bytes
                    // still queued for the client.
                    let mut trailing_byte = [0_u8; 1];
                    match pipe.read(&mut trailing_byte).await {
                        Ok(0) => Ok(()),
                        Ok(_) => Err(IpcError::MalformedRequest),
                        Err(error) => Err(from_io(error)),
                    }
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(IpcError::Timeout),
                }
            }
        };

        match request_result {
            Ok(()) => tracing::debug!(
                component = "ipc",
                event = "request_completed",
                "IPC request completed"
            ),
            Err(IpcError::Timeout) => tracing::warn!(
                component = "ipc",
                event = "request_timed_out",
                error_code = "timeout",
                "IPC request exceeded its deadline"
            ),
            Err(error) => tracing::warn!(
                component = "ipc",
                event = "request_rejected",
                error_code = error.code(),
                diagnostic = %error.diagnostic_summary(),
                "IPC request failed"
            ),
        }

        pipe.disconnect().map_err(from_io)?;

        // Each request is isolated; malformed clients and slow readers do not
        // take down the host or prevent the next UI request from connecting.
    }
}

#[cfg(windows)]
pub(crate) async fn run_on_event_pipe(
    pipe_name: &str,
    mut events: tokio::sync::watch::Receiver<crate::protocol::PlaybackStateEvent>,
) -> Result<(), IpcError> {
    use tokio::{io::AsyncReadExt, net::windows::named_pipe::ServerOptions, time};

    use crate::{
        error::from_io,
        protocol::{
            Command, IO_TIMEOUT, Response, ResponseResult, decode_request, encode_message,
            read_frame, write_frame,
        },
    };

    loop {
        let mut pipe = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)
            .map_err(from_io)?;
        pipe.connect().await.map_err(from_io)?;

        let client_result = async {
            let result = time::timeout(IO_TIMEOUT, read_frame(&mut pipe))
                .await
                .map_err(|_| IpcError::Timeout)?;
            let request = decode_request(&result?)?;
            if request.command != Command::SubscribePlaybackState {
                return Err(IpcError::MalformedRequest);
            }

            let request_id = request.request_id;
            let initial = events.borrow_and_update().clone();
            let response = encode_message(&Response::new(
                request_id,
                ResponseResult::PlaybackState(initial),
            ))?;
            time::timeout(IO_TIMEOUT, write_frame(&mut pipe, &response))
                .await
                .map_err(|_| IpcError::Timeout)??;

            let mut trailing_byte = [0_u8; 1];
            loop {
                tokio::select! {
                    biased;
                    read = pipe.read(&mut trailing_byte) => match read {
                        Ok(0) => break,
                        Ok(_) => return Err(IpcError::MalformedRequest),
                        Err(error) => return Err(from_io(error)),
                    },
                    changed = events.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let event = events.borrow_and_update().clone();
                        let response = encode_message(&Response::new(
                            request_id,
                            ResponseResult::PlaybackState(event),
                        ))?;
                        time::timeout(IO_TIMEOUT, write_frame(&mut pipe, &response))
                            .await
                            .map_err(|_| IpcError::Timeout)??;
                    }
                }
            }
            Ok::<(), IpcError>(())
        }
        .await;

        if let Err(error) = client_result {
            tracing::warn!(
                component = "ipc",
                event = "playback_event_client_ended",
                error_code = error.code(),
                diagnostic = %error.diagnostic_summary(),
                "Playback event client connection ended"
            );
        }
        let _ = pipe.disconnect();
    }
}

#[cfg(not(windows))]
pub(crate) async fn run_on_pipe(_pipe_name: &str) -> Result<(), IpcError> {
    Err(IpcError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub(crate) async fn run_on_pipe_with_commands(
    _pipe_name: &str,
    _commands: Option<crate::protocol::PlaybackCommandSender>,
) -> Result<(), IpcError> {
    Err(IpcError::UnsupportedPlatform)
}

#[cfg(not(windows))]
async fn run_on_event_pipe(
    _pipe_name: &str,
    _events: tokio::sync::watch::Receiver<crate::protocol::PlaybackStateEvent>,
) -> Result<(), IpcError> {
    Err(IpcError::UnsupportedPlatform)
}
