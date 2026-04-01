use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, str::FromStr};

use anyhow::{Context, Result, bail};
use bip39::{Language, Mnemonic};
use clap::{Parser, Subcommand};

use boltz::{
    AlchemyConfig, BoltzConfig, BoltzError, BoltzEventListener, BoltzService, BoltzStorage,
    BoltzSwapEvent,
};

#[derive(Clone, clap::ValueEnum)]
enum Chain {
    Arbitrum,
    Berachain,
    Conflux,
    Corn,
    Ethereum,
    Flare,
    Hedera,
    HyperEvm,
    Ink,
    Mantle,
    MegaEth,
    Monad,
    Morph,
    Optimism,
    Plasma,
    Polygon,
    Rootstock,
    Sei,
    Stable,
    Unichain,
    XLayer,
}

impl From<Chain> for boltz::Chain {
    fn from(c: Chain) -> Self {
        match c {
            Chain::Arbitrum => Self::Arbitrum,
            Chain::Berachain => Self::Berachain,
            Chain::Conflux => Self::Conflux,
            Chain::Corn => Self::Corn,
            Chain::Ethereum => Self::Ethereum,
            Chain::Flare => Self::Flare,
            Chain::Hedera => Self::Hedera,
            Chain::HyperEvm => Self::HyperEvm,
            Chain::Ink => Self::Ink,
            Chain::Mantle => Self::Mantle,
            Chain::MegaEth => Self::MegaEth,
            Chain::Monad => Self::Monad,
            Chain::Morph => Self::Morph,
            Chain::Optimism => Self::Optimism,
            Chain::Plasma => Self::Plasma,
            Chain::Polygon => Self::Polygon,
            Chain::Rootstock => Self::Rootstock,
            Chain::Sei => Self::Sei,
            Chain::Stable => Self::Stable,
            Chain::Unichain => Self::Unichain,
            Chain::XLayer => Self::XLayer,
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
    #[arg(
        long,
        env = "ALCHEMY_GAS_POLICY_ID",
        default_value = "dcf46730-a11c-4869-a38b-35bcd73fe73f"
    )]
    alchemy_gas_policy_id: String,

    /// Boltz referral ID.
    #[arg(long, env = "BOLTZ_REFERRAL_ID", default_value = "breez-sdk")]
    referral_id: String,

