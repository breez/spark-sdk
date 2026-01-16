//! Breez SDK Performance Benchmark CLI
//!
//! Measures payment/transfer performance.
//! Supports both regtest (with automatic funding) and mainnet (with persistent wallets).

mod operation_detector;
mod scenarios;
mod stats;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use bip39::{Language, Mnemonic};
use clap::Parser;
use tracing::{debug, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use breez_sdk_spark::{
    BreezSdk, EventListener, GetInfoRequest, Network, PaymentType, PrepareSendPaymentRequest,
    ReceivePaymentMethod, ReceivePaymentRequest, SdkBuilder, SdkEvent, Seed, SendPaymentRequest,
    SyncWalletRequest, default_config,
};
use tokio::sync::mpsc;

use operation_detector::{OperationDetectionGuard, OperationDetectorLayer, create_operation_flag};
use scenarios::{
    DEFAULT_MAX_AMOUNT, DEFAULT_MAX_DELAY_MS, DEFAULT_MIN_AMOUNT, DEFAULT_MIN_DELAY_MS,
    DEFAULT_PAYMENT_COUNT, DEFAULT_RETURN_INTERVAL, DEFAULT_SEED, MAX_INITIAL_FUNDING,
    ScenarioConfig, ScenarioPreset, generate_payments,
};
use stats::{BenchmarkResults, PaymentMeasurement};

const PHRASE_FILE_NAME: &str = "phrase";
const MIN_BALANCE_FOR_BENCHMARK: u64 = 10_000; // Minimum sats needed to run benchmark

#[derive(Parser, Debug)]
#[command(name = "breez-sdk-bench")]
#[command(about = "Performance benchmarks for Breez SDK payments")]
struct Args {
    /// Network to use (regtest, mainnet)
    #[arg(long, default_value = "regtest")]
    network: String,

    /// Data directory for sender wallet (mainnet only). Contains 'phrase' file with mnemonic.
    #[arg(long, default_value = ".mainnet_sender")]
    sender_data_dir: Option<String>,

    /// Data directory for receiver wallet (mainnet only). Contains 'phrase' file with mnemonic.
    #[arg(long, default_value = ".mainnet_receiver")]
    receiver_data_dir: Option<String>,

    /// Random seed for reproducible benchmarks
    #[arg(long, default_value_t = DEFAULT_SEED)]
    seed: u64,

    /// Number of payments to execute
    #[arg(long, default_value_t = DEFAULT_PAYMENT_COUNT)]
    payments: usize,

    /// Minimum payment amount in satoshis
    #[arg(long, default_value_t = DEFAULT_MIN_AMOUNT)]
    min_amount: u64,

    /// Maximum payment amount in satoshis
    #[arg(long, default_value_t = DEFAULT_MAX_AMOUNT)]
    max_amount: u64,

    /// Minimum delay between payments in milliseconds
    #[arg(long, default_value_t = DEFAULT_MIN_DELAY_MS)]
    min_delay_ms: u64,

    /// Maximum delay between payments in milliseconds
    #[arg(long, default_value_t = DEFAULT_MAX_DELAY_MS)]
    max_delay_ms: u64,

    /// How often receiver sends funds back (every N payments). 0 to disable.
    #[arg(long, default_value_t = DEFAULT_RETURN_INTERVAL)]
    return_interval: usize,

    /// Scenario preset: random, edge-cases, small-payments, large-payments
    #[arg(long, default_value = "random")]
    scenario: String,

    /// Sender wallet multiplicity (optimization parameter)
    #[arg(long)]
    sender_multiplicity: Option<u8>,

    /// Receiver wallet multiplicity (optimization parameter)
    #[arg(long, default_value_t = 0)]
    receiver_multiplicity: u8,
}

/// SDK instance wrapper with event channel
struct BenchSdkInstance {
    sdk: BreezSdk,
    events: mpsc::Receiver<SdkEvent>,
    #[allow(dead_code)]
    temp_dir: Option<tempdir::TempDir>, // Keep alive for regtest
}

/// Event listener that forwards events to a channel
struct ChannelEventListener {
    tx: mpsc::Sender<SdkEvent>,
}

#[async_trait::async_trait]
impl EventListener for ChannelEventListener {
    async fn on_event(&self, event: SdkEvent) {
        let _ = self.tx.send(event).await;
    }
}

fn is_mainnet(network: Network) -> bool {
    matches!(network, Network::Mainnet)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Parse network
    let network = match args.network.to_lowercase().as_str() {
        "regtest" => Network::Regtest,
        "mainnet" => Network::Mainnet,
        _ => bail!(
            "Invalid network '{}'. Use 'regtest' or 'mainnet'",
            args.network
        ),
    };

    // Set up tracing with swap and cancellation detection layers
    let swap_flag = create_operation_flag();
    let swap_layer = OperationDetectorLayer::new_swap_detector(swap_flag.clone());

    let cancellation_flag = create_operation_flag();
    let cancellation_layer =
        OperationDetectorLayer::new_cancellation_detector(cancellation_flag.clone());

    // Filter for console output: info for bench, errors only for SDK internals
    let console_filter = "breez_sdk_bench=info,\
                          breez_sdk_spark=error,\
                          spark=error,\
                          spark_wallet=error,\
                          breez_sdk_common=error,\
                          breez_sdk_itest=error,\
                          warn";
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(console_filter));

    // Filter for operation detection: need trace level from spark::tree::service and info from spark::services::leaf_optimizer
    let swap_filter = EnvFilter::new("spark::tree::service=trace");
    let cancellation_filter = EnvFilter::new("spark::services::leaf_optimizer=info");

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_filter(console_filter),
        )
        .with(swap_layer.with_filter(swap_filter))
        .with(cancellation_layer.with_filter(cancellation_filter))
        .init();

    info!("Breez SDK Performance Benchmark");
    info!("================================");
    info!("Network: {:?}", network);

    // Parse scenario preset
    let preset = ScenarioPreset::from_str(&args.scenario).unwrap_or_else(|| {
        warn!("Unknown scenario '{}', using 'random'", args.scenario);
        ScenarioPreset::Random
    });

    // Build scenario config
    let config = ScenarioConfig {
        seed: args.seed,
        payment_count: args.payments,
        min_amount: args.min_amount,
        max_amount: args.max_amount,
        min_delay_ms: args.min_delay_ms,
        max_delay_ms: args.max_delay_ms,
        return_interval: args.return_interval,
    };

    // Validate config
    if let Err(e) = config.validate() {
        bail!("Invalid configuration: {}", e);
    }

    info!("Scenario: {:?}", preset);
    info!("Seed: {}", config.seed);
    info!("Payments: {}", config.payment_count);
    info!(
        "Amount range: {} - {} sats",
        config.min_amount, config.max_amount
    );
    info!(
        "Delay range: {} - {} ms",
        config.min_delay_ms, config.max_delay_ms
    );
    if config.return_interval > 0 {
        info!(
            "Return interval: every {} payments receiver sends funds back",
            config.return_interval
        );
    }

    // Generate payment specifications
    let payments = generate_payments(&config, preset);

    // Initialize SDK instances based on network
    info!("Initializing SDK instances...");
    let (mut sender, mut receiver) = match network {
        Network::Regtest => {
            initialize_regtest_sdk_pair(args.sender_multiplicity, args.receiver_multiplicity)
                .await?
        }
        Network::Mainnet => {
            let sender_dir = args
                .sender_data_dir
                .ok_or_else(|| anyhow::anyhow!("--sender-data-dir is required for mainnet"))?;
            let receiver_dir = args
                .receiver_data_dir
                .ok_or_else(|| anyhow::anyhow!("--receiver-data-dir is required for mainnet"))?;
            initialize_mainnet_sdk_pair(
                &sender_dir,
                &receiver_dir,
                args.sender_multiplicity,
                args.receiver_multiplicity,
            )
            .await?
        }
    };

    // Get addresses for both parties
    let receiver_address = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Receiver address: {}", receiver_address);

    let sender_address = sender
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Sender address: {}", sender_address);

    // Wait for initial sync before starting benchmark
    info!("Waiting for sender initial sync...");
    wait_for_synced_event(&mut sender.events, 120).await?;
    info!("Sender synced");

    info!("Waiting for receiver initial sync...");
    wait_for_synced_event(&mut receiver.events, 120).await?;
    info!("Receiver synced");

    // Handle funding based on network
    match network {
        Network::Regtest => {
            // Use itest faucet for regtest
            info!(
                "Funding sender with {} sats (single deposit)...",
                MAX_INITIAL_FUNDING
            );
            fund_via_faucet(&mut sender, MAX_INITIAL_FUNDING).await?;
        }
        Network::Mainnet => {
            // Check balances and consolidate to sender
            info!("Checking balances...");
            sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
            receiver.sdk.sync_wallet(SyncWalletRequest {}).await?;

            let sender_balance = sender
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?
                .balance_sats;
            let receiver_balance = receiver
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?
                .balance_sats;

            info!("Sender balance: {} sats", sender_balance);
            info!("Receiver balance: {} sats", receiver_balance);

            // Move all receiver funds to sender first (if any)
            if receiver_balance > 0 {
                info!(
                    "Moving {} sats from receiver to sender...",
                    receiver_balance
                );
                let prepare = receiver
                    .sdk
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request: sender_address.clone(),
                        amount: Some(receiver_balance as u128),
                        token_identifier: None,
                        conversion_options: None,
                    })
                    .await?;

                receiver
                    .sdk
                    .send_payment(SendPaymentRequest {
                        prepare_response: prepare,
                        options: None,
                        idempotency_key: None,
                    })
                    .await?;

                // Wait for sender to receive
                wait_for_payment_event(&mut sender.events, PaymentType::Receive, 120).await?;
                info!("Funds consolidated to sender");
            }

            // Now check if sender has minimum balance
            sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let sender_balance = sender
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?
                .balance_sats;

            if sender_balance < MIN_BALANCE_FOR_BENCHMARK {
                println!();
                println!("============================================================");
                println!("INSUFFICIENT FUNDS");
                println!("============================================================");
                println!("Sender balance: {} sats", sender_balance);
                println!("Minimum required: {} sats", MIN_BALANCE_FOR_BENCHMARK);
                println!();
                println!("Please fund the SENDER wallet:");
                println!("  Address: {}", sender_address);
                println!();
                println!("After funding, run the benchmark again.");
                println!("============================================================");
                return Ok(());
            }
        }
    };

    // Run the benchmark
    info!("Starting benchmark...");
    let mut results = BenchmarkResults::new(config.seed);

    // Track cumulative amount sent by sender
    let mut cumulative_sent: u64 = 0;

    for (i, payment_spec) in payments.iter().enumerate() {
        // Wait for the specified delay
        if payment_spec.delay.as_millis() > 0 {
            tokio::time::sleep(payment_spec.delay).await;
        }

        // Check sender balance and replenish if needed
        let sender_balance = sender
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;

        let required_balance = payment_spec.amount_sats;
        if sender_balance < required_balance {
            info!(
                "  [Low balance: {} sats, need {} sats - requesting funds from receiver]",
                sender_balance, required_balance
            );

            // Try to get funds back from receiver
            receiver.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let receiver_balance = receiver
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?
                .balance_sats;

            let return_amount = receiver_balance;
            if return_amount > 0 {
                match return_funds_to_sender(
                    &mut receiver,
                    &mut sender,
                    &sender_address,
                    return_amount,
                )
                .await
                {
                    Ok(()) => {
                        info!("  [Returned {} sats to sender]", return_amount);
                        cumulative_sent = 0;
                    }
                    Err(e) => {
                        warn!("  [Failed to return funds: {}]", e);
                    }
                }
            }

            // Re-check sender balance after potential return
            sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let new_sender_balance = sender
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?
                .balance_sats;

            if new_sender_balance < required_balance {
                warn!(
                    "Payment {}/{}: Skipping - insufficient funds ({} sats, need {} sats)",
                    i + 1,
                    payments.len(),
                    new_sender_balance,
                    required_balance
                );
                continue;
            }
        }

        info!(
            "Payment {}/{}: {} sats",
            i + 1,
            payments.len(),
            payment_spec.amount_sats
        );

        // Reset detection for this payment
        let swap_guard = OperationDetectionGuard::new(swap_flag.clone());
        let cancellation_guard = OperationDetectionGuard::new(cancellation_flag.clone());

        // Measure payment time
        let start = Instant::now();

        // Prepare and send payment from sender to receiver
        let prepare_result = sender
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: receiver_address.clone(),
                amount: Some(payment_spec.amount_sats as u128),
                token_identifier: None,
                conversion_options: None,
            })
            .await;

        let prepare = match prepare_result {
            Ok(p) => p,
            Err(e) => {
                warn!("  Failed to prepare payment: {} - skipping", e);
                continue;
            }
        };

        let send_result = sender
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await;

        if let Err(e) = send_result {
            warn!("  Failed to send payment: {} - skipping", e);
            continue;
        }

        let duration = start.elapsed();

        // Wait for receiver to get the payment
        if let Err(e) =
            wait_for_payment_event(&mut receiver.events, PaymentType::Receive, 120).await
        {
            warn!("  Failed waiting for payment receipt: {} - skipping", e);
            continue;
        }

        let had_swap = swap_guard.had_operation();
        let had_cancellation = cancellation_guard.had_operation();

        info!(
            "  Completed in {:?} (swap: {}, cancellation: {})",
            duration,
            if had_swap { "yes" } else { "no" },
            if had_cancellation { "yes" } else { "no" }
        );

        results.add(PaymentMeasurement {
            duration,
            had_swap,
            had_cancellation,
            amount_sats: payment_spec.amount_sats,
        });

        cumulative_sent += payment_spec.amount_sats;

        // Check if receiver should send funds back to sender (not benchmarked)
        let payment_num = i + 1;
        if config.return_interval > 0
            && payment_num % config.return_interval == 0
            && cumulative_sent > 0
        {
            let return_amount = (cumulative_sent * 80) / 100;
            if return_amount > 0 {
                info!(
                    "  [Return payment: receiver sending {} sats back to sender (not benchmarked)]",
                    return_amount
                );

                match return_funds_to_sender(
                    &mut receiver,
                    &mut sender,
                    &sender_address,
                    return_amount,
                )
                .await
                {
                    Ok(()) => {
                        debug!("  [Return payment completed]");
                        cumulative_sent = 0;
                    }
                    Err(e) => {
                        warn!("  [Return payment failed: {}]", e);
                    }
                }
            }
        }
    }

    // Report if some payments were skipped
    let completed = results.measurements.len();
    let total = payments.len();
    if completed < total {
        warn!(
            "Completed {}/{} payments ({} skipped due to errors)",
            completed,
            total,
            total - completed
        );
    }

    // On mainnet, move all funds back to sender at the end
    if is_mainnet(network) {
        info!("Moving all funds back to sender...");
        receiver.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let receiver_final_balance = receiver
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;

        if receiver_final_balance > 0 {
            match return_funds_to_sender(
                &mut receiver,
                &mut sender,
                &sender_address,
                receiver_final_balance,
            )
            .await
            {
                Ok(()) => info!("All funds returned to sender"),
                Err(e) => warn!("Failed to return all funds to sender: {}", e),
            }
        }

        // Print final balances
        sender.sdk.sync_wallet(SyncWalletRequest {}).await?;
        receiver.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let sender_final = sender
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;
        let receiver_final = receiver
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;
        info!(
            "Final balances - Sender: {} sats, Receiver: {} sats",
            sender_final, receiver_final
        );
    }

    // Print results
    results.print_report();

    Ok(())
}

