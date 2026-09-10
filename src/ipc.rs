//! Versioned local IPC protocol and client.

mod client;
mod compatibility;
mod error;
mod framing;
pub mod protocol;
mod transport;

pub use client::{ClientLogEvent, ClientLogStream, ServiceClient};
pub use error::{
    InvalidServiceResponse, ProtocolVersionMismatch, ServiceRejected, ServiceTimeout,
    ServiceUnavailable, StaleQueueMoveError, is_service_unavailable,
};
pub(crate) use framing::{decode_request, send_response};
pub use protocol::{
    IPC_VERSION, IpcError, IpcErrorCode, IpcErrorDetails, IpcRequest, IpcResponse, JobDto,
    JobStateDto, LogStream, ServiceStatus,
};

#[cfg(test)]
pub(crate) use framing::{decode_response, encode_request, encode_response};

#[cfg(test)]
use crate::{Job, StokerPaths};
#[cfg(test)]
use transport::connect_with_retry;
#[cfg(test)]
use uuid::Uuid;

#[cfg(test)]
fn test_job_dto(id: Uuid) -> JobDto {
    JobDto::from(Job {
        id,
        name: "test-job".to_owned(),
        user: "alice".to_owned(),
        cwd: std::path::PathBuf::from("workspace"),
        command: vec!["echo".to_owned()],
        command_line: Some("echo".to_owned()),
        state: crate::JobState::Queued,
        queue_order: Some(1),
        created_at: chrono::Utc::now(),
        committed_at: None,
        started_at: None,
        finished_at: None,
        exit_code: None,
        pid: None,
        failure_detail: None,
        description: None,
        description_revision: 0,
    })
}