    /// Slippage tolerance in basis points (100 = 1%). Defaults to 100.
    #[arg(long)]
    slippage_bps: Option<u32>,

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
        /// USDT amount (e.g. 1.5 for 1.50 USDT). Mutually exclusive with --sats.
        #[arg(long, value_parser = parse_usdt_amount, conflicts_with = "sats")]
        usdt: Option<u64>,
        /// Input amount in sats. Mutually exclusive with --usdt.
        #[arg(long, conflicts_with = "usdt")]
        sats: Option<u64>,
        /// Destination EVM address.
        destination: String,
        /// Destination chain.
        #[arg(value_enum)]
        chain: Chain,
    },

    /// Full swap flow: prepare -> create -> wait for payment -> complete.
    Swap {
        /// USDT amount (e.g. 1.5 for 1.50 USDT). Mutually exclusive with --sats.
        #[arg(long, value_parser = parse_usdt_amount, conflicts_with = "sats")]
        usdt: Option<u64>,
        /// Input amount in sats. Mutually exclusive with --usdt.
        #[arg(long, conflicts_with = "usdt")]
        sats: Option<u64>,
        /// Destination EVM address.
        destination: String,
        /// Destination chain.
        #[arg(value_enum)]
        chain: Chain,
    },

    /// Recover unclaimed swaps by scanning the blockchain (from mnemonic alone).
    Recover {
        /// Destination EVM address for recovered USDT.
        destination: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Ensure data directory exists
    fs::create_dir_all(&cli.data_dir)
        .with_context(|| format!("Failed to create data dir: {}", cli.data_dir.display()))?;

    init_logging(&cli.data_dir)?;

    // Resolve mnemonic: CLI arg > phrase file > generate new
    let mnemonic = if let Some(m) = &cli.mnemonic {
        Mnemonic::from_str(m).context("Invalid mnemonic")?
    } else {
        get_or_create_mnemonic(&cli.data_dir)?
    };
    let seed = mnemonic.to_seed("");

    let mut config = BoltzConfig::mainnet(
        AlchemyConfig {
            api_key: cli.alchemy_api_key,
            gas_policy_id: cli.alchemy_gas_policy_id,
        },
        cli.referral_id,
    );
    if let Some(slippage_bps) = cli.slippage_bps {
        config.slippage_bps = slippage_bps;
    }

    match cli.command {
        Command::Info => cmd_info(&seed)?,
        Command::Limits => {
            let svc = init_service(config, &seed, &cli.data_dir).await?;
            cmd_limits(&svc).await?;
        }
        Command::Prepare {
            usdt,
            sats,
            destination,
            chain,
        } => {
            let svc = init_service(config, &seed, &cli.data_dir).await?;
            let prepared = prepare(&svc, &destination, chain.into(), usdt, sats).await?;
            print_json(&prepared);
        }
        Command::Swap {
            usdt,
            sats,
            destination,
            chain,
        } => {
            let svc = init_service(config, &seed, &cli.data_dir).await?;
            cmd_swap(&svc, &destination, chain.into(), usdt, sats).await?;
        }
        Command::Recover { destination } => {
            let svc = init_service(config, &seed, &cli.data_dir).await?;
            cmd_recover(&svc, &destination).await?;
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

async fn init_service(config: BoltzConfig, seed: &[u8], data_dir: &Path) -> Result<BoltzService> {
    let store = Arc::new(FileBoltzStorage::new(data_dir));
    let svc = BoltzService::new(config, seed, store)
        .await
        .context("Failed to initialize BoltzService")?;

    // Register a global listener that prints status updates for all swaps.
    svc.add_event_listener(Box::new(PrintingEventListener))
        .await;

    // Resume any active swaps from a previous run.
    let resumed = svc.resume_swaps().await.context("Failed to resume swaps")?;
    if !resumed.is_empty() {
        println!("Resumed {} active swap(s)", resumed.len());
    }
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
    print_json(&limits);
    Ok(())
}

async fn prepare(
    svc: &BoltzService,
    destination: &str,
    chain: boltz::Chain,
    usdt: Option<u64>,
    sats: Option<u64>,
) -> Result<boltz::PreparedSwap> {
    match (usdt, sats) {
        (Some(usdt_amount), _) => Ok(svc
            .prepare_reverse_swap(destination, chain, usdt_amount)
            .await?),
        (_, Some(sats_amount)) => Ok(svc
            .prepare_reverse_swap_from_sats(destination, chain, sats_amount)
            .await?),
        _ => bail!("Either --usdt or --sats must be provided"),
    }
}

async fn cmd_swap(
    svc: &BoltzService,
    destination: &str,
    chain: boltz::Chain,
    usdt: Option<u64>,
    sats: Option<u64>,
) -> Result<()> {
    // Step 1: Prepare
    println!("Fetching quote...\n");
    let prepared = prepare(svc, destination, chain, usdt, sats).await?;
    print_json(&prepared);

    // Confirm
    if !confirm("\nProceed with swap?")? {
        println!("Cancelled.");
        return Ok(());
    }

    // Step 2: Register a channel listener to wait for this swap's terminal event.
    // The global PrintingEventListener (registered in init_service) handles
    // printing status updates for all swaps.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<BoltzSwapEvent>(32);
    let listener_id = svc
        .add_event_listener(Box::new(ChannelEventListener { tx: event_tx }))
        .await;

    // Step 3: Create — swap monitoring starts automatically
    println!("\nCreating swap on Boltz...");
    let created = svc.create_reverse_swap(&prepared).await?;
    println!("\nSwap created:");
    print_json(&created);
    println!("\n>>> PAY THIS INVOICE to continue <<<\n");

    // Step 4: Wait for this swap to reach a terminal state
    while let Some(BoltzSwapEvent::SwapUpdated { swap }) = event_rx.recv().await {
        if swap.id == created.swap_id && swap.status.is_terminal() {
            break;
        }
    }

    svc.remove_event_listener(&listener_id).await;
    Ok(())
}

/// Global event listener that prints swap status updates to stdout.
struct PrintingEventListener;

#[macros::async_trait]
impl BoltzEventListener for PrintingEventListener {
    async fn on_event(&self, event: BoltzSwapEvent) {
        match &event {
            BoltzSwapEvent::SwapUpdated { swap } => {
                println!("[{}] Status: {:?}", swap.id, swap.status);
                if swap.status.is_terminal() {
                    println!("  Final state:");
                    print_json(swap);
                }
            }
        }
    }
}

/// Event listener that forwards events to an mpsc channel.
struct ChannelEventListener {
    tx: tokio::sync::mpsc::Sender<BoltzSwapEvent>,
}

#[macros::async_trait]
impl BoltzEventListener for ChannelEventListener {
    async fn on_event(&self, event: BoltzSwapEvent) {
        let _ = self.tx.send(event).await;
    }
}

async fn cmd_recover(svc: &BoltzService, destination: &str) -> Result<()> {
    println!("Scanning blockchain for recoverable swaps...");
    println!("This may take a few minutes.\n");

    let result = svc.recover(destination).await?;
    print_json(&result);

    Ok(())
}

const USDT_FIELDS: &[&str] = &["usdt_amount", "usdt_delivered"];

fn print_json(value: &impl serde::Serialize) {
    let mut json = serde_json::to_value(value).unwrap();
    format_usdt_fields(&mut json);
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}

fn format_usdt_fields(value: &mut serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        for (key, val) in obj.iter_mut() {
            if USDT_FIELDS.contains(&key.as_str())
                && let Some(raw) = val.as_u64()
            {
                *val = serde_json::Value::String(format!(
                    "{}.{:06} USDT",
                    raw / 1_000_000,
                    raw % 1_000_000
                ));
            }
        }
    }
}

// ─── Logging ────────────────────────────────────────────────────────────

fn init_logging(data_dir: &Path) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        "debug,h2=warn,rustls=warn,hyper=warn,tonic=warn"
            .parse()
            .unwrap()
    });

    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("boltz.log"))
        .with_context(|| "Failed to open log file")?;

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_file)
                .with_ansi(false),
        )
        .try_init()
        .ok(); // Ignore if already initialized

    Ok(())
}