/// Initialize SDK pair for regtest using temp directories
async fn initialize_regtest_sdk_pair(
    sender_multiplicity: Option<u8>,
    receiver_multiplicity: u8,
) -> Result<(BenchSdkInstance, BenchSdkInstance)> {
    use breez_sdk_itest::build_sdk_with_custom_config;
    use rand::RngCore;
    use tempdir::TempDir;

    // Create sender SDK - keep TempDir alive by storing in BenchSdkInstance
    let sender_dir = TempDir::new("breez-bench-sender")?;
    let sender_path = sender_dir.path().to_string_lossy().to_string();
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);

    let mut sender_config = default_config(Network::Regtest);
    if let Some(multiplicity) = sender_multiplicity {
        sender_config.optimization_config.multiplicity = multiplicity;
    }

    let itest_sender =
        build_sdk_with_custom_config(sender_path, sender_seed, sender_config, None, true).await?;

    // Create receiver SDK
    let receiver_dir = TempDir::new("breez-bench-receiver")?;
    let receiver_path = receiver_dir.path().to_string_lossy().to_string();
    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);

    let mut receiver_config = default_config(Network::Regtest);
    receiver_config.optimization_config.multiplicity = receiver_multiplicity;

    let itest_receiver =
        build_sdk_with_custom_config(receiver_path, receiver_seed, receiver_config, None, true)
            .await?;

    Ok((
        BenchSdkInstance {
            sdk: itest_sender.sdk,
            events: itest_sender.events,
            temp_dir: Some(sender_dir), // Keep alive
        },
        BenchSdkInstance {
            sdk: itest_receiver.sdk,
            events: itest_receiver.events,
            temp_dir: Some(receiver_dir), // Keep alive
        },
    ))
}

