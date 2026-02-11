mod command;
mod config;

use std::borrow::Cow::{self, Owned};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use bip39::Mnemonic;
use bitcoin::hashes::Hash;
use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::HistoryHinter;
use rustyline::{Completer, Editor, Helper, Hinter, Validator};
use spark_wallet::{DefaultSigner, Network, SparkWalletConfig};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::command::Command;
use crate::config::MempoolConfig;

#[derive(Clone, Debug, Parser)]
#[command(name = "spark-cli")]
#[command(about = "CLI for interacting with Spark wallets")]
struct Args {
    /// Network to use (mainnet, regtest)
    #[arg(long, default_value = "mainnet")]
    pub network: String,

    /// Path to the data directory
    #[arg(short, long, default_value = ".spark")]
    pub data_dir: PathBuf,
}

#[derive(Helper, Completer, Hinter, Validator)]
pub(crate) struct CliHelper {
    #[rustyline(Hinter)]
    pub(crate) hinter: HistoryHinter,
}

impl Highlighter for CliHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Parse network (only mainnet or regtest allowed)
    let network = match args.network.to_lowercase().as_str() {
        "mainnet" => Network::Mainnet,
        "regtest" => Network::Regtest,
        _ => {
            eprintln!(
                "Invalid network '{}'. Use 'mainnet' or 'regtest'.",
                args.network
            );
            std::process::exit(1);
        }
    };

    // Load .env if present (from current directory before changing)
    let _ = dotenvy::dotenv();

    // Prompt for mnemonic
    print!("Enter your BIP-39 mnemonic phrase: ");
    std::io::stdout().flush()?;
    let mut mnemonic_input = String::new();
    std::io::stdin().read_line(&mut mnemonic_input)?;
    let mnemonic: Mnemonic = mnemonic_input
        .trim()
        .parse()
        .map_err(|e| format!("Invalid mnemonic: {e}"))?;

    // Prompt for passphrase
    print!("Enter passphrase (or press Enter for none): ");
    std::io::stdout().flush()?;
    let mut passphrase = String::new();
    std::io::stdin().read_line(&mut passphrase)?;
    let passphrase = passphrase.trim().to_string();

    let seed = mnemonic.to_seed(&passphrase);

    // Create wallet directory: <data-dir>/<network>/<seed_hash>/
    let seed_hash = bitcoin::hashes::sha256::Hash::hash(&seed);
    let seed_hash_hex = hex::encode(&seed_hash[0..4]);
    let network_str = match network {
        Network::Mainnet => "mainnet",
        Network::Regtest => "regtest",
        _ => unreachable!(),
    };
    let wallet_dir = args.data_dir.join(network_str).join(&seed_hash_hex);
    std::fs::create_dir_all(&wallet_dir)?;
    std::env::set_current_dir(&wallet_dir)?;

    // Setup logging
    let log_filter = std::env::var("SPARK_LOG_FILTER")
        .unwrap_or_else(|_| "spark_wallet=info,spark=info,info".to_string());
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("spark.log")?;
    tracing_subscriber::registry()
        .with(EnvFilter::new(&log_filter))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(log_file),
        )
        .init();

    // Use default config for the network
    let spark_config = SparkWalletConfig::default_config(network);
    let mempool_config = MempoolConfig::from_env(network);

    // Connect to wallet
    let signer = DefaultSigner::new(&seed, network)?;
    let wallet =
        Arc::new(spark_wallet::SparkWallet::connect(spark_config, Arc::new(signer)).await?);

    // Spawn event listener
    let clone = Arc::clone(&wallet);
    tokio::spawn(async move {
        let mut receiver = clone.subscribe_events();
        loop {
            tokio::select! {
                Ok(event) = receiver.recv() => {
                    match event {
                        spark_wallet::WalletEvent::DepositConfirmed(tree_node_id) => info!("Deposit confirmed: {tree_node_id}"),
                        spark_wallet::WalletEvent::StreamConnected => info!("Connected to Spark server."),
                        spark_wallet::WalletEvent::StreamDisconnected => warn!("Disconnected from Spark server."),
                        spark_wallet::WalletEvent::Synced => info!("Synced"),
                        spark_wallet::WalletEvent::TransferClaimed(transfer) => info!("Transfer claimed: {}", transfer.id),
                        spark_wallet::WalletEvent::TransferClaimStarting(transfer) => info!("Transfer claim starting: {}", transfer.id),
                        spark_wallet::WalletEvent::Optimization(event) => info!("Optimization event: {:?}", event),
                    }
                }
                else => warn!("Event stream closed."),
            }
        }
    });

    // Setup readline
    let rl = &mut Editor::new()?;
    rl.set_helper(Some(CliHelper {
        hinter: HistoryHinter {},
    }));
    let _ = rl.load_history("history.txt");

    let cli_prompt = match network {
        Network::Mainnet => "spark-cli [mainnet]> ",
        Network::Regtest => "spark-cli [regtest]> ",
        _ => unreachable!(),
    };

    // REPL loop
    loop {
        let line_res = rl.readline(cli_prompt);
        match line_res {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }

                rl.add_history_entry(line.as_str())?;
                let mut vec = shellwords::split(&line)?;
                vec.insert(0, "".to_string());
                let command_res = Command::try_parse_from(vec);
                if let Err(e) = command_res {
                    eprintln!("Error: {e}");
                    continue;
                }
                if let Err(e) = command::handle_command(
                    rl,
                    network,
                    &mempool_config,
                    &wallet,
                    command_res.unwrap(),
                )
                .await
                {
                    eprintln!("Error: {e}");
                }
            }
            Err(ReadlineError::Interrupted) => break,
            Err(ReadlineError::Eof) => break,
            Err(_) => break,
        }
    }

    rl.save_history("history.txt").unwrap();
    Ok(())
}
