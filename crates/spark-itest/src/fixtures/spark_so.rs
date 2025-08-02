use anyhow::Result;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::SecretKey;
use serde_json::json;
use spark_wallet::Identifier;
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;
use testcontainers::GenericImage;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{ContainerPort, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tokio_postgres::NoTls;
use tracing::{info, warn};

use crate::fixtures::bitcoind::BitcoindFixture;
use crate::fixtures::log::TracingConsumer;
use crate::fixtures::wait_log::WaitForLogConsumer;

const POSTGRES_USER: &str = "postgres";
const POSTGRES_PASSWORD: &str = "postgres";
const POSTGRES_DB: &str = "postgres";
const POSTGRES_PORT: u16 = 5432;

// Default ports for operators - starting from 8535
const OPERATOR_PORT: u16 = 8535;
pub const NUM_OPERATORS: usize = 3; // Using 3 operators by default
pub const MIN_SIGNERS: usize = 2; // Threshold for signing

pub struct OperatorFixture {
    pub postgres: ContainerAsync<Postgres>,
    pub container: ContainerAsync<GenericImage>,
    pub index: usize,
    pub identifier: Identifier,
    pub public_key: bitcoin::secp256k1::PublicKey,
    pub host_port: u16,
    pub internal_port: u16,
    pub internal_ip: IpAddr,
    pub postgres_connectionstring: String,
}

// Log patterns to wait for
const STARTUP_COMPLETE_PATTERN: &str = "All startup tasks completed";
const SERVER_READY_PATTERN: &str = "Serving on port";
const LOG_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

// Database query constants
const KEYSHARE_CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const KEYSHARE_CHECK_INTERVAL: Duration = Duration::from_millis(500);
const KEYSHARE_MIN_UUID: &str = "01954639-8d50-7e47-b3f0-ddb307fab7c2";
const KEYSHARE_STATUS_AVAILABLE: &str = "AVAILABLE";

pub struct SparkSoFixture {
    pub operators: Vec<OperatorFixture>,
    // Store receivers separately to avoid borrowing issues
    startup_receivers: Vec<(usize, oneshot::Receiver<()>)>,
    server_receivers: Vec<(usize, oneshot::Receiver<()>)>,
}

impl SparkSoFixture {
    pub async fn new(bitcoind_fixture: &BitcoindFixture) -> Result<Self> {
        let config_dir = testdir::testdir!();
        let operators_json_path = config_dir.join("operators.json");
        fs::create_dir_all(config_dir)?;
        fs::write(&operators_json_path, "{}")?;

        let secp = Secp256k1::new();

        // Create an array of futures for each operator
        let mut operator_futures = Vec::with_capacity(NUM_OPERATORS);

        for i in 0..NUM_OPERATORS {
            // Clone references to data needed inside the async block
            let operators_json_path = operators_json_path.clone();
            let internal_rpc_url = bitcoind_fixture.internal_rpc_url.clone();
            let internal_zmqpubrawblock_url = bitcoind_fixture.internal_zmqpubrawblock_url.clone();
            let secp = secp.clone();

            // Create async task for each operator
            let operator_future = tokio::spawn(async move {
                // Each operator gets their own postgres container for simplicity.
                let postgres = Postgres::default().start().await?;
                let postgres_ip = postgres.get_bridge_ip_address().await?;
                let postgres_port = postgres.get_host_port_ipv4(POSTGRES_PORT).await?;
                let internal_postgres_connectionstring = format!(
                    "postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@{postgres_ip}:{POSTGRES_PORT}/{POSTGRES_DB}?sslmode=disable"
                );
                let postgres_connectionstring = format!(
                    "postgres://{}:{}@{}:{}/{}?sslmode=disable",
                    POSTGRES_USER, POSTGRES_PASSWORD, "127.0.0.1", postgres_port, POSTGRES_DB
                );
                let _migrations_container = GenericImage::new("spark-migrations", "latest")
                    .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout("sql statements")))
                    .with_cmd([
                        "migrate",
                        "apply",
                        "--url",
                        internal_postgres_connectionstring.as_str(),
                    ])
                    .with_log_consumer(TracingConsumer::new(format!("migrations {i}")))
                    .start()
                    .await?;

                let secret_key = SecretKey::from_slice(&[i as u8 + 1; 32])?;

                // Create channels for detecting log messages
                let (startup_complete_tx, startup_complete_rx) = oneshot::channel();
                let (server_ready_tx, server_ready_rx) = oneshot::channel();

                // Create a custom log consumer that will signal when specific log messages are seen
                let log_consumer = WaitForLogConsumer::new(
                    format!("operator {i}"),
                    STARTUP_COMPLETE_PATTERN,
                    SERVER_READY_PATTERN,
                    startup_complete_tx,
                    server_ready_tx,
                );

                // Create container for this operator
                let container = GenericImage::new("spark-so", "latest")
                    .with_exposed_port(ContainerPort::Tcp(OPERATOR_PORT))
                    .with_log_consumer(log_consumer)
                    .with_mount(Mount::bind_mount(
                        operators_json_path.display().to_string(),
                        "/config/operators.json",
                    ))
                    // Basic configuration
                    .with_env_var("SPARK_OPERATOR_INDEX", i.to_string())
                    .with_env_var("SPARK_OPERATOR_KEY", hex::encode(secret_key.secret_bytes()))
                    .with_env_var("SPARK_THRESHOLD", MIN_SIGNERS.to_string())
                    // Postgres configuration
                    .with_env_var("POSTGRES_HOST", postgres_ip.to_string())
                    .with_env_var("POSTGRES_PORT", POSTGRES_PORT.to_string())
                    .with_env_var("POSTGRES_USER", POSTGRES_USER)
                    .with_env_var("POSTGRES_PASSWORD", POSTGRES_PASSWORD)
                    .with_env_var("DB_NAME", POSTGRES_DB)
                    // Bitcoind connection
                    .with_env_var("BITCOIND_HOST", &internal_rpc_url)
                    .with_env_var("BITCOIND_ZMQPUBRAWBLOCK", &internal_zmqpubrawblock_url)
                    .start()
                    .await?;

                let host_port = container.get_host_port_ipv4(OPERATOR_PORT).await?;

                info!("Operator {} running on port {}", i, host_port);

                let identifier = Identifier::deserialize(&hex::decode(format!("{:0>64}", i + 1))?)?;
                let public_key = secret_key.public_key(&secp);
                let internal_ip = container.get_bridge_ip_address().await?;

                let operator = OperatorFixture {
                    postgres,
                    container,
                    identifier,
                    host_port,
                    index: i,
                    public_key,
                    internal_port: OPERATOR_PORT,
                    internal_ip,
                    postgres_connectionstring,
                };

                Ok::<_, anyhow::Error>((operator, startup_complete_rx, server_ready_rx))
            });

            operator_futures.push(operator_future);
        }

        // Await all operator futures simultaneously
        let mut operators = Vec::with_capacity(NUM_OPERATORS);
        let mut startup_receivers = Vec::with_capacity(NUM_OPERATORS);
        let mut server_receivers = Vec::with_capacity(NUM_OPERATORS);

        for future in operator_futures {
            match future.await {
                Ok(Ok((operator, startup_rx, server_rx))) => {
                    let index = operator.index;
                    operators.push(operator);
                    startup_receivers.push((index, startup_rx));
                    server_receivers.push((index, server_rx));
                }
                Ok(Err(e)) => return Err(anyhow::anyhow!("Failed to create operator: {}", e)),
                Err(e) => return Err(anyhow::anyhow!("Task join error: {}", e)),
            }
        }

        // Sort operators by index to ensure consistent ordering
        operators.sort_by_key(|op| op.index);

        // Update the operators.json file with actual port assignments
        Self::create_operator_config(&operators_json_path, &operators);

        Ok(Self {
            operators,
            startup_receivers,
            server_receivers,
        })
    }

    fn create_operator_config(operators_json_path: &Path, operators: &[OperatorFixture]) {
        // Create JSON array of operators with actual port information
        let mut operator_entries = Vec::with_capacity(NUM_OPERATORS);

        for operator in operators {
            let operator_entry = json!({
                "id": operator.index,
                "address": format!("{}:{}", operator.internal_ip, operator.internal_port),
                "external_address": format!("localhost:{}", operator.host_port),
                "identity_public_key": operator.public_key.to_string(),
            });

            operator_entries.push(operator_entry);
        }

        // Convert to JSON string
        let json_content = json!(operator_entries).to_string();

        // Write to file
        fs::write(operators_json_path, &json_content).expect("Failed to write operators.json");
        info!(
            "Created operator config with actual ports at: {}",
            operators_json_path.display()
        );
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Waiting for all operators to complete initialization...");

        // Take the receivers out of self to avoid borrowing issues
        let startup_receivers = std::mem::take(&mut self.startup_receivers);
        let server_receivers = std::mem::take(&mut self.server_receivers);

        // Wait for all startup complete signals
        for (index, startup_rx) in startup_receivers {
            match timeout(LOG_WAIT_TIMEOUT, startup_rx).await {
                Ok(Ok(())) => info!("Operator {} startup tasks completed", index),
                _ => info!(
                    "Timeout waiting for operator {} startup tasks to complete",
                    index
                ),
            }
        }

        // Wait for all server ready signals
        for (index, server_rx) in server_receivers {
            match timeout(LOG_WAIT_TIMEOUT, server_rx).await {
                Ok(Ok(())) => info!("Operator {} server ready", index),
                _ => info!("Timeout waiting for operator {} server to be ready", index),
            }
        }

        // Wait for all keyshares to be available in the database
        self.wait_for_keyshares().await?;

        info!("All operators are initialized and ready");
        Ok(())
    }

    // Wait for signing keyshares to be available in all operator databases
    async fn wait_for_keyshares(&self) -> Result<()> {
        info!("Checking for available signing keyshares in all operators...");

        // Set a timeout for the entire keyshare checking process
        let result = timeout(KEYSHARE_CHECK_TIMEOUT, async {
            // Continue checking until keyshares are available for all operators
            loop {
                let mut all_ready = true;

                // Check each operator's database
                for operator in &self.operators {
                    // Connect to the operator's database
                    match tokio_postgres::connect(&operator.postgres_connectionstring, NoTls).await
                    {
                        Ok((client, connection)) => {
                            // Spawn a background task to drive the connection
                            tokio::spawn(async move {
                                if let Err(e) = connection.await {
                                    warn!("Database connection error: {}", e);
                                }
                            });

                            // Query for available keyshares
                            let query = format!(
                                "SELECT COUNT(*) FROM signing_keyshares 
                                WHERE status = '{}' 
                                AND coordinator_index = {} 
                                AND id > '{}'",
                                KEYSHARE_STATUS_AVAILABLE, operator.index, KEYSHARE_MIN_UUID
                            );

                            match client.query_one(&query, &[]).await {
                                Ok(row) => {
                                    let count: i64 = row.get(0);
                                    info!(
                                        "Operator {} has {} available keyshares",
                                        operator.index, count
                                    );

                                    // If no keyshares are available, not all operators are ready
                                    if count == 0 {
                                        all_ready = false;
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to query keyshares for operator {}: {}",
                                        operator.index, e
                                    );
                                    all_ready = false;
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to connect to database for operator {}: {}",
                                operator.index, e
                            );
                            all_ready = false;
                        }
                    }
                }

                // If all operators have keyshares, we're done
                if all_ready {
                    info!("All operators have available keyshares");
                    return Ok(());
                }

                // Wait before checking again
                sleep(KEYSHARE_CHECK_INTERVAL).await;
            }
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => {
                warn!("Timeout waiting for keyshares to be available");
                Err(anyhow::anyhow!(
                    "Timeout waiting for keyshares to be available"
                ))
            }
        }
    }
}
