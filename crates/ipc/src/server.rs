use crate::IpcError;

/// Run the wallpaper host's named-pipe request loop until the process exits.
pub async fn run() -> Result<(), IpcError> {
    run_on_pipe(crate::protocol::PIPE_NAME).await
}

#[cfg(windows)]
pub(crate) async fn run_on_pipe(pipe_name: &str) -> Result<(), IpcError> {
    use tokio::{io::AsyncReadExt, net::windows::named_pipe::ServerOptions, time};

    use crate::{
        error::from_io,
        protocol::{IO_TIMEOUT, Response, decode_request, encode_message, read_frame, write_frame},
    };

    let mut options = ServerOptions::new();
    options.first_pipe_instance(true).max_instances(1);
    let mut pipe = options.create(pipe_name).map_err(from_io)?;

    loop {
        pipe.connect().await.map_err(from_io)?;

        let _request_result = time::timeout(IO_TIMEOUT, async {
            let payload = read_frame(&mut pipe).await?;
            let request = decode_request(&payload)?;
            let response = encode_message(&Response::pong(request.request_id))?;
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
        .await;

        pipe.disconnect().map_err(from_io)?;

        // Each request is isolated; malformed clients and slow readers do not
        // take down the host or prevent the next UI request from connecting.
    }
}

#[cfg(not(windows))]
pub(crate) async fn run_on_pipe(_pipe_name: &str) -> Result<(), IpcError> {
    Err(IpcError::UnsupportedPlatform)
}