#[cfg(test)]
fn test_error(code: IpcErrorCode, message: &str) -> IpcResponse {
    IpcResponse::Error(IpcError::new(code, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Job, JobState};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn fixture_job() -> Job {
        let id = Uuid::nil();
        Job {
            id,
            name: "queued-job".to_owned(),
            user: "alice".to_owned(),
            cwd: PathBuf::from("workspace"),
            command: vec!["echo".to_owned(), "hello".to_owned()],
            command_line: Some("echo hello".to_owned()),
            state: JobState::Queued,
            queue_order: Some(1),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            committed_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-01-02T03:05:05Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            started_at: None,
            finished_at: None,
            exit_code: None,
            pid: None,
            failure_detail: None,
            description: Some("fixture".to_owned()),
            description_revision: 4,
        }
    }

    fn request_cases() -> Vec<(&'static str, IpcRequest)> {
        let id = Uuid::nil();
        vec![
            ("status", IpcRequest::Status),
            ("stop", IpcRequest::Stop),
            ("commit", IpcRequest::Commit { id }),
            ("commit_many", IpcRequest::CommitMany { ids: vec![id] }),
            ("commit_all", IpcRequest::CommitAll),
            (
                "commit_user",
                IpcRequest::CommitUser {
                    user: "alice".to_owned(),
                },
            ),
            ("cancel", IpcRequest::Cancel { id }),
            ("follow_logs", IpcRequest::FollowLogs { id }),
            ("lock_queue", IpcRequest::LockQueue),
            ("unlock_queue", IpcRequest::UnlockQueue),
            (
                "move_queued",
                IpcRequest::MoveQueued {
                    id,
                    target_order: 2,
                },
            ),
        ]
    }

    fn response_cases() -> Vec<(&'static str, IpcResponse)> {
        vec![
            ("ack", IpcResponse::Ack),
            (
                "job",
                IpcResponse::Job {
                    job: fixture_job().into(),
                },
            ),
            (
                "jobs",
                IpcResponse::Jobs {
                    jobs: vec![fixture_job().into()],
                },
            ),
            (
                "queue",
                IpcResponse::Queue {
                    jobs: vec![fixture_job().into()],
                    locked: true,
                },
            ),
            (
                "status",
                IpcResponse::Status(ServiceStatus {
                    pid: 42,
                    active_job: None,
                    queued_jobs: 3,
                    queue_locked: true,
                }),
            ),
            (
                "stdout_chunk",
                IpcResponse::LogChunk {
                    stream: LogStream::Stdout,
                    bytes: b"hi\n".to_vec(),
                },
            ),
            (
                "stderr_chunk",
                IpcResponse::LogChunk {
                    stream: LogStream::Stderr,
                    bytes: b"err\n".to_vec(),
                },
            ),
            ("log_end", IpcResponse::LogEnd),
            (
                "error",
                IpcResponse::Error(IpcError::new(IpcErrorCode::Internal, "fixture failure")),
            ),
        ]
    }

    #[test]
    fn all_protocol_variants_round_trip() {
        for (_, request) in request_cases() {
            let encoded = encode_request(&request).unwrap();
            assert_eq!(decode_request(&encoded).unwrap(), request);
        }
        for (_, response) in response_cases() {
            let encoded = encode_response(&response).unwrap();
            assert_eq!(decode_response(&encoded).unwrap(), response);
        }
    }

    #[test]
    fn protocol_v4_matches_literal_golden_fixtures() {
        let request_fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/ipc/v4/requests.json")).unwrap();
        for (name, request) in request_cases() {
            let encoded = encode_request(&request).unwrap();
            let actual: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(actual, request_fixtures[name], "request fixture {name}");
            let literal = serde_json::to_vec(&request_fixtures[name]).unwrap();
            assert_eq!(decode_request(&literal).unwrap(), request);
        }

        let response_fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/ipc/v4/responses.json")).unwrap();
        for (name, response) in response_cases() {
            let encoded = encode_response(&response).unwrap();
            let actual: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(actual, response_fixtures[name], "response fixture {name}");
            let literal = serde_json::to_vec(&response_fixtures[name]).unwrap();
            assert_eq!(decode_response(&literal).unwrap(), response);
        }
    }

    #[test]
    fn frames_include_protocol_version() {
        let id = Uuid::nil();
        for request in [
            IpcRequest::Status,
            IpcRequest::Commit { id },
            IpcRequest::CommitMany { ids: vec![id] },
            IpcRequest::CommitAll,
            IpcRequest::CommitUser {
                user: "alice".to_owned(),
            },
            IpcRequest::LockQueue,
            IpcRequest::UnlockQueue,
            IpcRequest::MoveQueued {
                id,
                target_order: 2,
            },
        ] {
            let encoded = encode_request(&request).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(value["version"], IPC_VERSION);
        }
    }

    #[test]
    fn scheduler_models_map_explicitly_to_ipc_dtos() {
        let status = crate::scheduler::SchedulerStatus {
            pid: 42,
            active_job: Some(Uuid::nil()),
            queued_jobs: 3,
            queue_locked: true,
        };
        assert_eq!(
            ServiceStatus::from(status),
            ServiceStatus {
                pid: 42,
                active_job: Some(Uuid::nil()),
                queued_jobs: 3,
                queue_locked: true,
            }
        );
        assert_eq!(
            LogStream::from(crate::scheduler::OutputStream::Stdout),
            LogStream::Stdout
        );
        assert_eq!(
            LogStream::from(crate::scheduler::OutputStream::Stderr),
            LogStream::Stderr
        );
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        let frame = serde_json::json!({
            "version": 1,
            "request": "Status"
        });
        let error = decode_request(&serde_json::to_vec(&frame).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported IPC protocol version")
        );
    }

    #[tokio::test]
    async fn missing_service_returns_without_waiting_for_retry_window() {
        let root = tempfile::tempdir().unwrap();
        let paths = StokerPaths {
            database: root.path().join("stoker.db"),
            runs: root.path().join("runs"),
            lock: root.path().join("stoker.lock"),
            endpoint: root.path().join("stoker.sock"),
            root: root.path().to_path_buf(),
        };
        let started = Instant::now();

        let error = connect_with_retry(&paths).await.unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "missing service took {:?} to report unavailable",
            started.elapsed()
        );
    }
}

