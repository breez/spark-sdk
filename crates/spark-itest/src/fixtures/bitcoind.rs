use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Result;
use bitcoin::{Address, Amount, Network, Transaction, Txid};
use futures::TryFutureExt;
use platform_utils::{
    ContentType, DefaultHttpClient, HttpClient, add_basic_auth_header, add_content_type_header,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, WaitFor, wait::LogWaitStrategy},
    runners::AsyncRunner,
};
use tokio::time::sleep;
use tracing::info;

use crate::fixtures::{log::TracingConsumer, setup::FixtureId};

const BITCOIND_VERSION: &str = "v28.0";
const BITCOIND_DOCKER_IMAGE: &str = "lncm/bitcoind";
const REGTEST_RPC_USER: &str = "rpcuser";
const REGTEST_RPC_PASSWORD: &str = "rpcpassword";
const REGTEST_RPC_PORT: u16 = 8332;
const ZMQPUBRAWBLOCK_RPC_PORT: u16 = 28332;
const DEFAULT_MINING_ADDRESS: &str = "bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw";

pub struct BitcoindFixture {
    pub container: ContainerAsync<GenericImage>,
    pub rpc_url: String,
    pub zmqpubrawblock_url: String,
    pub internal_rpc_url: String,
    pub internal_zmqpubrawblock_url: String,
    pub rpcuser: String,
    pub rpcpassword: String,
    pub mining_address: Address,
    http_client: DefaultHttpClient,
}

#[derive(Serialize, Deserialize, Debug)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<Value>,
    // id: Value,
}

impl BitcoindFixture {
    pub async fn new(fixture_id: &FixtureId) -> anyhow::Result<Self> {
        // Define bitcoind container with command line arguments
        let container_name = format!("bitcoind-{fixture_id}");
        let container = GenericImage::new(BITCOIND_DOCKER_IMAGE, BITCOIND_VERSION)
            .with_exposed_port(ContainerPort::Tcp(REGTEST_RPC_PORT))
            .with_exposed_port(ContainerPort::Tcp(ZMQPUBRAWBLOCK_RPC_PORT))
            .with_wait_for(WaitFor::Log(LogWaitStrategy::stdout(
                "init message: Done loading",
            )))
            .with_network(fixture_id.to_network())
            .with_container_name(&container_name)
            .with_log_consumer(TracingConsumer::new("bitcoind"))
            .with_cmd([
                "-regtest",
                "-server",
                "-logtimestamps",
                "-nolisten",
                "-addresstype=bech32",
                "-txindex",
                "-fallbackfee=0.00000253",
                "-debug=mempool",
                "-debug=rpc",
                format!("-rpcport={REGTEST_RPC_PORT}").as_str(),
                format!("-rpcuser={REGTEST_RPC_USER}").as_str(),
                format!("-rpcpassword={REGTEST_RPC_PASSWORD}").as_str(),
                format!("-zmqpubrawblock=tcp://0.0.0.0:{ZMQPUBRAWBLOCK_RPC_PORT}").as_str(),
                format!("-zmqpubrawtx=tcp://0.0.0.0:{ZMQPUBRAWBLOCK_RPC_PORT}").as_str(),
                "-rpcbind=0.0.0.0",
                "-rpcallowip=0.0.0.0/0",
            ])
            .start()
            .await?;

        info!("Bitcoind container running");
        let host_rpc_port = container.get_host_port_ipv4(REGTEST_RPC_PORT).await?;
        let host_zmq_port = container
            .get_host_port_ipv4(ZMQPUBRAWBLOCK_RPC_PORT)
            .await?;
        let rpc_url = format!("http://127.0.0.1:{host_rpc_port}/");
        let zmqpubrawblock_url = format!("tcp://127.0.0.1:{host_zmq_port}");

        let internal_rpc_url = format!("{container_name}:{REGTEST_RPC_PORT}");
        let internal_zmqpubrawblock_url =
            format!("tcp://{container_name}:{ZMQPUBRAWBLOCK_RPC_PORT}");
        info!(
            "Got bitcoind exposed rpc and zmq ports: {} and. {}",
            host_rpc_port, host_zmq_port
        );
        // Create instance with RPC URL
        let instance = Self {
            container,
            rpc_url,
            zmqpubrawblock_url,
            internal_rpc_url,
            internal_zmqpubrawblock_url,
            rpcuser: REGTEST_RPC_USER.to_string(),
            rpcpassword: REGTEST_RPC_PASSWORD.to_string(),
            mining_address: Address::from_str(DEFAULT_MINING_ADDRESS)?
                .require_network(Network::Regtest)?,
            http_client: DefaultHttpClient::default(),
        };

        info!("Created bitcoind container. Ensure wallet created.");

        // Wait for RPC to be available and create wallet using the RPC API
        instance.ensure_wallet_created().await?;

        info!("Bitcoin wallet is created.");
        Ok(instance)
    }