/// Initialize SDK pair for mainnet using persistent data directories
async fn initialize_mainnet_sdk_pair(
    sender_data_dir: &str,
    receiver_data_dir: &str,
    sender_multiplicity: Option<u8>,
    receiver_multiplicity: u8,
) -> Result<(BenchSdkInstance, BenchSdkInstance)> {
    let sender = initialize_mainnet_sdk(sender_data_dir, "sender", sender_multiplicity).await?;
    let receiver =
        initialize_mainnet_sdk(receiver_data_dir, "receiver", Some(receiver_multiplicity)).await?;
    Ok((sender, receiver))
}

/// Initialize a single SDK instance for mainnet
async fn initialize_mainnet_sdk(
    data_dir: &str,
    name: &str,
    multiplicity: Option<u8>,
) -> Result<BenchSdkInstance> {
    let data_path = expand_path(data_dir);
    fs::create_dir_all(&data_path)?;

    let mnemonic = get_or_create_mnemonic(&data_path)?;
    info!("{} mnemonic loaded from: {}", name, data_path.display());

    let breez_api_key = std::env::var_os("BREEZ_API_KEY")
        .map(|var| var.into_string().expect("Expected valid API key string"));

    let mut config = default_config(Network::Mainnet);
    config.api_key = breez_api_key;
    if let Some(multiplicity) = multiplicity {
        config.optimization_config.multiplicity = multiplicity;
    }

    let seed = Seed::Mnemonic {
        mnemonic: mnemonic.to_string(),
        passphrase: None,
    };

    let sdk = SdkBuilder::new(config, seed)
        .with_default_storage(data_path.to_string_lossy().to_string())
        .build()
        .await?;

    // Set up event listener
    let (tx, rx) = mpsc::channel(100);
    let event_listener = Box::new(ChannelEventListener { tx });
    sdk.add_event_listener(event_listener).await;

    // Initial sync
    let _ = sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    Ok(BenchSdkInstance {
        sdk,
        events: rx,
        temp_dir: None,
    })
}

