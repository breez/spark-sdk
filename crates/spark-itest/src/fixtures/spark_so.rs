use anyhow::Result;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::SecretKey;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use serde_json::json;
use spark_wallet::Identifier;
use std::collections::HashSet;
use std::fs;
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
use crate::fixtures::setup::FixtureId;
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
    pub host_name: String,
    pub postgres_connectionstring: String,
    pub ca_cert: String,
}

// Log patterns to wait for
const STARTUP_COMPLETE_PATTERN: &str = "All startup tasks completed";
const LOG_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

// Database query constants
const KEYSHARE_CHECK_TIMEOUT: Duration = Duration::from_secs(180);
const KEYSHARE_CHECK_INTERVAL: Duration = Duration::from_millis(500);
const KEYSHARE_MIN_UUID: &str = "01954639-8d50-7e47-b3f0-ddb307fab7c2";
const KEYSHARE_STATUS_AVAILABLE: &str = "AVAILABLE";
// Must match dkg.min_available_keys in docker/so.config.yaml. An operator's DKG task keeps
// generating batches until it sees this many available keyshares of its own, so requiring it
// here guarantees no further inserts will race the cross-database readiness check below.
const KEYSHARE_MIN_PER_COORDINATOR: usize = 100;

pub struct SparkSoFixture {
    pub operators: Vec<OperatorFixture>,
    // Store receivers separately to avoid borrowing issues
    startup_receivers: Vec<(usize, oneshot::Receiver<()>)>,
    // Store references to log consumers for each operator
    log_consumers: Vec<(usize, WaitForLogConsumer)>,
}

// Function to generate a self-signed certificate for all operator hostnames
fn generate_self_signed_certificate(host_names: &[String]) -> Result<(String, String)> {
    let CertifiedKey { cert, signing_key } = generate_simple_self_signed(host_names).unwrap();
    Ok((signing_key.serialize_pem(), cert.pem()))
}

