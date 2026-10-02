pub mod bitcoind;
pub mod database;
pub mod keyshares;
pub mod ldk_server;
pub mod log;
pub mod setup;
pub mod spark_so;
pub mod sspd;
pub mod state_snapshot;
pub mod wait_log;

use anyhow::{Context, Result};
use std::ops::Deref;
use std::sync::{Arc, Condvar, OnceLock};
use std::time::Duration;

use std::future::Future;
use testcontainers::core::ContainerPort;
use testcontainers::{ContainerAsync, Image};

/// The tail of a container's stdout and stderr, which say why it stopped.
pub async fn last_output<I: Image>(container: &ContainerAsync<I>) -> String {
    let mut output = container.stdout_to_vec().await.unwrap_or_default();
    output.extend(container.stderr_to_vec().await.unwrap_or_default());
    let output = String::from_utf8_lossy(&output);
    let lines: Vec<&str> = output.lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

/// A container removed after the test that used it has returned. Dropping a
/// testcontainers container instead blocks the dropping thread until docker has
/// removed it, one container at a time for the whole process.
pub struct Container<I: Image + 'static>(Option<ContainerAsync<I>>);

impl<I: Image + 'static> Container<I> {
    pub fn new(container: ContainerAsync<I>) -> Self {
        Self(Some(container))
    }
}

impl<I: Image + 'static> Drop for Container<I> {
    fn drop(&mut self) {
        let Some(container) = self.0.take() else {
            return;
        };
        remover().remove(async move {
            if let Err(e) = container.rm().await {
                tracing::warn!("removing a container: {e}");
            }
        });
    }
}

impl<I: Image + 'static> Deref for Container<I> {
    type Target = ContainerAsync<I>;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("a container is taken only by its drop")
    }
}

/// Removes containers off the tests' clock, and holds the process open until it
/// has, so a run leaves no cluster behind.
struct Remover {
    runtime: tokio::runtime::Runtime,
    /// How many removals are still running, and a signal for the last one. The
    /// exit handler waits on this rather than on the runtime: entering tokio from
    /// an atexit handler touches thread locals that may already be gone, and a
    /// panic there cannot unwind, so it aborts the process.
    outstanding: Arc<(std::sync::Mutex<usize>, Condvar)>,
}

/// Long enough for docker to remove a run's worth of containers. A wedged docker
/// costs the run this much and leaves its containers behind.
const REMOVAL_TIMEOUT: Duration = Duration::from_secs(120);

static REMOVER: OnceLock<Remover> = OnceLock::new();

fn remover() -> &'static Remover {
    REMOVER.get_or_init(|| {
        // SAFETY: the handler only waits on this process's own remover.
        unsafe { libc::atexit(wait_at_exit) };
        Remover {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("itest-removals")
                .build()
                .expect("a runtime to remove containers on"),
            outstanding: Arc::new((std::sync::Mutex::new(0), Condvar::new())),
        }
    })
}

extern "C" fn wait_at_exit() {
    if let Some(remover) = REMOVER.get() {
        remover.wait();
    }
}

impl Remover {
    fn remove(&self, removal: impl Future<Output = ()> + Send + 'static) {
        let (count, _) = &*self.outstanding;
        *count.lock().expect("the removal count") += 1;
        let outstanding = Arc::clone(&self.outstanding);
        self.runtime.spawn(async move {
            removal.await;
            let (count, done) = &*outstanding;
            *count.lock().expect("the removal count") -= 1;
            done.notify_all();
        });
    }

    fn wait(&self) {
        let (count, done) = &*self.outstanding;
        let mut outstanding = count.lock().expect("the removal count");
        while *outstanding > 0 {
            let (guard, timeout) = done
                .wait_timeout(outstanding, REMOVAL_TIMEOUT)
                .expect("the removal count");
            outstanding = guard;
            if timeout.timed_out() {
                eprintln!("itest: {} container removals did not finish", *outstanding);
                return;
            }
        }
    }
}

/// Retries for up to 30 seconds: a running container can briefly inspect as having
/// no host port binding, and an exited one never gets one.
pub async fn published_port<I: Image>(container: &ContainerAsync<I>, port: u16) -> Result<u16> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match container.get_host_port_ipv4(ContainerPort::Tcp(port)).await {
            Ok(host_port) => return Ok(host_port),
            Err(e) if std::time::Instant::now() < deadline => {
                tracing::debug!("no host binding for {port} yet ({e}), retrying");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("docker never published a host port for {port}: the container is most likely not running")
                });
            }
        }
    }
}
