use uuid::Uuid;

/// Runtime-owned scheduler status. IPC and HTTP adapters map this explicitly
/// into their own wire DTOs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerStatus {
    pub pid: u32,
    pub active_job: Option<Uuid>,
    pub queued_jobs: usize,
    pub queue_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogEvent {
    pub stream: OutputStream,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogMessage {
    Chunk(LogEvent),
    End,
}

#[cfg(test)]
mod tests {
    use super::{LogEvent, LogMessage, OutputStream, SchedulerStatus};

    #[test]
    fn scheduler_models_are_transport_neutral_values() {
        let status = SchedulerStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 2,
            queue_locked: true,
        };
        assert_eq!(status.queued_jobs, 2);
        assert!(status.queue_locked);

        let event = LogEvent {
            stream: OutputStream::Stderr,
            offset: 7,
            bytes: b"failure".to_vec(),
        };
        assert!(matches!(
            LogMessage::Chunk(event),
            LogMessage::Chunk(LogEvent {
                stream: OutputStream::Stderr,
                offset: 7,
                ..
            })
        ));
        assert!(matches!(LogMessage::End, LogMessage::End));
    }
}
