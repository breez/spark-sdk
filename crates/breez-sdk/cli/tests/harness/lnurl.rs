//! LNURL server fixture driven through the docker CLI, mirroring
//! `breez-itest/src/fixtures/lnurl.rs` (same image tag and env vars, so a
//! locally built image is shared with the breez itests). Not testcontainers:
//! that would pull its dependency tree into the workspace test job for what
//! is three docker commands.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const IMAGE: &str = "breez-lnurl-built:latest";
const HTTP_PORT: u16 = 8080;
const START_TIMEOUT: Duration = Duration::from_mins(2);

pub struct LnurlFixture {
    container_id: String,
    pub http_url: String,
}

impl LnurlFixture {
    pub async fn start() -> Result<Self> {
        build_image_if_missing().await?;

        let output = docker(&[
            "run",
            "-d",
            "--rm",
            "-p",
            "127.0.0.1:0:8080",
            "--add-host",
            "host.docker.internal:host-gateway",
            "-e",
            "BREEZ_LNURL_NETWORK=regtest",
            "-e",
            "BREEZ_LNURL_AUTO_MIGRATE=true",
            "-e",
            "BREEZ_LNURL_DB_URL=:memory:",
            "-e",
            "BREEZ_LNURL_LOG_LEVEL=lnurl=trace,info",
            "-e",
            "BREEZ_LNURL_DOMAINS=",
            "-e",
            "BREEZ_LNURL_SCHEME=http",
            "-e",
            "BREEZ_LNURL_NSEC=nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl",
            "-e",
            "BREEZ_LNURL_MIN_SENDABLE=1000",
            "-e",
            "BREEZ_LNURL_MAX_SENDABLE=1000000000",
            "-e",
            "BREEZ_LNURL_DEV_DONT_USE_LNURL_INCLUDE_SPARK_ADDRESS=false",
            IMAGE,
        ])
        .await?;
        // Construct before the readiness wait so the container is removed on
        // failure too.
        let mut fixture = Self {
            container_id: output.trim().to_string(),
            http_url: String::new(),
        };

        wait_for_log(&fixture.container_id, "starting lnurl server").await?;

        let port_line =
            docker(&["port", &fixture.container_id, &format!("{HTTP_PORT}/tcp")]).await?;
        let port = port_line
            .lines()
            .next()
            .and_then(|l| l.trim().rsplit(':').next())
            .context("unexpected `docker port` output")?;
        fixture.http_url = format!("http://127.0.0.1:{port}");
        Ok(fixture)
    }
}

async fn wait_for_log(container_id: &str, needle: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now()
        .checked_add(START_TIMEOUT)
        .expect("deadline overflow");
    loop {
        let logs = docker(&["logs", container_id]).await?;
        if logs.contains(needle) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("lnurl server did not log '{needle}' within {START_TIMEOUT:?}:\n{logs}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

impl Drop for LnurlFixture {
    fn drop(&mut self) {
        std::process::Command::new("docker")
            .args(["rm", "-f", &self.container_id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok();
    }
}

/// Build the lnurl server image from the workspace unless it already exists
/// (parallel runs and the breez itests share the tag). To force a rebuild:
/// `docker image rm breez-lnurl-built:latest`.
async fn build_image_if_missing() -> Result<()> {
    if docker(&["image", "inspect", IMAGE]).await.is_ok() {
        return Ok(());
    }
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let dockerfile = workspace_root.join("crates/breez-sdk/lnurl/Dockerfile");
    eprintln!("building {IMAGE} (first run only, this can take several minutes)");
    docker(&[
        "build",
        "-f",
        &dockerfile.to_string_lossy(),
        "-t",
        IMAGE,
        "--build-arg",
        "CARGO_FEATURES=dev",
        &workspace_root.to_string_lossy(),
    ])
    .await
    .context("failed to build the lnurl server image")?;
    Ok(())
}

/// Run a docker command, returning stdout on success.
async fn docker(args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("docker")
        .args(args)
        .output()
        .await
        .context("failed to run docker (is it installed and running?)")?;
    if !output.status.success() {
        bail!(
            "docker {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