    async fn ensure_wallet_created(&self) -> Result<()> {
        // Try to create wallet with retries
        let max_retries = 10;
        let mut retries = 0;
        let mut last_error = None;

        while retries < max_retries {
            match self.create_wallet_rpc().await {
                Ok(_) => {
                    info!("Successfully created or confirmed bitcoin wallet");
                    return Ok(());
                }
                Err(e) => {
                    retries += 1;
                    info!(
                        "Failed to create wallet (retry {}/{}): {}",
                        retries, max_retries, &e
                    );
                    last_error = Some(e);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Failed to create wallet after retries")))
    }

    async fn create_wallet_rpc(&self) -> Result<()> {
        let result: Result<Value> = self.rpc_call("createwallet", &[json!("default")]).await;

        match result {
            Ok(_) => {
                info!("Successfully created or confirmed bitcoin wallet");
                Ok(())
            }
            Err(e) => {
                let s = e.to_string();
                if s.contains("already exists") {
                    return Ok(());
                }

                Err(e)
            }
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Create a new address from bitcoind's internal wallet
        let new_address = self.get_new_address().await?;
        info!("Created new mining address: {}", new_address);

        // Update the mining address
        self.mining_address = Address::from_str(&new_address)?.require_network(Network::Regtest)?;

        // Generate some blocks to mature the coinbase
        self.generate_blocks(101).await?;
        info!("Generated 101 blocks for bitcoind fixture");
        Ok(())
    }

    async fn get_new_address(&self) -> Result<String> {
        // Call the getnewaddress RPC method to create a new address
        self.rpc_call::<String>("getnewaddress", &[json!("mining"), json!("bech32")])
            .await
    }

    pub async fn generate_blocks(&self, count: u64) -> Result<Vec<String>> {
        let address = self.mining_address.to_string();
        self.rpc_call::<Vec<String>>("generatetoaddress", &[json!(count), json!(address)])
            .await
    }

    pub async fn broadcast_transaction(&self, tx: &Transaction) -> Result<Txid> {
        let tx_hex = hex::encode(bitcoin::consensus::serialize(tx));
        self.rpc_call::<String>("sendrawtransaction", &[json!(tx_hex)])
            .map_ok(|txid_str| txid_str.parse().unwrap())
            .await
    }

    pub async fn get_transaction(&self, txid: &Txid) -> Result<Transaction> {
        let tx_hex = self
            .rpc_call::<String>("getrawtransaction", &[json!(txid.to_string())])
            .await?;

        let tx_bytes = hex::decode(tx_hex)?;
        Ok(bitcoin::consensus::deserialize(&tx_bytes)?)
    }

    pub async fn fund_address(&self, address: &Address, amount: Amount) -> Result<Txid> {
        self.rpc_call::<String>(
            "sendtoaddress",
            &[json!(address.to_string()), json!(amount.to_btc())],
        )
        .map_ok(|txid_str| txid_str.parse().unwrap())
        .await
    }

    pub async fn wait_for_tx_confirmation(
        &self,
        txid: &Txid,
        min_confirmations: u64,
    ) -> Result<()> {
        loop {
            let result: Value = self
                .rpc_call("gettransaction", &[json!(txid.to_string())])
                .await?;

            if let Some(confirmations) = result.get("confirmations").and_then(|c| c.as_u64())
                && confirmations >= min_confirmations
            {
                return Ok(());
            }

            sleep(Duration::from_millis(500)).await;
        }
    }

    async fn rpc_call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<T> {
        let request = json!({
            "jsonrpc": "1.0",
            "id": "rust-client",
            "method": method,
            "params": params,
        });

        let body = serde_json::to_string(&request)?;

        let mut headers = HashMap::new();
        add_basic_auth_header(&mut headers, &self.rpcuser, &self.rpcpassword);
        add_content_type_header(&mut headers, ContentType::Json);

        let response = self
            .http_client
            .post(self.rpc_url.clone(), Some(headers), Some(body))
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request failed: {e:?}"))?;

        if !response.is_success() {
            return Err(anyhow::anyhow!(
                "bitcoind returned error status: {}",
                response.status
            ));
        }

        let rpc_response: RpcResponse<T> = response.json()?;
        match (rpc_response.result, rpc_response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(anyhow::anyhow!("RPC error: {:?}", error)),
            _ => Err(anyhow::anyhow!("Invalid RPC response")),
        }
    }
}
