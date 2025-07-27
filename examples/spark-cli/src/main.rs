mod command;
mod config;

use std::borrow::Cow::{self, Owned};
use std::fs::{OpenOptions, canonicalize};
use std::path::PathBuf;
use std::sync::Arc;

use bitcoin::hashes::Hash;
use clap::Parser;
use figment::{
    Figment,
    providers::{Env, Format, Yaml},
};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::HistoryHinter;
use rustyline::{Completer, Editor, Helper, Hinter, Validator};
use spark_wallet::{DefaultSigner, Network};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::command::Command;
use crate::config::{Config, DEFAULT_CONFIG};

const HISTORY_FILE_NAME: &str = "history.txt";

#[derive(Clone, Debug, Parser)]
struct Args {
    /// Config path, relative to the working directory.
    #[arg(long, default_value = "spark.conf")]
    pub config: PathBuf,

    /// Working directory
    #[arg(long, default_value = ".spark")]
    pub working_directory: PathBuf,
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
    std::fs::create_dir_all(&args.working_directory)?;
    std::env::set_current_dir(&args.working_directory)?;

    let config_file = canonicalize(&args.config).ok();
    let mut figment = Figment::new().merge(Yaml::string(DEFAULT_CONFIG));
    if let Some(config_file) = &config_file {
        figment = figment.merge(Yaml::file(config_file));
    } else {
        std::fs::write(&args.config, DEFAULT_CONFIG)?;
    }

    let _ = dotenvy::dotenv();
    let config: Config = figment.merge(Env::prefixed("SPARK_")).extract()?;
    let seed = config.mnemonic.to_seed(config.passphrase.clone());

    let log_path = match config.log_path.to_str() {
        Some("spark.log") => {
            let seed_hash = bitcoin::hashes::sha256::Hash::hash(&seed);
            let log_file_name = format!("spark.{}.log", hex::encode(&seed_hash[0..4]));
            PathBuf::from(log_file_name)
        }
        _ => config.log_path.clone(),
    };
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path.clone())?;
    tracing_subscriber::registry()
        .with(EnvFilter::new(&config.log_filter))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(log_file),
        )
        .init();

    let network = config.spark_config.network;
    let signer = DefaultSigner::new(&seed, network)?;
    let wallet =
        Arc::new(spark_wallet::SparkWallet::connect(config.spark_config.clone(), signer).await?);
    let clone = Arc::clone(&wallet);
    tokio::spawn(async move {
        let mut receiver = clone.subscribe_events();
        loop {
            tokio::select! {
                Ok(event) = receiver.recv() => {
                    match event {
                        spark_wallet::WalletEvent::DepositConfirmed(tree_node_id) => println!("Deposit confirmed: {tree_node_id}"),
                        spark_wallet::WalletEvent::StreamConnected => println!("Connected to Spark server."),
                        spark_wallet::WalletEvent::StreamDisconnected => println!("Disconnected from Spark server."),
                        spark_wallet::WalletEvent::Synced => println!("Synced"),
                        spark_wallet::WalletEvent::TransferClaimed(transfer_id) => println!("Transfer claimed: {transfer_id}"),
                    }
                }
                else => eprintln!("Event stream closed."),
            }
        }
    });
    let rl = &mut Editor::new()?;
    rl.set_helper(Some(CliHelper {
        hinter: HistoryHinter {},
    }));
    let _ = rl.load_history(HISTORY_FILE_NAME);

    let cli_prompt = match network {
        Network::Mainnet => "spark-cli [mainnet]> ",
        Network::Testnet => "spark-cli [testnet]> ",
        Network::Regtest => "spark-cli [regtest]> ",
        Network::Signet => "spark-cli [signet]> ",
    };

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
                if command_res.is_err() {
                    eprintln!("{}", command_res.unwrap_err());
                    continue;
                }
                if let Err(e) =
                    command::handle_command(rl, &config, &wallet, command_res.unwrap()).await
                {
                    eprintln!("Error: {e}");
                }
            }
            Err(ReadlineError::Interrupted) => break,
            Err(ReadlineError::Eof) => break,
            Err(_) => break,
        }
    }

    rl.save_history(HISTORY_FILE_NAME).unwrap();
    Ok(())
}
