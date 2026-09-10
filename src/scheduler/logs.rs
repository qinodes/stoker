//! Runtime log file observation and event publication.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::time::sleep;

use super::{LogEvent, LogMessage, OutputStream};

pub(super) async fn watch_logs(
    stdout: PathBuf,
    stderr: PathBuf,
    sender: broadcast::Sender<LogMessage>,
) {
    let mut offsets = [0_u64, 0_u64];
    loop {
        let paths = [&stdout, &stderr];
        for (index, path) in paths.iter().enumerate() {
            if let Ok(bytes) = tokio::fs::read(path).await {
                let offset = offsets[index] as usize;
                if bytes.len() > offset {
                    let chunk = bytes[offset..].to_vec();
                    offsets[index] = bytes.len() as u64;
                    let _ = sender.send(LogMessage::Chunk(LogEvent {
                        stream: if index == 0 {
                            OutputStream::Stdout
                        } else {
                            OutputStream::Stderr
                        },
                        offset: offset as u64,
                        bytes: chunk,
                    }));
                }
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

pub(super) async fn flush_log_events(
    stdout: &Path,
    stderr: &Path,
    sender: &broadcast::Sender<LogMessage>,
) {
    for (path, stream) in [
        (stdout, OutputStream::Stdout),
        (stderr, OutputStream::Stderr),
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            let _ = sender.send(LogMessage::Chunk(LogEvent {
                stream,
                offset: 0,
                bytes,
            }));
        }
    }
}
