use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, str::FromStr};

use anyhow::{Context, Result, bail};
use bip39::{Language, Mnemonic};
use clap::{Parser, Subcommand};

use boltz::{AlchemyConfig, BoltzConfig, BoltzService, MemoryBoltzStore, PreparedSwap};

#[derive(Clone, clap::ValueEnum)]
enum Chain {
    Arbitrum,
    Ethereum,
    Base,
    Optimism,
    Polygon,
}

impl From<Chain> for boltz::Chain {
    fn from(c: Chain) -> Self {
        match c {
            Chain::Arbitrum => Self::Arbitrum,
            Chain::Ethereum => Self::Ethereum,
            Chain::Base => Self::Base,
            Chain::Optimism => Self::Optimism,
            Chain::Polygon => Self::Polygon,
        }
    }
}

const PHRASE_FILE_NAME: &str = "phrase";

#[derive(Parser)]
#[command(
    name = "boltz-cli",
    about = "Test CLI for the Boltz LN -> USDT reverse swap flow"
)]
struct Cli {
    /// BIP-39 mnemonic (12 or 24 words). If not provided, reads from data-dir or generates new.
    #[arg(long, env = "BOLTZ_MNEMONIC")]
    mnemonic: Option<String>,

    /// Data directory for persisting mnemonic and state.
    #[arg(long, env = "BOLTZ_DATA_DIR", default_value = "./.data-boltz")]
    data_dir: PathBuf,

    /// Alchemy API key.
    #[arg(long, env = "ALCHEMY_API_KEY", default_value = "R-iU8US4vKEe2GH6VlCTg")]
    alchemy_api_key: String,

    /// Alchemy gas policy ID.
    #[arg(long, env = "ALCHEMY_GAS_POLICY_ID", default_value = "dcf46730-a11c-4869-a38b-35bcd73fe73f")]
    alchemy_gas_policy_id: String,

    /// Boltz referral ID.
    #[arg(long, env = "BOLTZ_REFERRAL_ID", default_value = "boltz_webapp_desktop")]
    referral_id: String,

    /// Boltz API URL (without /v2).
    #[arg(
        long,
        env = "BOLTZ_API_URL",
        default_value = "https://api.boltz.exchange"
    )]
    api_url: String,

    /// Arbitrum RPC URL for read-only operations.
    #[arg(
        long,
        env = "ARBITRUM_RPC_URL",
        default_value = "https://arb1.arbitrum.io/rpc"
    )]
    arbitrum_rpc_url: String,

    /// Slippage tolerance in basis points (100 = 1%).
    #[arg(long, default_value = "100")]
    slippage_bps: u32,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show derived EVM addresses (gas signer, first preimage key).
    Info,

    /// Get current swap limits (min/max sats).
    Limits,

    /// Get a quote for a LN -> USDT swap (no commitment).
    Prepare {
        /// USDT amount (e.g. 1.5 for 1.50 USDT).
        #[arg(value_parser = parse_usdt_amount)]
        usdt_amount: u64,
        /// Destination EVM address.
        destination: String,
        /// Destination chain.
        #[arg(value_enum)]
        chain: Chain,
    },

    /// Full swap flow: prepare -> create -> wait for payment -> complete.
    Swap {
        /// USDT amount (e.g. 1.5 for 1.50 USDT).
        #[arg(value_parser = parse_usdt_amount)]
        usdt_amount: u64,
        /// Destination EVM address.
        destination: String,
        /// Destination chain.
        #[arg(value_enum)]
        chain: Chain,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Ensure data directory exists
    fs::create_dir_all(&cli.data_dir)
        .with_context(|| format!("Failed to create data dir: {}", cli.data_dir.display()))?;

    // Resolve mnemonic: CLI arg > phrase file > generate new
    let mnemonic = if let Some(m) = &cli.mnemonic {
        Mnemonic::from_str(m).context("Invalid mnemonic")?
    } else {
        get_or_create_mnemonic(&cli.data_dir)?
    };
    let seed = mnemonic.to_seed("");

    // Build config
    let config = BoltzConfig {
        api_url: cli.api_url,
        alchemy_config: AlchemyConfig {
            api_key: cli.alchemy_api_key,
            gas_policy_id: cli.alchemy_gas_policy_id,
        },
        arbitrum_rpc_url: cli.arbitrum_rpc_url,
        chain_id: boltz::ARBITRUM_CHAIN_ID,
        referral_id: cli.referral_id,
        slippage_bps: cli.slippage_bps,
    };

    match cli.command {
        Command::Info => cmd_info(&seed)?,
        Command::Limits => {
            let svc = init_service(config, &seed).await?;
            cmd_limits(&svc).await?;
        }
        Command::Prepare {
            usdt_amount,
            destination,
            chain,
        } => {
            let svc = init_service(config, &seed).await?;
            cmd_prepare(&svc, &destination, chain.into(), usdt_amount).await?;
        }
        Command::Swap {
            usdt_amount,
            destination,
            chain,
        } => {
            let svc = init_service(config, &seed).await?;
            cmd_swap(&svc, &destination, chain.into(), usdt_amount).await?;
        }
    }

    Ok(())
}

