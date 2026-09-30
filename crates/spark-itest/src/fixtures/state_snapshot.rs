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
use tracing::Instrument;

use crate::fixtures::spark_so::{MIN_SIGNERS, NUM_OPERATORS};

pub const MOUNT_PATH: &str = "/snapshot";

const POSTGRES_IMAGE: &str = "postgres:11-alpine";

/// Bumped by hand when a change the rest of the manifest does not show makes
/// earlier snapshots wrong: to what the daemon stores, to what the snapshot
/// holds, or to how it is captured.
const BOOTSTRAP_EPOCH: u32 = 3;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub bootstrap_epoch: u32,
    /// The operator image the snapshot was captured against, which covers the
    /// pinned commit, the operator's configuration and its entrypoint: all three
    /// decide what the captured databases hold.
    pub operator_image: String,
    pub num_operators: usize,
    pub min_signers: usize,
    pub postgres_image: String,
    /// The node that wrote the chain the snapshot ships.
    pub bitcoind_image: String,
    pub sspd_migrations: String,
    pub pool_leaves_per_denomination: u32,
    pub pool_max_denomination_power: u32,
    pub sspd_wallet_seed: String,
}

impl SnapshotManifest {
    pub fn expected() -> Result<Self> {
        Ok(Self {
            bootstrap_epoch: BOOTSTRAP_EPOCH,
            operator_image: crate::images::tag(crate::images::SPARK_SO)?,
            num_operators: NUM_OPERATORS,
            min_signers: MIN_SIGNERS,
            postgres_image: POSTGRES_IMAGE.to_string(),
            bitcoind_image: format!(
                "{}:{}",
                crate::fixtures::bitcoind::BITCOIND_DOCKER_IMAGE,
                crate::fixtures::bitcoind::BITCOIND_VERSION
            ),
            sspd_migrations: sspd_migrations_digest()?,
            pool_leaves_per_denomination: crate::fixtures::sspd::LEAVES_PER_DENOMINATION,
            pool_max_denomination_power: crate::fixtures::sspd::MAX_DENOMINATION_POWER,
            sspd_wallet_seed: crate::fixtures::setup::SSPD_WALLET_SEED_HEX.to_string(),
        })
    }
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
    // A manifest this harness cannot read is one an older harness wrote, which is
    // a snapshot to rebuild rather than a file to repair.
    let manifest: SnapshotManifest = serde_json::from_str(&manifest).map_err(|e| {
        anyhow::anyhow!(
            "{} was written by another version of this harness ({e}). Rebuild the \
             snapshot with `make capture-itest-state`.",
            manifest_path.display()
        )
    })?;
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

/// Fails unless the snapshot's database image holds the keyshares a cluster
/// signs with, and a non-empty pool for `identity_public_key` in which every
/// leaf has its exit chain.
pub async fn verify(identity_public_key: &[u8]) -> Result<()> {
    use spark::tree::TreeStore;

    let database = crate::fixtures::database::DatabaseFixture::start(
        &crate::fixtures::setup::FixtureId::new(),
        true,
    )
    .await
    .context("starting the database the bootstrap is checked in")?;
    verify_keyshares(&database).await?;

    let url = database.host_url(crate::fixtures::database::SSPD_DATABASE);
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

/// Every operator's database must hold keyshares for every coordinator: a
/// coordinator reserves the id it picks on all of them, and one that has fewer
/// than the operator's own DKG floor starts a refill instead of signing.
async fn verify_keyshares(database: &crate::fixtures::database::DatabaseFixture) -> Result<()> {
    use crate::fixtures::database::operator_database;
    use crate::fixtures::keyshares::{MIN_AVAILABLE_KEYS, available_per_coordinator};

    for operator in 0..NUM_OPERATORS {
        let url = database.host_url(&operator_database(operator));
        let counts = available_per_coordinator(&url).await?;
        for coordinator in 0..NUM_OPERATORS {
            let available = counts.get(&(coordinator as i64)).copied().unwrap_or(0);
            if available <= MIN_AVAILABLE_KEYS {
                bail!(
                    "operator {operator}'s database holds {available} keyshares for coordinator \
                     {coordinator}, need more than {MIN_AVAILABLE_KEYS}"
                );
            }
        }
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

pub fn write_manifest() -> Result<()> {
    std::fs::create_dir_all(snapshot_dir())?;
    let path = manifest_path();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&SnapshotManifest::expected()?)?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

const DATABASE_USER: &str = "postgres";
const DATABASE_PASSWORD: &str = "postgres";

/// Dumps `database` on `host` to the snapshot's `<name>.dump`.
pub async fn capture_database(network: &str, host: &str, database: &str, name: &str) -> Result<()> {
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
            database,
        ],
    )
    .await
    .with_context(|| format!("dumping {name}"))
}

/// What the snapshot holds, hashed when it is captured. The image tags are
/// derived from it, so a snapshot restored from a clone or a CI artifact keeps
/// the tags it was captured with: mtimes do not survive either.
fn digest_path() -> PathBuf {
    snapshot_dir().join("digest")
}

/// Hashes the dumps and the chain, for [`write_digest`] to record.
fn content_digest() -> Result<String> {
    let mut hasher = Sha256::new();
    let mut files: Vec<PathBuf> = (0..NUM_OPERATORS).map(operator_dump).collect();
    files.push(sspd_dump());
    collect_files(&bitcoind_datadir(), &mut files)?;
    files.sort();
    for file in files {
        hasher.update(file.to_string_lossy().as_bytes());
        hasher.update(std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?);
    }
    Ok(hex::encode(&hasher.finalize()[..8]))
}

/// Records what the capture wrote, beside the dumps it wrote.
pub fn write_digest() -> Result<()> {
    let digest = content_digest()?;
    std::fs::write(digest_path(), &digest)
        .with_context(|| format!("failed to write {}", digest_path().display()))
}

/// Recorded by the capture. A snapshot that predates the record is hashed once,
/// here, rather than sending every test process over its contents.
fn digest() -> Result<String> {
    if let Ok(digest) = std::fs::read_to_string(digest_path()) {
        return Ok(digest.trim().to_string());
    }
    write_digest()?;
    std::fs::read_to_string(digest_path())
        .map(|digest| digest.trim().to_string())
        .with_context(|| format!("failed to read {}", digest_path().display()))
}

const DATABASE_IMAGE: &str = "spark-itest-state";

/// The Postgres image that holds the snapshot's databases, built from the dumps
/// on first use. Restoring the dumps into every test's server takes seconds; a
/// container from this image starts with them in place.
pub async fn database_image() -> Result<(String, String)> {
    static IMAGE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let tag = IMAGE
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(build_database_image)
                .instrument(tracing::debug_span!("snapshot.database_image"))
                .await?
        })
        .await?;
    Ok((DATABASE_IMAGE.to_string(), tag.clone()))
}

