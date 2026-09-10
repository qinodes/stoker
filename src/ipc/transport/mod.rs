//! Cross-platform client transport selection.

use std::fs::OpenOptions;
#[cfg(windows)]
use std::time::Duration;

use fs2::FileExt;

use crate::StokerPaths;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(super) type IpcStream = tokio::net::UnixStream;
#[cfg(windows)]
pub(super) type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

pub(super) async fn connect(paths: &StokerPaths) -> std::io::Result<IpcStream> {
    #[cfg(unix)]
    return unix::connect(paths).await;
    #[cfg(windows)]
    return windows::connect(paths).await;
}

pub(super) async fn connect_with_retry(paths: &StokerPaths) -> std::io::Result<IpcStream> {
    #[cfg(unix)]
    return connect(paths).await;
    #[cfg(windows)]
    return windows::connect_with_retry(paths).await;
}

pub(super) fn service_lock_is_available(paths: &StokerPaths) -> bool {
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)
    else {
        return false;
    };
    if lock.try_lock_exclusive().is_err() {
        return false;
    }
    let _ = FileExt::unlock(&lock);
    true
}

#[cfg(windows)]
pub(super) const CONNECT_RETRY_WINDOW: Duration = Duration::from_secs(2);
