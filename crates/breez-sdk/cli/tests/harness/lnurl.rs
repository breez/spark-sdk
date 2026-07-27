//! LNURL server fixture driven through the docker CLI, mirroring
//! `breez-itest/src/fixtures/lnurl.rs` (same image tag and env vars, so a
//! locally built image is shared with the breez itests). Not testcontainers:
//! that would pull its dependency tree into the workspace test job for what
//! is a handful of docker commands.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const IMAGE: &str = "breez-lnurl-built:latest";
const POSTGRES_IMAGE: &str = "postgres:16-alpine";
const HTTP_PORT: u16 = 8080;
const POSTGRES_PORT: u16 = 5432;
const START_TIMEOUT: Duration = Duration::from_mins(2);

pub struct LnurlFixture {
    container_id: String,
    /// Backing database. Postgres is the server's only supported backend.
    postgres_container_id: String,
    pub http_url: String,
}

impl LnurlFixture {
    pub async fn start() -> Result<Self> {
        build_image_if_missing().await?;

        // Construct before starting anything, so Drop removes whichever
        // containers came up even if a later step fails.
        let mut fixture = Self {
            container_id: String::new(),
            postgres_container_id: String::new(),
            http_url: String::new(),
        };

        fixture.postgres_container_id = docker(&[
            "run",
            "-d",
            // All interfaces rather than 127.0.0.1: the lnurl server reaches
            // this from inside its own container through the host gateway,
            // which on Linux is the bridge address and not loopback.
            "-p",
            "0.0.0.0:0:5432",
            "-e",
            "POSTGRES_PASSWORD=postgres",
            POSTGRES_IMAGE,
        ])
        .await?
        .trim()
        .to_string();

        wait_for_postgres(&fixture.postgres_container_id).await?;
        let db_port = published_port(&fixture.postgres_container_id, POSTGRES_PORT).await?;
        let db_url = format!(
            "BREEZ_LNURL_DB_URL=postgres://postgres:postgres@host.docker.internal:{db_port}/postgres"
        );

        fixture.container_id = docker(&[
            "run",
            "-d",
            "-p",
            "127.0.0.1:0:8080",
            "--add-host",
            "host.docker.internal:host-gateway",
            "-e",
            "BREEZ_LNURL_NETWORK=regtest",
            "-e",
            "BREEZ_LNURL_AUTO_MIGRATE=true",
            "-e",
            &db_url,
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
        .await?
        .trim()
        .to_string();

        let port = published_port(&fixture.container_id, HTTP_PORT).await?;
        wait_for_http(port, &fixture.container_id).await?;

        fixture.http_url = format!("http://127.0.0.1:{port}");
        Ok(fixture)
    }
}

/// Wait until the server answers HTTP on its published port.
///
/// Readiness cannot be taken from the logs: the server's startup line is
/// printed before it migrates the database and binds, and it logs nothing
/// afterwards. Nor is reaching the port enough, since docker's proxy accepts
/// connections there from the moment the container starts and closes them
/// again while nothing is listening inside. Only a complete response shows the
/// server is serving.
async fn wait_for_http(port: u16, container_id: &str) -> Result<()> {
    let deadline = start_deadline();
    loop {
        if http_responds(port).await.is_ok() {
            return Ok(());
        }
        ensure_running(container_id, "lnurl server").await?;
        if tokio::time::Instant::now() >= deadline {
            let logs = docker(&["logs", container_id]).await.unwrap_or_default();
            bail!("lnurl server did not serve HTTP within {START_TIMEOUT:?}:\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Any status answers the question, so this only checks that a response came
/// back. HTTP/1.0 so the server closes the connection when it is done.
async fn http_responds(port: u16) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    if !response.starts_with(b"HTTP/") {
        bail!("no HTTP response on port {port}");
    }
    Ok(())
}

async fn wait_for_postgres(container_id: &str) -> Result<()> {
    let deadline = start_deadline();
    loop {
        // Probe TCP explicitly: while initialising, the image's entrypoint
        // runs a temporary socket-only server that the default `pg_isready`
        // check would accept as ready.
        let ready = docker(&[
            "exec",
            container_id,
            "pg_isready",
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
        ])
        .await
        .is_ok();
        if ready {
            return Ok(());
        }
        ensure_running(container_id, "postgres").await?;
        if tokio::time::Instant::now() >= deadline {
            bail!("postgres did not accept connections within {START_TIMEOUT:?}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Fail with the container's logs once it has exited. A container that dies on
/// startup otherwise surfaces only as a readiness timeout, or as an unrelated
/// `docker` error on the next command, neither of which names the real cause.
async fn ensure_running(container_id: &str, name: &str) -> Result<()> {
    let state = docker(&["inspect", "-f", "{{.State.Running}}", container_id]).await?;
    if state.trim() == "true" {
        return Ok(());
    }
    let logs = docker(&["logs", container_id]).await.unwrap_or_default();
    bail!("{name} container exited before it was ready:\n{logs}");
}

fn start_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now()
        .checked_add(START_TIMEOUT)
        .expect("deadline overflow")
}

/// Host port that `container_port` was published to.
async fn published_port(container_id: &str, container_port: u16) -> Result<u16> {
    let port_line = docker(&["port", container_id, &format!("{container_port}/tcp")]).await?;
    port_line
        .lines()
        .next()
        .and_then(|l| l.trim().rsplit(':').next())
        .context("unexpected `docker port` output")?
        .parse()
        .context("`docker port` returned a non-numeric port")
}

impl Drop for LnurlFixture {
    fn drop(&mut self) {
        for container_id in [&self.container_id, &self.postgres_container_id] {
            if container_id.is_empty() {
                continue;
            }
            std::process::Command::new("docker")
                .args(["rm", "-f", container_id])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
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