fn database_dumps() -> Vec<(String, PathBuf)> {
    let operators = (0..NUM_OPERATORS).map(|index| {
        (
            crate::fixtures::database::operator_database(index),
            operator_dump(index),
        )
    });
    operators
        .chain([(
            crate::fixtures::database::SSPD_DATABASE.to_string(),
            sspd_dump(),
        )])
        .collect()
}

/// Restores into a data directory outside the base image's volume, so the data
/// stays in the image's layers.
fn database_dockerfile() -> String {
    let mut script = String::from(
        "set -eu; \\\n\
         mkdir -p \"$PGDATA\"; chown postgres:postgres \"$PGDATA\"; chmod 700 \"$PGDATA\"; \\\n\
         su-exec postgres initdb --username=postgres --auth=trust > /dev/null; \\\n\
         echo 'host all all all trust' >> \"$PGDATA/pg_hba.conf\"; \\\n\
         su-exec postgres pg_ctl -w -o '-c fsync=off' start > /dev/null; \\\n",
    );
    for (database, dump) in database_dumps() {
        let file = dump
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        script.push_str(&format!(
            "su-exec postgres createdb -U postgres {database}; \\\n\
             su-exec postgres pg_restore -U postgres -j 4 -d {database} {MOUNT_PATH}/{file}; \\\n"
        ));
    }
    script.push_str("su-exec postgres pg_ctl -w -m fast stop > /dev/null\n");
    format!(
        "FROM {POSTGRES_IMAGE}\n\
         ENV PGDATA=/var/lib/postgresql/snapshot\n\
         RUN --mount=type=bind,target={MOUNT_PATH} {script}"
    )
}

