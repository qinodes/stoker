//! Source-compatible adapters for public APIs that predate typed log events.

use std::io::Write;

use futures_util::StreamExt;
use uuid::Uuid;

use super::{ClientLogEvent, LogStream, ServiceClient};

impl ServiceClient {
    /// Follow a job's output and write chunks to the matching local streams.
    ///
    /// New integrations should use [`Self::follow_log_stream`] so presentation
    /// remains outside the IPC client.
    #[deprecated(note = "use follow_log_stream to consume typed log events")]
    pub async fn follow_logs(&self, id: Uuid) -> anyhow::Result<()> {
        let mut events = self.follow_log_stream(id).await?;
        while let Some(event) = events.next().await {
            match event? {
                ClientLogEvent::Chunk {
                    stream: LogStream::Stdout,
                    bytes,
                } => std::io::stdout().write_all(&bytes)?,
                ClientLogEvent::Chunk {
                    stream: LogStream::Stderr,
                    bytes,
                } => std::io::stderr().write_all(&bytes)?,
                ClientLogEvent::End => return Ok(()),
            }
        }
        anyhow::bail!("scheduler closed the log stream")
    }
}
