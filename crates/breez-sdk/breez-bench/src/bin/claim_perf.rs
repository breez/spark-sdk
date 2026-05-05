//! Concurrent Claims Performance Testing Tool
//!
//! Benchmarks concurrent transfer claiming with different
//! `max_concurrent_claims` settings to measure throughput improvements.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Parser;
use futures::{StreamExt, stream};
use rand::{Rng, RngCore};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use breez_sdk_itest::{RegtestFaucet, build_sdk_with_tree_store_config, drop_postgres_database};
use breez_sdk_spark::{
    BreezSdk, GetInfoRequest, ListPaymentsRequest, Network, PaymentStatus, PaymentType,
    PrepareSendPaymentRequest, ReceivePaymentMethod, ReceivePaymentRequest, SdkEvent,
    SendPaymentRequest, SyncWalletRequest, default_config,
};

use breez_bench::events::{wait_for_claimed_event, wait_for_synced_event};
use breez_bench::stats::DurationStats;

#[derive(Parser, Debug)]
#[command(name = "claim-perf")]
#[command(about = "Concurrent claims performance testing for Breez SDK")]
struct Args {
    /// Number of pending transfers to create
    #[arg(long, default_value = "10")]
    pending_transfers: u32,

    /// Comma-separated list of concurrency levels to test (e.g., "1,2,4,8")
    #[arg(long, default_value = "1,2,4")]
    concurrency_levels: String,

    /// Minimum payment amount in satoshis
    #[arg(long, default_value = "100")]
    min_amount: u64,

    /// Maximum payment amount in satoshis
    #[arg(long, default_value = "2000")]
    max_amount: u64,

    /// PostgreSQL connection string for sender tree store (e.g., "host=localhost user=postgres dbname=sender")
    #[arg(long)]
    sender_postgres: Option<String>,

    /// PostgreSQL connection string for receiver tree store (e.g., "host=localhost user=postgres dbname=receiver")
    #[arg(long)]
    receiver_postgres: Option<String>,

    /// Clean (drop) specified PostgreSQL databases before starting the test
    #[arg(long)]
    clean_postgres: bool,
}

/// Result of a single claim benchmark run
struct ClaimBenchmarkResult {
    concurrency: u32,
    total_duration: Duration,
    successful_claims: u32,
    #[allow(dead_code)]
    failed_claims: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Set up tracing
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "claim_perf=info,\
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

    // Clean postgres databases if requested
    if args.clean_postgres {
        if let Some(conn_str) = &args.sender_postgres {
            drop_postgres_database(conn_str).await?;
        }
        if let Some(conn_str) = &args.receiver_postgres {
            drop_postgres_database(conn_str).await?;
        }
        if args.sender_postgres.is_none() && args.receiver_postgres.is_none() {
            tracing::warn!(
                "--clean-postgres specified but no --sender-postgres or --receiver-postgres provided, skipping cleanup"
            );
        }
    }

    let concurrency_levels: Vec<u32> = args
        .concurrency_levels
        .split(',')
        .map(|s| s.trim().parse::<u32>())
        .collect::<Result<Vec<_>, _>>()?;

    info!("Concurrent Claims Benchmark");
    info!("===========================");
    info!("Pending transfers: {}", args.pending_transfers);
    info!("Concurrency levels: {:?}", concurrency_levels);
    info!(
        "Amount range: {} - {} sats",
        args.min_amount, args.max_amount
    );
    info!("");

    let mut results: Vec<ClaimBenchmarkResult> = Vec::new();

    for &concurrency in &concurrency_levels {
        info!("Testing concurrency level: {}", concurrency);
        let result = run_single_claim_benchmark(
            args.pending_transfers,
            concurrency,
            args.min_amount,
            args.max_amount,
            args.sender_postgres.clone(),
            args.receiver_postgres.clone(),
        )
        .await?;
        results.push(result);
    }

    print_claim_benchmark_summary(&results, args.pending_transfers);
    Ok(())
}

