//! Memory Baseline Test for Rust SDK
//!
//! This test exercises the SDK continuously to establish a memory baseline for
//! comparison with Go bindings. It tracks:
//! - Resident set size (RSS) from /proc/self/statm (Linux) or mach_task_info (macOS)
//! - Heap allocation stats via jemalloc
//!
//! Run with: `cargo test -p breez-sdk-itest memory_test -- --ignored --nocapture`
//!
//! This test is ignored by default because it runs for an extended duration.

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use std::fmt;
use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tempdir::TempDir;
use tracing::info;

/// Parse a 32-byte seed from a hex-encoded environment variable.
fn parse_seed_from_env(env_var: &str) -> Result<[u8; 32]> {
    let hex_str = std::env::var(env_var)
        .map_err(|_| anyhow!("{} env var required (64 hex chars)", env_var))?;
    let decoded = hex::decode(&hex_str)
        .map_err(|_| anyhow!("{} must be valid hex", env_var))?;
    if decoded.len() != 32 {
        return Err(anyhow!("{} must be exactly 64 hex chars (32 bytes)", env_var));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&decoded);
    Ok(seed)
}

/// Memory statistics sample
#[derive(Debug, Clone)]
struct MemoryStats {
    timestamp: Instant,
    wall_clock: DateTime<Utc>,
    rss_bytes: u64,
    heap_allocated: u64,
    heap_resident: u64,
    payment_count: i64,
}

impl MemoryStats {
    fn rss_mb(&self) -> f64 {
        self.rss_bytes as f64 / 1024.0 / 1024.0
    }

    fn heap_allocated_mb(&self) -> f64 {
        self.heap_allocated as f64 / 1024.0 / 1024.0
    }
}

/// Get jemalloc heap statistics (allocated, resident)
fn get_jemalloc_stats() -> (u64, u64) {
    use tikv_jemalloc_ctl::{epoch, stats};

    // Advance epoch to get fresh stats
    let _ = epoch::advance();

    let allocated = stats::allocated::read().unwrap_or(0) as u64;
    let resident = stats::resident::read().unwrap_or(0) as u64;

    (allocated, resident)
}

/// Get current RSS (Resident Set Size) in bytes
fn get_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        // Read from /proc/self/statm
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = statm.split_whitespace().collect();
            if parts.len() >= 2 {
                // Second field is RSS in pages
                if let Ok(pages) = parts[1].parse::<u64>() {
                    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                    return pages * page_size;
                }
            }
        }
        0
    }

    #[cfg(target_os = "macos")]
    {
        use std::mem::MaybeUninit;

        // Use mach_task_basic_info on macOS
        let task = unsafe { libc::mach_task_self() };
        let mut info = MaybeUninit::<mach_task_basic_info>::uninit();
        let mut count = (std::mem::size_of::<mach_task_basic_info>()
            / std::mem::size_of::<libc::integer_t>()) as u32;

        let result = unsafe {
            libc::task_info(
                task,
                MACH_TASK_BASIC_INFO,
                info.as_mut_ptr().cast(),
                &mut count,
            )
        };

        if result == libc::KERN_SUCCESS {
            let info = unsafe { info.assume_init() };
            return info.resident_size as u64;
        }
        0
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Fallback: return 0 on unsupported platforms
        0
    }
}

#[cfg(target_os = "macos")]
const MACH_TASK_BASIC_INFO: libc::task_flavor_t = 20;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Copy, Clone)]
struct mach_task_basic_info {
    virtual_size: u64,
    resident_size: u64,
    resident_size_max: u64,
    user_time: libc::time_value_t,
    system_time: libc::time_value_t,
    policy: libc::policy_t,
    suspend_count: libc::integer_t,
}

/// Memory tracker that samples memory usage over time
struct MemoryTracker {
    samples: Vec<MemoryStats>,
    payment_count: Arc<AtomicI64>,
    start_time: Instant,
    csv_file: Option<File>,
}

