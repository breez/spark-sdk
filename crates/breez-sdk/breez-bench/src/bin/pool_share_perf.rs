//! Demonstrates the connection-count economics of sharing a single
//! `PostgresConnectionPool` across many SDK instances vs giving each
//! instance its own pool.
//!
//! This bench has no payments, faucet, or operator round-trips — just
//! SDK init + a no-op query through every SDK to force pool acquisition,
//! plus snapshots of `pg_stat_activity` taken from a separate connection.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use futures::future::try_join_all;
use rand::RngCore;
use tracing::info;
use tracing_subscriber::EnvFilter;

use breez_sdk_itest::{drop_postgres_database, ensure_postgres_database_exists};
use breez_sdk_spark::{
    BreezSdk, GetInfoRequest, Network, SdkBuilder, SdkContext, SdkContextConfig, Seed,
    default_config, default_postgres_storage_config, new_shared_sdk_context,
};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Mode {
    /// One pool, shared across every SDK instance.
    Shared,
    /// One pool per SDK instance, all targeting the same database.
    Separate,
}

#[derive(Parser, Debug)]
#[command(name = "pool-share-perf")]
#[command(about = "Measures DB connection-count economics of shared vs separate pools")]
struct Args {
    /// Postgres connection string (key-value form: `host=… port=… user=… password=… dbname=…`).
    #[arg(long)]
    postgres: String,

    /// Number of SDK instances to spin up.
    #[arg(long, default_value = "32")]
    instances: u32,

    /// Pool topology under test.
    #[arg(long, value_enum, default_value = "shared")]
    mode: Mode,

    /// `max_pool_size` to apply to each pool. Lower this when running
    /// `--mode separate` with many instances to fit under Postgres'
    /// `max_connections` (default 100).
    #[arg(long, default_value = "8")]
    max_pool_size: u32,

    /// Drop and recreate the database before the run.
    #[arg(long)]
    clean: bool,

    /// Optional label for the printed summary.
    #[arg(long)]
    label: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "pool_share_perf=info,\
             breez_sdk_spark=error,\
             breez_sdk_itest=error,\
             warn",
        )
    });
    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(filter)
        .init();

    let args = Args::parse();
    if args.instances == 0 {
        bail!("--instances must be > 0");
    }

    if args.clean {
        drop_postgres_database(&args.postgres).await?;
    }
    // Make sure the target database exists before we snapshot or build any
    // SDK (the SDK's pool-creation path expects the DB to exist).
    ensure_postgres_database_exists(&args.postgres).await?;

    info!("Pool sharing economics test");
    info!("===========================");
    info!("Mode:           {:?}", args.mode);
    info!("Instances:      {}", args.instances);
    info!("Max pool size:  {}", args.max_pool_size);
    info!("");

    let dbname = extract_dbname(&args.postgres).unwrap_or_else(|| "postgres".to_string());

    // Snapshot before anything is built.
    let conn_idle_baseline = pg_connection_count(&args.postgres, &dbname).await?;

    // Build one shared SdkContext up-front (or None for the per-instance path).
    let shared_context: Option<Arc<SdkContext>> = match args.mode {
        Mode::Shared => Some(make_context(&args.postgres, args.max_pool_size).await?),
        Mode::Separate => None,
    };

    info!("Building {} SDK instance(s)...", args.instances);
    let init_start = Instant::now();
    let mut sdks: Vec<BreezSdk> = Vec::with_capacity(args.instances as usize);
    for _ in 0..args.instances {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);

        let mut config = default_config(Network::Regtest);
        config.api_key = None;
        config.real_time_sync_server_url = None;
        config.lnurl_domain = None;
        config.leaf_optimization_config.auto_enabled = false;

        let mut builder = SdkBuilder::new(config, Seed::Entropy(seed.to_vec()));
        builder = match (&shared_context, args.mode) {
            (Some(ctx), _) => builder.with_shared_context(Arc::clone(ctx)),
            (None, Mode::Separate) => {
                let ctx = make_context(&args.postgres, args.max_pool_size).await?;
                builder.with_shared_context(ctx)
            }
            (None, Mode::Shared) => unreachable!(),
        };
        let sdk = builder.build().await?;
        sdks.push(sdk);
    }
    let init_duration = init_start.elapsed();

    // After init, before any queries: how many connections are sitting idle?
    let conn_after_init = pg_connection_count(&args.postgres, &dbname).await?;

    // Force every SDK to actually acquire a connection at the same time —
    // this is the peak load measurement.
    info!("Firing concurrent get_info() through every SDK...");
    let stress_start = Instant::now();
    let futs = sdks.iter().map(|sdk| {
        sdk.get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
    });
    try_join_all(futs).await?;
    let stress_duration = stress_start.elapsed();

    let conn_peak = pg_connection_count(&args.postgres, &dbname).await?;

    // Brief settle, then re-snapshot to see how many connections the pool retains.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let conn_idle_after_stress = pg_connection_count(&args.postgres, &dbname).await?;

    info!("Disconnecting all SDKs...");
    for sdk in &sdks {
        sdk.disconnect().await?;
    }
    drop(sdks);

    // For separate-context mode, dropping the SDKs drops their contexts (and
    // pools) — connections close. For shared-context mode, the context is
    // still alive (held by `shared_context`), so connections stay until we
    // drop it below.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let conn_after_disconnect = pg_connection_count(&args.postgres, &dbname).await?;

    drop(shared_context);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let conn_after_pool_drop = pg_connection_count(&args.postgres, &dbname).await?;

    print_summary(
        args.mode,
        args.instances,
        args.max_pool_size,
        args.label.as_deref(),
        init_duration,
        stress_duration,
        Snapshots {
            idle_baseline: conn_idle_baseline,
            after_init: conn_after_init,
            peak: conn_peak,
            idle_after_stress: conn_idle_after_stress,
            after_disconnect: conn_after_disconnect,
            after_pool_drop: conn_after_pool_drop,
        },
    );

    Ok(())
}