// ─── File-backed BoltzStorage ─────────────────────────────────────────────
// Persists key indices to `{data_dir}/key_index_{chain_id}` and swap state to
// `{data_dir}/swaps/{swap_id}.json` so that active swaps survive CLI restarts.
//
// Known limitations (acceptable for a CLI tool):
// - Writes are not atomic (fs::write, not write-to-temp-then-rename). A crash
//   mid-write could produce corrupted JSON. The SDK should provide its own
//   BoltzStorage with atomic writes.
// - Uses blocking I/O (std::fs) inside async trait methods. Tolerable with
//   tokio's multi-threaded runtime.

struct FileBoltzStorage {
    data_dir: PathBuf,
}

impl FileBoltzStorage {
    fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    fn index_path(&self, chain_id: u64) -> PathBuf {
        self.data_dir.join(format!("key_index_{chain_id}"))
    }

    fn swaps_dir(&self) -> PathBuf {
        self.data_dir.join("swaps")
    }

    fn swap_path(&self, id: &str) -> PathBuf {
        self.swaps_dir().join(format!("{id}.json"))
    }

    fn read_index(&self, chain_id: u64) -> Result<u32, BoltzError> {
        match fs::read_to_string(self.index_path(chain_id)) {
            Ok(s) => s
                .trim()
                .parse()
                .map_err(|e| BoltzError::Store(format!("Invalid key index: {e}"))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(BoltzError::Store(format!("Failed to read key index: {e}"))),
        }
    }

    fn write_index(&self, chain_id: u64, index: u32) -> Result<(), BoltzError> {
        fs::write(self.index_path(chain_id), index.to_string())
            .map_err(|e| BoltzError::Store(format!("Failed to write key index: {e}")))
    }

    fn write_swap(&self, swap: &boltz::BoltzSwap) -> Result<(), BoltzError> {
        let dir = self.swaps_dir();
        fs::create_dir_all(&dir)
            .map_err(|e| BoltzError::Store(format!("Failed to create swaps dir: {e}")))?;
        let json = serde_json::to_string_pretty(swap)
            .map_err(|e| BoltzError::Store(format!("Failed to serialize swap: {e}")))?;
        fs::write(self.swap_path(&swap.id), json)
            .map_err(|e| BoltzError::Store(format!("Failed to write swap: {e}")))
    }

    fn read_swap(&self, id: &str) -> Result<Option<boltz::BoltzSwap>, BoltzError> {
        let path = self.swap_path(id);
        match fs::read_to_string(&path) {
            Ok(json) => {
                let swap: boltz::BoltzSwap = serde_json::from_str(&json)
                    .map_err(|e| BoltzError::Store(format!("Failed to parse swap: {e}")))?;
                Ok(Some(swap))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BoltzError::Store(format!("Failed to read swap: {e}"))),
        }
    }
}

#[macros::async_trait]
impl BoltzStorage for FileBoltzStorage {
    async fn insert_swap(&self, swap: &boltz::BoltzSwap) -> Result<(), BoltzError> {
        self.write_swap(swap)
    }

