//! Parallel Payment Performance Testing Tool
//!
//! Tests parallel payment throughput in regtest by executing
//! multiple Spark transfers and Lightning payments concurrently.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Parser;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use breez_sdk_itest::{RegtestFaucet, build_sdk_with_custom_config};
use breez_sdk_spark::{
    BreezSdk, GetInfoRequest, Network, PrepareSendPaymentRequest, ReceivePaymentMethod,
    ReceivePaymentRequest, SdkEvent, SendPaymentRequest, SyncWalletRequest, default_config,
};

use breez_bench::events::{wait_for_claimed_event, wait_for_synced_event};
use breez_bench::stats::DurationStats;

#[derive(Parser, Debug)]
#[command(name = "parallel-perf")]
#[command(about = "Parallel payment performance testing for Breez SDK")]
struct Args {
    /// Number of Spark transfer payments
    #[arg(long, default_value = "10")]
    transfers: u32,

    /// Number of Lightning payments
    #[arg(long, default_value = "10")]
    lightning: u32,

    /// Delay between starting payments in milliseconds (not waiting for completion)
    #[arg(long, default_value = "100")]
    delay_ms: u64,

    /// Minimum payment amount in satoshis
    #[arg(long, default_value = "100")]
    min_amount: u64,

    /// Maximum payment amount in satoshis
    #[arg(long, default_value = "2000")]
    max_amount: u64,

    /// Random seed for reproducibility
    #[arg(long)]
    seed: Option<u64>,

    /// Disable automatic leaf optimization
    #[arg(long)]
    no_auto_optimize: bool,

    /// Run leaf optimization before test starts with specified multiplicity (0-5)
    #[arg(long, value_name = "MULTIPLICITY")]
    pre_optimize: Option<u8>,
}

/// Type of payment to execute
#[derive(Debug, Clone)]
enum PaymentType_ {
    Transfer { address: String, amount: u64 },
    Lightning { invoice: String, amount: u64 },
}

impl PaymentType_ {
    fn name(&self) -> &'static str {
        match self {
            PaymentType_::Transfer { .. } => "Transfer",
            PaymentType_::Lightning { .. } => "Lightning",
        }
    }

    fn amount(&self) -> u64 {
        match self {
            PaymentType_::Transfer { amount, .. } => *amount,
            PaymentType_::Lightning { amount, .. } => *amount,
        }
    }
}

/// A single payment task to execute
#[derive(Debug, Clone)]
struct PaymentTask {
    id: usize,
    payment_type: PaymentType_,
}

/// Result of a single payment execution
#[derive(Debug)]
struct PaymentResult {
    id: usize,
    payment_type: PaymentType_,
    duration: Duration,
    success: bool,
    error: Option<String>,
}

