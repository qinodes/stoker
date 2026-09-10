//! Service-side log history and live event streaming.

use super::error_mapping::ServiceFailure;
use crate::ipc::{IpcResponse, LogStream, send_response};
use crate::scheduler::{LogMessage, OutputStream, Scheduler};
use tokio::sync::watch;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub(super) async fn stream_logs<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    scheduler: &Scheduler,
    id: uuid::Uuid,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if *shutdown_rx.borrow() {
        return Err(ServiceFailure::ShuttingDown {
            operation: "log follow",
        }
        .into());
    }
    let job = scheduler.job_exists(id)?;
    if job.state == crate::JobState::Draft {
        return Err(ServiceFailure::InvalidLogState {
            id,
            state: job.state,
        }
        .into());
    }
    let (stdout, stderr) = scheduler.log_paths(id);
    let mut receiver = scheduler.log_receiver(id);
    // A commit response only acknowledges persistence. Give the scheduler a
    // short window to claim the queued job and create its run directory before
    // a follower attempts its initial read.
    if receiver.is_none()
        && matches!(
            job.state,
            crate::JobState::Queued | crate::JobState::Starting | crate::JobState::Running
        )
    {
        // Do not impose a wall-clock limit: a queued job may legitimately wait
        // behind arbitrarily long work. Stop waiting only when its durable
        // state becomes terminal (or it is canceled before starting).
        loop {
            if *shutdown_rx.borrow() {
                return Err(ServiceFailure::ShuttingDown {
                    operation: "log follow",
                }
                .into());
            }
            if let Some(found) = scheduler.log_receiver(id) {
                receiver = Some(found);
                break;
            }
            let state = scheduler.job_exists(id)?.state;
            if matches!(
                state,
                crate::JobState::Succeeded
                    | crate::JobState::Failed
                    | crate::JobState::Cancelled
                    | crate::JobState::Lost
            ) {
                break;
            }
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return Err(ServiceFailure::ShuttingDown {
                            operation: "log follow",
                        }
                        .into());
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
            }
        }
    }
    let mut delivered = [0_u64, 0_u64];
    for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
        match tokio::fs::read(path).await {
            Ok(bytes) if !bytes.is_empty() => {
                delivered[index] = bytes.len() as u64;
                send_response(
                    framed,
                    &IpcResponse::LogChunk {
                        stream: if index == 0 {
                            LogStream::Stdout
                        } else {
                            LogStream::Stderr
                        },
                        bytes,
                    },
                )
                .await?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if receiver.is_none() {
                    return Err(ServiceFailure::LogUnavailable {
                        path: (*path).to_path_buf(),
                    }
                    .into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    let Some(mut receiver) = receiver.take() else {
        send_response(framed, &IpcResponse::LogEnd).await?;
        return Ok(());
    };
    loop {
        if *shutdown_rx.borrow() {
            return Err(ServiceFailure::ShuttingDown {
                operation: "log follow",
            }
            .into());
        }
        let message = tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return Err(ServiceFailure::ShuttingDown {
                        operation: "log follow",
                    }
                    .into());
                }
                continue;
            }
            message = receiver.recv() => message,
        };
        match message {
            Ok(LogMessage::End) => {
                // The final flush can race the End notification; read once
                // more so followers never miss trailing bytes.
                for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        let start = delivered[index] as usize;
                        if bytes.len() > start {
                            send_response(
                                framed,
                                &IpcResponse::LogChunk {
                                    stream: if index == 0 {
                                        LogStream::Stdout
                                    } else {
                                        LogStream::Stderr
                                    },
                                    bytes: bytes[start..].to_vec(),
                                },
                            )
                            .await?;
                        }
                    }
                }
                send_response(framed, &IpcResponse::LogEnd).await?;
                return Ok(());
            }
            Ok(LogMessage::Chunk(event)) => {
                let index = if event.stream == OutputStream::Stdout {
                    0
                } else {
                    1
                };
                let start = delivered[index].max(event.offset);
                let skip = start.saturating_sub(event.offset) as usize;
                if skip < event.bytes.len() {
                    send_response(
                        framed,
                        &IpcResponse::LogChunk {
                            stream: event.stream.into(),
                            bytes: event.bytes[skip..].to_vec(),
                        },
                    )
                    .await?;
                    delivered[index] = event.offset + event.bytes.len() as u64;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                for (index, path) in [stdout.as_path(), stderr.as_path()].iter().enumerate() {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        let start = delivered[index] as usize;
                        if bytes.len() > start {
                            send_response(
                                framed,
                                &IpcResponse::LogChunk {
                                    stream: if index == 0 {
                                        LogStream::Stdout
                                    } else {
                                        LogStream::Stderr
                                    },
                                    bytes: bytes[start..].to_vec(),
                                },
                            )
                            .await?;
                            delivered[index] = bytes.len() as u64;
                        }
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                send_response(framed, &IpcResponse::LogEnd).await?;
                return Ok(());
            }
        }
    }
}
