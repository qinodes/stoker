//! IPC request dispatch independent of OS listener transport.

use std::sync::Arc;

use tokio::sync::watch;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::error_mapping::protocol_error;
use super::log_stream::stream_logs;
use crate::ipc::{IpcRequest, IpcResponse, decode_request, send_response};
use crate::scheduler::Scheduler;

pub(super) async fn handle_client<S>(
    stream: S,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    loop {
        let frame = tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return;
                }
                continue;
            }
            frame = futures_util::StreamExt::next(&mut framed) => frame,
        };
        let Some(frame) = frame else { return };
        let frame = match frame {
            Ok(frame) => frame,
            // A malformed/truncated frame affects only this connection.
            Err(_) => return,
        };
        let request = match decode_request(&frame) {
            Ok(request) => request,
            Err(error) => {
                let _ = send_response(
                    &mut framed,
                    &IpcResponse::Error(protocol_error(&error, "decode request")),
                )
                .await;
                return;
            }
        };
        if let IpcRequest::FollowLogs { id } = request {
            if let Err(error) = stream_logs(&mut framed, &scheduler, id, &mut shutdown_rx).await {
                let _ = send_response(
                    &mut framed,
                    &IpcResponse::Error(protocol_error(&error, "follow logs")),
                )
                .await;
            }
            return;
        }
        let stop = matches!(request, IpcRequest::Stop);
        let response = match request {
            IpcRequest::Status => match scheduler.scheduler_status() {
                Ok(status) => IpcResponse::Status(status.into()),
                Err(error) => IpcResponse::Error(protocol_error(&error, "status")),
            },
            IpcRequest::Stop => {
                // Close scheduler intake before waiting for active cleanup.
                // The endpoint remains alive until this handler replies and
                // the accept loop drains, allowing the client to observe a
                // completed stop rather than a fire-and-forget request.
                scheduler.begin_shutdown();
                let _ = shutdown_tx.send(true);
                match scheduler.stop_active().await {
                    Ok(()) => IpcResponse::Ack,
                    Err(error) => IpcResponse::Error(protocol_error(&error, "stop")),
                }
            }
            IpcRequest::Commit { id } => match scheduler.handle_commit(id, &wake_tx) {
                Ok(job) => IpcResponse::Job { job: job.into() },
                Err(error) => IpcResponse::Error(protocol_error(&error, "commit")),
            },
            IpcRequest::CommitMany { ids } => match scheduler.handle_commit_many(&ids, &wake_tx) {
                Ok(jobs) => IpcResponse::Jobs {
                    jobs: jobs.into_iter().map(Into::into).collect(),
                },
                Err(error) => IpcResponse::Error(protocol_error(&error, "commit many")),
            },
            IpcRequest::CommitAll => match scheduler.handle_commit_all(&wake_tx) {
                Ok(jobs) => IpcResponse::Jobs {
                    jobs: jobs.into_iter().map(Into::into).collect(),
                },
                Err(error) => IpcResponse::Error(protocol_error(&error, "commit all")),
            },
            IpcRequest::CommitUser { user } => {
                match scheduler.handle_commit_user(&user, &wake_tx) {
                    Ok(jobs) => IpcResponse::Jobs {
                        jobs: jobs.into_iter().map(Into::into).collect(),
                    },
                    Err(error) => IpcResponse::Error(protocol_error(&error, "commit user")),
                }
            }
            IpcRequest::Cancel { id } => match scheduler.handle_cancel(id).await {
                Ok(job) => IpcResponse::Job { job: job.into() },
                Err(error) => IpcResponse::Error(protocol_error(&error, "cancel")),
            },
            IpcRequest::LockQueue => match scheduler.handle_lock_queue() {
                Ok(()) => match scheduler.queue_snapshot() {
                    Ok((jobs, locked)) => IpcResponse::Queue {
                        jobs: jobs.into_iter().map(Into::into).collect(),
                        locked,
                    },
                    Err(error) => IpcResponse::Error(protocol_error(&error, "lock queue")),
                },
                Err(error) => IpcResponse::Error(protocol_error(&error, "lock queue")),
            },
            IpcRequest::UnlockQueue => match scheduler.handle_unlock_queue(&wake_tx) {
                Ok(()) => match scheduler.queue_snapshot() {
                    Ok((jobs, locked)) => IpcResponse::Queue {
                        jobs: jobs.into_iter().map(Into::into).collect(),
                        locked,
                    },
                    Err(error) => IpcResponse::Error(protocol_error(&error, "unlock queue")),
                },
                Err(error) => IpcResponse::Error(protocol_error(&error, "unlock queue")),
            },
            IpcRequest::MoveQueued { id, target_order } => {
                match scheduler.handle_move_queued(id, target_order) {
                    Ok(jobs) => IpcResponse::Queue {
                        jobs: jobs.into_iter().map(Into::into).collect(),
                        locked: true,
                    },
                    Err(error) => IpcResponse::Error(protocol_error(&error, "move queue")),
                }
            }
            IpcRequest::FollowLogs { .. } => unreachable!(),
        };
        if send_response(&mut framed, &response).await.is_err() {
            return;
        }
        if stop {
            let _ = shutdown_tx.send(true);
            return;
        }
    }
}