    async fn update_swap(&self, swap: &boltz::BoltzSwap) -> Result<(), BoltzError> {
        if !self.swap_path(&swap.id).exists() {
            return Err(BoltzError::Store(format!("Swap not found: {}", swap.id)));
        }
        self.write_swap(swap)
    }

    async fn get_swap(&self, id: &str) -> Result<Option<boltz::BoltzSwap>, BoltzError> {
        self.read_swap(id)
    }

    async fn list_active_swaps(&self) -> Result<Vec<boltz::BoltzSwap>, BoltzError> {
        let dir = self.swaps_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut active = Vec::new();
        let entries = fs::read_dir(&dir)
            .map_err(|e| BoltzError::Store(format!("Failed to read swaps dir: {e}")))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| BoltzError::Store(format!("Failed to read dir entry: {e}")))?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let json = fs::read_to_string(&path)
                    .map_err(|e| BoltzError::Store(format!("Failed to read swap file: {e}")))?;
                let swap: boltz::BoltzSwap = serde_json::from_str(&json)
                    .map_err(|e| BoltzError::Store(format!("Failed to parse swap: {e}")))?;
                if !swap.status.is_terminal() {
                    active.push(swap);
                }
            }
        }
        Ok(active)
    }

    async fn increment_key_index(&self, chain_id: u64) -> Result<u32, BoltzError> {
        let current = self.read_index(chain_id)?;
        let next = current
            .checked_add(1)
            .ok_or_else(|| BoltzError::Store("Key index overflow".to_string()))?;
        self.write_index(chain_id, next)?;
        Ok(current)
    }

    async fn set_key_index_if_higher(&self, chain_id: u64, value: u32) -> Result<(), BoltzError> {
        let current = self.read_index(chain_id)?;
        if value > current {
            self.write_index(chain_id, value)?;
        }
        Ok(())
    }
}

fn rand_entropy() -> [u8; 16] {
    let mut out = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut out);
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
