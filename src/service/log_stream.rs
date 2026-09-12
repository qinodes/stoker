//! Service-side log history and live event streaming.

use super::error_mapping::ServiceFailure;
use crate::ipc::{IpcResponse, LogStream, send_response};
use crate::log_storage;
use crate::scheduler::{LOG_CHUNK_SIZE, LogMessage, OutputStream, Scheduler};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
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
        match send_file_chunks(framed, path, 0, index == 0).await {
            Ok(Some(offset)) => delivered[index] = offset,
            Ok(None) if receiver.is_none() && log_storage::read_metadata(path)?.is_none() => {
                return Err(ServiceFailure::LogUnavailable {
                    path: (*path).to_path_buf(),
                }
                .into());
            }
            Ok(None) => {}
            Err(error) => return Err(error),
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
                    if let Some(offset) =
                        send_file_chunks(framed, path, delivered[index], index == 0).await?
                    {
                        delivered[index] = offset;
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
                    if let Some(offset) =
                        send_file_chunks(framed, path, delivered[index], index == 0).await?
                    {
                        delivered[index] = offset;
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

/// Send the current file contents after `requested_offset` without creating a
/// single allocation proportional to the file's total size.
async fn send_file_chunks<S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    path: &std::path::Path,
    requested_offset: u64,
    stdout: bool,
) -> anyhow::Result<Option<u64>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let metadata = log_storage::read_metadata(path)?;
    let segments = log_storage::list_segments(path)?;
    if segments.is_empty() {
        return Ok(metadata.map(|metadata| metadata.earliest_offset + metadata.retained_bytes));
    }
    let earliest = metadata
        .as_ref()
        .map_or(0, |metadata| metadata.earliest_offset);
    let lengths =
        futures_util::future::try_join_all(segments.iter().cloned().map(|segment| async move {
            Ok::<_, std::io::Error>(tokio::fs::metadata(segment).await?.len())
        }))
        .await?;
    let end = earliest + lengths.iter().copied().sum::<u64>();
    let mut offset = requested_offset.clamp(earliest, end);
    let mut skip = offset.saturating_sub(earliest);
    let stream = if stdout {
        LogStream::Stdout
    } else {
        LogStream::Stderr
    };
    let mut buffer = vec![0_u8; LOG_CHUNK_SIZE];
    for (segment, length) in segments.into_iter().zip(lengths) {
        if skip >= length {
            skip -= length;
            continue;
        }
        let mut file = tokio::fs::File::open(segment).await?;
        if skip > 0 {
            file.seek(SeekFrom::Start(skip)).await?;
            skip = 0;
        }
        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            send_response(
                framed,
                &IpcResponse::LogChunk {
                    stream,
                    bytes: buffer[..bytes_read].to_vec(),
                },
            )
            .await?;
            offset += bytes_read as u64;
        }
    }
    Ok(Some(offset))
}