fn get_or_create_mnemonic(data_dir: &Path) -> Result<Mnemonic> {
    let filename = data_dir.join(PHRASE_FILE_NAME);

    match fs::read_to_string(&filename) {
        Ok(phrase) => {
            let mnemonic = Mnemonic::from_str(phrase.trim())?;
            println!("Loaded mnemonic from {}\n", filename.display());
            Ok(mnemonic)
        }
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                bail!("Can't read from file: {}, err {e}", filename.display());
            }
            let mnemonic = Mnemonic::from_entropy_in(Language::English, &rand_entropy())?;
            fs::write(&filename, mnemonic.to_string())?;
            println!(
                "Generated new mnemonic (saved to {}):\n  {mnemonic}\n",
                filename.display()
            );
            Ok(mnemonic)
        }
    }
}

async fn init_service(config: BoltzConfig, seed: &[u8]) -> Result<BoltzService> {
    let store = Arc::new(MemoryBoltzStore::new());
    let svc = BoltzService::new(config, seed, store)
        .await
        .context("Failed to initialize BoltzService")?;
    Ok(svc)
}

fn cmd_info(seed: &[u8]) -> Result<()> {
    let km = boltz::EvmKeyManager::from_seed(seed)?;
    let chain_id = u32::try_from(boltz::ARBITRUM_CHAIN_ID).context("Chain ID overflow")?;
    let gas = km.derive_gas_signer(chain_id)?;
    let preimage_key = km.derive_preimage_key(chain_id, 0)?;

    println!(
        "EVM Key Info (Arbitrum, chain_id={}):",
        boltz::ARBITRUM_CHAIN_ID
    );
    println!("  Gas signer address:     {}", gas.address_hex());
    println!(
        "  Preimage key[0] pubkey: {}",
        hex::encode(&preimage_key.public_key)
    );
    println!("  Preimage key[0] addr:   {}", preimage_key.address_hex());
    Ok(())
}

async fn cmd_limits(svc: &BoltzService) -> Result<()> {
    let limits = svc.get_limits().await?;
    println!("Swap limits:");
    println!("  Min: {} sats", limits.min_sats);
    println!("  Max: {} sats", limits.max_sats);
    Ok(())
}

async fn cmd_prepare(svc: &BoltzService, destination: &str, chain: boltz::Chain, usdt_amount: u64) -> Result<()> {
    let prepared = svc
        .prepare_reverse_swap(destination, chain, usdt_amount)
        .await?;
    print_prepared(&prepared);
    Ok(())
}