/// Get or create mnemonic from phrase file
fn get_or_create_mnemonic(data_dir: &Path) -> Result<Mnemonic> {
    let filename = data_dir.join(PHRASE_FILE_NAME);

    match fs::read_to_string(&filename) {
        Ok(phrase) => {
            let mnemonic = Mnemonic::from_str(phrase.trim())
                .context("Failed to parse mnemonic from phrase file")?;
            Ok(mnemonic)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            info!("No phrase file found, generating new mnemonic...");
            let mnemonic = Mnemonic::generate_in(Language::English, 12)?;
            fs::write(&filename, mnemonic.to_string())?;
            info!("New mnemonic saved to: {}", filename.display());
            Ok(mnemonic)
        }
        Err(e) => {
            bail!("Failed to read phrase file {}: {}", filename.display(), e);
        }
    }
}

/// Expand ~ to home directory
fn expand_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        dirs::home_dir()
            .expect("Could not find home directory")
            .join(stripped)
    } else {
        PathBuf::from(path)
    }
}

/// Wait for a payment event
async fn wait_for_payment_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    payment_type: PaymentType,
    timeout_secs: u64,
) -> Result<()> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Timeout waiting for {:?} payment event after {} seconds",
                payment_type,
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::PaymentSucceeded { payment }))
                if payment.payment_type == payment_type =>
            {
                return Ok(());
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("Event channel closed"),
            Err(_) => bail!(
                "Timeout waiting for {:?} payment event after {} seconds",
                payment_type,
                timeout_secs
            ),
        }
    }
}

