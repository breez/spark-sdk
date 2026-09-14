#![cfg_attr(
    not(test),
    warn(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::string_slice,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::unwrap_used
    )
)]

use std::path::PathBuf;

use bitcoin::Network;
use clap::{CommandFactory, FromArgMatches, Parser, parser::ValueSource};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
#[cfg(not(unix))]
use tokio::signal;
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize, Parser)]
#[command(version, about = "Spark Service Provider Daemon", long_about = None)]
struct Args {
    #[arg(long, default_value = "sspd.conf")]
    pub config: PathBuf,

    /// Bitcoin network. Valid values are bitcoin, testnet, signet, regtest.
    #[arg(long, default_value = "bitcoin")]
    #[serde_as(as = "DisplayFromStr")]
    pub network: Network,

    /// Loglevel to use. Can be used to filter logs through the env filter
    /// format.
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (args, config_file) = load_args()?;

    tracing_subscriber::registry()
        .with(EnvFilter::new(&args.log_level))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .init();

    if let Some(config_file) = &config_file {
        info!("starting sspd with config file: {}", config_file.display());
    } else {
        info!("starting sspd without config file");
    }

    info!("sspd started");

    let signal = wait_for_shutdown_signal().await?;
    info!("shutdown complete: {signal}");
    Ok(())
}

/// Each source overrides the one before: defaults, the config file, `SSPD_*`
/// environment variables, flags given on the command line.
fn load_args() -> Result<(Args, Option<PathBuf>), Box<dyn std::error::Error>> {
    let command = Args::command();
    let flags: std::collections::HashSet<String> = command
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect();
    let matches = command.get_matches();
    let cli = Args::from_arg_matches(&matches)?;
    let given =
        |id: &str| flags.contains(id) && matches.value_source(id) == Some(ValueSource::CommandLine);

    let config_file = std::fs::canonicalize(&cli.config).ok();
    if config_file.is_none() && given("config") {
        return Err(format!("config file {} cannot be read", cli.config.display()).into());
    }

    let mut explicit = serde_json::to_value(&cli)?;
    if let Some(fields) = explicit.as_object_mut() {
        fields.retain(|id, _| given(id));
    }

    let mut figment = Figment::new().merge(Serialized::defaults(&cli));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Toml::file(config_file));
    }
    let args = figment
        .merge(Env::prefixed("SSPD_"))
        .merge(Serialized::defaults(explicit))
        .extract()?;
    Ok((args, config_file))
}

/// Docker, systemd and Kubernetes stop a process with SIGTERM by default, and
/// SIGTERM's default action ends the process without a graceful shutdown.
#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<&'static str> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(tokio::select! {
        _ = terminate.recv() => "SIGTERM",
        _ = interrupt.recv() => "SIGINT",
    })
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> std::io::Result<&'static str> {
    signal::ctrl_c().await?;
    Ok("ctrl-c")
}
