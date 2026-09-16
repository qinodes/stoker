//! Job log command orchestration.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use futures_util::StreamExt;
use uuid::Uuid;

use crate::application;
use crate::domain::JobState;
use crate::ipc::{ClientLogEvent, LogStream};
use crate::log_storage;
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
    if let Ok(definition) = store.standalone_definition(id)
        && definition.mode == crate::domain::flow::ExecutionMode::Scheduled
    {
        anyhow::bail!("scheduled standalone logs require --run <RUN_ID>");
    }
    if stream_hidden_attempt_logs(paths, &store, id)? {
        return Ok(());
    }
    // Read only metadata here; stream the actual bytes below so a large log
    // cannot be materialized as one Vec before it reaches the terminal.
    let logs =
        application::logs::read_logs(&store, paths, id, Some(0)).map_err(application_cli_error)?;
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
    if logs.stdout.truncated {
        eprintln!("stdout log is truncated; older output was discarded");
    }
    if logs.stderr.truncated {
        eprintln!("stderr log is truncated; older output was discarded");
    }
    for (name, path) in [
        ("stdout", paths.runs.join(id.to_string()).join("stdout.log")),
        ("stderr", paths.runs.join(id.to_string()).join("stderr.log")),
    ] {
        if let Some(metadata) = log_storage::read_metadata(&path)?
            && let Some(error) = metadata.capture_error
        {
            eprintln!("{name} log capture degraded: {error}");
        }
    }
    stream_log(
        &paths.runs.join(id.to_string()).join("stdout.log"),
        &mut std::io::stdout(),
    )?;
    stream_log(
        &paths.runs.join(id.to_string()).join("stderr.log"),
        &mut std::io::stderr(),
    )?;
    Ok(())
}

fn stream_hidden_attempt_logs(
    paths: &StokerPaths,
    store: &Store,
    job_id: Uuid,
) -> anyhow::Result<bool> {
    let flow_id = format!("standalone/{job_id}");
    let Ok(flow) = store.get_flow(&flow_id) else {
        return Ok(false);
    };
    let Some(task) = flow.tasks.first() else {
        return Ok(false);
    };
    if task.retry == 0 {
        return Ok(false);
    }
    let runs = store.list_flow_runs(&flow_id)?;
    if runs.is_empty() {
        return Ok(false);
    }
    let mut found = false;
    for run in runs {
        let Some(task_run) = run.tasks.iter().find(|task| task.task_id == "job") else {
            continue;
        };
        for attempt in 1..=task_run.attempt_count {
            let directory = paths
                .runs
                .join("flows")
                .join(run.run_id.to_string())
                .join("job")
                .join(format!("attempt-{attempt}"));
            for stream in ["stdout", "stderr"] {
                let path = directory.join(format!("{stream}.log"));
                if path.is_file() {
                    found = true;
                    println!("--- run={} attempt={} {} ---", run.run_id, attempt, stream);
                    if stream == "stdout" {
                        stream_log(&path, &mut std::io::stdout())?;
                    } else {
                        stream_log(&path, &mut std::io::stderr())?;
                    }
                }
            }
        }
    }
    Ok(found)
}

fn stream_log(path: &Path, output: &mut impl Write) -> anyhow::Result<()> {
    let segments = log_storage::list_segments(path)?;
    if segments.is_empty() {
        return Ok(());
    }
    let mut buffer = vec![0_u8; crate::scheduler::LOG_CHUNK_SIZE];
    for segment in segments {
        let mut file = File::open(segment)?;
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            output.write_all(&buffer[..bytes_read])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_log_preserves_large_output_without_one_file_sized_buffer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stdout.log");
        let payload = vec![b'q'; crate::scheduler::LOG_CHUNK_SIZE * 2 + 5];
        std::fs::write(&path, &payload).unwrap();

        let mut output = Vec::new();
        stream_log(&path, &mut output).unwrap();
        assert_eq!(output, payload);
    }

    #[test]
    fn stream_log_treats_a_removed_stream_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let mut output = Vec::new();

        stream_log(&directory.path().join("missing.log"), &mut output).unwrap();

        assert!(output.is_empty());
    }
}
