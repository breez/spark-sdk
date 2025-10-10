use std::sync::Arc;

use anyhow::Result;
use bitcoin::{Amount, Txid, Address, Network as BtcNetwork};
use breez_sdk_spark::*;
use rstest::*;
use serde_json::{json, Value};
use test_log::test;
use tokio_with_wasm::alias as tokio;
use tracing::{info, debug};

use spark_itest::fixtures::setup::TestFixtures;

// ---------------------
// Bitcoind-backed ChainService for tests
// ---------------------
struct BitcoindChainService {
    rpc_url: String,
    rpcuser: String,
    rpcpassword: String,
    client: reqwest::Client,
}

impl BitcoindChainService {
    fn new(rpc_url: String, rpcuser: String, rpcpassword: String) -> Self {
        Self { rpc_url, rpcuser, rpcpassword, client: reqwest::Client::new() }
    }

    async fn rpc_call<T: for<'de> serde::Deserialize<'de>>(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<T> {
        let request = json!({
            "jsonrpc": "1.0",
            "id": "breez-sdk-itest",
            "method": method,
            "params": params,
        });

        let response = self
            .client
            .post(&self.rpc_url)
            .basic_auth(&self.rpcuser, Some(&self.rpcpassword))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("bitcoind returned error status: {}", response.status());
        }

        #[derive(serde::Deserialize)]
        struct RpcResponse<T> { result: Option<T>, error: Option<Value> }

        let response: RpcResponse<T> = response.json().await?;
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => anyhow::bail!("RPC error: {:?}", error),
            _ => anyhow::bail!("Invalid RPC response"),
        }
    }
}

#[macros::async_trait]
impl BitcoinChainService for BitcoindChainService {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        // Use listunspent to get UTXOs for the address
        #[derive(serde::Deserialize)]
        struct Unspent {
            txid: String,
            vout: u32,
            amount: f64,
            confirmations: u64,
        }
        let list: Vec<Unspent> = self
            .rpc_call("listunspent", &[json!(0), json!(9999999), json!([address])])
            .await
            .map_err(|e| ChainServiceError::ServiceConnectivity(e.to_string()))?;

        let utxos = list
            .into_iter()
            .map(|u| Utxo {
                txid: u.txid,
                vout: u.vout,
                value: Amount::from_btc(u.amount).expect("valid btc").to_sat(),
                status: TxStatus {
                    confirmed: u.confirmations > 0,
                    block_height: None,
                    block_time: None,
                },
            })
            .collect();
        Ok(utxos)
    }

    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError> {
        let v: Value = self
            .rpc_call("gettransaction", &[json!(txid)])
            .await
            .map_err(|e| ChainServiceError::ServiceConnectivity(e.to_string()))?;
        let confirmations = v.get("confirmations").and_then(|c| c.as_u64()).unwrap_or(0);
        Ok(TxStatus {
            confirmed: confirmations > 0,
            block_height: None,
            block_time: None,
        })
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        let tx_hex: String = self
            .rpc_call("getrawtransaction", &[json!(txid)])
            .await
            .map_err(|e| ChainServiceError::ServiceConnectivity(e.to_string()))?;
        Ok(tx_hex)
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        let _txid: String = self
            .rpc_call("sendrawtransaction", &[json!(tx)])
            .await
            .map_err(|e| ChainServiceError::ServiceConnectivity(e.to_string()))?;
        Ok(())
    }
}

// ---------------------
// Fixtures
// ---------------------
#[fixture]
async fn fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

async fn build_sdk(
    fixtures: &TestFixtures,
    storage_dir: String,
    seed_bytes: [u8; 32],
) -> Result<BreezSdk> {
    // Config
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // ensure invoices embed spark address
    config.sync_interval_secs = 60;

    // Storage
    let storage = default_storage(storage_dir)?;

    // Seed
    let seed = Seed::Entropy(seed_bytes.to_vec());

    // Wallet config from local test operators
    let wallet_config = fixtures.create_wallet_config().await?;

    // Chain service backed by local bitcoind
    let chain_service = Arc::new(BitcoindChainService::new(
        fixtures.bitcoind.rpc_url.clone(),
        fixtures.bitcoind.rpcuser.clone(),
        fixtures.bitcoind.rpcpassword.clone(),
    ));

    // Build SDK
    let builder = SdkBuilder::new(config, seed, storage)
        .with_chain_service(chain_service)
        .with_spark_wallet_config(wallet_config);
    let sdk = builder.build().await?;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest { ensure_synced: Some(true) })
        .await?;

    Ok(sdk)
}