/// Run a single claim benchmark iteration with a specific concurrency level
async fn run_single_claim_benchmark(
    num_transfers: u32,
    concurrency: u32,
    min_amount: u64,
    max_amount: u64,
    sender_postgres: Option<String>,
    receiver_postgres: Option<String>,
) -> Result<ClaimBenchmarkResult> {
    // Generate receiver seed upfront so we can get its address before creating the SDK
    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);

    // 1. Create sender SDK
    let sender_dir = tempfile::Builder::new()
        .prefix("claim-bench-sender")
        .tempdir()?;
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);
    let mut sender_config = default_config(Network::Regtest);
    sender_config.optimization_config.auto_enabled = false;
    let itest_sender = build_sdk_with_tree_store_config(
        sender_dir.path().to_string_lossy().to_string(),
        sender_seed,
        sender_config,
        None,
        true,
        sender_postgres,
    )
    .await?;
    let sender_sdk = Arc::new(itest_sender.sdk);
    let mut sender_events = itest_sender.events;

    // 2. Create a temporary receiver just to get the Spark address, then disconnect it
    let receiver_dir = tempfile::Builder::new()
        .prefix("claim-bench-receiver")
        .tempdir()?;
    let mut temp_receiver_config = default_config(Network::Regtest);
    temp_receiver_config.optimization_config.auto_enabled = false;
    let mut temp_receiver = build_sdk_with_tree_store_config(
        receiver_dir.path().to_string_lossy().to_string(),
        receiver_seed,
        temp_receiver_config,
        None,
        true,
        receiver_postgres.clone(),
    )
    .await?;

    // Wait for initial syncs
    info!("Waiting for sender sync...");
    wait_for_synced_event(&mut sender_events, 120).await?;
    info!("Waiting for receiver sync...");
    wait_for_synced_event(&mut temp_receiver.events, 120).await?;

    // Get receiver's Spark address
    let receiver_address = temp_receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Receiver address: {}", receiver_address);

    // Disconnect temporary receiver - we'll recreate it after sending
    temp_receiver.sdk.disconnect().await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Fund sender
    let funding = max_amount * u64::from(num_transfers) + 10_000;
    info!("Funding sender with {} sats...", funding);
    fund_sdk_via_faucet(&sender_sdk, &mut sender_events, funding).await?;

    // 4. Send N transfers in parallel (receiver is disconnected, so no interference)
    const SEND_CONCURRENCY: usize = 10;
    let mut rng = rand::thread_rng();
    let amounts: Vec<u64> = (0..num_transfers)
        .map(|_| rng.gen_range(min_amount..=max_amount))
        .collect();
    let expected_total: u64 = amounts.iter().sum();

    info!(
        "Sending {} transfers ({} sats total) with {} concurrent requests...",
        num_transfers, expected_total, SEND_CONCURRENCY
    );

    let completed = Arc::new(AtomicU32::new(0));

    let results: Vec<Result<()>> = stream::iter(amounts)
        .map(|amount| {
            let sdk = sender_sdk.clone();
            let address = receiver_address.clone();
            let completed = completed.clone();
            let total = num_transfers;
            async move {
                let prepare = sdk
                    .prepare_send_payment(PrepareSendPaymentRequest {
                        payment_request: address,
                        amount: Some(u128::from(amount)),
                        token_identifier: None,
                        conversion_options: None,
                        fee_policy: None,
                    })
                    .await?;

                sdk.send_payment(SendPaymentRequest {
                    prepare_response: prepare,
                    options: None,
                    idempotency_key: None,
                })
                .await?;

                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done.is_multiple_of(10) || done == total {
                    info!("Sent {}/{} transfers", done, total);
                }
                Ok::<(), anyhow::Error>(())
            }
        })
        .buffer_unordered(SEND_CONCURRENCY)
        .collect()
        .await;

    // Check for send errors
    let send_failed: Vec<_> = results.iter().filter(|r| r.is_err()).collect();
    let sends_succeeded = num_transfers - send_failed.len() as u32;
    if !send_failed.is_empty() {
        warn!("{} transfers failed to send", send_failed.len());
    }
    info!(
        "Sent {}/{} transfers successfully",
        sends_succeeded, num_transfers
    );

    // Disconnect sender
    sender_sdk.disconnect().await.ok();
    info!("Sender disconnected");

    // 5. Now create fresh receiver with the specific max_concurrent_claims to measure claiming
    info!(
        "Creating receiver with max_concurrent_claims={}...",
        concurrency
    );
    let mut receiver_config = default_config(Network::Regtest);
    receiver_config.optimization_config.auto_enabled = false;
    receiver_config.max_concurrent_claims = concurrency;

    // Start timing from SDK creation since claims start during initialization
    let start = Instant::now();

    let itest_receiver = build_sdk_with_tree_store_config(
        receiver_dir.path().to_string_lossy().to_string(),
        receiver_seed, // Same seed = same wallet
        receiver_config,
        None,
        true,
        receiver_postgres,
    )
    .await?;
    let receiver_sdk = itest_receiver.sdk;

    // Check how many payments already completed during SDK initialization
    let init_payments = receiver_sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Completed]),
            type_filter: Some(vec![PaymentType::Receive]),
            limit: Some(sends_succeeded + 10),
            ..Default::default()
        })
        .await?;
    let init_completed = init_payments.payments.len() as u32;
    let init_duration = start.elapsed();
    info!(
        "After SDK init ({:.2}s): {}/{} payments already completed",
        init_duration.as_secs_f64(),
        init_completed,
        sends_succeeded
    );

    // 6. Trigger additional claims if needed and wait for all to complete
    info!(
        "Triggering claims with concurrency {}, expecting {} payments...",
        concurrency, sends_succeeded
    );

    // Poll until all payments are completed or timeout
    let claim_timeout = Duration::from_secs(300); // 5 minute timeout
    let poll_interval = Duration::from_millis(500);
    let mut last_completed = 0u32;

    loop {
        // Count completed receive payments
        let payments = receiver_sdk
            .list_payments(ListPaymentsRequest {
                status_filter: Some(vec![PaymentStatus::Completed]),
                type_filter: Some(vec![PaymentType::Receive]),
                limit: Some(sends_succeeded + 10), // A bit more than expected
                ..Default::default()
            })
            .await?;

        let completed_count = payments.payments.len() as u32;

        if completed_count != last_completed {
            info!(
                "Claims progress: {}/{} completed ({:.1}s elapsed)",
                completed_count,
                sends_succeeded,
                start.elapsed().as_secs_f64()
            );
            last_completed = completed_count;
        }

        if completed_count >= sends_succeeded {
            info!("All {} claims completed!", completed_count);
            break;
        }

        if start.elapsed() >= claim_timeout {
            warn!(
                "Timeout waiting for claims: {}/{} completed",
                completed_count, sends_succeeded
            );
            break;
        }

        tokio::time::sleep(poll_interval).await;
    }

    let total_duration = start.elapsed();

    // 7. Final verification - check balance and payment count
    let final_info = receiver_sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

    let final_payments = receiver_sdk
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Completed]),
            type_filter: Some(vec![PaymentType::Receive]),
            limit: Some(sends_succeeded + 10),
            ..Default::default()
        })
        .await?;

    let successful_claims = final_payments.payments.len() as u32;
    let actual_balance = final_info.balance_sats;

    info!(
        "Final verification: {} completed payments, {} sats balance (expected {} sats)",
        successful_claims, actual_balance, expected_total
    );

    if successful_claims != sends_succeeded {
        warn!(
            "Payment count mismatch: got {} expected {}",
            successful_claims, sends_succeeded
        );
    }

    if actual_balance != expected_total {
        warn!(
            "Balance mismatch: got {} expected {}",
            actual_balance, expected_total
        );
    }

    // Cleanup
    receiver_sdk.disconnect().await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(sender_dir);
    drop(receiver_dir);

    info!(
        "Concurrency {}: {} claims in {:.2}s",
        concurrency,
        successful_claims,
        total_duration.as_secs_f64()
    );

    Ok(ClaimBenchmarkResult {
        concurrency,
        total_duration,
        successful_claims,
        failed_claims: sends_succeeded.saturating_sub(successful_claims),
    })
}

