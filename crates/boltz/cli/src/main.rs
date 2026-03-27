use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use boltz::{
    AlchemyConfig, BoltzConfig, BoltzService, Chain, MemoryBoltzStore, PreparedSwap,
};

#[derive(Parser)]
#[command(name = "boltz-cli", about = "Test CLI for the Boltz LN -> USDT reverse swap flow")]
struct Cli {
    /// BIP-39 mnemonic (12 or 24 words). If not provided, generates a new one.
    #[arg(long, env = "BOLTZ_MNEMONIC")]
    mnemonic: Option<String>,

    /// Alchemy API key (required).
    #[arg(long, env = "ALCHEMY_API_KEY")]
    alchemy_api_key: String,

    /// Alchemy gas policy ID (required).
    #[arg(long, env = "ALCHEMY_GAS_POLICY_ID")]
    alchemy_gas_policy_id: String,

    /// Boltz referral ID.
    #[arg(long, env = "BOLTZ_REFERRAL_ID", default_value = "breez_sdk")]
    referral_id: String,

    /// Boltz API URL (without /v2).
    #[arg(long, env = "BOLTZ_API_URL", default_value = "https://api.boltz.exchange")]
    api_url: String,

    /// Arbitrum RPC URL for read-only operations.
    #[arg(long, env = "ARBITRUM_RPC_URL", default_value = "https://arb1.arbitrum.io/rpc")]
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
        /// USDT amount (6 decimals, e.g. 1000000 = 1 USDT).
        usdt_amount: u64,
        /// Destination EVM address on Arbitrum.
        destination: String,
    },

    /// Full swap flow: prepare -> create -> wait for payment -> complete.
    Swap {
        /// USDT amount (6 decimals, e.g. 1000000 = 1 USDT).
        usdt_amount: u64,
        /// Destination EVM address on Arbitrum.
        destination: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Derive seed from mnemonic
    let mnemonic_str = if let Some(m) = &cli.mnemonic {
        m.clone()
    } else {
        let entropy: [u8; 16] = rand_entropy();
        let m = bip39::Mnemonic::from_entropy(&entropy)
            .context("Failed to generate mnemonic")?;
        let words = m.to_string();
        println!("Generated new mnemonic (save this!):\n  {words}\n");
        words
    };

    let mnemonic = bip39::Mnemonic::parse_normalized(&mnemonic_str)
        .context("Invalid mnemonic")?;
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
        } => {
            let svc = init_service(config, &seed).await?;
            cmd_prepare(&svc, &destination, usdt_amount).await?;
        }
        Command::Swap {
            usdt_amount,
            destination,
        } => {
            let svc = init_service(config, &seed).await?;
            cmd_swap(&svc, &destination, usdt_amount).await?;
        }
    }

    Ok(())
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
    let chain_id = u32::try_from(boltz::ARBITRUM_CHAIN_ID)
        .context("Chain ID overflow")?;
    let gas = km.derive_gas_signer(chain_id)?;
    let preimage_key = km.derive_preimage_key(chain_id, 0)?;

    println!("EVM Key Info (Arbitrum, chain_id={}):", boltz::ARBITRUM_CHAIN_ID);
    println!("  Gas signer address:     {}", gas.address_hex());
    println!("  Preimage key[0] pubkey: {}", hex::encode(&preimage_key.public_key));
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

async fn cmd_prepare(svc: &BoltzService, destination: &str, usdt_amount: u64) -> Result<()> {
    let prepared = svc
        .prepare_reverse_swap(destination, Chain::Arbitrum, usdt_amount)
        .await?;
    print_prepared(&prepared);
    Ok(())
}

async fn cmd_swap(svc: &BoltzService, destination: &str, usdt_amount: u64) -> Result<()> {
    // Step 1: Prepare
    println!("Fetching quote...\n");
    let prepared = svc
        .prepare_reverse_swap(destination, Chain::Arbitrum, usdt_amount)
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
    println!("  USDT amount: {} (6 decimals)", completed.usdt_delivered);
    println!("  Destination: {}", completed.destination_address);
    println!("  Chain:       {:?}", completed.destination_chain);

    Ok(())
}

fn print_prepared(p: &PreparedSwap) {
    println!("Quote:");
    println!("  Destination:      {}", p.destination_address);
    println!("  Chain:            {:?}", p.destination_chain);
    println!("  USDT requested:   {} (6 decimals)", p.usdt_amount);
    println!("  Invoice amount:   {} sats", p.invoice_amount_sats);
    println!("  Boltz fee:        {} sats", p.boltz_fee_sats);
    println!("  Onchain (tBTC):   {} sats", p.estimated_onchain_amount);
    println!("  Est. USDT output: {} (6 decimals)", p.estimated_usdt_output);
    println!("  Slippage:         {} bps", p.slippage_bps);
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
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