impl SparkSoFixture {
    pub async fn new(fixture_id: &FixtureId, bitcoind_fixture: &BitcoindFixture) -> Result<Self> {
        let config_dir = testdir::testdir!();
        let operators_json_path = config_dir.join("operators.json");

        // Create a shared server certificate file
        let key_path = config_dir.join("server.key");
        let cert_path = config_dir.join("server.crt");

        fs::create_dir_all(&config_dir)?;
        fs::write(&operators_json_path, "{}")?;

        // Generate a list of operator host names before creating the operators
        // These must match exactly what Docker uses internally for DNS resolution
        let mut operator_host_names = Vec::with_capacity(NUM_OPERATORS);
        for i in 0..NUM_OPERATORS {
            let operator_host_name = format!(
                "spark-so-{i}-{}",
                fixture_id.to_network().replace("network-", "")
            );
            operator_host_names.push(operator_host_name);
        }

        // Generate a self-signed certificate valid for all operator host names and localhost
        let cert_host_names = [operator_host_names.clone(), vec!["127.0.0.1".to_string()]].concat();
        let (key_pem, cert_pem) = generate_self_signed_certificate(&cert_host_names)?;
        fs::write(&key_path, key_pem)?;
        fs::write(&cert_path, cert_pem.clone())?;

        let secp = Secp256k1::new();

        // Create an array of futures for each operator
        let mut operator_futures = Vec::with_capacity(NUM_OPERATORS);

        for (i, operator_host_name) in operator_host_names
            .into_iter()
            .enumerate()
            .take(NUM_OPERATORS)
        {
            // Clone references to data needed inside the async block
            let internal_rpc_url = bitcoind_fixture.internal_rpc_url.clone();
            let internal_zmqpubrawblock_url = bitcoind_fixture.internal_zmqpubrawblock_url.clone();
            let secp = secp.clone();
            let fixture_id = fixture_id.clone();
            let cert_path = cert_path.clone();
            let key_path = key_path.clone();
            let operators_json_path = operators_json_path.clone();
            let cert_pem_clone = cert_pem.clone();

            // Create async task for each operator
            let operator_future = tokio::spawn(async move {
                // Each operator gets their own postgres container for simplicity.
                let postgres_container_name = format!("postgres-{i}-{fixture_id}");
                let postgres = Postgres::default()
                    .with_network(fixture_id.to_network())
                    .with_container_name(&postgres_container_name)
                    .start()
                    .await?;

                let postgres_port = postgres.get_host_port_ipv4(POSTGRES_PORT).await?;
                let internal_postgres_connectionstring = format!(
                    "postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@{postgres_container_name}:{POSTGRES_PORT}/{POSTGRES_DB}?sslmode=disable"
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
                    .with_network(fixture_id.to_network())
                    .with_container_name(format!("migrations-{i}-{fixture_id}"))
                    .with_log_consumer(TracingConsumer::new(format!("migrations {i}")))
                    .start()
                    .await?;

                let secret_key = SecretKey::from_slice(&[i as u8 + 1; 32])?;

                // Create channel for detecting log messages
                let (startup_complete_tx, startup_complete_rx) = oneshot::channel();

                // Create a custom log consumer that will signal when specific log messages are seen
                let log_consumer = WaitForLogConsumer::new(
                    format!("operator {i}"),
                    STARTUP_COMPLETE_PATTERN,
                    startup_complete_tx,
                );

                // Store a reference to the log consumer for later use
                let log_consumer_ref = log_consumer.clone();

                // Create container for this operator using the pre-generated host name
                let container = GenericImage::new("spark-so", "latest")
                    .with_exposed_port(ContainerPort::Tcp(OPERATOR_PORT))
                    .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout(
                        "Waiting for updated operators.json file",
                    )))
                    .with_log_consumer(log_consumer)
                    .with_network(fixture_id.to_network())
                    .with_container_name(&operator_host_name)
                    .with_mount(Mount::bind_mount(
                        operators_json_path.display().to_string(),
                        "/config/operators.json",
                    ))
                    .with_mount(Mount::bind_mount(
                        cert_path.display().to_string(),
                        "/data/server.crt",
                    ))
                    .with_mount(Mount::bind_mount(
                        key_path.display().to_string(),
                        "/data/server.key",
                    ))
                    // Basic configuration
                    .with_env_var("SPARK_OPERATOR_INDEX", i.to_string())
                    .with_env_var("SPARK_OPERATOR_KEY", hex::encode(secret_key.secret_bytes()))
                    .with_env_var("SPARK_THRESHOLD", MIN_SIGNERS.to_string())
                    // Postgres configuration
                    .with_env_var("POSTGRES_HOST", &postgres_container_name)
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

                let operator = OperatorFixture {
                    postgres,
                    container,
                    identifier,
                    host_port,
                    index: i,
                    public_key,
                    internal_port: OPERATOR_PORT,
                    host_name: operator_host_name,
                    postgres_connectionstring,
                    ca_cert: cert_pem_clone,
                };

                Ok::<_, anyhow::Error>((operator, startup_complete_rx, log_consumer_ref))
            });

            operator_futures.push(operator_future);
        }

        // Await all operator futures simultaneously
        let mut operators = Vec::with_capacity(NUM_OPERATORS);
        let mut startup_receivers = Vec::with_capacity(NUM_OPERATORS);
        let mut log_consumers = Vec::with_capacity(NUM_OPERATORS);

        for future in operator_futures {
            match future.await {
                Ok(Ok((operator, startup_rx, log_consumer))) => {
                    let index = operator.index;
                    operators.push(operator);
                    startup_receivers.push((index, startup_rx));
                    log_consumers.push((index, log_consumer));
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
            log_consumers,
        })
    }

