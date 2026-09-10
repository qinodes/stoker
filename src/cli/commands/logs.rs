//! Job log command orchestration.

use std::io::Write;

use futures_util::StreamExt;
use uuid::Uuid;

use crate::application;
use crate::domain::JobState;
use crate::ipc::{ClientLogEvent, LogStream};
use crate::{ServiceClient, StokerPaths, Store};

use super::super::{application_cli_error, runtime};

pub(crate) fn logs(paths: &StokerPaths, id: Uuid, follow: bool) -> anyhow::Result<()> {
    if follow {
        let paths = paths.clone();
        return runtime()?.block_on(async move {
            let mut events = ServiceClient::new(paths).follow_log_stream(id).await?;
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
        });
    }
    let store = Store::open(&paths.database)?;
    let logs =
        application::logs::read_logs(&store, paths, id, None).map_err(application_cli_error)?;
    if !logs.stdout.available && !logs.stderr.available {
        match logs.job.state {
            JobState::Draft => anyhow::bail!(
                "Job {id} is still DRAFT; run `stoker commit {id}` before viewing its logs."
            ),
            JobState::Queued => anyhow::bail!(
                "Job {id} is QUEUED; logs will be available after the scheduler starts it."
            ),
            _ => anyhow::bail!("No logs are available for job {id} yet."),
        }
    }
    std::io::stdout().write_all(&logs.stdout.bytes)?;
    std::io::stderr().write_all(&logs.stderr.bytes)?;
    Ok(())
}
