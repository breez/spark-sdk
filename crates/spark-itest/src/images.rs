//! The images a cluster's containers run, each tagged by what it is built from.
//!
//! A worktree that pins another operator, or builds another daemon, gets another
//! tag, so worktrees neither overwrite each other's images nor run one another's.
//! A build is skipped when its tag is already present, and CI caches them by tag.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SPARK_SO: &str = "spark-so";
pub const MIGRATIONS: &str = "spark-migrations";
pub const LDK_SERVER: &str = "ldk-server";
pub const SSPD: &str = "sspd";

pub const ALL: [&str; 4] = [SPARK_SO, MIGRATIONS, LDK_SERVER, SSPD];

/// The image a fixture runs, under the tag it is built with.
pub fn image(name: &str) -> Result<testcontainers::GenericImage> {
    Ok(testcontainers::GenericImage::new(name, tag(name)?.as_str()))
}

/// `<name>:<tag>`, as `cargo xtask` builds and CI caches the image.
pub fn reference(image: &str) -> Result<String> {
    Ok(format!("{image}:{}", tag(image)?))
}

/// Hashing an image's inputs walks the workspace and shells out to cargo, and
/// nothing they read changes while the process runs.
pub fn tag(image: &str) -> Result<String> {
    static TAGS: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
        std::sync::Mutex::new(None);

    let mut tags = TAGS.lock().expect("the image tags");
    let tags = tags.get_or_insert_with(std::collections::HashMap::new);
    if let Some(tag) = tags.get(image) {
        return Ok(tag.clone());
    }
    let tag = hash_inputs(image)?;
    tags.insert(image.to_string(), tag.clone());
    Ok(tag)
}

fn hash_inputs(image: &str) -> Result<String> {
    let docker = manifest_dir().join("docker");
    let mut inputs = match image {
        SPARK_SO => vec![
            docker.join("spark-so.dockerfile"),
            docker.join("entrypoint.sh"),
            docker.join("so.config.yaml"),
        ],
        MIGRATIONS => vec![docker.join("migrations.dockerfile")],
        LDK_SERVER => vec![docker.join("ldk-server.dockerfile")],
        // Built from the crates its binaries are built from, so a change to a
        // crate it does not use leaves the daemon's image alone.
        SSPD => {
            let root = workspace_root();
            // Everything the build reads from the repository root: the
            // dockerfile copies the whole tree, and rustup picks the toolchain
            // the pin names rather than the base image's.
            let mut inputs = vec![
                docker.join("sspd.dockerfile"),
                root.join("Cargo.toml"),
                root.join("Cargo.lock"),
                root.join("rust-toolchain.toml"),
                root.join(".cargo/config.toml"),
                root.join(".dockerignore"),
            ];
            for package in sspd_packages()? {
                collect_sources(&package, &mut inputs)?;
            }
            inputs
        }
        other => anyhow::bail!("no such itest image: {other}"),
    };
    inputs.sort();

    let mut hasher = Sha256::new();
    for path in inputs {
        let contents = std::fs::read(&path)
            .with_context(|| format!("reading {} to tag {image}", path.display()))?;
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(contents);
    }
    Ok(hex::encode(&hasher.finalize()[..6]))
}

/// The directories of the workspace crates the daemon's binaries are built
/// from, itself included.
fn sspd_packages() -> Result<Vec<PathBuf>> {
    let metadata = std::process::Command::new(std::env::var("CARGO").as_deref().unwrap_or("cargo"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .context("running cargo metadata to tag the daemon's image")?;
    anyhow::ensure!(
        metadata.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    let packages = metadata["packages"]
        .as_array()
        .context("cargo metadata without packages")?;

    let mut wanted: Vec<String> = vec!["sspd".to_string(), "ssp-cli".to_string()];
    let mut directories = Vec::new();
    let mut visited = std::collections::HashSet::new();
    while let Some(name) = wanted.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(package) = packages
            .iter()
            .find(|package| package["name"].as_str() == Some(name.as_str()))
        else {
            continue;
        };
        let manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .context("a package without a manifest path")?,
        );
        directories.push(
            manifest
                .parent()
                .context("a manifest without a directory")?
                .to_path_buf(),
        );
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            // A dependency with a path is another crate of this workspace.
            if dependency["path"].is_string()
                && let Some(name) = dependency["name"].as_str()
            {
                wanted.push(name.to_string());
            }
        }
    }
    Ok(directories)
}

/// Every file under `dir`, minus what no image is built from: prose, build
/// output, and the state snapshot's hundreds of megabytes.
fn collect_sources(dir: &Path, into: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name == "target"
            || name == "state-snapshot"
            || name.starts_with('.')
            || name.ends_with(".md")
        {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, into)?;
        } else {
            into.push(path);
        }
    }
    Ok(())
}

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn workspace_root() -> PathBuf {
    manifest_dir().join("../..")
}