async fn make_context(conn_str: &str, max_pool_size: u32) -> Result<Arc<SdkContext>> {
    let mut cfg = default_postgres_storage_config(conn_str.to_string());
    cfg.max_pool_size = max_pool_size;
    Ok(new_shared_sdk_context(SdkContextConfig {
        storage: Some(breez_sdk_spark::postgres_storage(cfg)?),
        ..SdkContextConfig::new(Network::Regtest)
    })
    .await?)
}

struct Snapshots {
    idle_baseline: i64,
    after_init: i64,
    peak: i64,
    idle_after_stress: i64,
    after_disconnect: i64,
    after_pool_drop: i64,
}

#[allow(clippy::too_many_arguments)]
fn print_summary(
    mode: Mode,
    instances: u32,
    max_pool_size: u32,
    label: Option<&str>,
    init: Duration,
    stress: Duration,
    snap: Snapshots,
) {
    println!();
    println!("============================================================");
    if let Some(l) = label {
        println!("SUMMARY [{l}]");
    } else {
        println!("SUMMARY");
    }
    println!("============================================================");
    println!("Mode:                          {mode:?}");
    println!("Instances:                     {instances}");
    println!("max_pool_size per pool:        {max_pool_size}");
    println!();
    println!("SDK init time (sequential):    {init:?}");
    println!("Concurrent get_info() time:    {stress:?}");
    println!();
    println!("DB connections to this database (delta from baseline):");
    println!("  Baseline (before init):      {} ", snap.idle_baseline);
    println!(
        "  After init, no queries:      {}  ({:+})",
        snap.after_init,
        snap.after_init - snap.idle_baseline
    );
    println!(
        "  Peak (concurrent queries):   {}  ({:+})",
        snap.peak,
        snap.peak - snap.idle_baseline
    );
    println!(
        "  Idle after stress:           {}  ({:+})",
        snap.idle_after_stress,
        snap.idle_after_stress - snap.idle_baseline
    );
    println!(
        "  After SDK disconnect:        {}  ({:+})",
        snap.after_disconnect,
        snap.after_disconnect - snap.idle_baseline
    );
    println!(
        "  After pool dropped:          {}  ({:+})",
        snap.after_pool_drop,
        snap.after_pool_drop - snap.idle_baseline
    );
    println!("============================================================");
    println!();
}

/// Snapshots the number of connections currently open to the named database,
/// excluding the snapshotting connection itself. Uses a fresh tokio_postgres
/// client (not the SDK's pool) so the snapshot itself adds at most 1 to the
/// count and we explicitly exclude it.
async fn pg_connection_count(conn_str: &str, dbname: &str) -> Result<i64> {
    // Connect to the same database we're measuring. We exclude pg_backend_pid()
    // to remove this snapshot's own connection from the count.
    let (client, connection) = tokio_postgres::connect(conn_str, tokio_postgres::NoTls).await?;
    let conn_handle = tokio::spawn(connection);
    let row = client
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
            &[&dbname],
        )
        .await?;
    let count: i64 = row.get(0);
    drop(client);
    let _ = conn_handle.await;
    Ok(count)
}

/// Extract the `dbname=…` value from a key-value Postgres connection string.
fn extract_dbname(conn_str: &str) -> Option<String> {
    for part in conn_str.split_whitespace() {
        if let Some((key, value)) = part.split_once('=')
            && key == "dbname"
        {
            return Some(value.to_string());
        }
    }
    None
}