impl MemoryTracker {
    fn new(payment_count: Arc<AtomicI64>) -> Self {
        Self {
            samples: Vec::with_capacity(1000),
            payment_count,
            start_time: Instant::now(),
            csv_file: None,
        }
    }

    fn init_csv(&mut self, path: &str) -> Result<()> {
        let mut file = File::create(path)?;
        // Write header
        writeln!(
            file,
            "timestamp,elapsed_sec,rss_bytes,heap_allocated_bytes,heap_resident_bytes,payments"
        )?;
        file.flush()?;
        self.csv_file = Some(file);
        info!("CSV file initialized: {}", path);
        Ok(())
    }

    fn sample(&mut self) {
        let (heap_allocated, heap_resident) = get_jemalloc_stats();
        let stats = MemoryStats {
            timestamp: Instant::now(),
            wall_clock: Utc::now(),
            rss_bytes: get_rss_bytes(),
            heap_allocated,
            heap_resident,
            payment_count: self.payment_count.load(Ordering::Relaxed),
        };

        let elapsed = stats.timestamp.duration_since(self.start_time);
        let delta_str = if let Some(prev) = self.samples.last() {
            let delta = stats.rss_bytes as i64 - prev.rss_bytes as i64;
            let delta_mb = delta as f64 / 1024.0 / 1024.0;
            if delta >= 0 {
                format!(" (+{delta_mb:.2}MB)")
            } else {
                format!(" ({delta_mb:.2}MB)")
            }
        } else {
            String::new()
        };

        info!(
            "[{:02}:{:02}:{:02}] RSS={:.2}MB{} HeapAlloc={:.2}MB Payments={}",
            elapsed.as_secs() / 3600,
            (elapsed.as_secs() % 3600) / 60,
            elapsed.as_secs() % 60,
            stats.rss_mb(),
            delta_str,
            stats.heap_allocated_mb(),
            stats.payment_count
        );

        // Write to CSV in real-time if configured
        if let Some(ref mut file) = self.csv_file {
            let _ = writeln!(
                file,
                "{},{:.2},{},{},{},{}",
                stats.wall_clock.to_rfc3339(),
                elapsed.as_secs_f64(),
                stats.rss_bytes,
                stats.heap_allocated,
                stats.heap_resident,
                stats.payment_count
            );
            let _ = file.flush();
        }

        self.samples.push(stats);
    }

    fn generate_report(&self) -> MemoryReport {
        if self.samples.len() < 2 {
            return MemoryReport::default();
        }

        let start_rss = self.samples.first().unwrap().rss_mb();
        let end_rss = self.samples.last().unwrap().rss_mb();
        let max_rss = self
            .samples
            .iter()
            .map(|s| s.rss_mb())
            .fold(0.0f64, |a, b| a.max(b));
        let start_heap = self.samples.first().unwrap().heap_allocated_mb();
        let end_heap = self.samples.last().unwrap().heap_allocated_mb();
        let max_heap = self
            .samples
            .iter()
            .map(|s| s.heap_allocated_mb())
            .fold(0.0f64, |a, b| a.max(b));
        let total_payments = self.samples.last().unwrap().payment_count;

        // Linear regression: y = RSS (KB), x = time (minutes)
        let n = self.samples.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xy = 0.0;
        let mut sum_x2 = 0.0;
        let mut sum_y2 = 0.0;

        for sample in &self.samples {
            let x = sample
                .timestamp
                .duration_since(self.start_time)
                .as_secs_f64()
                / 60.0;
            let y = sample.rss_bytes as f64 / 1024.0; // KB
            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_x2 += x * x;
            sum_y2 += y * y;
        }

        let slope = if (n * sum_x2 - sum_x * sum_x) != 0.0 {
            (n * sum_xy - sum_x * sum_y) / (n * sum_x2 - sum_x * sum_x)
        } else {
            0.0
        };

        let numerator = n * sum_xy - sum_x * sum_y;
        let denom1 = n * sum_x2 - sum_x * sum_x;
        let denom2 = n * sum_y2 - sum_y * sum_y;
        let r_squared = if denom1 > 0.0 && denom2 > 0.0 {
            (numerator * numerator) / (denom1 * denom2)
        } else {
            0.0
        };

        // Detect leak: positive slope > 100KB/min with R² > 0.7
        let leak_detected = slope > 100.0 && r_squared > 0.7;

        MemoryReport {
            start_rss_mb: start_rss,
            end_rss_mb: end_rss,
            max_rss_mb: max_rss,
            start_heap_mb: start_heap,
            end_heap_mb: end_heap,
            max_heap_mb: max_heap,
            total_payments,
            slope_kb_per_min: slope,
            r_squared,
            leak_detected,
        }
    }

