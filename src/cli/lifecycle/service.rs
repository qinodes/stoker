//! Scheduler process lifecycle and cleanup.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::Context;

use crate::process::spawn_detached;
use crate::service::Service;
use crate::{ServiceClient, StokerPaths, is_service_unavailable};

use super::super::{print_success, print_warning, request_confirmation, runtime};

pub(crate) fn terminate_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceProbe {
    Ready,
    Unavailable,
}

trait StartupGateway {
    fn probe(&mut self) -> anyhow::Result<ServiceProbe>;
    fn child_exit(&mut self) -> anyhow::Result<Option<String>>;
    fn timed_out(&self) -> bool;
    fn pause(&mut self, duration: Duration);
    fn cleanup(&mut self);
}

struct SystemStartupGateway {
    paths: StokerPaths,
    child: Child,
    started: std::time::Instant,
    timeout: Duration,
}

impl StartupGateway for SystemStartupGateway {
    fn probe(&mut self) -> anyhow::Result<ServiceProbe> {
        match runtime()?.block_on(ServiceClient::new(self.paths.clone()).status()) {
            Ok(_) => Ok(ServiceProbe::Ready),
            Err(error) if is_service_unavailable(&error) => Ok(ServiceProbe::Unavailable),
            Err(error) => Err(error),
        }
    }

    fn child_exit(&mut self) -> anyhow::Result<Option<String>> {
        self.child
            .try_wait()
            .context("check scheduler service")
            .map(|status| status.map(|status| status.to_string()))
    }

    fn timed_out(&self) -> bool {
        self.started.elapsed() >= self.timeout
    }

