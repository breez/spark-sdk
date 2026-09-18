use std::collections::HashMap;
use std::fs;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bitcoin::Amount;
use bitcoin::address::NetworkUnchecked;
use bitcoin::secp256k1::PublicKey;
use spark::signer::{DefaultSigner, derive_identity_public_key};
use testcontainers::core::{ContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tonic::transport::Channel;
use tracing::info;

use crate::fixtures::bitcoind::BitcoindFixture;
use crate::fixtures::log::TracingConsumer;
use crate::fixtures::setup::FixtureId;
use crate::fixtures::spark_so::OperatorFixture;
use crate::fixtures::state_snapshot;

pub mod internal_api {
    #![allow(clippy::pedantic, clippy::all)]
    tonic::include_proto!("ssp_internal");
}

use internal_api::{
    onchain_wallet_client::OnchainWalletClient, pool_client::PoolClient,
    ssp_manager_client::SspManagerClient,
};

const GRAPHQL_PORT: u16 = 8080;
const INTERNAL_PORT: u16 = 59050;
const POSTGRES_PORT: u16 = 5432;
const POSTGRES_USER: &str = "postgres";
const POSTGRES_PASSWORD: &str = "postgres";
const POSTGRES_DB: &str = "postgres";

pub const LEAVES_PER_DENOMINATION: u32 = 8;

pub const MAX_DENOMINATION_POWER: u32 = 16;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Short enough for a test to wait out a HODL receive whose preimage is never
/// revealed.
pub const RECEIVE_LEAF_TRANSFER_EXPIRY_SECS: u64 = 20;

/// A full pool costs about 10.4M sats, one whole tree per denomination, and the
/// daemon builds only the trees its funds cover.
pub const FULL_POOL_ONCHAIN_SATS: u64 = 12_000_000;

pub struct LdkSettings {
    /// `<container_name>:<port>` on the shared network.
    pub internal_url: String,
    pub api_key: String,
    pub cert_pem: Vec<u8>,
    /// Unset, the daemon refuses to issue an invoice that carries a Spark address.
    pub invoice_signing_key_hex: Option<String>,
}

pub struct SspdFixture {
    pub container: ContainerAsync<GenericImage>,
    pub postgres: ContainerAsync<Postgres>,
    pub base_url: String,
    pub internal_url: String,
    pub wallet_seed_hex: String,
    pub identity_public_key: PublicKey,
}

impl SspdFixture {
    pub async fn tree_store(&self) -> Result<spark_postgres::PostgresTreeStore> {
        let port = self.postgres.get_host_port_ipv4(POSTGRES_PORT).await?;
        let url = format!(
            "postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@127.0.0.1:{port}/{POSTGRES_DB}"
        );
        spark_postgres::PostgresTreeStore::from_config(
            spark_postgres::PostgresStorageConfig::with_defaults(&url),
            &self.identity_public_key.serialize(),
        )
        .await
        .context("opening the daemon's tree store")
    }

    /// `wallet_seed_hex` has to be [`crate::fixtures::setup::SSPD_WALLET_SEED_HEX`]
    /// when a state snapshot is restored: the restored pool belongs to the
    /// identity that seed derives.
    pub async fn start(
        fixture_id: &FixtureId,
        bitcoind: &BitcoindFixture,
        operators: &[OperatorFixture],
        wallet_seed_hex: &str,
        ldk: Option<&LdkSettings>,
    ) -> Result<Self> {
        let config_dir = fixture_id.testdir()?;
        let container_name = format!("sspd-{fixture_id}");

        let postgres_container_name = format!("sspd-postgres-{fixture_id}");
        let postgres = Postgres::default()
            .with_network(fixture_id.to_network())
            .with_container_name(&postgres_container_name)
            .with_mount(state_snapshot::mount())
            .start()
            .await
            .context("starting sspd's postgres")?;

        let restored = state_snapshot::is_current();
        if restored {
            state_snapshot::restore_database(
                &fixture_id.to_network(),
                &postgres_container_name,
                "sspd",
            )
            .await?;
        }
        let db_url = format!(
            "postgres://{POSTGRES_USER}:{POSTGRES_PASSWORD}@{postgres_container_name}:{POSTGRES_PORT}/{POSTGRES_DB}"
        );

        let mut config = String::new();
        for operator in operators {
            config.push_str(&format!(
                "[[operators]]\nid = {}\nidentifier = \"{}\"\naddress = \"https://{}:{}\"\nidentity_public_key = \"{}\"\nca_cert_pem = \"\"\"\n{}\n\"\"\"\n\n",
                operator.index,
                hex::encode(operator.identifier.serialize()),
                operator.host_name,
                operator.internal_port,
                operator.public_key,
                operator.ca_cert,
            ));
        }
        let config_path = config_dir.join("sspd.toml");
        fs::write(&config_path, &config)?;

        let mut image = GenericImage::new("sspd", "latest")
            .with_exposed_port(ContainerPort::Tcp(GRAPHQL_PORT))
            .with_exposed_port(ContainerPort::Tcp(INTERNAL_PORT))
            .with_network(fixture_id.to_network())
            .with_container_name(&container_name)
            .with_log_consumer(TracingConsumer::new("sspd".to_string()))
            .with_mount(Mount::bind_mount(
                config_path.display().to_string(),
                "/config/sspd.toml",
            ))
            // The daemon takes no secret as a flag, only from its config file or
            // the environment.
            .with_env_var("SSPD_DB_URL", db_url)
            .with_env_var("SSPD_BITCOIND_RPC_PASSWORD", bitcoind.rpcpassword.clone())
            .with_env_var("SSPD_WALLET_SEED", wallet_seed_hex.to_string());

        let mut cmd = vec![
            "--config".to_string(),
            "/config/sspd.toml".to_string(),
            "--address".to_string(),
            format!("0.0.0.0:{GRAPHQL_PORT}"),
            "--internal-address".to_string(),
            format!("0.0.0.0:{INTERNAL_PORT}"),
            "--network".to_string(),
            "regtest".to_string(),
            // Safe over a restored database: the dump carries sqlx's record of
            // applied migrations.
            "--auto-migrate".to_string(),
            // The default chain poll is a minute, too slow for tests that wait on
            // blocks they mine.
            "--chain-poll-interval-seconds".to_string(),
            "1".to_string(),
            "--bitcoind-rpc-address".to_string(),
            // sspd takes a URL, and without the scheme it cannot reach bitcoind.
            format!("http://{}", bitcoind.internal_rpc_url),
            "--bitcoind-rpc-user".to_string(),
            bitcoind.rpcuser.clone(),
            "--leaves-per-denomination".to_string(),
            LEAVES_PER_DENOMINATION.to_string(),
            "--max-denomination-power".to_string(),
            MAX_DENOMINATION_POWER.to_string(),
            "--receive-leaf-transfer-expiry-seconds".to_string(),
            RECEIVE_LEAF_TRANSFER_EXPIRY_SECS.to_string(),
        ];

        if let Some(ldk) = ldk {
            let cert_path = config_dir.join("ldk-server.crt");
            fs::write(&cert_path, &ldk.cert_pem)?;
            image = image.with_mount(Mount::bind_mount(
                cert_path.display().to_string(),
                "/config/ldk-server.crt",
            ));
            cmd.extend([
                "--ldk-server-url".to_string(),
                ldk.internal_url.clone(),
                "--ldk-server-cert-path".to_string(),
                "/config/ldk-server.crt".to_string(),
            ]);
            image = image.with_env_var("SSPD_LDK_SERVER_API_KEY", ldk.api_key.clone());
            if let Some(key) = &ldk.invoice_signing_key_hex {
                image = image.with_env_var("SSPD_LDK_SERVER_INVOICE_SIGNING_KEY", key.clone());
            }
        }

        let container = image.with_cmd(cmd).start().await.context("starting sspd")?;

        let (base_url, internal_url) = wait_until_answering(&container).await?;
        info!("sspd ready at {base_url} (internal {internal_url})");

        let signer = DefaultSigner::new(&hex::decode(wallet_seed_hex)?, spark::Network::Regtest)?;
        let identity_public_key = derive_identity_public_key(&signer).await?;

        Ok(Self {
            container,
            postgres,
            base_url,
            internal_url,
            wallet_seed_hex: wallet_seed_hex.to_string(),
            identity_public_key,
        })
    }

    pub async fn pool_client(&self) -> Result<PoolClient<Channel>> {
        PoolClient::connect(self.internal_url.clone())
            .await
            .context("connecting to the sspd pool api")
    }

    pub async fn onchain_client(&self) -> Result<OnchainWalletClient<Channel>> {
        OnchainWalletClient::connect(self.internal_url.clone())
            .await
            .context("connecting to the sspd onchain wallet api")
    }

    pub async fn manager_client(&self) -> Result<SspManagerClient<Channel>> {
        SspManagerClient::connect(self.internal_url.clone())
            .await
            .context("connecting to the sspd manager api")
    }

    pub async fn fund_onchain(
        &self,
        bitcoind: &BitcoindFixture,
        sats: u64,
        count: usize,
    ) -> Result<()> {
        let mut client = self.onchain_client().await?;
        for _ in 0..count {
            let address = client
                .new_address(internal_api::NewAddressRequest {})
                .await
                .context("asking sspd for an onchain address")?
                .into_inner()
                .address;
            let address = address
                .parse::<bitcoin::Address<NetworkUnchecked>>()?
                .assume_checked();
            bitcoind
                .fund_address(&address, Amount::from_sat(sats))
                .await?;
        }
        // The daemon learns of a UTXO only from a block its chain monitor has seen.
        bitcoind.generate_blocks(1).await?;
        Ok(())
    }

    pub async fn wait_for_onchain_balance(
        &self,
        bitcoind: &BitcoindFixture,
        min_sats: u64,
        timeout: Duration,
    ) -> Result<()> {
        let mut client = self.onchain_client().await?;
        let deadline = Instant::now() + timeout;
        let mut last = 0;
        while Instant::now() < deadline {
            last = client
                .balance(internal_api::BalanceRequest {})
                .await?
                .into_inner()
                .confirmed_sats;
            if last >= min_sats {
                info!("sspd holds {last} sats onchain");
                return Ok(());
            }
            bitcoind.generate_blocks(1).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!("sspd onchain balance stuck at {last} sats, wanted {min_sats}")
    }

    pub async fn pool_leaf_counts(&self) -> Result<HashMap<u64, u32>> {
        let status = self
            .pool_client()
            .await?
            .pool_status(internal_api::PoolStatusRequest {})
            .await
            .context("reading the sspd pool status")?
            .into_inner();
        Ok(status
            .available
            .into_iter()
            .map(|count| (count.denomination_sats, count.count))
            .collect())
    }

    /// The daemon's 100 most recent swaps, newest first.
    pub async fn swaps(&self) -> Result<Vec<internal_api::Swap>> {
        Ok(self
            .manager_client()
            .await?
            .list_swaps(internal_api::ListSwapsRequest { limit: 0 })
            .await
            .context("listing the sspd swaps")?
            .into_inner()
            .swaps)
    }

    pub async fn restart(&mut self) -> Result<()> {
        self.restart_stopped().await?;
        self.start_again().await
    }

    /// Stops the daemon, leaving it down until [`Self::start_again`].
    pub async fn restart_stopped(&self) -> Result<()> {
        self.container.stop().await.context("stopping sspd")?;
        Ok(())
    }

    /// Docker can map different host ports when the container starts again.
    pub async fn start_again(&mut self) -> Result<()> {
        self.container
            .start()
            .await
            .context("starting sspd again")?;
        (self.base_url, self.internal_url) = wait_until_answering(&self.container).await?;
        Ok(())
    }

    pub async fn lightning_request(
        &self,
        id: &str,
    ) -> Result<internal_api::GetLightningRequestResponse> {
        Ok(self
            .manager_client()
            .await?
            .get_lightning_request(internal_api::GetLightningRequestRequest { id: id.to_string() })
            .await
            .context("reading a lightning request from sspd")?
            .into_inner())
    }

    /// Mines while it waits, since a tree's leaves reach the pool only after its
    /// funding transaction confirms.
    pub async fn wait_for_pool(
        &self,
        bitcoind: &BitcoindFixture,
        min_per_denomination: u32,
        timeout: Duration,
    ) -> Result<()> {
        let denominations = pool_denominations();
        let deadline = Instant::now() + timeout;
        let mut short = Vec::new();
        while Instant::now() < deadline {
            bitcoind.generate_blocks(1).await?;
            let counts = self.pool_leaf_counts().await?;
            short = denominations
                .iter()
                .filter(|d| counts.get(d).copied().unwrap_or(0) < min_per_denomination)
                .copied()
                .collect();
            if short.is_empty() {
                info!("sspd pool stocked: {counts:?}");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!(
            "sspd pool still short of {min_per_denomination} leaves for denominations {short:?}"
        )
    }
}

fn pool_denominations() -> Vec<u64> {
    (0..=MAX_DENOMINATION_POWER).map(|p| 1u64 << p).collect()
}

/// Returns the daemon's GraphQL and internal URLs once both answer a real call:
/// docker can publish a port after the container starts, and accepts connections
/// on it before the daemon listens.
async fn wait_until_answering(
    container: &ContainerAsync<GenericImage>,
) -> Result<(String, String)> {
    let graphql_port = crate::fixtures::published_port(container, GRAPHQL_PORT).await?;
    let internal_port = crate::fixtures::published_port(container, INTERNAL_PORT).await?;
    let base_url = format!("http://127.0.0.1:{graphql_port}");
    let internal_url = format!("http://127.0.0.1:{internal_port}");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match answers(graphql_port, &internal_url).await {
            Ok(()) => return Ok((base_url, internal_url)),
            Err(e) if Instant::now() >= deadline => {
                return Err(e).context("sspd did not answer after starting");
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

async fn answers(graphql_port: u16, internal_url: &str) -> Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", graphql_port)).await?;
    stream.write_all(b"GET /health HTTP/1.0\r\n\r\n").await?;
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(1), stream.read_to_string(&mut response)).await??;
    anyhow::ensure!(
        response.split(' ').nth(1) == Some("200"),
        "health check answered {response:?}"
    );
    SspManagerClient::connect(internal_url.to_string())
        .await?
        .list_swaps(internal_api::ListSwapsRequest { limit: 0 })
        .await?;
    Ok(())
}