/// Changes with the manifest, the dumps and the recipe, so a worktree never picks
/// up another snapshot's image.
fn database_image_tag(dockerfile: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(std::fs::read(manifest_path())?);
    hasher.update(dockerfile.as_bytes());
    hasher.update(digest()?.as_bytes());
    Ok(hex::encode(&hasher.finalize()[..8]))
}

fn build_database_image() -> Result<String> {
    let dockerfile = database_dockerfile();
    let tag = database_image_tag(&dockerfile)?;
    build_snapshot_image(DATABASE_IMAGE, &tag, &dockerfile)?;
    Ok(tag)
}

const CHAIN_IMAGE: &str = "spark-itest-chain";

/// The bitcoind image that holds the snapshot's chain. Copying the data into
/// every test's container costs more than starting one that has it.
pub async fn chain_image() -> Result<(String, String)> {
    static IMAGE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let tag = IMAGE
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(build_chain_image)
                .instrument(tracing::debug_span!("snapshot.chain_image"))
                .await?
        })
        .await?;
    Ok((CHAIN_IMAGE.to_string(), tag.clone()))
}

/// The node's data stays out of the base image's volume, so it lives in the
/// image's own layers rather than being copied into a volume at every start.
pub fn chain_datadir() -> &'static str {
    "/snapshot/bitcoin"
}

fn chain_dockerfile() -> String {
    let datadir = chain_datadir();
    format!(
        "FROM {}:{}\n\
         COPY --chown=bitcoin:bitcoin bitcoind/ {datadir}/\n\
         ENV BITCOIN_DATA={datadir}\n",
        crate::fixtures::bitcoind::BITCOIND_DOCKER_IMAGE,
        crate::fixtures::bitcoind::BITCOIND_VERSION,
    )
}

fn build_chain_image() -> Result<String> {
    let dockerfile = chain_dockerfile();
    let mut hasher = Sha256::new();
    hasher.update(dockerfile.as_bytes());
    hasher.update(digest()?.as_bytes());
    let tag = hex::encode(&hasher.finalize()[..8]);
    build_snapshot_image(CHAIN_IMAGE, &tag, &dockerfile)?;
    Ok(tag)
}

fn collect_files(dir: &Path, into: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, into)?;
        } else {
            into.push(path);
        }
    }
    Ok(())
}

/// Builds `<image>:<tag>` from `dockerfile` with the snapshot as its context,
/// unless that tag is already there.
fn build_snapshot_image(image: &str, tag: &str, dockerfile: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let image = format!("{image}:{tag}");
    let present = Command::new("docker")
        .args(["image", "inspect", &image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running docker")?
        .success();
    if present {
        return Ok(());
    }

    tracing::info!("Building {image} from the state snapshot");
    let mut build = Command::new("docker")
        // The recipe mounts the snapshot rather than sending it as context, which
        // only BuildKit understands, and an engine can have it turned off.
        .env("DOCKER_BUILDKIT", "1")
        .args(["build", "--quiet", "-t", &image, "-f", "-"])
        .arg(snapshot_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("running docker build")?;
    build
        .stdin
        .take()
        .context("docker build's stdin")?
        .write_all(dockerfile.as_bytes())?;
    let status = build.wait()?;
    if !status.success() {
        bail!("building {image} failed with {status}");
    }
    Ok(())
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