async fn cmd_swap(svc: &BoltzService, destination: &str, chain: boltz::Chain, usdt_amount: u64) -> Result<()> {
    // Step 1: Prepare
    println!("Fetching quote...\n");
    let prepared = svc
        .prepare_reverse_swap(destination, chain, usdt_amount)
        .await?;
    print_prepared(&prepared);

    // Confirm
    if !confirm("\nProceed with swap?")? {
        println!("Cancelled.");
        return Ok(());
    }

    // Step 2: Create
    println!("\nCreating swap on Boltz...");
    let created = svc.create_reverse_swap(&prepared).await?;
    println!("\nSwap created!");
    println!("  Swap ID:       {}", created.swap_id);
    println!("  Boltz ID:      {}", created.boltz_id);
    println!("  Invoice:       {}", created.invoice);
    println!("  Amount:        {} sats", created.invoice_amount_sats);
    println!("  Timeout block: {}", created.timeout_block_height);
    println!("\n>>> PAY THIS INVOICE to continue <<<\n");

    // Step 3: Complete (blocks until done)
    println!("Monitoring swap... (waiting for lockup + claim)");
    let completed = svc.complete_reverse_swap(&created.swap_id).await?;
    println!("\nSwap completed!");
    println!("  Claim tx:    {}", completed.claim_tx_hash);
    println!("  USDT amount: {} USDT", format_usdt(completed.usdt_delivered));
    println!("  Destination: {}", completed.destination_address);
    println!("  Chain:       {:?}", completed.destination_chain);

    Ok(())
}

fn format_usdt(raw: u64) -> String {
    let whole = raw / 1_000_000;
    let frac = raw % 1_000_000;
    format!("{whole}.{frac:06}")
}

fn print_prepared(p: &PreparedSwap) {
    println!("Quote:");
    println!("  Destination:      {}", p.destination_address);
    println!("  Chain:            {:?}", p.destination_chain);
    println!("  USDT requested:   {} USDT", format_usdt(p.usdt_amount));
    println!("  Invoice amount:   {} sats", p.invoice_amount_sats);
    println!("  Boltz fee:        {} sats", p.boltz_fee_sats);
    println!("  Onchain (tBTC):   {} sats", p.estimated_onchain_amount);
    println!(
        "  Est. USDT output: {} USDT",
        format_usdt(p.estimated_usdt_output)
    );
    println!("  Slippage:         {} bps", p.slippage_bps);
}

/// Generate 16 bytes of entropy using system time (good enough for test CLI).
fn rand_entropy() -> [u8; 16] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut hasher = DefaultHasher::new();
    nanos.hash(&mut hasher);
    let h1 = hasher.finish();

    (nanos.wrapping_add(1)).hash(&mut hasher);
    let h2 = hasher.finish();

    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h1.to_le_bytes());
    out[8..].copy_from_slice(&h2.to_le_bytes());
    out
}

/// Parse a human-readable USDT amount (e.g. "1.5") into 6-decimal raw units (1500000).
fn parse_usdt_amount(s: &str) -> std::result::Result<u64, String> {
    const DECIMALS: u32 = 6;
    let parts: Vec<&str> = s.split('.').collect();
    match parts.len() {
        1 => {
            let whole: u64 = parts[0].parse().map_err(|e| format!("{e}"))?;
            whole
                .checked_mul(10u64.pow(DECIMALS))
                .ok_or_else(|| "amount too large".to_string())
        }
        2 => {
            let whole: u64 = parts[0].parse().map_err(|e| format!("{e}"))?;
            let frac_str = parts[1];
            if frac_str.len() > DECIMALS as usize {
                return Err(format!("too many decimal places (max {DECIMALS})"));
            }
            let padded = format!("{frac_str:0<width$}", width = DECIMALS as usize);
            let frac: u64 = padded.parse().map_err(|e| format!("{e}"))?;
            whole
                .checked_mul(10u64.pow(DECIMALS))
                .and_then(|w| w.checked_add(frac))
                .ok_or_else(|| "amount too large".to_string())
        }
        _ => Err("invalid amount format".to_string()),
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}
