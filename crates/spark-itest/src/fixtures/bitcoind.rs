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

const BITCOIND_IMAGE: &str = "spark-itest-bitcoind";
const BITCOIND_TAG: &str = "29.0";
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
        let container = GenericImage::new(BITCOIND_IMAGE, BITCOIND_TAG)
            .with_exposed_port(ContainerPort::Tcp(REGTEST_RPC_PORT))
            .with_exposed_port(ContainerPort::Tcp(ZMQPUBRAWBLOCK_RPC_PORT))
            .with_wait_for(WaitFor::Log(
                LogWaitStrategy::stdout("init message: Done loading").with_times(1),
            ))
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
        // The container wait strategy already blocks until "init message: Done
        // loading" appears on stdout, so the RPC server should be up. A short
        // retry loop covers the brief gap between that log line and the RPC
        // server accepting connections; the outer timeout bounds the wait so
        // a genuinely stuck bitcoind surfaces as a clean failure instead of
        // hanging the test.
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if self.create_wallet_rpc().await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("Timed out waiting for bitcoind to create wallet"))?;
        Ok(())
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

    /// Broadcast a transaction with maxfeerate=0, bypassing fee-rate checks.
    /// Useful for v3 transactions with ephemeral anchors on regtest.
    pub async fn broadcast_transaction_no_fee_check(&self, tx: &Transaction) -> Result<Txid> {
        let tx_hex = hex::encode(bitcoin::consensus::serialize(tx));
        self.rpc_call::<String>("sendrawtransaction", &[json!(tx_hex), json!(0)])
            .map_ok(|txid_str| txid_str.parse().unwrap())
            .await
    }

    /// Submit a package of transactions to bitcoind via the `submitpackage` RPC.
    ///
    /// This is required for v3 transactions with ephemeral anchors, which cannot
    /// be broadcast individually because the 0-value anchor output is non-standard
    /// on its own.
    pub async fn submit_package(&self, txs: &[&Transaction]) -> Result<Value> {
        let hex_array: Vec<String> = txs
            .iter()
            .map(|tx| hex::encode(bitcoin::consensus::serialize(tx)))
            .collect();
        self.rpc_call::<Value>("submitpackage", &[json!(hex_array)])
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
        self.wait_for_tx_confirmation_with_timeout(txid, min_confirmations, Duration::from_secs(60))
            .await
    }

    pub async fn wait_for_tx_confirmation_with_timeout(
        &self,
        txid: &Txid,
        min_confirmations: u64,
        timeout: Duration,
    ) -> Result<()> {
        let poll = async {
            loop {
                let result: Value = self
                    .rpc_call("gettransaction", &[json!(txid.to_string())])
                    .await?;

                if let Some(confirmations) = result.get("confirmations").and_then(|c| c.as_u64())
                    && confirmations >= min_confirmations
                {
                    return Ok::<(), anyhow::Error>(());
                }

                sleep(Duration::from_millis(500)).await;
            }
        };

        match tokio::time::timeout(timeout, poll).await {
            Ok(res) => res,
            Err(_) => Err(anyhow::anyhow!(
                "Timed out after {:?} waiting for tx {} to reach {} confirmations",
                timeout,
                txid,
                min_confirmations,
            )),
        }
    }

    /// Public JSON-RPC accessor for tests and helpers that need to reach
    /// bitcoind methods not already wrapped here.
    pub async fn rpc<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<T> {
        self.rpc_call(method, params).await
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
