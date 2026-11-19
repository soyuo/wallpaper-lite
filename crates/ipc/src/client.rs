use std::sync::atomic::{AtomicU64, Ordering};

use crate::{IpcError, protocol::PIPE_NAME};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Send one bounded ping request to the wallpaper host.
pub async fn ping() -> Result<(), IpcError> {
    ping_at(PIPE_NAME).await
}

#[cfg(windows)]
pub(crate) async fn ping_at(pipe_name: &str) -> Result<(), IpcError> {
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
    let request = encode_message(&Request::ping(request_id))?;

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
