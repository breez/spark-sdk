//! A local cluster's starting state: the operators' databases, the daemon's
//! database and bitcoind's chain.
//!
//! The parts reference each other (the daemon's pool leaves are operator tree
//! nodes funded on that chain), so they are captured from one cluster and
//! restored together.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use testcontainers::core::wait::ExitWaitStrategy;
use testcontainers::core::{Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

use crate::fixtures::spark_so::{MIN_SIGNERS, NUM_OPERATORS};

pub const MOUNT_PATH: &str = "/snapshot";

const POSTGRES_IMAGE: &str = "postgres:11-alpine";

/// Bumped by hand when the daemon changes what it stores without changing its
/// schema.
const BOOTSTRAP_EPOCH: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub bootstrap_epoch: u32,
    pub operator_version: String,
    pub num_operators: usize,
    pub min_signers: usize,
    pub postgres_image: String,
    pub sspd_migrations: String,
    pub pool_leaves_per_denomination: u32,
    pub pool_max_denomination_power: u32,
    pub sspd_wallet_seed: String,
}

impl SnapshotManifest {
    pub fn expected() -> Result<Self> {
        Ok(Self {
            bootstrap_epoch: BOOTSTRAP_EPOCH,
            operator_version: pinned_operator_version()?,
            num_operators: NUM_OPERATORS,
            min_signers: MIN_SIGNERS,
            postgres_image: POSTGRES_IMAGE.to_string(),
            sspd_migrations: sspd_migrations_digest()?,
            pool_leaves_per_denomination: crate::fixtures::sspd::LEAVES_PER_DENOMINATION,
            pool_max_denomination_power: crate::fixtures::sspd::MAX_DENOMINATION_POWER,
            sspd_wallet_seed: crate::fixtures::setup::SSPD_WALLET_SEED_HEX.to_string(),
        })
    }
}

fn pinned_operator_version() -> Result<String> {
    let dockerfile = manifest_dir().join("docker/spark-so.dockerfile");
    let contents = std::fs::read_to_string(&dockerfile)
        .with_context(|| format!("failed to read {}", dockerfile.display()))?;
    contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("ARG VERSION="))
        .map(str::to_string)
        .with_context(|| format!("no `ARG VERSION=` in {}", dockerfile.display()))
}