/// SDK instance wrapper with event channel
struct BenchSdkInstance {
    sdk: BreezSdk,
    events: mpsc::Receiver<SdkEvent>,
    #[allow(dead_code)]
    temp_dir: Option<tempdir::TempDir>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Set up tracing
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "parallel_perf=info,\
             breez_sdk_spark=error,\
             spark=error,\
             spark_wallet=error,\
             breez_sdk_common=error,\
             breez_sdk_itest=error,\
             warn",
        )
    });

    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(filter)
        .init();

    // Determine seed
    let seed = args.seed.unwrap_or_else(|| {
        let s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        info!("Using random seed: {}", s);
        s
    });

    info!("Parallel Payment Performance Test");
    info!("==================================");
    info!("Transfers: {}", args.transfers);
    info!("Lightning: {}", args.lightning);
    info!("Delay: {}ms", args.delay_ms);
    info!(
        "Amount range: {} - {} sats",
        args.min_amount, args.max_amount
    );
    info!("Seed: {}", seed);
    info!(
        "Auto-optimize: {}",
        if args.no_auto_optimize {
            "disabled"
        } else {
            "enabled"
        }
    );
    if let Some(mult) = args.pre_optimize {
        info!("Pre-optimize: multiplicity {}", mult);
    }
    info!("");

    let total_payments = args.transfers + args.lightning;
    if total_payments == 0 {
        bail!("At least one payment must be specified");
    }

    // Calculate required funding
    let max_total_amount = args.max_amount * total_payments as u64;
    let funding_amount = max_total_amount.clamp(10_000, 50_000);

    // Initialize SDKs
    info!("Initializing sender and receiver SDKs...");
    let (mut sender, mut receiver) =
        initialize_sdk_pair(args.no_auto_optimize, args.pre_optimize).await?;

    // Wait for initial sync
    info!("Waiting for sender sync...");
    wait_for_synced_event(&mut sender.events, 120).await?;
    info!("Waiting for receiver sync...");
    wait_for_synced_event(&mut receiver.events, 120).await?;

    // Fund sender
    info!("Funding sender with {} sats...", funding_amount);
    fund_via_faucet(&mut sender, funding_amount).await?;

    // Run pre-optimization if requested
    if args.pre_optimize.is_some() {
        run_pre_optimization(&sender.sdk).await?;
    }

    // Get receiver's Spark address for transfers
    let receiver_address = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Receiver address: {}", receiver_address);

    // Generate random amounts
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    // Pre-create Lightning invoices with random amounts
    info!("Pre-creating {} Lightning invoices...", args.lightning);
    let mut lightning_invoices = Vec::with_capacity(args.lightning as usize);
    for _ in 0..args.lightning {
        let amount = rng.gen_range(args.min_amount..=args.max_amount);
        let invoice = receiver
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::Bolt11Invoice {
                    description: "parallel-perf test".to_string(),
                    amount_sats: Some(amount),
                    expiry_secs: Some(3600),
                    payment_hash: None,
                },
            })
            .await?
            .payment_request;
        lightning_invoices.push((invoice, amount));
    }
    info!("Created {} Lightning invoices", lightning_invoices.len());

    // Build payment queue
    let mut payments: Vec<PaymentTask> = Vec::with_capacity(total_payments as usize);
    let mut id = 0;

    // Add transfers
    for _ in 0..args.transfers {
        let amount = rng.gen_range(args.min_amount..=args.max_amount);
        payments.push(PaymentTask {
            id,
            payment_type: PaymentType_::Transfer {
                address: receiver_address.clone(),
                amount,
            },
        });
        id += 1;
    }

    // Add lightning payments
    for (invoice, amount) in lightning_invoices {
        payments.push(PaymentTask {
            id,
            payment_type: PaymentType_::Lightning { invoice, amount },
        });
        id += 1;
    }

    // Shuffle the payment queue
    payments.shuffle(&mut rng);

    // Execute payments
    info!("");
    info!(
        "Starting {} payments with {}ms delay between starts...",
        payments.len(),
        args.delay_ms
    );
    info!("");

    let sender_sdk = Arc::new(sender.sdk);
    let (results, total_duration) =
        execute_payments(sender_sdk.clone(), payments, args.delay_ms).await;

    // Print summary
    print_summary(&results, args.transfers, args.lightning, total_duration);

    // Cleanup: disconnect both SDKs
    info!("Disconnecting SDKs...");
    if let Err(e) = sender_sdk.disconnect().await {
        warn!("Failed to disconnect sender SDK: {}", e);
    }
    if let Err(e) = receiver.sdk.disconnect().await {
        warn!("Failed to disconnect receiver SDK: {}", e);
    }
    info!("Cleanup complete");

    Ok(())
}

/// Execute payments with delay between starts
/// Returns the results and total wall-clock duration
async fn execute_payments(
    sender: Arc<BreezSdk>,
    payments: Vec<PaymentTask>,
    delay_ms: u64,
) -> (Vec<PaymentResult>, Duration) {
    let mut handles = Vec::with_capacity(payments.len());
    let total_start = Instant::now();

    for payment in payments {
        let sender = sender.clone();
        let payment_id = payment.id;
        let payment_type_name = payment.payment_type.name();
        let payment_amount = payment.payment_type.amount();

        println!(
            "[START] {} #{}: {} sats",
            payment_type_name, payment_id, payment_amount
        );

        let handle = tokio::spawn(async move {
            let start = Instant::now();
            let result = execute_single_payment(&sender, &payment.payment_type).await;
            let duration = start.elapsed();

            let payment_result = PaymentResult {
                id: payment.id,
                payment_type: payment.payment_type,
                duration,
                success: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
            };

            // Print result immediately when payment completes
            if payment_result.success {
                println!(
                    "[OK]   {} #{}: {} sats in {:.2}s",
                    payment_result.payment_type.name(),
                    payment_result.id,
                    payment_result.payment_type.amount(),
                    payment_result.duration.as_secs_f64()
                );
            } else {
                println!(
                    "[FAIL] {} #{}: {}",
                    payment_result.payment_type.name(),
                    payment_result.id,
                    payment_result.error.as_deref().unwrap_or("unknown error")
                );
            }

            payment_result
        });

        handles.push(handle);

        // Delay before starting next payment
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    // Wait for all payments to complete and collect results
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(result) => {
                results.push(result);
            }
            Err(e) => {
                warn!("Task join error: {}", e);
            }
        }
    }

    let total_duration = total_start.elapsed();
    println!();
    println!(
        "All payments completed in {:.2}s",
        total_duration.as_secs_f64()
    );

    (results, total_duration)
}