#[cfg(all(test, unix))]
mod unix_client_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::future::Future;
    use std::path::Path;
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;
    use tokio_util::codec::{Framed, LengthDelimitedCodec};

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    async fn client_with_responses(
        responses: Vec<IpcResponse>,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let listener = UnixListener::bind(&paths.endpoint).unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            for response in responses {
                framed
                    .send(encode_response(&response).unwrap().into())
                    .await
                    .unwrap();
            }
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn client_with_raw_response(
        response: Vec<u8>,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let listener = UnixListener::bind(&paths.endpoint).unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            framed.send(response.into()).await.unwrap();
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn assert_response_error<T, F, Fut>(response: IpcResponse, operation: F, expected: &str)
    where
        F: FnOnce(ServiceClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (_directory, client, task) = client_with_responses(vec![response]).await;
        let error = operation(client)
            .await
            .err()
            .expect("operation unexpectedly succeeded");
        assert!(error.to_string().contains(expected), "{error:#}");
        task.await.unwrap();
    }

    async fn collect_logs(client: ServiceClient, id: Uuid) -> anyhow::Result<Vec<ClientLogEvent>> {
        let mut stream = client.follow_log_stream(id).await?;
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            let event = event?;
            let done = event == ClientLogEvent::End;
            events.push(event);
            if done {
                break;
            }
        }
        Ok(events)
    }

    fn sample_status() -> ServiceStatus {
        ServiceStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 0,
            queue_locked: false,
        }
    }

    #[tokio::test]
    async fn client_methods_validate_response_variants() {
        let (_directory, client, task) =
            client_with_responses(vec![IpcResponse::Status(sample_status())]).await;
        assert_eq!(client.status().await.unwrap(), sample_status());
        task.await.unwrap();

        for response in [
            IpcResponse::Ack,
            IpcResponse::Job {
                job: test_job_dto(Uuid::nil()),
            },
            IpcResponse::Jobs { jobs: Vec::new() },
            IpcResponse::Queue {
                jobs: Vec::new(),
                locked: false,
            },
        ] {
            assert_response_error(
                response,
                |client| async move { client.status().await },
                "invalid status",
            )
            .await;
        }
        for response in [
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ] {
            assert_response_error(
                response,
                |client| async move { client.status().await },
                "invalid status",
            )
            .await;
        }
        assert_response_error(
            test_error(IpcErrorCode::Internal, "status failed"),
            |client| async move { client.status().await },
            "status failed",
        )
        .await;

        let id = Uuid::nil();
        for (operation, expected) in [
            ("commit", "invalid commit"),
            ("cancel", "invalid cancel"),
            ("lock queue", "invalid lock queue"),
            ("unlock queue", "invalid unlock queue"),
        ] {
            let response = IpcResponse::Status(sample_status());
            match operation {
                "commit" => {
                    assert_response_error(
                        response,
                        |client| async move { client.commit(id).await },
                        expected,
                    )
                    .await
                }
                "cancel" => {
                    assert_response_error(
                        response,
                        |client| async move { client.cancel(id).await },
                        expected,
                    )
                    .await
                }
                "lock queue" => {
                    assert_response_error(
                        response,
                        |client| async move { client.lock_queue().await },
                        expected,
                    )
                    .await
                }
                "unlock queue" => {
                    assert_response_error(
                        response,
                        |client| async move { client.unlock_queue().await },
                        expected,
                    )
                    .await
                }
                _ => unreachable!(),
            }
        }

        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "commit failed"),
            |client| async move { client.commit(id).await },
            "commit failed",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "commit many failed"),
            |client| async move { client.commit_many(vec![id]).await },
            "commit many failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_many(vec![id]).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "commit user failed"),
            |client| async move { client.commit_user("alice".to_owned()).await },
            "commit user failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_user("alice".to_owned()).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.commit_all().await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "cancel failed"),
            |client| async move { client.cancel(id).await },
            "cancel failed",
        )
        .await;

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Jobs {
            jobs: (0..3).map(|_| test_job_dto(Uuid::new_v4())).collect(),
        }])
        .await;
        assert_eq!(client.commit_all().await.unwrap(), 3);
        task.await.unwrap();

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Jobs {
            jobs: (0..2).map(|_| test_job_dto(Uuid::new_v4())).collect(),
        }])
        .await;
        assert_eq!(
            client.commit_many(vec![id, Uuid::new_v4()]).await.unwrap(),
            2
        );
        task.await.unwrap();

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Jobs {
            jobs: (0..4).map(|_| test_job_dto(Uuid::new_v4())).collect(),
        }])
        .await;
        assert_eq!(client.commit_user("alice".to_owned()).await.unwrap(), 4);
        task.await.unwrap();

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Job {
            job: test_job_dto(id),
        }])
        .await;
        client.commit(id).await.unwrap();
        task.await.unwrap();

        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Queue {
            jobs: Vec::new(),
            locked: true,
        }])
        .await;
        assert!(client.move_queued(id, 1).await.unwrap().is_empty());
        task.await.unwrap();
        assert_response_error(
            test_error(IpcErrorCode::StaleQueue, "job disappeared"),
            |client| async move { client.move_queued(id, 1).await },
            "job disappeared",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::QueueUnlocked, "move failed"),
            |client| async move { client.move_queued(id, 1).await },
            "move failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.move_queued(id, 1).await },
            "invalid move queued",
        )
        .await;
    }

    #[tokio::test]
    async fn stop_and_follow_logs_handle_success_errors_and_closed_streams() {
        let (_directory, client, task) = client_with_responses(vec![IpcResponse::Ack]).await;
        client.stop().await.unwrap();
        task.await.unwrap();

        assert_response_error(
            IpcResponse::Status(sample_status()),
            |client| async move { client.stop().await },
            "invalid stop",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::Internal, "stop failed"),
            |client| async move { client.stop().await },
            "stop failed",
        )
        .await;

        let (_directory, client, task) = client_with_responses(vec![
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogChunk {
                stream: LogStream::Stderr,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ])
        .await;
        let events = collect_logs(client, Uuid::nil()).await.unwrap();
        assert_eq!(events.len(), 3);
        task.await.unwrap();

        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "log follow failed"),
            |client| async move { collect_logs(client, Uuid::nil()).await },
            "log follow failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { collect_logs(client, Uuid::nil()).await },
            "invalid log response",
        )
        .await;

        let (_directory, client, task) = client_with_responses(Vec::new()).await;
        let error = collect_logs(client, Uuid::nil()).await.unwrap_err();
        assert!(error.to_string().contains("closed the log stream"));
        task.await.unwrap();
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn legacy_follow_logs_adapter_preserves_console_routing_contract() {
        let (_directory, client, task) = client_with_responses(vec![
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogChunk {
                stream: LogStream::Stderr,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ])
        .await;

        client.follow_logs(Uuid::nil()).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_response_and_response_timeout_are_typed() {
        let (_directory, client, task) = client_with_raw_response(b"not-json".to_vec()).await;
        assert!(client.status().await.unwrap_err().is::<serde_json::Error>());
        task.await.unwrap();

        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let listener = UnixListener::bind(&paths.endpoint).unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            std::future::pending::<()>().await;
        });
        let client = ServiceClient::with_timeout(paths, std::time::Duration::from_millis(20));
        assert!(client.status().await.unwrap_err().is::<ServiceTimeout>());
        task.abort();
        let _ = task.await;
    }
}

