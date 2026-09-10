use std::time::Duration;

use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use super::{CONNECT_RETRY_WINDOW, service_lock_is_available};
use crate::StokerPaths;

const ERROR_PIPE_BUSY: i32 = 231;

pub(super) async fn connect(paths: &StokerPaths) -> std::io::Result<NamedPipeClient> {
    ClientOptions::new().open(paths.ipc_endpoint())
}

pub(super) async fn connect_with_retry(paths: &StokerPaths) -> std::io::Result<NamedPipeClient> {
    let deadline = tokio::time::Instant::now() + CONNECT_RETRY_WINDOW;
    loop {
        match connect(paths).await {
            Ok(client) => return Ok(client),
            Err(error)
                if is_retryable_pipe_connect_error(paths, &error)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_retryable_pipe_connect_error(paths: &StokerPaths, error: &std::io::Error) -> bool {
    (error.kind() == std::io::ErrorKind::NotFound && !service_lock_is_available(paths))
        || error.raw_os_error() == Some(ERROR_PIPE_BUSY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_pipe_is_retryable_without_message_matching() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StokerPaths {
            root: directory.path().to_path_buf(),
            database: directory.path().join("stoker.db"),
            runs: directory.path().join("runs"),
            lock: directory.path().join("stoker.lock"),
            endpoint: directory.path().join("stoker.sock"),
        };
        let error = std::io::Error::from_raw_os_error(ERROR_PIPE_BUSY);
        assert!(is_retryable_pipe_connect_error(&paths, &error));
    }
}