    fn export_csv(&self, path: &str) -> Result<()> {
        // If we were writing in real-time, just log completion
        if self.csv_file.is_some() {
            info!("CSV export complete: {}", path);
            return Ok(());
        }

        // Fallback: write all samples at once (legacy mode)
        let mut file = File::create(path)?;

        // Write header (matching Go format)
        writeln!(
            file,
            "timestamp,elapsed_sec,rss_bytes,heap_allocated_bytes,heap_resident_bytes,payments"
        )?;

        for sample in &self.samples {
            let elapsed = sample.timestamp.duration_since(self.start_time).as_secs_f64();
            writeln!(
                file,
                "{},{:.2},{},{},{},{}",
                sample.wall_clock.to_rfc3339(),
                elapsed,
                sample.rss_bytes,
                sample.heap_allocated,
                sample.heap_resident,
                sample.payment_count
            )?;
        }

        info!("CSV exported to: {}", path);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct MemoryReport {
    start_rss_mb: f64,
    end_rss_mb: f64,
    max_rss_mb: f64,
    start_heap_mb: f64,
    end_heap_mb: f64,
    max_heap_mb: f64,
    total_payments: i64,
    slope_kb_per_min: f64,
    r_squared: f64,
    leak_detected: bool,
}

impl MemoryReport {
    fn print(&self) {
        info!("\n=== Memory Trend Report ===");
        info!(
            "RSS: {:.2}MB -> {:.2}MB (max: {:.2}MB)",
            self.start_rss_mb, self.end_rss_mb, self.max_rss_mb
        );
        info!(
            "Heap: {:.2}MB -> {:.2}MB (max: {:.2}MB)",
            self.start_heap_mb, self.end_heap_mb, self.max_heap_mb
        );
        info!("Total payments: {}", self.total_payments);
        info!(
            "Linear regression (RSS): {:.1} KB/min (R²={:.2})",
            self.slope_kb_per_min, self.r_squared
        );

        if self.leak_detected {
            info!(
                "!!! POTENTIAL LEAK DETECTED: Consistent linear growth: +{:.1} KB/min (R²={:.2})",
                self.slope_kb_per_min, self.r_squared
            );
        } else {
            info!("Verdict: No significant leak detected");
        }
    }
}

/// Payment type for the memory test
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PaymentType {
    #[default]
    Spark,
    Lightning,
    Both,
}

impl fmt::Display for PaymentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaymentType::Spark => write!(f, "spark"),
            PaymentType::Lightning => write!(f, "lightning"),
            PaymentType::Both => write!(f, "both"),
        }
    }
}

impl PaymentType {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "lightning" | "ln" => PaymentType::Lightning,
            "both" | "all" => PaymentType::Both,
            _ => PaymentType::Spark,
        }
    }
}

/// Configuration for the memory test
struct MemTestConfig {
    duration: Duration,
    payment_interval: Duration,
    memory_interval: Duration,
    amount_sats: u64,
    payment_type: PaymentType,
    reconnect_cycles: bool,
    reconnect_every: i64,
    csv_file: Option<String>,
    extra_instances: usize,
    frequent_sync: bool,
    payment_history_queries: bool,
    payment_history_limit: Option<u32>,
}