    fn pause(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn cleanup(&mut self) {
        terminate_child(&mut self.child);
    }
}

fn wait_for_startup<G: StartupGateway>(gateway: &mut G) -> anyhow::Result<()> {
    loop {
        match gateway.probe() {
            Ok(ServiceProbe::Ready) => return Ok(()),
            Ok(ServiceProbe::Unavailable) => {}
            Err(error) => {
                gateway.cleanup();
                return Err(error);
            }
        }

        match gateway.child_exit() {
            Ok(Some(status)) => {
                gateway.cleanup();
                anyhow::bail!("scheduler service exited during startup ({status})");
            }
            Ok(None) => {}
            Err(error) => {
                gateway.cleanup();
                return Err(error);
            }
        }

        if gateway.timed_out() {
            gateway.cleanup();
            anyhow::bail!("timed out waiting for scheduler service to start");
        }
        gateway.pause(Duration::from_millis(50));
    }
}

pub(crate) fn start(paths: &StokerPaths) -> anyhow::Result<()> {
    // An active endpoint is authoritative for the user-facing already-running
    // notice; stale endpoints are cleaned by the child after it acquires the lock.
    match runtime()?.block_on(ServiceClient::new(paths.clone()).status()) {
        Ok(_) => {
            print_warning("Scheduler service is already running.");
            return Ok(());
        }
        Err(error) if is_service_unavailable(&error) => {}
        Err(error) => return Err(error),
    }

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.service_log())
        .context("open scheduler service log")?;
    let log_err = log.try_clone().context("duplicate scheduler service log")?;
    let executable = std::env::current_exe().context("locate stoker executable")?;
    let mut command = Command::new(executable);
    command
        .arg("service-run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    let child = spawn_detached(&mut command).context("start scheduler service")?;

    let mut gateway = SystemStartupGateway {
        paths: paths.clone(),
        child,
        started: std::time::Instant::now(),
        timeout: Duration::from_secs(5),
    };
    wait_for_startup(&mut gateway)?;
    print_success("Scheduler started.");
    Ok(())
}

pub(crate) fn service_run(paths: &StokerPaths) -> anyhow::Result<()> {
    runtime()?.block_on(Service::new(paths.clone())?.run())
}

pub(crate) fn stop(paths: &StokerPaths, yes: bool) -> anyhow::Result<()> {
    let client = ServiceClient::new(paths.clone());
    match runtime()?.block_on(client.status()) {
        Ok(status) => {
            if let Some(id) = status.active_job
                && !yes
                && !request_confirmation(&format!(
                    "Job {id} is active. Force-cancel it and stop the scheduler"
                ))?
            {
                print_warning("Stop cancelled.");
                return Ok(());
            }
            runtime()?.block_on(client.stop())?;
            print_success("Scheduler stopped.");
            Ok(())
        }
        Err(error) if is_service_unavailable(&error) => {
            print_warning("Scheduler is not running.");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeStartupGateway {
        probes: VecDeque<anyhow::Result<ServiceProbe>>,
        child_exits: VecDeque<anyhow::Result<Option<String>>>,
        timed_out: bool,
        pauses: Vec<Duration>,
        cleanup_calls: usize,
    }

    impl FakeStartupGateway {
        fn waiting() -> Self {
            Self {
                probes: VecDeque::from([Ok(ServiceProbe::Unavailable)]),
                child_exits: VecDeque::from([Ok(None)]),
                timed_out: false,
                pauses: Vec::new(),
                cleanup_calls: 0,
            }
        }
    }

    impl StartupGateway for FakeStartupGateway {
        fn probe(&mut self) -> anyhow::Result<ServiceProbe> {
            self.probes.pop_front().unwrap_or(Ok(ServiceProbe::Ready))
        }

        fn child_exit(&mut self) -> anyhow::Result<Option<String>> {
            self.child_exits.pop_front().unwrap_or(Ok(None))
        }

        fn timed_out(&self) -> bool {
            self.timed_out
        }

        fn pause(&mut self, duration: Duration) {
            self.pauses.push(duration);
        }

        fn cleanup(&mut self) {
            self.cleanup_calls += 1;
        }
    }

    #[test]
    fn startup_retry_is_injected_and_does_not_really_sleep() {
        let mut gateway = FakeStartupGateway::waiting();
        gateway.probes.push_back(Ok(ServiceProbe::Ready));

        wait_for_startup(&mut gateway).unwrap();

        assert_eq!(gateway.pauses, vec![Duration::from_millis(50)]);
        assert_eq!(gateway.cleanup_calls, 0);
    }

    #[test]
    fn startup_timeout_cleans_up_the_child() {
        let mut gateway = FakeStartupGateway::waiting();
        gateway.timed_out = true;

        let error = wait_for_startup(&mut gateway).unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert_eq!(gateway.cleanup_calls, 1);
        assert!(gateway.pauses.is_empty());
    }

    #[test]
    fn startup_probe_error_cleans_up_the_child() {
        let mut gateway = FakeStartupGateway::waiting();
        gateway.probes = VecDeque::from([Err(anyhow::anyhow!("probe failed"))]);

        let error = wait_for_startup(&mut gateway).unwrap_err();

        assert_eq!(error.to_string(), "probe failed");
        assert_eq!(gateway.cleanup_calls, 1);
    }

    #[test]
    fn early_child_exit_is_reported_and_cleaned_up() {
        let mut gateway = FakeStartupGateway::waiting();
        gateway.child_exits = VecDeque::from([Ok(Some("exit code: 7".to_owned()))]);

        let error = wait_for_startup(&mut gateway).unwrap_err();

        assert!(error.to_string().contains("exit code: 7"));
        assert_eq!(gateway.cleanup_calls, 1);
    }

    #[test]
    fn child_status_error_cleans_up_the_child() {
        let mut gateway = FakeStartupGateway::waiting();
        gateway.child_exits = VecDeque::from([Err(anyhow::anyhow!("wait failed"))]);

        let error = wait_for_startup(&mut gateway).unwrap_err();

        assert_eq!(error.to_string(), "wait failed");
        assert_eq!(gateway.cleanup_calls, 1);
    }
}