/// Wait for SDK sync event
async fn wait_for_synced_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Timeout waiting for Synced event after {} seconds",
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::Synced)) => {
                return Ok(());
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("Event channel closed"),
            Err(_) => bail!(
                "Timeout waiting for Synced event after {} seconds",
                timeout_secs
            ),
        }
    }
}

/// Return funds from receiver to sender
async fn return_funds_to_sender(
    receiver: &mut BenchSdkInstance,
    sender: &mut BenchSdkInstance,
    sender_address: &str,
    amount: u64,
) -> Result<()> {
    let prepare = receiver
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: sender_address.to_string(),
            amount: Some(amount as u128),
            token_identifier: None,
            conversion_options: None,
        })
        .await?;

    receiver
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    wait_for_payment_event(&mut sender.events, PaymentType::Receive, 120).await?;
    Ok(())
}

/// Fund wallet via regtest faucet
async fn fund_via_faucet(sdk_instance: &mut BenchSdkInstance, min_balance: u64) -> Result<()> {
    use breez_sdk_itest::RegtestFaucet;

    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;

    if info.balance_sats >= min_balance {
        info!("Already have {} sats, no funding needed", info.balance_sats);
        return Ok(());
    }

    let needed = min_balance - info.balance_sats;
    let fund_amount = needed.clamp(10_000, 50_000);
    info!(
        "Need {} sats, requesting {} from faucet",
        needed, fund_amount
    );

    // Get deposit address
    let receive = sdk_instance
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress,
        })
        .await?;
    let deposit_address = receive.payment_request;
    info!("Deposit address: {}", deposit_address);

    // Fund via faucet
    let faucet = RegtestFaucet::new()?;
    let txid = faucet.fund_address(&deposit_address, fund_amount).await?;
    info!("Faucet sent {} sats in txid: {}", fund_amount, txid);

    // Wait for claim event
    wait_for_claimed_event(&mut sdk_instance.events, 180).await?;

    // Wait for balance to update
    wait_for_balance(&sdk_instance.sdk, info.balance_sats + 1, 20).await?;

    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let final_info = sdk_instance
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?;
    info!("Funded. New balance: {} sats", final_info.balance_sats);

    Ok(())
}