fn sspd_migrations_digest() -> Result<String> {
    let dir = manifest_dir().join("../spark-service-provider/sspd/src/postgresql/migrations");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    names.sort();

    let mut hasher = Sha256::new();
    for path in names {
        hasher.update(path.file_name().unwrap_or_default().as_encoded_bytes());
        hasher.update(std::fs::read(&path)?);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

pub fn snapshot_dir() -> PathBuf {
    manifest_dir().join("state-snapshot")
}

fn manifest_path() -> PathBuf {
    snapshot_dir().join("manifest.json")
}

pub fn operator_dump(index: usize) -> PathBuf {
    snapshot_dir().join(format!("operator-{index}.dump"))
}

pub fn sspd_dump() -> PathBuf {
    snapshot_dir().join("sspd.dump")
}

pub fn bitcoind_datadir() -> PathBuf {
    snapshot_dir().join("bitcoind")
}

pub fn mount() -> Mount {
    Mount::bind_mount(snapshot_dir().display().to_string(), MOUNT_PATH)
}

/// Fails unless a snapshot for the current cluster is on disk and complete.
pub fn check() -> Result<()> {
    let manifest_path = manifest_path();
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| {
        anyhow::anyhow!(
            "no state snapshot at {}: {e}. Build one with `make capture-itest-state`.",
            manifest_path.display()
        )
    })?;
    let manifest: SnapshotManifest = serde_json::from_str(&manifest)
        .with_context(|| format!("{} is not a snapshot manifest", manifest_path.display()))?;
    let expected = SnapshotManifest::expected()?;
    if manifest != expected {
        bail!(
            "the state snapshot was captured against {manifest:?} but this cluster is \
             {expected:?}. Rebuild it with `make capture-itest-state`."
        );
    }

    let mut required: Vec<PathBuf> = (0..NUM_OPERATORS).map(operator_dump).collect();
    required.push(sspd_dump());
    required.push(bitcoind_datadir());
    for path in required {
        if !path.exists() {
            bail!(
                "the state snapshot manifest is current but {} is missing. Rebuild it with \
                 `make capture-itest-state`.",
                path.display()
            );
        }
    }
    Ok(())
}

/// Fails unless the daemon's dump holds a non-empty pool for
/// `identity_public_key` in which every leaf has its exit chain.
pub async fn verify(identity_public_key: &[u8]) -> Result<()> {
    use spark::tree::TreeStore;
    use testcontainers_modules::postgres::Postgres;

    let network = format!("bootstrap-verify-{}", std::process::id());
    let host = format!("bootstrap-verify-postgres-{}", std::process::id());
    let postgres = Postgres::default()
        .with_network(&network)
        .with_container_name(&host)
        .with_mount(mount())
        .start()
        .await
        .context("starting the database the bootstrap is checked in")?;
    restore_database(&network, &host, "sspd").await?;

    let port = postgres.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let store = spark_postgres::PostgresTreeStore::from_config(
        spark_postgres::PostgresStorageConfig::with_defaults(&url),
        identity_public_key,
    )
    .await
    .context("opening the restored tree store")?;

    let leaves = store
        .get_leaves()
        .await
        .context("reading the restored leaf pool")?;
    if leaves.available.is_empty() {
        bail!("the bootstrap restored an empty leaf pool");
    }

    let missing = store
        .leaves_missing_exit_chains()
        .await
        .context("reading the restored exit chains")?;
    if !missing.is_empty() {
        bail!(
            "{} of the bootstrap's leaves have no chain to exit along",
            missing.len()
        );
    }
    Ok(())
}

pub fn bootstrap_key() -> Result<String> {
    let manifest = serde_json::to_string(&SnapshotManifest::expected()?)?;
    let digest = Sha256::digest(manifest.as_bytes());
    Ok(format!("{digest:x}"))
}

pub fn discard() -> Result<()> {
    let dir = snapshot_dir();
    if !dir.exists() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))?
    {
        let path = entry?.path();
        // The directory's .gitignore is committed.
        if path.file_name().is_some_and(|n| n == ".gitignore") {
            continue;
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        }
        .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

pub fn is_current() -> bool {
    check().is_ok()
}

pub fn write_manifest() -> Result<()> {
    std::fs::create_dir_all(snapshot_dir())?;
    let path = manifest_path();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&SnapshotManifest::expected()?)?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

const DATABASE: &str = "postgres";
const DATABASE_USER: &str = "postgres";
const DATABASE_PASSWORD: &str = "postgres";

pub async fn capture_database(network: &str, host: &str, name: &str) -> Result<()> {
    std::fs::create_dir_all(snapshot_dir())?;
    run_client(
        network,
        &format!("snapshot-dump-{name}-{network}"),
        &[
            "pg_dump",
            "-Fc",
            "-h",
            host,
            "-U",
            DATABASE_USER,
            "-f",
            &format!("{MOUNT_PATH}/{name}.dump"),
            DATABASE,
        ],
    )
    .await
    .with_context(|| format!("dumping {name}"))
}

/// The dump carries the schema, so this has to run before anything migrates the
/// database: restoring over existing tables fails.
pub async fn restore_database(network: &str, host: &str, name: &str) -> Result<()> {
    run_client(
        network,
        // Named per network, since concurrent restores would otherwise collide on
        // the container name.
        &format!("snapshot-restore-{name}-{network}"),
        &[
            "pg_restore",
            "-h",
            host,
            "-U",
            DATABASE_USER,
            "-d",
            DATABASE,
            "-j",
            "4",
            &format!("{MOUNT_PATH}/{name}.dump"),
        ],
    )
    .await
    .with_context(|| format!("restoring {name}"))
}

/// A container of its own rather than an exec in the database's container:
/// testcontainers' exec future is not `Send`, so it cannot be awaited in a
/// spawned task.
async fn run_client(network: &str, container_name: &str, argv: &[&str]) -> Result<()> {
    GenericImage::new("postgres", "11-alpine")
        .with_wait_for(WaitFor::Exit(ExitWaitStrategy::new().with_exit_code(0)))
        .with_network(network)
        .with_container_name(container_name)
        .with_mount(mount())
        .with_env_var("PGPASSWORD", DATABASE_PASSWORD)
        .with_log_consumer(crate::fixtures::log::TracingConsumer::new(
            container_name.to_string(),
        ))
        .with_cmd(argv.iter().copied())
        .start()
        .await
        .with_context(|| format!("running {argv:?}"))?;
    Ok(())
}