    fn create_operator_config(operators_json_path: &Path, operators: &[OperatorFixture]) {
        // Create JSON array of operators with actual port information
        let mut operator_entries = Vec::with_capacity(NUM_OPERATORS);

        for operator in operators {
            let operator_entry = json!({
                "id": operator.index,
                "address": format!("{}:{}", operator.host_name, operator.internal_port),
                "external_address": format!("localhost:{}", operator.host_port),
                "identity_public_key": operator.public_key.to_string(),
                "cert_path": "/data/server.crt"   // Path to the mounted certificate inside container
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

        // Wait for all keyshares to be available in the database
        self.wait_for_keyshares().await?;

        info!("All operators are initialized and ready");
        Ok(())
    }

    // Wait for a specific log message to appear in any of the operators' logs
    pub async fn wait_for_log(&self, log_pattern: &str) -> Result<()> {
        info!(
            "Waiting for log pattern: {} in any operator's log",
            log_pattern
        );

        // First, check if the log pattern already exists in the buffer of any log consumer
        for (index, log_consumer) in &self.log_consumers {
            if log_consumer.check_log_buffer(log_pattern) {
                info!(
                    "Log pattern '{}' already found in operator {}'s logs",
                    log_pattern, index
                );
                return Ok(());
            }
        }

        // If not found in buffer, we need to set up a watch for the pattern on one of the operators
        // Here we'll just pick the first operator's log consumer
        if let Some((index, log_consumer)) = self.log_consumers.first() {
            let (tx, rx) = oneshot::channel();

            log_consumer.set_custom_pattern(log_pattern.to_string(), tx);
            info!(
                "Set up pattern watcher for '{}' on operator {}",
                log_pattern, index
            );

            rx.await?;
            return Ok(());
        }

        warn!(
            "No log consumers available to watch for pattern: {}",
            log_pattern
        );
        Err(anyhow::anyhow!(
            "No log consumers available to watch for pattern: {}",
            log_pattern
        ))
    }

    // Wait until signing keyshares are usable across all operator databases.
    //
    // A keyshare is only usable when the same row is AVAILABLE in every operator's database:
    // the coordinator reserves the chosen id on all operators, and an operator whose database
    // has not yet caught up rejects the reservation ("keyshares are not all available"). DKG
    // batches commit to each operator's database independently, so readiness requires every
    // keyshare a coordinator can pick to be visible in the other operators' databases, not
    // just each operator having keyshares of its own. The snapshot must also be identical on
    // two consecutive polls, so a DKG batch that is mid-commit cannot slip past the check.
    async fn wait_for_keyshares(&self) -> Result<()> {
        info!("Checking for available signing keyshares in all operators...");

        let result = timeout(KEYSHARE_CHECK_TIMEOUT, async {
            let mut previous: Option<Vec<Vec<(String, i64)>>> = None;

            loop {
                match self.query_available_keyshares().await {
                    Ok(snapshot) => {
                        let counts: Vec<usize> = snapshot.iter().map(Vec::len).collect();
                        info!("Available keyshares per operator database: {:?}", counts);

                        if previous.as_ref() == Some(&snapshot) && Self::keyshares_ready(&snapshot)
                        {
                            info!("Keyshares are propagated across all operator databases");
                            return Ok(());
                        }
                        previous = Some(snapshot);
                    }
                    Err(e) => {
                        warn!("Failed to query keyshares: {}", e);
                        previous = None;
                    }
                }

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

    // Fetch the set of available (id, coordinator_index) keyshares from each operator's
    // database, ordered by id so the per-operator sets compare deterministically.
    async fn query_available_keyshares(&self) -> Result<Vec<Vec<(String, i64)>>> {
        let query = format!(
            "SELECT id::text, COALESCE(coordinator_index, -1)::bigint FROM signing_keyshares
            WHERE status = '{KEYSHARE_STATUS_AVAILABLE}'
            AND id > '{KEYSHARE_MIN_UUID}'
            ORDER BY id"
        );

        let mut keyshare_sets = Vec::with_capacity(self.operators.len());
        for operator in &self.operators {
            let (client, connection) =
                tokio_postgres::connect(&operator.postgres_connectionstring, NoTls).await?;

            // Spawn a background task to drive the connection
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    warn!("Database connection error: {}", e);
                }
            });

            let rows = client.query(&query, &[]).await?;
            keyshare_sets.push(
                rows.iter()
                    .map(|row| (row.get(0), row.get(1)))
                    .collect::<Vec<(String, i64)>>(),
            );
        }

        Ok(keyshare_sets)
    }

    // Mirrors the reservation invariant: a coordinator picks from the available keyshares
    // with its own coordinator_index in its own database, then reserves that id on every
    // other operator, so each of those ids must be available in every other database.
    fn keyshares_ready(snapshot: &[Vec<(String, i64)>]) -> bool {
        let id_sets: Vec<HashSet<&str>> = snapshot
            .iter()
            .map(|set| set.iter().map(|(id, _)| id.as_str()).collect())
            .collect();

        snapshot.iter().enumerate().all(|(coordinator, own_set)| {
            let coordinated: Vec<&str> = own_set
                .iter()
                .filter(|(_, coordinator_index)| *coordinator_index == coordinator as i64)
                .map(|(id, _)| id.as_str())
                .collect();

            // An operator's DKG keeps inserting batches until it has
            // KEYSHARE_MIN_PER_COORDINATOR keyshares of its own; below that, more inserts
            // are still coming and could race this check.
            coordinated.len() >= KEYSHARE_MIN_PER_COORDINATOR
                && id_sets.iter().enumerate().all(|(other, ids)| {
                    other == coordinator || coordinated.iter().all(|id| ids.contains(id))
                })
        })
    }
}