impl Default for MemTestConfig {
    fn default() -> Self {
        Self {
            // Default: 10 minutes for CI, override with env var for longer runs
            duration: Duration::from_secs(
                std::env::var("MEMTEST_DURATION_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(600),
            ),
            payment_interval: Duration::from_secs(5),
            memory_interval: Duration::from_secs(30),
            amount_sats: 1000,
            payment_type: std::env::var("MEMTEST_PAYMENT_TYPE")
                .map(|s| PaymentType::from_str(&s))
                .unwrap_or_default(),
            reconnect_cycles: std::env::var("MEMTEST_RECONNECT_CYCLES").is_ok(),
            reconnect_every: std::env::var("MEMTEST_RECONNECT_EVERY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100),
            csv_file: std::env::var("MEMTEST_CSV_FILE").ok(),
            extra_instances: std::env::var("MEMTEST_EXTRA_INSTANCES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            frequent_sync: std::env::var("MEMTEST_FREQUENT_SYNC").is_ok(),
            payment_history_queries: std::env::var("MEMTEST_PAYMENT_HISTORY_QUERIES").is_ok(),
            payment_history_limit: std::env::var("MEMTEST_PAYMENT_HISTORY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok()),
        }
    }
}

/// Manages an SDK instance with support for reconnection
struct PersistentSdk {
    seed: [u8; 32],
    storage_dir: String,
    sdk: Option<BreezSdk>,
    _temp_dir: TempDir,
}

impl PersistentSdk {
    async fn new(name: &str, seed: [u8; 32]) -> Result<Self> {
        let temp_dir = TempDir::new(&format!("memtest-{name}"))?;
        let storage_dir = temp_dir.path().to_string_lossy().to_string();

        let mut instance = Self {
            seed,
            storage_dir,
            sdk: None,
            _temp_dir: temp_dir,
        };

        instance.connect().await?;
        Ok(instance)
    }

    async fn connect(&mut self) -> Result<()> {
        let mut config = default_config(Network::Regtest);
        config.api_key = None;
        config.lnurl_domain = None;
        config.prefer_spark_over_lightning = true;
        config.sync_interval_secs = 5;
        config.real_time_sync_server_url = None;

        let seed = Seed::Entropy(self.seed.to_vec());
        let builder = SdkBuilder::new(config, seed).with_default_storage(self.storage_dir.clone());
        let sdk = builder.build().await?;

        // Ensure initial sync completes
        let _ = sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(true),
            })
            .await?;

        self.sdk = Some(sdk);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(sdk) = self.sdk.take() {
            sdk.disconnect().await?;
        }
        Ok(())
    }

    async fn reconnect(&mut self) -> Result<()> {
        self.disconnect().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.connect().await
    }

    fn sdk(&self) -> &BreezSdk {
        self.sdk.as_ref().expect("SDK not connected")
    }

    async fn spark_address(&self) -> Result<String> {
        Ok(self
            .sdk()
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request)
    }

    async fn bitcoin_address(&self) -> Result<String> {
        Ok(self
            .sdk()
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::BitcoinAddress,
            })
            .await?
            .payment_request)
    }

    async fn create_bolt11_invoice(&self, amount_sats: u64) -> Result<String> {
        Ok(self
            .sdk()
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::Bolt11Invoice {
                    description: "memtest payment".to_string(),
                    amount_sats: Some(amount_sats),
                    expiry_secs: None,
                },
            })
            .await?
            .payment_request)
    }

    async fn get_balance(&self) -> Result<u64> {
        Ok(self
            .sdk()
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats)
    }
}