fn find_vout_for_address(tx: &bitcoin::Transaction, address: &Address) -> Option<u32> {
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(a) = Address::from_script(&output.script_pubkey, BtcNetwork::Regtest) {
            if &a == address { return Some(vout as u32); }
        }
    }
    None
}

// ---------------------
// Tests
// ---------------------

#[rstest]
#[tokio::test]
#[test]
#[ignore]
async fn test_breez_sdk_deposit_claim(#[future] fixtures: TestFixtures) -> Result<()> {
    let fixtures = fixtures.await;

    // Create SDK (alice)
    let data_dir = tempdir::TempDir::new("breez-sdk-deposit")?;
    let sdk = build_sdk(&fixtures, data_dir.path().to_string_lossy().to_string(), [1u8; 32]).await?;

    // Get a static deposit address
    let receive = sdk
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::BitcoinAddress })
        .await?;
    let deposit_address: Address = receive.payment_request.parse()?;
    info!("Generated deposit address: {}", deposit_address);

    // Fund address
    let amount = Amount::from_sat(100_000);
    let txid: Txid = fixtures.bitcoind.fund_address(&deposit_address, amount).await?;

    // Fetch tx and find vout
    let tx = fixtures.bitcoind.get_transaction(&txid).await?;
    let vout = find_vout_for_address(&tx, &deposit_address).expect("vout not found for deposit address");

    // Claim deposit explicitly via SDK using direct claim path for regtest
    let tx_hex = hex::encode(bitcoin::consensus::serialize(&tx));
    #[allow(unused)]
    #[cfg(feature = "test-utils")]
    {
        sdk.claim_deposit_with_tx(tx_hex, vout).await?;
    }
    #[cfg(not(feature = "test-utils"))]
    {
        // Fallback for builds without test-utils: use public API
        let _ = sdk
            .claim_deposit(ClaimDepositRequest { txid: txid.to_string(), vout, max_fee: None })
            .await?;
    }

    // Trigger a sync to persist and update balance
    let _ = sdk.sync_wallet(SyncWalletRequest {}).await?;

    let info_res = sdk.get_info(GetInfoRequest { ensure_synced: Some(false) }).await?;
    debug!("Wallet balance after claim: {} sats", info_res.balance_sats);

    assert!(info_res.balance_sats > 0, "Balance should increase after deposit claim");
    Ok(())
}

#[rstest]
#[tokio::test]
#[test]
#[ignore]
async fn test_breez_sdk_send_payment_prefer_spark(#[future] fixtures: TestFixtures) -> Result<()> {
    let fixtures = fixtures.await;

    // Create SDKs for Alice and Bob
    let alice_dir = tempdir::TempDir::new("breez-sdk-alice")?;
    let bob_dir = tempdir::TempDir::new("breez-sdk-bob")?;

    let alice = build_sdk(&fixtures, alice_dir.path().to_string_lossy().to_string(), [2u8; 32]).await?;
    let bob = build_sdk(&fixtures, bob_dir.path().to_string_lossy().to_string(), [3u8; 32]).await?;

    // Fund Alice via deposit and claim
    let alice_deposit_addr: Address = alice
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::BitcoinAddress })
        .await?
        .payment_request
        .parse()?;
    let txid = fixtures
        .bitcoind
        .fund_address(&alice_deposit_addr, Amount::from_sat(120_000))
        .await?;
    let tx = fixtures.bitcoind.get_transaction(&txid).await?;
    let vout = find_vout_for_address(&tx, &alice_deposit_addr).expect("vout not found for deposit address");
    alice
        .claim_deposit(ClaimDepositRequest { txid: txid.to_string(), vout, max_fee: None })
        .await?;
    // Ensure balance reflects claimed deposit
    alice.sync_wallet(SyncWalletRequest {}).await?;

    // Bob exposes a Spark address (no SSP required)
    let bob_spark_address = bob
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::SparkAddress })
        .await?
        .payment_request;

    // Alice prepares and sends the payment, preferring spark transfer
    let prepare = alice
        .prepare_send_payment(PrepareSendPaymentRequest { payment_request: bob_spark_address.clone(), amount_sats: Some(5_000) })
        .await?;

    let send_resp = alice
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.status, PaymentStatus::Completed | PaymentStatus::Pending));

    // Bob syncs and verifies he received the payment
    bob.sync_wallet(SyncWalletRequest {}).await?;
    let payments = bob
        .list_payments(ListPaymentsRequest { offset: Some(0), limit: Some(50) })
        .await?
        .payments;
    let received = payments
        .into_iter()
        .find(|p| p.payment_type == PaymentType::Receive && p.amount >= 5_000);
    assert!(received.is_some(), "Bob should have a received payment >= 5000 sats");

    Ok(())
}