/// Fund SDK wallet via regtest faucet
async fn fund_sdk_via_faucet(
    sdk: &BreezSdk,
    events: &mut mpsc::Receiver<SdkEvent>,
    amount: u64,
) -> Result<()> {
    sdk.sync_wallet(SyncWalletRequest {}).await?;

    // Get deposit address
    let receive = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?;
    let deposit_address = receive.payment_request;
    info!("Deposit address: {}", deposit_address);

    // Fund via faucet
    let faucet = RegtestFaucet::new()?;
    let txid = faucet.fund_address(&deposit_address, amount).await?;
    info!("Faucet sent {} sats in txid: {}", amount, txid);

    // Wait for claim event
    wait_for_claimed_event(events, 180).await?;

    // Wait for balance to update
    let start = Instant::now();
    let timeout = Duration::from_secs(30);
    loop {
        sdk.sync_wallet(SyncWalletRequest {}).await?;
        let info = sdk
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

/// Print summary of claim benchmark results
fn print_claim_benchmark_summary(results: &[ClaimBenchmarkResult], num_transfers: u32) {
    println!();
    println!("============================================================");
    println!("CONCURRENT CLAIMS BENCHMARK RESULTS");
    println!("============================================================");
    println!("Pending transfers: {}", num_transfers);
    println!();
    println!("| Concurrency | Total Time | Avg/Claim | Throughput  |");
    println!("|-------------|------------|-----------|-------------|");

    for r in results {
        let avg_per_claim = r.total_duration / r.successful_claims.max(1);
        let throughput = if r.total_duration.as_secs_f64() > 0.0 {
            f64::from(r.successful_claims) / r.total_duration.as_secs_f64()
        } else {
            0.0
        };

        println!(
            "| {:>11} | {:>10} | {:>9} | {:>9.1}/s |",
            r.concurrency,
            DurationStats::format_duration(r.total_duration),
            DurationStats::format_duration(avg_per_claim),
            throughput,
        );
    }

    println!();

    // Calculate speedup vs sequential
    if let (Some(baseline), Some(best)) = (
        results.iter().find(|r| r.concurrency == 1),
        results.iter().min_by_key(|r| r.total_duration),
    ) {
        let speedup = baseline.total_duration.as_secs_f64() / best.total_duration.as_secs_f64();
        println!(
            "Best speedup: {:.2}x at concurrency {} vs sequential",
            speedup, best.concurrency
        );
    }
}