#[cfg(all(test, windows))]
mod windows_client_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::future::Future;
    use std::path::Path;
    use tokio::net::windows::named_pipe::ServerOptions;
    use tokio::task::JoinHandle;
    use tokio_util::codec::{Framed, LengthDelimitedCodec};

    fn paths(root: &Path) -> StokerPaths {
        StokerPaths {
            root: root.to_path_buf(),
            database: root.join("stoker.db"),
            runs: root.join("runs"),
            lock: root.join("stoker.lock"),
            endpoint: root.join("stoker.sock"),
        }
    }

    async fn client_with_response(
        response: IpcResponse,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let server = ServerOptions::new().create(paths.ipc_endpoint()).unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut framed = Framed::new(server, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            framed
                .send(encode_response(&response).unwrap().into())
                .await
                .unwrap();
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn client_with_raw_response(
        response: Vec<u8>,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let server = ServerOptions::new().create(paths.ipc_endpoint()).unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut framed = Framed::new(server, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            framed.send(response.into()).await.unwrap();
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn client_with_responses(
        responses: Vec<IpcResponse>,
    ) -> (tempfile::TempDir, ServiceClient, JoinHandle<()>) {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let server = ServerOptions::new().create(paths.ipc_endpoint()).unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut framed = Framed::new(server, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            for response in responses {
                framed
                    .send(encode_response(&response).unwrap().into())
                    .await
                    .unwrap();
            }
        });
        (directory, ServiceClient::new(paths), task)
    }

    async fn assert_response_error<T, F, Fut>(response: IpcResponse, operation: F, expected: &str)
    where
        F: FnOnce(ServiceClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (_directory, client, task) = client_with_response(response).await;
        let error = operation(client)
            .await
            .err()
            .expect("operation unexpectedly succeeded");
        assert!(error.to_string().contains(expected), "{error:#}");
        task.await.unwrap();
    }

    async fn collect_logs(client: ServiceClient, id: Uuid) -> anyhow::Result<Vec<ClientLogEvent>> {
        let mut stream = client.follow_log_stream(id).await?;
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            let event = event?;
            let done = event == ClientLogEvent::End;
            events.push(event);
            if done {
                break;
            }
        }
        Ok(events)
    }

    #[tokio::test]
    async fn commit_batch_clients_handle_success_errors_and_invalid_responses() {
        let id = Uuid::nil();

        let (_directory, client, task) = client_with_response(IpcResponse::Jobs {
            jobs: (0..2).map(|_| test_job_dto(Uuid::new_v4())).collect(),
        })
        .await;
        assert_eq!(client.commit_many(vec![id]).await.unwrap(), 2);
        task.await.unwrap();

        let (_directory, client, task) = client_with_response(IpcResponse::Jobs {
            jobs: (0..4).map(|_| test_job_dto(Uuid::new_v4())).collect(),
        })
        .await;
        assert_eq!(client.commit_user("alice".to_owned()).await.unwrap(), 4);
        task.await.unwrap();

        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "commit many failed"),
            |client| async move { client.commit_many(vec![id]).await },
            "commit many failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_many(vec![id]).await },
            "invalid commit",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "commit user failed"),
            |client| async move { client.commit_user("alice".to_owned()).await },
            "commit user failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.commit_user("alice".to_owned()).await },
            "invalid commit",
        )
        .await;
    }

    #[tokio::test]
    async fn client_methods_cover_status_queue_actions_and_log_streams() {
        let id = Uuid::nil();
        let status = ServiceStatus {
            pid: 42,
            active_job: None,
            queued_jobs: 2,
            queue_locked: false,
        };

        let (_directory, client, task) =
            client_with_response(IpcResponse::Status(status.clone())).await;
        assert_eq!(client.status().await.unwrap(), status);
        task.await.unwrap();

        assert_response_error(
            IpcResponse::Ack,
            |client| async move { client.status().await },
            "invalid status",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::Internal, "status failed"),
            |client| async move { client.status().await },
            "status failed",
        )
        .await;

        let (_directory, client, task) = client_with_response(IpcResponse::Ack).await;
        client.stop().await.unwrap();
        task.await.unwrap();
        assert_response_error(
            IpcResponse::Status(status.clone()),
            |client| async move { client.stop().await },
            "invalid stop",
        )
        .await;
        assert_response_error(
            test_error(IpcErrorCode::Internal, "stop failed"),
            |client| async move { client.stop().await },
            "stop failed",
        )
        .await;

        for (operation, response, expected) in [
            (
                "commit",
                IpcResponse::Job {
                    job: test_job_dto(id),
                },
                "",
            ),
            (
                "cancel",
                IpcResponse::Job {
                    job: test_job_dto(id),
                },
                "",
            ),
            (
                "lock",
                IpcResponse::Queue {
                    jobs: Vec::new(),
                    locked: true,
                },
                "",
            ),
            (
                "unlock",
                IpcResponse::Queue {
                    jobs: Vec::new(),
                    locked: false,
                },
                "",
            ),
        ] {
            let (_directory, client, task) = client_with_response(response).await;
            match operation {
                "commit" => client.commit(id).await.unwrap(),
                "cancel" => client.cancel(id).await.unwrap(),
                "lock" => client.lock_queue().await.unwrap(),
                "unlock" => client.unlock_queue().await.unwrap(),
                _ => unreachable!("{expected}"),
            }
            task.await.unwrap();
        }

        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "cancel failed"),
            |client| async move { client.cancel(id).await },
            "cancel failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(status.clone()),
            |client| async move { client.lock_queue().await },
            "invalid lock queue",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(status.clone()),
            |client| async move { client.unlock_queue().await },
            "invalid unlock queue",
        )
        .await;

        let (_directory, client, task) = client_with_response(IpcResponse::Queue {
            jobs: Vec::new(),
            locked: true,
        })
        .await;
        assert!(client.move_queued(id, 1).await.unwrap().is_empty());
        task.await.unwrap();
        assert_response_error(
            test_error(IpcErrorCode::StaleQueue, "job disappeared"),
            |client| async move { client.move_queued(id, 1).await },
            "job disappeared",
        )
        .await;
        assert_response_error(
            IpcResponse::Status(status),
            |client| async move { client.move_queued(id, 1).await },
            "invalid move queued",
        )
        .await;

        let (_directory, client, task) = client_with_responses(vec![
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogChunk {
                stream: LogStream::Stderr,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ])
        .await;
        let events = collect_logs(client, id).await.unwrap();
        assert_eq!(events.len(), 3);
        task.await.unwrap();
        assert_response_error(
            test_error(IpcErrorCode::InvalidState, "log follow failed"),
            |client| async move { collect_logs(client, id).await },
            "log follow failed",
        )
        .await;
        assert_response_error(
            IpcResponse::Ack,
            |client| async move { collect_logs(client, id).await },
            "invalid log response",
        )
        .await;
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn legacy_follow_logs_adapter_preserves_console_routing_contract() {
        let (_directory, client, task) = client_with_responses(vec![
            IpcResponse::LogChunk {
                stream: LogStream::Stdout,
                bytes: Vec::new(),
            },
            IpcResponse::LogChunk {
                stream: LogStream::Stderr,
                bytes: Vec::new(),
            },
            IpcResponse::LogEnd,
        ])
        .await;

        client.follow_logs(Uuid::nil()).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_response_and_response_timeout_are_typed() {
        let (_directory, client, task) = client_with_raw_response(b"not-json".to_vec()).await;
        assert!(client.status().await.unwrap_err().is::<serde_json::Error>());
        task.await.unwrap();

        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let server = ServerOptions::new().create(paths.ipc_endpoint()).unwrap();
        let task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut framed = Framed::new(server, LengthDelimitedCodec::new());
            let _request = framed.next().await.unwrap().unwrap();
            std::future::pending::<()>().await;
        });
        let client = ServiceClient::with_timeout(paths, std::time::Duration::from_millis(20));
        assert!(client.status().await.unwrap_err().is::<ServiceTimeout>());
        task.abort();
        let _ = task.await;
    }
}
