//! Runtime log file observation and event publication.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::{Mutex, broadcast};
use tokio::time::sleep;

use super::LOG_CHUNK_SIZE;
use super::{LogEvent, LogMessage, OutputStream};
use crate::log_storage;

pub(super) async fn watch_logs(
    stdout: PathBuf,
    stderr: PathBuf,
    sender: broadcast::Sender<LogMessage>,
    offsets: Arc<Mutex<[u64; 2]>>,
) {
    loop {
        let paths = [&stdout, &stderr];
        for (index, path) in paths.iter().enumerate() {
            let stream = if index == 0 {
                OutputStream::Stdout
            } else {
                OutputStream::Stderr
            };
            let requested_offset = offsets.lock().await[index];
            if let Ok(offset) = publish_file_chunks(path, requested_offset, stream, &sender).await {
                offsets.lock().await[index] = offset;
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub(super) async fn flush_log_events(
    stdout: &Path,
    stderr: &Path,
    sender: &broadcast::Sender<LogMessage>,
    offsets: &Arc<Mutex<[u64; 2]>>,
) {
    for (index, (path, stream)) in [
        (stdout, OutputStream::Stdout),
        (stderr, OutputStream::Stderr),
    ]
    .into_iter()
    .enumerate()
    {
        let requested_offset = offsets.lock().await[index];
        if let Ok(offset) = publish_file_chunks(path, requested_offset, stream, sender).await {
            offsets.lock().await[index] = offset;
        }
    }
}

/// Publish a file from `requested_offset` in bounded chunks.
///
/// The watcher and final flush intentionally share this implementation so a
/// large durable log can never become one large allocation or broadcast event.
/// If retention/rotation has truncated the file before the caller's offset,
/// reading resumes at the earliest retained logical offset.
pub(super) async fn publish_file_chunks(
    path: &Path,
    requested_offset: u64,
    stream: OutputStream,
    sender: &broadcast::Sender<LogMessage>,
) -> std::io::Result<u64> {
    let metadata = log_storage::read_metadata(path)?;
    let segments = log_storage::list_segments(path)?;
    if segments.is_empty() {
        return Ok(metadata.map_or(0, |metadata| {
            metadata.earliest_offset + metadata.retained_bytes
        }));
    }
    let mut lengths = Vec::with_capacity(segments.len());
    for segment in &segments {
        lengths.push(tokio::fs::metadata(segment).await?.len());
    }
    let earliest = metadata
        .as_ref()
        .map_or(0, |metadata| metadata.earliest_offset);
    let end = earliest + lengths.iter().copied().sum::<u64>();
    let mut offset = requested_offset.clamp(earliest, end);
    let mut skip = offset.saturating_sub(earliest);
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
            let bytes = buffer[..bytes_read].to_vec();
            let _ = sender.send(LogMessage::Chunk(LogEvent {
                stream,
                offset,
                bytes,
            }));
            offset += bytes_read as u64;
        }
    }
    Ok(offset)
}
