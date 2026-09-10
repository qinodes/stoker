//! Windows named-pipe listener.

use std::sync::Arc;

use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::{mpsc, watch};

use crate::scheduler::Scheduler;

use super::super::Service;
use super::super::dispatch::handle_client;

pub(super) async fn run(
    service: Service,
    scheduler: Arc<Scheduler>,
    wake_tx: watch::Sender<u64>,
    wake_rx: watch::Receiver<u64>,
) -> anyhow::Result<()> {
    let name = service.paths.ipc_endpoint();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let (scheduler_error_tx, mut scheduler_error_rx) = mpsc::unbounded_channel();
    let scheduler_for_task = scheduler.clone();
    let scheduler_shutdown_rx = shutdown_rx.clone();
    let scheduler_task = tokio::spawn(async move {
        let result = scheduler_for_task.run(wake_rx, scheduler_shutdown_rx).await;
        if let Err(error) = &result {
            let _ = scheduler_error_tx.send(error.to_string());
        }
        result
    });
    let mut handlers = Vec::new();
    let result = loop {
        let server = match ServerOptions::new().create(&name) {
            Ok(server) => server,
            Err(error) => {
                break Err(anyhow::Error::new(error).context(format!("create IPC endpoint {name}")));
            }
        };
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    break Ok(());
                }
            },
            scheduler_error = scheduler_error_rx.recv() => {
                let error = match scheduler_error {
                    Some(error) => error,
                    None if *shutdown_rx.borrow() => break Ok(()),
                    None => "scheduler task ended without a result (possible panic)".to_owned(),
                };
                let _ = shutdown_tx.send(true);
                break Err(anyhow::anyhow!("scheduler task failed: {error}"));
            },
            connected = server.connect() => {
                match connected {
                    Ok(()) => {
                        handlers.push(tokio::spawn(handle_client(
                            server,
                            Arc::clone(&scheduler),
                            wake_tx.clone(),
                            shutdown_tx.clone(),
                            shutdown_rx.clone(),
                        )));
                    }
                    Err(error) => break Err(anyhow::Error::new(error).context("accept scheduler IPC client")),
                }
            }
        }
    };
    let _ = shutdown_tx.send(true);
    for handler in handlers {
        let _ = handler.await;
    }
    let scheduler_result = scheduler_task
        .await
        .map_err(|error| anyhow::anyhow!("scheduler task failed: {error}"))?;
    result.and(scheduler_result)
}
