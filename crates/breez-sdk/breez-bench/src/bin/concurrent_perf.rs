use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::Parser;
use rand::{Rng, RngCore, SeedableRng};
use tempfile::TempDir;
use tokio::sync::{Semaphore, mpsc};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use breez_sdk_itest::{RegtestFaucet, build_sdk_with_tree_store_config, drop_postgres_database};
use breez_sdk_spark::{
    BreezSdk, GetInfoRequest, Network, PrepareSendPaymentRequest, ReceivePaymentMethod,
    ReceivePaymentRequest, SdkEvent, SendPaymentRequest, SyncWalletRequest, default_config,
};

use breez_bench::events::{wait_for_claimed_event, wait_for_synced_event};
use breez_bench::stats::DurationStats;

#[derive(Parser, Debug)]
#[command(name = "concurrent-perf")]
#[command(about = "Concurrency-bounded Spark transfer throughput tester")]
struct Args {
    #[arg(long, default_value = "100")]
    total_payments: u32,

    #[arg(long, default_value = "6")]
    concurrency: u32,

    #[arg(long, default_value = "100")]
    min_amount: u64,

    #[arg(long, default_value = "2000")]
    max_amount: u64,

    #[arg(long)]
    seed: Option<u64>,

    #[arg(long)]
    no_auto_optimize: bool,

    #[arg(long, value_name = "MULTIPLICITY")]
    pre_optimize: Option<u8>,

    #[arg(long)]
    sender_postgres: Option<String>,

    #[arg(long)]
    receiver_postgres: Option<String>,

    #[arg(long)]
    clean_postgres: bool,

    #[arg(long, default_value = "1.5")]
    funding_buffer: f64,

    #[arg(long, default_value = "120")]
    bucket_secs: u64,

    #[arg(long)]
    label: Option<String>,

    #[arg(long, default_value = "1")]
    sender_instances: u32,
}

#[derive(Debug, Clone)]
struct PaymentTask {
    id: usize,
    amount: u64,
}

#[derive(Debug)]
struct PaymentResult {
    id: usize,
    amount: u64,
    duration: Duration,
    completed_at: Duration,
    success: bool,
    error: Option<String>,
}

struct BenchSdkInstance {
    sdk: BreezSdk,
    events: mpsc::Receiver<SdkEvent>,
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "concurrent_perf=info,\
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

    if args.clean_postgres {
        if let Some(conn_str) = &args.sender_postgres {
            drop_postgres_database(conn_str).await?;
        }
        if let Some(conn_str) = &args.receiver_postgres {
            drop_postgres_database(conn_str).await?;
        }
        if args.sender_postgres.is_none() && args.receiver_postgres.is_none() {
            warn!(
                "--clean-postgres specified but no --sender-postgres or --receiver-postgres provided, skipping cleanup"
            );
        }
    }

    if args.total_payments == 0 {
        bail!("--total-payments must be > 0");
    }
    if args.concurrency == 0 {
        bail!("--concurrency must be > 0");
    }
    if args.min_amount == 0 || args.max_amount < args.min_amount {
        bail!("invalid amount range");
    }

    let seed = args.seed.unwrap_or_else(|| {
        let s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        info!("Using random seed: {}", s);
        s
    });

    info!("Concurrent Spark Transfer Test");
    info!("==============================");
    info!("Total payments: {}", args.total_payments);
    info!("Concurrency:    {}", args.concurrency);
    info!(
        "Amount range:   {} - {} sats",
        args.min_amount, args.max_amount
    );
    info!("Seed:           {}", seed);
    info!("");

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    let amounts: Vec<u64> = (0..args.total_payments)
        .map(|_| rng.gen_range(args.min_amount..=args.max_amount))
        .collect();
    let total_send: u64 = amounts.iter().sum();

    let funding_amount = ((total_send as f64) * args.funding_buffer).ceil() as u64;
    let funding_amount = funding_amount.max(10_000);
    info!(
        "Total to send: {} sats; funding sender with {} sats (buffer x{:.2})",
        total_send, funding_amount, args.funding_buffer
    );

