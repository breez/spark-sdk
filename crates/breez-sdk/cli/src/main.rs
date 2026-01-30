mod command;
mod persist;

use crate::command::CliHelper;
use crate::persist::CliPersistence;
use anyhow::{Result, anyhow};
use breez_sdk_spark::{
    EventListener, Network, SdkBuilder, SdkEvent, Seed, StableBalanceConfig,
    create_postgres_storage, default_config, default_postgres_storage_config,
};
use clap::Parser;
use command::{Command, execute_command};
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::hint::HistoryHinter;
use std::{fs, path::PathBuf};
use tracing::{error, info};

#[derive(Parser)]
#[command(version, about = "CLI client for Breez SDK with Spark", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Path to the data directory
    #[arg(short, long, default_value = "./.data")]
    data_dir: String,

    /// Network to use (mainnet, regtest)
    #[arg(long, default_value = "regtest")]
    network: String,

    /// Account number to use for the Spark signer
    #[arg(long)]
    account_number: Option<u32>,

    /// `PostgreSQL` connection string (enables `PostgreSQL` storage instead of `SQLite`)
    #[arg(long)]
    postgres_connection_string: Option<String>,

    /// Stable balance token identifer
    #[arg(long)]
    stable_balance_token_identifier: Option<String>,

    /// Stable balance threshold, in sats
    #[arg(long)]
    stable_balance_threshold: Option<u64>,
}

fn expand_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        dirs::home_dir()
            .expect("Could not find home directory")
            .join(stripped)
    } else {
        PathBuf::from(path)
    }
}

/// Parse a command string into a Commands enum using clap
fn parse_command(input: &str) -> Result<Command> {
    // Handle exit command specially since it's not exposed in non-interactive mode
    if input.trim() == "exit" || input.trim() == "quit" {
        return Ok(Command::Exit);
    }

    // Create args for clap by adding program name at the beginning
    let mut args = vec!["breez-cli".to_string()];
    match shlex::split(input) {
        Some(split_args) => args.extend(split_args),
        None => return Err(anyhow!("Failed to parse input string: {}", input)),
    }

    // Use clap to parse the command
    match Command::try_parse_from(args) {
        Ok(cmd) => Ok(cmd),
        Err(e) => Err(anyhow!("Command parsing error: {}", e)),
    }
}

struct CliEventListener {}

#[async_trait::async_trait]
impl EventListener for CliEventListener {
    async fn on_event(&self, event: SdkEvent) {
        info!(
            "Event: {}",
            serde_json::to_string(&event)
                .unwrap_or_else(|_| "Failed to serialize event".to_string())
        );
    }
}

async fn run_interactive_mode(
    data_dir: PathBuf,
    network: Network,
    account_number: Option<u32>,
    postgres_connection_string: Option<String>,
    stable_balance_config: Option<StableBalanceConfig>,
) -> Result<()> {
    breez_sdk_spark::init_logging(Some(data_dir.to_string_lossy().into()), None, None)?;
    let persistence = CliPersistence {
        data_dir: data_dir.clone(),
    };
    let history_file = &persistence.history_file();

    let rl = &mut Editor::new()?;
    rl.set_helper(Some(CliHelper {
        hinter: HistoryHinter {},
    }));

    if rl.load_history(history_file).is_err() {
        info!("No history found");
    }

    let mnemonic = persistence.get_or_create_mnemonic()?;
    fs::create_dir_all(&data_dir)?;

    let breez_api_key = std::env::var_os("BREEZ_API_KEY")
        .map(|var| var.into_string().expect("Expected valid API key string"));
    let mut config = default_config(network);
    config.api_key = breez_api_key;
    config.stable_balance_config = stable_balance_config;

    let seed = Seed::Mnemonic {
        mnemonic: mnemonic.to_string(),
        passphrase: None,
    };

    let mut sdk_builder = SdkBuilder::new(config, seed);
    if let Some(connection_string) = postgres_connection_string {
        let postgres_config = default_postgres_storage_config(connection_string);
        let storage = create_postgres_storage(postgres_config).await?;
        sdk_builder = sdk_builder.with_storage(storage);
    } else {
        sdk_builder = sdk_builder.with_default_storage(data_dir.to_string_lossy().to_string());
    }
    if let Some(account_number) = account_number {
        sdk_builder = sdk_builder.with_key_set(breez_sdk_spark::KeySetConfig {
            key_set_type: breez_sdk_spark::KeySetType::Default,
            use_address_index: false,
            account_number: Some(account_number),
        });
    }

    let sdk = sdk_builder.build().await?;

    let listener = Box::new(CliEventListener {});
    sdk.add_event_listener(listener).await;

    let token_issuer = sdk.get_token_issuer();

    println!("Breez SDK CLI Interactive Mode");
    println!("Type 'help' for available commands or 'exit' to quit");

    let cli_prompt = match network {
        Network::Mainnet => "breez-spark-cli [mainnet]> ",
        Network::Regtest => "breez-spark-cli [regtest]> ",
    };

    loop {
        let readline = rl.readline(cli_prompt);
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                rl.add_history_entry(line.as_str())?;

                match parse_command(trimmed) {
                    Ok(command) => {
                        match Box::pin(execute_command(rl, command, &sdk, &token_issuer)).await {
                            Ok(continue_loop) => {
                                if !continue_loop {
                                    break;
                                }
                            }
                            Err(e) => {
                                println!("Error: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        println!("{e}");
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {err:?}");
                break;
            }
        }
    }

    if let Err(e) = sdk.disconnect().await {
        error!("Failed to gracefully stop SDK: {:?}", e);
    }

    rl.save_history(history_file)?;

    println!("Goodbye!");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();
    let data_dir = expand_path(&cli.data_dir);
    fs::create_dir_all(&data_dir)?;

    let network = match cli.network.to_lowercase().as_str() {
        "regtest" => Network::Regtest,
        "mainnet" => Network::Mainnet,
        _ => return Err(anyhow!("Invalid network. Use 'regtest' or 'mainnet'")),
    };
    let stable_balance_config =
        cli.stable_balance_token_identifier
            .map(|token_identifier| StableBalanceConfig {
                token_identifier,
                threshold_sats: cli.stable_balance_threshold,
                max_slippage_bps: None,
                reserved_sats: None,
            });

    Box::pin(run_interactive_mode(
        data_dir,
        network,
        cli.account_number,
        cli.postgres_connection_string,
        stable_balance_config,
    ))
    .await?;

    Ok(())
}