/// Send a Spark payment from sender to receiver
async fn send_spark_payment(sender: &BreezSdk, receiver_addr: &str, amount_sats: u64) -> Result<()> {
    let prepare = sender
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: receiver_addr.to_string(),
            pay_amount: Some(PayAmount::Bitcoin { amount_sats }),
            conversion_options: None,
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

/// Send a Lightning payment: receiver creates invoice, sender pays it
async fn send_lightning_payment(
    sender: &BreezSdk,
    receiver: &PersistentSdk,
    amount_sats: u64,
) -> Result<()> {
    // Receiver creates a Bolt11 invoice
    let invoice = receiver.create_bolt11_invoice(amount_sats).await?;

    // Sender pays the invoice
    let prepare = sender
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: invoice,
            pay_amount: None, // Amount is in the invoice
            conversion_options: None,
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

/// Check balance and fund from faucet if too low (handles Lightning fee drain)
async fn check_and_refund_if_needed(
    sdk: &PersistentSdk,
    name: &str,
    faucet: &RegtestFaucet,
) -> Result<()> {
    let min_balance = 5000u64;
    let balance = sdk.get_balance().await?;

    if balance >= min_balance {
        return Ok(());
    }

    info!(
        "\n[Refunding] {} balance too low ({} sats), requesting funds from faucet...",
        name, balance
    );

    // Fund from faucet (max 50k sats)
    let btc_addr = sdk.bitcoin_address().await?;
    faucet.fund_address(&btc_addr, 50_000).await?;

    // Wait for funds to be confirmed
    let target_balance = 10_000u64;
    let max_wait = Duration::from_secs(300);
    let poll_interval = Duration::from_secs(5);
    let start = Instant::now();

    loop {
        sdk.sdk().sync_wallet(SyncWalletRequest {}).await?;
        let new_balance = sdk.get_balance().await?;

        if new_balance >= target_balance {
            info!("[Refunding] {} funded: {} sats\n", name, new_balance);
            return Ok(());
        }

        if start.elapsed() > max_wait {
            anyhow::bail!(
                "Timeout waiting for {} funds: {} sats",
                name,
                new_balance
            );
        }

        info!(
            "[Refunding] Waiting for {} funds... {} sats",
            name, new_balance
        );
        tokio::time::sleep(poll_interval).await;
    }
}

/// Memory leak baseline test
///
/// This test runs for an extended period (configurable via MEMTEST_DURATION_SECS)
/// and tracks memory usage to establish a baseline for comparison with Go bindings.
///
/// # Environment Variables
/// - `MEMTEST_DURATION_SECS`: Test duration in seconds (default: 600 = 10 min)
/// - `MEMTEST_PAYMENT_TYPE`: Payment type: spark, lightning, or both (default: spark)
/// - `MEMTEST_RECONNECT_CYCLES`: Enable disconnect/reconnect cycles if set
/// - `MEMTEST_RECONNECT_EVERY`: Number of payments between reconnects (default: 100)
///
/// # Run
/// ```bash
/// cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "Long-running memory test - run with --ignored"]
async fn memory_baseline_test() -> Result<()> {
    // Initialize tracing - only show memory_test logs, suppress SDK verbosity
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("warn".parse().unwrap())
                .add_directive("memory_test=info".parse().unwrap()),
        )
        .init();

    info!("=== Starting Rust Memory Baseline Test ===");

    let config = MemTestConfig::default();
    info!("Duration: {:?}", config.duration);
    info!("Payment interval: {:?}", config.payment_interval);
    info!("Memory interval: {:?}", config.memory_interval);
    info!("Payment type: {}", config.payment_type);
    info!("Reconnect cycles: {}", config.reconnect_cycles);
    info!("Extra instances: {}", config.extra_instances);
    info!("Frequent sync: {}", config.frequent_sync);
    info!(
        "Payment history queries: {} (limit: {})",
        config.payment_history_queries,
        config.payment_history_limit.map_or("unlimited".to_string(), |l| l.to_string())
    );

    // Seeds from environment variables (required)
    let alice_seed = parse_seed_from_env("ALICE_SEED")?;
    let bob_seed = parse_seed_from_env("BOB_SEED")?;

    // Create persistent SDK instances
    let mut alice = PersistentSdk::new("alice", alice_seed).await?;
    let mut bob = PersistentSdk::new("bob", bob_seed).await?;

    // Create extra instances (same seeds, different storage dirs)
    let mut extra_alices = Vec::new();
    let mut extra_bobs = Vec::new();
    for i in 0..config.extra_instances {
        extra_alices.push(PersistentSdk::new(&format!("extra-alice-{i}"), alice_seed).await?);
        extra_bobs.push(PersistentSdk::new(&format!("extra-bob-{i}"), bob_seed).await?);
    }

    info!("Alice and Bob SDKs initialized");
    if config.extra_instances > 0 {
        info!("Extra instances initialized: {} alice, {} bob", extra_alices.len(), extra_bobs.len());
    }

    // Fund both using Bitcoin addresses (faucet max 50k sats)
    let faucet = RegtestFaucet::default();

    let alice_btc_addr = alice.bitcoin_address().await?;
    info!("Funding Alice at {}", &alice_btc_addr[..20]);
    faucet.fund_address(&alice_btc_addr, 50_000).await?;

    let bob_btc_addr = bob.bitcoin_address().await?;
    info!("Funding Bob at {}", &bob_btc_addr[..20]);
    faucet.fund_address(&bob_btc_addr, 50_000).await?;

    // Wait for funding to be confirmed
    info!("Waiting for funds to be confirmed...");
    let min_balance = 10_000u64;
    let max_wait = Duration::from_secs(300);
    let poll_interval = Duration::from_secs(5);
    let funding_start = Instant::now();

    loop {
        alice.sdk().sync_wallet(SyncWalletRequest {}).await?;
        bob.sdk().sync_wallet(SyncWalletRequest {}).await?;

        let alice_balance = alice.get_balance().await?;
        let bob_balance = bob.get_balance().await?;

        if alice_balance >= min_balance && bob_balance >= min_balance {
            info!(
                "Funds confirmed: Alice={} sats, Bob={} sats",
                alice_balance, bob_balance
            );
            break;
        }

        if funding_start.elapsed() > max_wait {
            anyhow::bail!(
                "Timeout waiting for funds: Alice={}, Bob={}",
                alice_balance,
                bob_balance
            );
        }

        info!(
            "Waiting for funds... Alice={} sats, Bob={} sats",
            alice_balance, bob_balance
        );
        tokio::time::sleep(poll_interval).await;
    }

    // Get addresses for payments
    let alice_spark_address = alice.spark_address().await?;
    let bob_spark_address = bob.spark_address().await?;
    info!("Alice Spark: {}", &alice_spark_address[..20]);
    info!("Bob Spark: {}", &bob_spark_address[..20]);

    // Setup counters
    let payment_count = Arc::new(AtomicI64::new(0));
    let mut tracker = MemoryTracker::new(Arc::clone(&payment_count));

    // Initialize CSV for real-time writing if configured
    if let Some(csv_path) = &config.csv_file {
        tracker.init_csv(csv_path)?;
    }

    // Take initial sample
    info!("\n=== Starting {} test ===", config.duration.as_secs());
    tracker.sample();

    let start = Instant::now();
    let mut last_memory_sample = Instant::now();
    let mut alice_turn = true;
    let mut last_reconnect_count: i64 = 0;

    // Payment loop
    while start.elapsed() < config.duration {
        // Sample memory at intervals
        if last_memory_sample.elapsed() >= config.memory_interval {
            tracker.sample();
            last_memory_sample = Instant::now();
        }

        // Check for reconnect cycle
        let current_count = payment_count.load(Ordering::Relaxed);
        if config.reconnect_cycles
            && current_count > 0
            && current_count >= last_reconnect_count + config.reconnect_every
        {
            info!("\n=== Reconnect cycle at payment {} ===", current_count);
            last_reconnect_count = current_count;

            // Reconnect base instances
            alice.reconnect().await?;
            bob.reconnect().await?;

            // Reconnect extras
            for extra in &mut extra_alices {
                extra.reconnect().await?;
            }
            for extra in &mut extra_bobs {
                extra.reconnect().await?;
            }

            // Wait for sync
            tokio::time::sleep(Duration::from_secs(5)).await;
            alice.sdk().sync_wallet(SyncWalletRequest {}).await?;
            bob.sdk().sync_wallet(SyncWalletRequest {}).await?;

            info!("=== Reconnected ===\n");
        }

        // Perform frequent sync if enabled
        if config.frequent_sync {
            alice.sdk().sync_wallet(SyncWalletRequest {}).await?;
            bob.sdk().sync_wallet(SyncWalletRequest {}).await?;
        }

        // Query payment history if enabled
        if config.payment_history_queries {
            let req = ListPaymentsRequest {
                limit: config.payment_history_limit,
                ..Default::default()
            };
            alice.sdk().list_payments(req.clone()).await?;
            bob.sdk().list_payments(req).await?;
        }

        // Determine payment type for this iteration
        let use_lightning = match config.payment_type {
            PaymentType::Spark => false,
            PaymentType::Lightning => true,
            PaymentType::Both => current_count % 2 == 1, // Alternate
        };

        // Check and refund sender if balance is too low (Lightning fees drain funds)
        let (sender_sdk, sender_name) = if alice_turn {
            (&alice, "alice")
        } else {
            (&bob, "bob")
        };
        if let Err(e) = check_and_refund_if_needed(sender_sdk, sender_name, &faucet).await {
            info!("Refund error: {:?}", e);
            // Continue anyway, payment may still succeed
        }

        // Send payment
        let result = if use_lightning {
            // Lightning: receiver creates invoice, sender pays
            if alice_turn {
                send_lightning_payment(alice.sdk(), &bob, config.amount_sats).await
            } else {
                send_lightning_payment(bob.sdk(), &alice, config.amount_sats).await
            }
        } else {
            // Spark: direct transfer using Spark address
            if alice_turn {
                send_spark_payment(alice.sdk(), &bob_spark_address, config.amount_sats).await
            } else {
                send_spark_payment(bob.sdk(), &alice_spark_address, config.amount_sats).await
            }
        };

        let payment_type_str = if use_lightning { "Lightning" } else { "Spark" };

        match result {
            Ok(()) => {
                let count = payment_count.fetch_add(1, Ordering::Relaxed) + 1;
                info!(
                    "[Payment {}] {} -> {}: {} sats via {}",
                    count,
                    if alice_turn { "alice" } else { "bob" },
                    if alice_turn { "bob" } else { "alice" },
                    config.amount_sats,
                    payment_type_str
                );
            }
            Err(e) => {
                info!("Payment error: {:?}", e);
            }
        }

        alice_turn = !alice_turn;
        tokio::time::sleep(config.payment_interval).await;
    }

    // Final sample
    tracker.sample();

    // Generate and print report
    let report = tracker.generate_report();
    report.print();

    // Export CSV if configured
    if let Some(csv_path) = &config.csv_file {
        tracker.export_csv(csv_path)?;
    }

    // Cleanup - disconnect extras first, then base
    for extra in &mut extra_alices {
        extra.disconnect().await?;
    }
    for extra in &mut extra_bobs {
        extra.disconnect().await?;
    }
    alice.disconnect().await?;
    bob.disconnect().await?;

    // Assert no leak detected
    assert!(
        !report.leak_detected,
        "Memory leak detected: {:.1} KB/min growth (R²={:.2})",
        report.slope_kb_per_min,
        report.r_squared
    );

    Ok(())
}