    info!("Initializing sender and receiver SDKs...");
    let (mut sender, mut receiver, sender_seed) = initialize_sdk_pair(
        args.no_auto_optimize,
        args.pre_optimize,
        args.sender_postgres.clone(),
        args.receiver_postgres.clone(),
    )
    .await?;

    info!("Waiting for sender sync...");
    wait_for_synced_event(&mut sender.events, 120).await?;
    info!("Waiting for receiver sync...");
    wait_for_synced_event(&mut receiver.events, 120).await?;

    info!(
        "Funding sender with {} sats (need {} sats min)...",
        funding_amount, total_send
    );
    fund_via_faucet(&mut sender, funding_amount, total_send).await?;

    let optimizer_duration = if args.pre_optimize.is_some() {
        Some(run_optimization(&sender.sdk, "Pre-optimization").await?)
    } else {
        None
    };

    let receiver_address = receiver
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Receiver address: {}", receiver_address);

    let mut sender_instances: Vec<BenchSdkInstance> = Vec::new();
    sender_instances.push(sender);
    if args.sender_instances > 1 {
        if args.sender_postgres.is_none() {
            bail!(
                "--sender-instances > 1 requires --sender-postgres so the instances share a tree store"
            );
        }
        info!(
            "Spawning {} additional sender SDK instance(s) sharing the same wallet + postgres tree store",
            args.sender_instances - 1
        );
        for i in 1..args.sender_instances {
            let extra = build_extra_sender(
                sender_seed,
                args.no_auto_optimize,
                args.pre_optimize,
                args.sender_postgres.clone(),
                i,
            )
            .await?;
            sender_instances.push(extra);
        }
        for (i, inst) in sender_instances.iter_mut().enumerate() {
            info!("Waiting for sender instance {} initial sync", i);
            wait_for_synced_event(&mut inst.events, 120).await?;
        }
    }

    let payments: Vec<PaymentTask> = amounts
        .into_iter()
        .enumerate()
        .map(|(id, amount)| PaymentTask { id, amount })
        .collect();

    info!("");
    info!(
        "Running {} payments via {} sender instance(s), each with concurrency={} (total in-flight cap = {})",
        payments.len(),
        sender_instances.len(),
        args.concurrency,
        sender_instances.len() as u32 * args.concurrency,
    );
    info!("");

    let sender_sdks: Vec<Arc<BreezSdk>> = sender_instances
        .into_iter()
        .map(|i| Arc::new(i.sdk))
        .collect();
    let (results, total_duration) = execute_payments(
        sender_sdks.clone(),
        receiver_address,
        payments,
        args.concurrency,
    )
    .await;

    print_summary(
        &results,
        total_duration,
        args.concurrency,
        args.bucket_secs,
        args.label.as_deref(),
        optimizer_duration,
        args.pre_optimize,
    );

    info!("Disconnecting SDKs...");
    for (i, sdk) in sender_sdks.iter().enumerate() {
        if let Err(e) = sdk.disconnect().await {
            warn!("Failed to disconnect sender instance {}: {}", i, e);
        }
    }
    if let Err(e) = receiver.sdk.disconnect().await {
        warn!("Failed to disconnect receiver SDK: {}", e);
    }
    info!("Cleanup complete");

    Ok(())
}