/// Wait for deposit claim event
async fn wait_for_claimed_event(
    event_rx: &mut mpsc::Receiver<SdkEvent>,
    timeout_secs: u64,
) -> Result<()> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Timeout waiting for ClaimedDeposits event after {} seconds",
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(SdkEvent::ClaimedDeposits { claimed_deposits })) => {
                info!("Claimed {} deposits", claimed_deposits.len());
                return Ok(());
            }
            Ok(Some(SdkEvent::UnclaimedDeposits { unclaimed_deposits })) => {
                bail!(
                    "Deposit claim failed: {} unclaimed deposits",
                    unclaimed_deposits.len()
                );
            }
            Ok(Some(_)) => continue,
            Ok(None) => bail!("Event channel closed"),
            Err(_) => bail!(
                "Timeout waiting for ClaimedDeposits event after {} seconds",
                timeout_secs
            ),
        }
    }
}

/// Wait for balance to reach minimum
async fn wait_for_balance(sdk: &BreezSdk, min_balance: u64, timeout_secs: u64) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        let info = sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;

        if info.balance_sats >= min_balance {
            return Ok(());
        }

        if start.elapsed() >= timeout {
            bail!(
                "Timeout waiting for balance >= {} (current: {})",
                min_balance,
                info.balance_sats
            );
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
}
