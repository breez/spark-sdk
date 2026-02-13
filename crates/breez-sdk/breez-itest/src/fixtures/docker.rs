//! Shared utilities for building Docker images in test fixtures.

use anyhow::Result;
use std::process::Command;
use tracing::info;

/// Configuration for building a Docker image
#[derive(Debug, Clone)]
pub struct DockerImageConfig {
    /// Path to the repository (local path or git URL)
    pub context_path: String,
    /// Path to the Dockerfile (relative to context_path)
    pub dockerfile_path: String,
    /// Name for the built image
    pub image_name: String,
    /// Tag for the built image
    pub image_tag: String,
}

/// Build a Docker image from the specified context path
pub async fn build_docker_image(config: &DockerImageConfig) -> Result<()> {
    use std::path::Path;

    let image_tag = format!("{}:{}", config.image_name, config.image_tag);

    // Check if the image already exists (avoids "already exists" errors with
    // Docker buildx when multiple tests try to build the same image in parallel)
    let check = Command::new("docker")
        .args(["image", "inspect", &image_tag])
        .output();
    if let Ok(output) = check {
        if output.status.success() {
            info!("Docker image {} already exists, skipping build", image_tag);
            return Ok(());
        }
    }

    // Resolve the context path to absolute (skip for URLs)
    let context_path = Path::new(&config.context_path);
    let absolute_context_path = if config.context_path.starts_with("http://")
        || config.context_path.starts_with("https://")
    {
        // For Git URLs, use as-is
        Path::new(&config.context_path).to_path_buf()
    } else if context_path.is_absolute() {
        context_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(context_path)
    };

    // Resolve dockerfile path (skip absolute path resolution for Git URLs)
    let dockerfile_path = if config.context_path.starts_with("http://")
        || config.context_path.starts_with("https://")
    {
        // For Git URLs, dockerfile_path should be relative to repo root
        config.dockerfile_path.clone()
    } else if Path::new(&config.dockerfile_path).is_absolute() {
        config.dockerfile_path.clone()
    } else {
        absolute_context_path
            .join(&config.dockerfile_path)
            .to_string_lossy()
            .to_string()
    };

    info!(
        "Building Docker image from: {} with Dockerfile: {}",
        absolute_context_path.display(),
        dockerfile_path
    );

    let mut cmd = Command::new("docker");
    cmd.args([
        "build",
        "-f",
        &dockerfile_path,
        "-t",
        &format!("{}:{}", config.image_name, config.image_tag),
        &absolute_context_path.to_string_lossy(),
    ]);

    let output = cmd.output().map_err(|e| {
        anyhow::anyhow!(
            "Failed to run docker build command: {}. Make sure Docker is installed and running.",
            e
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Handle race condition: another parallel test may have built the image
        // while we were building, causing a "already exists" error from buildx
        if stderr.contains("already exists") {
            info!(
                "Docker image {} was built by another process, continuing",
                image_tag
            );
            return Ok(());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow::anyhow!(
            "Docker build failed:\nSTDOUT: {}\nSTDERR: {}",
            stdout,
            stderr
        ));
    }

    info!(
        "Successfully built Docker image: {}:{}",
        config.image_name, config.image_tag
    );
    Ok(())
}
