//! Unix-domain-socket listener and stale endpoint handling.

use std::fs::OpenOptions;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use fs2::FileExt;
use tokio::sync::{mpsc, watch};

use crate::scheduler::Scheduler;

use super::super::Service;
use super::super::dispatch::handle_client;

pub(super) async fn run(
    service: Service,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    wake_rx: watch::Receiver<u64>,
) -> anyhow::Result<()> {
    let endpoint = &service.paths.endpoint;
    remove_stale_socket(endpoint)?;
    let listener = tokio::net::UnixListener::bind(endpoint)
        .with_context(|| format!("bind IPC endpoint {}", endpoint.display()))?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (scheduler_error_tx, scheduler_error_rx) = mpsc::unbounded_channel();
    let scheduler_for_task = scheduler.clone();
    let scheduler_shutdown_rx = shutdown_rx.clone();
    let scheduler_task = tokio::spawn(async move {
        let result = scheduler_for_task.run(wake_rx, scheduler_shutdown_rx).await;
        if let Err(error) = &result {
            let _ = scheduler_error_tx.send(error.to_string());
        }
        result
    });
    let result = accept(
        listener,
        shutdown_tx,
        shutdown_rx,
        scheduler,
        wake_tx,
        scheduler_error_rx,
    )
    .await;
    let scheduler_result = scheduler_task
        .await
        .map_err(|error| anyhow::anyhow!("scheduler task failed: {error}"))?;
    std::fs::remove_file(endpoint)
        .with_context(|| format!("remove IPC endpoint {}", endpoint.display()))?;
    result.and(scheduler_result)
}

async fn accept(
    listener: tokio::net::UnixListener,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    mut scheduler_errors: mpsc::UnboundedReceiver<String>,
) -> anyhow::Result<()> {
    let mut handlers = Vec::new();
    let result = loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    break Ok(());
                }
            },
            scheduler_error = scheduler_errors.recv() => {
                let error = match scheduler_error {
                    Some(error) => error,
                    None if *shutdown_rx.borrow() => break Ok(()),
                    None => "scheduler task ended without a result (possible panic)".to_owned(),
                };
                let _ = shutdown_tx.send(true);
                break Err(anyhow::anyhow!("scheduler task failed: {error}"));
            },
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        handlers.push(tokio::spawn(handle_client(
                            stream,
                            Arc::clone(&scheduler),
                            wake_tx.clone(),
                            shutdown_tx.clone(),
                            shutdown_rx.clone(),
                        )));
                    }
                    Err(error) => break Err(anyhow::Error::new(error).context("accept scheduler IPC client")),
                }
            }
        }
    };
    let _ = shutdown_tx.send(true);
    for handler in handlers {
        let _ = handler.await;
    }
    result
}

fn remove_stale_socket(endpoint: &Path) -> anyhow::Result<()> {
    // Coordinate endpoint replacement with any other Stoker process touching
    // this directory. The service singleton lock handles normal startup, but
    // this narrower lock keeps inspect+unlink one critical section for stale
    // endpoint cleanup and makes path replacement by another Stoker process
    // impossible during the operation.
    let parent = endpoint.parent().unwrap_or_else(|| Path::new("."));
    let directory_lock = OpenOptions::new()
        .read(true)
        .open(parent)
        .with_context(|| format!("open IPC directory {}", parent.display()))?;
    directory_lock
        .lock_exclusive()
        .with_context(|| format!("lock IPC directory {}", parent.display()))?;

    let metadata = match std::fs::symlink_metadata(endpoint) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect IPC endpoint {}", endpoint.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        anyhow::bail!(
            "refusing to replace non-socket IPC endpoint {}",
            endpoint.display()
        );
    }

    // Re-lstat and compare the inode immediately before unlinking. If another
    // process swaps the path between the first check and this operation, do
    // not remove the replacement; the service will fail its bind and report a
    // clear startup error instead.
    let current = std::fs::symlink_metadata(endpoint)
        .with_context(|| format!("recheck IPC endpoint {}", endpoint.display()))?;
    if !current.file_type().is_socket()
        || current.dev() != metadata.dev()
        || current.ino() != metadata.ino()
    {
        anyhow::bail!("IPC endpoint changed while checking {}", endpoint.display());
    }
    std::fs::remove_file(endpoint)
        .with_context(|| format!("remove stale IPC endpoint {}", endpoint.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_is_removed_but_regular_file_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = directory.path().join("stoker.sock");
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        drop(listener);
        assert!(endpoint.exists());
        remove_stale_socket(&endpoint).unwrap();
        assert!(!endpoint.exists());

        std::fs::write(&endpoint, b"owner data").unwrap();
        let error = remove_stale_socket(&endpoint).unwrap_err();
        assert!(error.to_string().contains("non-socket IPC endpoint"));
        assert_eq!(std::fs::read(&endpoint).unwrap(), b"owner data");
    }
}