async fn execute_payments(
    senders: Vec<Arc<BreezSdk>>,
    receiver_address: String,
    payments: Vec<PaymentTask>,
    concurrency: u32,
) -> (Vec<PaymentResult>, Duration) {
    let per_instance_semaphores: Vec<Arc<Semaphore>> = senders
        .iter()
        .map(|_| Arc::new(Semaphore::new(concurrency as usize)))
        .collect();
    let mut handles = Vec::with_capacity(payments.len());
    let total_start = Instant::now();

    for (idx, payment) in payments.into_iter().enumerate() {
        let instance = idx % senders.len();
        let sender = senders[instance].clone();
        let receiver_address = receiver_address.clone();
        let semaphore = per_instance_semaphores[instance].clone();
        let id = payment.id;
        let amount = payment.amount;

        let total_start_inner = total_start;
        let handle = tokio::spawn(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .expect("semaphore should never close");

            println!("[START] #{} (instance {}): {} sats", id, instance, amount);
            let start = Instant::now();
            let res = execute_single_payment(&sender, &receiver_address, amount).await;
            let duration = start.elapsed();
            let completed_at = total_start_inner.elapsed();
            drop(permit);

            let success = res.is_ok();
            let error = res.err().map(|e| e.to_string());

            if success {
                println!(
                    "[OK]   #{} (instance {}): {} sats in {:.2}s",
                    id,
                    instance,
                    amount,
                    duration.as_secs_f64()
                );
            } else {
                println!(
                    "[FAIL] #{} (instance {}): {}",
                    id,
                    instance,
                    error.as_deref().unwrap_or("unknown error")
                );
            }

            PaymentResult {
                id,
                amount,
                duration,
                completed_at,
                success,
                error,
            }
        });

        handles.push(handle);
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(r) => results.push(r),
            Err(e) => warn!("Task join error: {}", e),
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

async fn execute_single_payment(
    sender: &BreezSdk,
    receiver_address: &str,
    amount: u64,
) -> Result<()> {
    let prepare = sender
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: receiver_address.to_string(),
            amount: Some(amount as u128),
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

fn print_summary(
    results: &[PaymentResult],
    total_duration: Duration,
    concurrency: u32,
    bucket_secs: u64,
    label: Option<&str>,
    optimizer_duration: Option<Duration>,
    multiplicity: Option<u8>,
) {
    println!();
    println!("============================================================");
    if let Some(l) = label {
        println!("SUMMARY [{}]", l);
    } else {
        println!("SUMMARY");
    }
    println!("============================================================");
    if let (Some(d), Some(m)) = (optimizer_duration, multiplicity) {
        println!(
            "Pre-optimization (multiplicity={}): {:.2}s",
            m,
            d.as_secs_f64()
        );
    } else if let Some(d) = optimizer_duration {
        println!("Pre-optimization: {:.2}s", d.as_secs_f64());
    }

    let total = results.len();
    let successful: Vec<_> = results.iter().filter(|r| r.success).collect();
    let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();

    println!("Total payments: {}", total);
    println!("Concurrency:    {}", concurrency);
    println!(
        "Success: {}/{} ({:.1}%)",
        successful.len(),
        total,
        if total > 0 {
            (successful.len() as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    );
    println!(
        "Failures: {}/{} ({:.1}%)",
        failed.len(),
        total,
        if total > 0 {
            (failed.len() as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    );

    let mins = total_duration.as_secs_f64() / 60.0;
    let throughput = if mins > 0.0 { total as f64 / mins } else { 0.0 };
    let success_throughput = if mins > 0.0 {
        successful.len() as f64 / mins
    } else {
        0.0
    };
    println!(
        "Wall-clock: {:.2}s; throughput: {:.1} payments/min ({:.1} successful/min)",
        total_duration.as_secs_f64(),
        throughput,
        success_throughput
    );

    if !successful.is_empty() {
        let durations: Vec<Duration> = successful.iter().map(|r| r.duration).collect();
        if let Some(s) = DurationStats::from_durations(&durations) {
            println!();
            println!("Per-payment latency (n={} successful):", successful.len());
            println!(
                "  Min: {}   Max: {}   Mean: {}",
                DurationStats::format_duration(s.min),
                DurationStats::format_duration(s.max),
                DurationStats::format_duration(s.mean),
            );
            println!(
                "  p50: {}   p95: {}   p99: {}",
                DurationStats::format_duration(s.p50),
                DurationStats::format_duration(s.p95),
                DurationStats::format_duration(s.p99),
            );
        }

        let amounts: Vec<u64> = successful.iter().map(|r| r.amount).collect();
        let total_sent: u64 = amounts.iter().sum();
        let min = amounts.iter().copied().min().unwrap_or(0);
        let max = amounts.iter().copied().max().unwrap_or(0);
        let mean = if !amounts.is_empty() {
            total_sent / amounts.len() as u64
        } else {
            0
        };
        println!();
        println!(
            "Amounts (successful): total {} sats, min {}, max {}, mean {}",
            total_sent, min, max, mean
        );
    }

    if !results.is_empty() && bucket_secs > 0 {
        let bucket = Duration::from_secs(bucket_secs);
        let total_buckets =
            ((total_duration.as_secs_f64() / bucket.as_secs_f64()).ceil() as usize).max(1);
        let mut succ_buckets = vec![0usize; total_buckets];
        let mut fail_buckets = vec![0usize; total_buckets];
        for r in results {
            let idx = ((r.completed_at.as_secs_f64() / bucket.as_secs_f64()) as usize)
                .min(total_buckets - 1);
            if r.success {
                succ_buckets[idx] += 1;
            } else {
                fail_buckets[idx] += 1;
            }
        }

        println!();
        println!(
            "Throughput histogram ({}s buckets, completion time):",
            bucket_secs
        );
        println!(
            "  {:<14} {:>5}  {:>5}  {:>10}  bar",
            "window", "ok", "fail", "rate/min"
        );
        let max_count = succ_buckets
            .iter()
            .zip(fail_buckets.iter())
            .map(|(s, f)| s + f)
            .max()
            .unwrap_or(0);
        for i in 0..total_buckets {
            let from = (i as u64) * bucket_secs;
            let to = from + bucket_secs;
            let s = succ_buckets[i];
            let f = fail_buckets[i];
            let total = s + f;
            let rate = (total as f64) * 60.0 / bucket.as_secs_f64();
            let bar_len = if max_count > 0 {
                (total as f64 / max_count as f64 * 40.0).round() as usize
            } else {
                0
            };
            let bar = "#".repeat(bar_len);
            println!(
                "  [{:>4}-{:<4}s]  {:>5}  {:>5}  {:>10.1}  {}",
                from, to, s, f, rate, bar
            );
        }
    }

    if !failed.is_empty() {
        println!();
        println!("Failed payments ({}):", failed.len());
        for r in &failed {
            println!(
                "  #{} ({} sats): {}",
                r.id,
                r.amount,
                r.error.as_deref().unwrap_or("unknown error")
            );
        }
        let mut by_error: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for r in &failed {
            let key = r.error.clone().unwrap_or_else(|| "unknown".to_string());
            *by_error.entry(key).or_insert(0) += 1;
        }
        println!();
        println!("Failure breakdown:");
        for (err, count) in &by_error {
            println!("  [{}x] {}", count, err);
        }
    }

    println!();
}

async fn initialize_sdk_pair(
    no_auto_optimize: bool,
    pre_optimize: Option<u8>,
    sender_postgres: Option<String>,
    receiver_postgres: Option<String>,
) -> Result<(BenchSdkInstance, BenchSdkInstance, [u8; 32])> {
    let sender_dir = tempfile::Builder::new()
        .prefix("concurrent-perf-sender")
        .tempdir()?;
    let sender_path = sender_dir.path().to_string_lossy().to_string();
    let mut sender_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sender_seed);

    let mut sender_config = default_config(Network::Regtest);
    if no_auto_optimize || pre_optimize.is_some() {
        sender_config.optimization_config.auto_enabled = false;
    }
    if let Some(multiplicity) = pre_optimize {
        sender_config.optimization_config.multiplicity = multiplicity;
    }
    let itest_sender = build_sdk_with_tree_store_config(
        sender_path,
        sender_seed,
        sender_config,
        None,
        true,
        sender_postgres,
    )
    .await?;

    let receiver_dir = tempfile::Builder::new()
        .prefix("concurrent-perf-receiver")
        .tempdir()?;
    let receiver_path = receiver_dir.path().to_string_lossy().to_string();
    let mut receiver_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut receiver_seed);

    let mut receiver_config = default_config(Network::Regtest);
    receiver_config.optimization_config.auto_enabled = false;
    let itest_receiver = build_sdk_with_tree_store_config(
        receiver_path,
        receiver_seed,
        receiver_config,
        None,
        true,
        receiver_postgres,
    )
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
        sender_seed,
    ))
}

async fn build_extra_sender(
    seed: [u8; 32],
    no_auto_optimize: bool,
    pre_optimize: Option<u8>,
    sender_postgres: Option<String>,
    instance_index: u32,
) -> Result<BenchSdkInstance> {
    let dir = tempfile::Builder::new()
        .prefix(format!("concurrent-perf-sender-{instance_index}").as_str())
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();

    let mut config = default_config(Network::Regtest);
    if no_auto_optimize || pre_optimize.is_some() {
        config.optimization_config.auto_enabled = false;
    }
    if let Some(multiplicity) = pre_optimize {
        config.optimization_config.multiplicity = multiplicity;
    }
    let itest =
        build_sdk_with_tree_store_config(path, seed, config, None, true, sender_postgres).await?;

    Ok(BenchSdkInstance {
        sdk: itest.sdk,
        events: itest.events,
        temp_dir: Some(dir),
    })
}

async fn run_optimization(sdk: &BreezSdk, label: &str) -> Result<Duration> {
    info!("Starting {}...", label.to_lowercase());
    sdk.start_leaf_optimization().await;

    let start = Instant::now();
    let timeout = Duration::from_secs(900);
    let poll_interval = Duration::from_millis(500);

    loop {
        let progress = sdk.get_leaf_optimization_progress();
        if !progress.is_running {
            let elapsed = start.elapsed();
            info!("{} complete in {:.2}s", label, elapsed.as_secs_f64());
            return Ok(elapsed);
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
}

async fn fund_via_faucet(
    sdk_instance: &mut BenchSdkInstance,
    target_amount: u64,
    min_required: u64,
) -> Result<()> {
    const FAUCET_MAX_PER_CALL: u64 = 50_000;
    const FAUCET_MIN_PER_CALL: u64 = 1_000;

    sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let receive = sdk_instance
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?;
    let deposit_address = receive.payment_request;
    info!("Deposit address: {}", deposit_address);

    let faucet = RegtestFaucet::new()?;

    let mut remaining = target_amount;
    let mut chunk_idx = 0u32;
    while remaining > 0 {
        sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let info = sdk_instance
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;
        if info.balance_sats >= min_required {
            info!(
                "Funded {} sats (min required {}).",
                info.balance_sats, min_required
            );
            return Ok(());
        }

        let mut chunk = remaining.min(FAUCET_MAX_PER_CALL);
        if chunk < FAUCET_MIN_PER_CALL {
            chunk = FAUCET_MIN_PER_CALL;
        }
        chunk_idx += 1;
        info!(
            "Funding chunk #{}: {} sats (balance: {}, min required: {})",
            chunk_idx, chunk, info.balance_sats, min_required
        );

        let txid = faucet.fund_address(&deposit_address, chunk).await?;
        info!("Faucet chunk #{} txid: {}", chunk_idx, txid);

        wait_for_claimed_event(&mut sdk_instance.events, 240).await?;

        sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let after = sdk_instance
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;
        info!(
            "Balance after chunk #{}: {} sats",
            chunk_idx, after.balance_sats
        );

        if after.balance_sats >= min_required {
            info!(
                "Funded {} sats (min required {}).",
                after.balance_sats, min_required
            );
            return Ok(());
        }
        remaining = remaining.saturating_sub(chunk);
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(60);
    loop {
        sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let info = sdk_instance
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?;
        if info.balance_sats >= min_required {
            info!(
                "Funded. Balance: {} sats (min required {})",
                info.balance_sats, min_required
            );
            return Ok(());
        }
        if start.elapsed() >= timeout {
            let needed = min_required.saturating_sub(info.balance_sats);
            info!(
                "Balance {} below min required {}; funding extra {} sats",
                info.balance_sats, min_required, needed
            );
            let extra = needed.clamp(FAUCET_MIN_PER_CALL, FAUCET_MAX_PER_CALL);
            let txid = faucet.fund_address(&deposit_address, extra).await?;
            info!("Top-up faucet txid: {}", txid);
            wait_for_claimed_event(&mut sdk_instance.events, 240).await?;
            sdk_instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let after = sdk_instance
                .sdk
                .get_info(GetInfoRequest {
                    ensure_synced: Some(false),
                })
                .await?;
            if after.balance_sats >= min_required {
                info!("Funded after top-up: {} sats", after.balance_sats);
                return Ok(());
            }
            bail!(
                "Failed to reach min required balance {}: have {}",
                min_required,
                after.balance_sats
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
