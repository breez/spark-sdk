//! A cluster's docker network, on a 10.x subnet.
//!
//! The operator serves the service provider's API only to 10.x addresses (its
//! production network) and loopback, while docker's default pools are 172.x and
//! 192.168.x. So a cluster creates its network itself rather than leaving that to
//! testcontainers, which joins an existing network and leaves it in place.

use std::process::Command;

use anyhow::{Context, Result, bail};
use rand::Rng;

use crate::fixtures::setup::FixtureId;

const LABEL: &str = "spark-itest";
const ATTEMPTS: usize = 20;

/// Removes the network when dropped. Drop it after the containers on it.
pub struct ClusterNetwork {
    name: String,
}

impl ClusterNetwork {
    pub fn create(fixture_id: &FixtureId) -> Result<Self> {
        prune_abandoned();
        let name = fixture_id.to_network();
        let mut rng = rand::thread_rng();
        // Concurrent clusters pick their subnets independently, so a pick can
        // collide with one that is taken.
        for _ in 0..ATTEMPTS {
            let subnet = format!(
                "10.{}.{}.0/24",
                rng.gen_range(100..200),
                rng.gen_range(0..=255)
            );
            let output = Command::new("docker")
                .args(["network", "create", "--subnet", &subnet])
                .args(["--label", LABEL, &name])
                .output()
                .context("running docker to create the cluster's network")?;
            if output.status.success() {
                return Ok(Self { name });
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("overlap") {
                bail!("creating docker network {name} on {subnet}: {stderr}");
            }
        }
        bail!("found no free 10.x subnet for docker network {name} in {ATTEMPTS} attempts")
    }
}

impl Drop for ClusterNetwork {
    fn drop(&mut self) {
        // Fails while a container is still attached, and prune_abandoned removes
        // the network later instead.
        let _ = Command::new("docker")
            .args(["network", "rm", &self.name])
            .output();
    }
}

/// Removes the networks a run that was killed left behind. The age filter spares
/// a network another cluster has only just created, before its first container
/// joins it.
fn prune_abandoned() {
    let _ = Command::new("docker")
        .args(["network", "prune", "--force"])
        .args([
            "--filter",
            &format!("label={LABEL}"),
            "--filter",
            "until=1h",
        ])
        .output();
}