/// Execute a single payment
async fn execute_single_payment(sender: &BreezSdk, payment_type: &PaymentType_) -> Result<()> {
    match payment_type {
        PaymentType_::Transfer { address, amount } => {
            let prepare = sender
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request: address.clone(),
                    amount: Some(*amount as u128),
                    token_identifier: None,
                    conversion_options: None,
                    fee_policy: None,
                })
                .await?;

            sender
                .send_payment(SendPaymentRequest {
                    prepare_response: prepare,
                    options: None,
                    idempotency_key: None,
                })
                .await?;

            Ok(())
        }
        PaymentType_::Lightning { invoice, .. } => {
            let prepare = sender
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request: invoice.clone(),
                    amount: None,
                    token_identifier: None,
                    conversion_options: None,
                    fee_policy: None,
                })
                .await?;

            sender
                .send_payment(SendPaymentRequest {
                    prepare_response: prepare,
                    options: None,
                    idempotency_key: None,
                })
                .await?;

            Ok(())
        }
    }
}

/// Print summary statistics
fn print_summary(
    results: &[PaymentResult],
    num_transfers: u32,
    num_lightning: u32,
    total_duration: Duration,
) {
    println!();
    println!("============================================================");
    println!("SUMMARY");
    println!("============================================================");

    let total = results.len();
    let successful: Vec<_> = results.iter().filter(|r| r.success).collect();
    let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();

    println!(
        "Total payments: {} ({} transfers + {} lightning)",
        total, num_transfers, num_lightning
    );
    println!(
        "Success rate: {}/{} ({:.1}%)",
        successful.len(),
        total,
        if total > 0 {
            (successful.len() as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    );

    // Throughput based on total wall-clock time
    let total_duration_mins = total_duration.as_secs_f64() / 60.0;
    let throughput = if total_duration_mins > 0.0 {
        total as f64 / total_duration_mins
    } else {
        0.0
    };
    println!("Throughput: {:.1} payments/minute", throughput);

    if !successful.is_empty() {
        let durations: Vec<Duration> = successful.iter().map(|r| r.duration).collect();

        if let Some(stats) = DurationStats::from_durations(&durations) {
            println!();
            println!("Duration Statistics (successful payments):");
            println!(
                "  Min: {}   Max: {}   Mean: {}",
                DurationStats::format_duration(stats.min),
                DurationStats::format_duration(stats.max),
                DurationStats::format_duration(stats.mean),
            );
            println!(
                "  p50: {}   p95: {}   p99: {}",
                DurationStats::format_duration(stats.p50),
                DurationStats::format_duration(stats.p95),
                DurationStats::format_duration(stats.p99),
            );
        }

        // Breakdown by payment type
        let transfer_results: Vec<_> = successful
            .iter()
            .filter(|r| matches!(r.payment_type, PaymentType_::Transfer { .. }))
            .collect();
        let lightning_results: Vec<_> = successful
            .iter()
            .filter(|r| matches!(r.payment_type, PaymentType_::Lightning { .. }))
            .collect();

        if !transfer_results.is_empty() {
            let transfer_durations: Vec<Duration> =
                transfer_results.iter().map(|r| r.duration).collect();
            if let Some(stats) = DurationStats::from_durations(&transfer_durations) {
                println!();
                println!("Transfer Statistics (n={}):", transfer_results.len());
                println!(
                    "  Min: {}   Max: {}   Mean: {}",
                    DurationStats::format_duration(stats.min),
                    DurationStats::format_duration(stats.max),
                    DurationStats::format_duration(stats.mean),
                );
                println!(
                    "  p50: {}   p95: {}   p99: {}",
                    DurationStats::format_duration(stats.p50),
                    DurationStats::format_duration(stats.p95),
                    DurationStats::format_duration(stats.p99),
                );
            }
        }

        if !lightning_results.is_empty() {
            let lightning_durations: Vec<Duration> =
                lightning_results.iter().map(|r| r.duration).collect();
            if let Some(stats) = DurationStats::from_durations(&lightning_durations) {
                println!();
                println!("Lightning Statistics (n={}):", lightning_results.len());
                println!(
                    "  Min: {}   Max: {}   Mean: {}",
                    DurationStats::format_duration(stats.min),
                    DurationStats::format_duration(stats.max),
                    DurationStats::format_duration(stats.mean),
                );
                println!(
                    "  p50: {}   p95: {}   p99: {}",
                    DurationStats::format_duration(stats.p50),
                    DurationStats::format_duration(stats.p95),
                    DurationStats::format_duration(stats.p99),
                );
            }
        }
    }

    // Print failure details
    if !failed.is_empty() {
        println!();
        println!("Failed Payments ({}):", failed.len());
        for r in &failed {
            println!(
                "  {} #{}: {}",
                r.payment_type.name(),
                r.id,
                r.error.as_deref().unwrap_or("unknown error")
            );
        }
    }

    println!();
}

/// Initialize SDK pair for regtest
async fn initialize_sdk_pair(
    no_auto_optimize: bool,
    pre_optimize: Option<u8>,
) -> Result<(BenchSdkInstance, BenchSdkInstance)> {
    use rand::RngCore;
    use tempdir::TempDir;

    // Create sender SDK
    let sender_dir = TempDir::new("parallel-perf-sender")?;
    let sender_path = sender_dir.path().to_string_lossy().to_string();
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);

    let mut sender_config = default_config(Network::Regtest);
    // Disable auto-optimization if requested, or if we're doing pre-optimization
    // (to avoid auto-opt triggering after funding)
    if no_auto_optimize || pre_optimize.is_some() {
        sender_config.optimization_config.auto_enabled = false;
    }
    // Set multiplicity if pre-optimization is requested
    if let Some(multiplicity) = pre_optimize {
        sender_config.optimization_config.multiplicity = multiplicity;
    }
    let itest_sender =
        build_sdk_with_custom_config(sender_path, sender_seed, sender_config, None, true).await?;

    // Create receiver SDK
    let receiver_dir = TempDir::new("parallel-perf-receiver")?;
    let receiver_path = receiver_dir.path().to_string_lossy().to_string();
    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);

    let mut receiver_config = default_config(Network::Regtest);
    // Disable auto-optimization for receiver if requested
    receiver_config.optimization_config.auto_enabled = false;
    let itest_receiver =
        build_sdk_with_custom_config(receiver_path, receiver_seed, receiver_config, None, true)
            .await?;

    Ok((
        BenchSdkInstance {
            sdk: itest_sender.sdk,
            events: itest_sender.events,
            temp_dir: Some(sender_dir),
        },
        BenchSdkInstance {
            sdk: itest_receiver.sdk,
            events: itest_receiver.events,
            temp_dir: Some(receiver_dir),
        },
    ))
}

