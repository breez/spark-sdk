//! Selects which backend to use for spark-wallet's session / tree / token
//! output stores in integration tests, driven by `USE_POSTGRES_BACKEND` /
//! `USE_MYSQL_BACKEND` env vars. Patterned on `breez-itest`'s tree-store
//! switch, but generalized: one env var, every SQL-eligible spark-wallet store.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;
use tracing::info;

/// Process-lifetime PostgreSQL container shared across every test in the run.
struct SharedPgContainer {
    _container: ContainerAsync<Postgres>,
    base_conn_str: String,
}

static PG_CONTAINER: OnceCell<SharedPgContainer> = OnceCell::const_new();
static PG_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Process-lifetime MySQL container shared across every test in the run.
struct SharedMysqlContainer {
    _container: ContainerAsync<Mysql>,
    /// Connection URL up to (but not including) the path. Callers append `/<dbname>`.
    base_url: String,
}

static MYSQL_CONTAINER: OnceCell<SharedMysqlContainer> = OnceCell::const_new();
static MYSQL_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A resolved backend selection. SQL variants carry the connection string for
/// the freshly-allocated database so all three spark-wallet stores can share
/// one connection (and one database) per wallet.
#[derive(Clone, Debug)]
pub enum Backend {
    InMemory,
    Postgres(String),
    Mysql(String),
}

async fn get_postgres_base_url() -> Option<&'static str> {
    if std::env::var("USE_POSTGRES_BACKEND").is_err() {
        return None;
    }
    let shared = PG_CONTAINER
        .get_or_init(|| async {
            info!("Starting shared PostgreSQL container for spark-itest...");
            let container = Postgres::default()
                .start()
                .await
                .expect("Failed to start PostgreSQL container for spark-itest");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get PostgreSQL container port");
            info!("Shared PostgreSQL container for spark-itest started on port {port}");
            SharedPgContainer {
                _container: container,
                base_conn_str: format!(
                    "host=127.0.0.1 port={port} user=postgres password=postgres"
                ),
            }
        })
        .await;
    Some(&shared.base_conn_str)
}

async fn get_mysql_base_url() -> Option<&'static str> {
    if std::env::var("USE_MYSQL_BACKEND").is_err() {
        return None;
    }
    let shared = MYSQL_CONTAINER
        .get_or_init(|| async {
            info!("Starting shared MySQL container for spark-itest...");
            let container = Mysql::default()
                .start()
                .await
                .expect("Failed to start MySQL container for spark-itest");
            let port = container
                .get_host_port_ipv4(3306)
                .await
                .expect("Failed to get MySQL container port");
            info!("Shared MySQL container for spark-itest started on port {port}");
            SharedMysqlContainer {
                _container: container,
                base_url: format!("mysql://root@127.0.0.1:{port}"),
            }
        })
        .await;
    Some(&shared.base_url)
}

/// Resolves which backend to use for the next wallet build. Allocates a fresh
/// database per call when a SQL backend is selected — two wallets in the same
/// test should each call this so they get isolated databases.
///
/// If both env vars are set, Postgres wins.
pub async fn resolve_backend() -> Result<Backend> {
    if let Some(base_url) = get_postgres_base_url().await {
        let counter = PG_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let conn_str = format!("{base_url} dbname=spark_itest_{counter}");
        ensure_postgres_database_exists(&conn_str).await?;
        return Ok(Backend::Postgres(conn_str));
    }
    if let Some(base_url) = get_mysql_base_url().await {
        let counter = MYSQL_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_name = format!("spark_itest_{counter}");
        let conn_str = format!("{base_url}/{db_name}");
        ensure_mysql_database_exists(base_url, &db_name).await?;
        return Ok(Backend::Mysql(conn_str));
    }
    Ok(Backend::InMemory)
}

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

fn postgres_admin_conn_str(conn_str: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut saw_dbname = false;
    for part in conn_str.split_whitespace() {
        if let Some((key, value)) = part.split_once('=') {
            if key == "dbname" {
                parts.push("dbname=postgres".to_string());
                saw_dbname = true;
            } else {
                parts.push(format!("{key}={value}"));
            }
        }
    }
    if !saw_dbname {
        parts.push("dbname=postgres".to_string());
    }
    parts.join(" ")
}

async fn ensure_postgres_database_exists(conn_str: &str) -> Result<()> {
    let db_name = extract_dbname(conn_str).unwrap_or_else(|| "postgres".to_string());
    let admin_conn_str = postgres_admin_conn_str(conn_str);

    info!("Ensuring postgres database '{}' exists...", db_name);

    let (client, connection) = tokio_postgres::connect(&admin_conn_str, tokio_postgres::NoTls)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to postgres admin database: {e}"))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Postgres connection error: {}", e);
        }
    });

    let row = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&db_name],
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to check if database exists: {e}"))?;
    let exists: bool = row.get(0);
    if !exists {
        client
            .simple_query(&format!("CREATE DATABASE {db_name}"))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create database '{}': {e}", db_name))?;
    }
    Ok(())
}

async fn ensure_mysql_database_exists(admin_url: &str, db_name: &str) -> Result<()> {
    use spark_mysql::mysql_async::{Pool, prelude::*};

    info!("Ensuring MySQL database '{}' exists...", db_name);

    let admin_conn_str = format!("{admin_url}/mysql");
    let pool = Pool::from_url(admin_conn_str.as_str())
        .map_err(|e| anyhow::anyhow!("Failed to parse MySQL admin URL: {e}"))?;
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to MySQL admin database: {e}"))?;

    // db_name is internally generated (`spark_itest_<u64>`), so identifier
    // interpolation is safe. CREATE DATABASE IF NOT EXISTS is idempotent.
    let stmt = format!("CREATE DATABASE IF NOT EXISTS `{db_name}`");
    conn.query_drop(stmt)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create MySQL database '{}': {e}", db_name))?;

    drop(conn);
    pool.disconnect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to disconnect MySQL admin pool: {e}"))?;

    Ok(())
}