/// Run leaf optimization and wait for completion
async fn run_pre_optimization(sdk: &BreezSdk) -> Result<()> {
    run_optimization(sdk, "Pre-optimization").await
}

/// Run leaf optimization with a label and wait for completion
async fn run_optimization(sdk: &BreezSdk, label: &str) -> Result<()> {
    info!("Starting {}...", label.to_lowercase());
    sdk.start_leaf_optimization();

    // Poll until optimization completes
    let start = Instant::now();
    let timeout = Duration::from_secs(300); // 5 minute timeout
    let poll_interval = Duration::from_millis(500);

    loop {
        let progress = sdk.get_leaf_optimization_progress();

        if !progress.is_running {
            let elapsed = start.elapsed();
            info!("{} complete in {:.2}s", label, elapsed.as_secs_f64());
            break;
        }

        info!(
            "Optimization progress: round {}/{}",
            progress.current_round, progress.total_rounds
        );

        if start.elapsed() >= timeout {
            bail!("Timeout waiting for optimization to complete");
        }

        tokio::time::sleep(poll_interval).await;
    }

    Ok(())
}

/// Fund wallet via regtest faucet
async fn fund_via_faucet(sdk_instance: &mut BenchSdkInstance, amount: u64) -> Result<()> {
    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

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
    let txid = faucet.fund_address(&deposit_address, amount).await?;
    info!("Faucet sent {} sats in txid: {}", amount, txid);

    // Wait for claim event
    wait_for_claimed_event(&mut sdk_instance.events, 180).await?;

    // Wait for balance to update
    let start = Instant::now();
    let timeout = Duration::from_secs(30);
    loop {
        sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let info = sdk_instance
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;

        if info.balance_sats > 0 {
            info!("Funded. Balance: {} sats", info.balance_sats);
            return Ok(());
        }

        if start.elapsed() >= timeout {
            bail!("Timeout waiting for balance after funding");
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
