//! `BitcoinChainService` backed by the local `BitcoindFixture`.
//!
//! It holds only bitcoind's RPC endpoint, not the operator cluster. The SDK's
//! background tasks keep the chain service alive, so pinning the whole
//! `TestFixtures` here would keep a finished test's operator containers running
//! — leaking clusters across the run and starving the runner.

use std::collections::HashMap;
use std::str::FromStr;

use anyhow::Result;
use bitcoin::{Address, Txid};
use breez_sdk_spark::{
    BitcoinChainService, ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo,
};
use platform_utils::{
    ContentType, DefaultHttpClient, HttpClient, add_basic_auth_header, add_content_type_header,
};
use serde::Deserialize;
use serde_json::{Value, json};
use spark_itest::fixtures::bitcoind::BitcoindFixture;

/// A detached bitcoind JSON-RPC client: the endpoint and credentials plus its
/// own HTTP client, holding no fixture, so the SDK never pins the cluster.
struct BitcoindRpc {
    rpc_url: String,
    rpcuser: String,
    rpcpassword: String,
    http: DefaultHttpClient,
}

#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<Value>,
}

impl BitcoindRpc {
    fn new(bitcoind: &BitcoindFixture) -> Self {
        Self {
            rpc_url: bitcoind.rpc_url.clone(),
            rpcuser: bitcoind.rpcuser.clone(),
            rpcpassword: bitcoind.rpcpassword.clone(),
            http: DefaultHttpClient::default(),
        }
    }

    async fn rpc<T: for<'de> Deserialize<'de>>(&self, method: &str, params: &[Value]) -> Result<T> {
        let body = serde_json::to_string(&json!({
            "jsonrpc": "1.0",
            "id": "rust-client",
            "method": method,
            "params": params,
        }))?;
        let mut headers = HashMap::new();
        add_basic_auth_header(&mut headers, &self.rpcuser, &self.rpcpassword);
        add_content_type_header(&mut headers, ContentType::Json);
        let response = self
            .http
            .post(self.rpc_url.clone(), Some(headers), Some(body))
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request failed: {e:?}"))?;
        if !response.is_success() {
            return Err(anyhow::anyhow!(
                "bitcoind error status: {}",
                response.status
            ));
        }
        let parsed: RpcResponse<T> = response.json()?;
        match (parsed.result, parsed.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(anyhow::anyhow!("RPC error: {error:?}")),
            _ => Err(anyhow::anyhow!("Invalid RPC response")),
        }
    }
}

pub struct LocalBitcoindChainService {
    bitcoind: BitcoindRpc,
}

impl LocalBitcoindChainService {
    #[must_use]
    pub fn new(bitcoind: &BitcoindFixture) -> Self {
        Self {
            bitcoind: BitcoindRpc::new(bitcoind),
        }
    }
}

fn to_chain_err<E: std::fmt::Display>(e: E) -> ChainServiceError {
    ChainServiceError::Generic(e.to_string())
}

#[macros::async_trait]
impl BitcoinChainService for LocalBitcoindChainService {
    async fn get_transaction_status(&self, txid: String) -> Result<TxStatus, ChainServiceError> {
        let parsed = Txid::from_str(&txid).map_err(to_chain_err)?;
        let result: Value = self
            .bitcoind
            .rpc(
                "getrawtransaction",
                &[json!(parsed.to_string()), json!(true)],
            )
            .await
            .map_err(to_chain_err)?;
        let confirmations = result
            .get("confirmations")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let block_height = result
            .get("blockheight")
            .and_then(|c| c.as_u64())
            .and_then(|h| u32::try_from(h).ok());
        let block_time = result.get("blocktime").and_then(|c| c.as_u64());
        Ok(TxStatus {
            confirmed: confirmations > 0,
            block_height,
            block_time,
        })
    }

    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        let parsed: Address<_> = address.parse().map_err(to_chain_err)?;
        let checked = parsed
            .require_network(bitcoin::Network::Regtest)
            .map_err(to_chain_err)?;
        let descriptor = format!("addr({checked})");
        let result: Value = self
            .bitcoind
            .rpc(
                "scantxoutset",
                &[json!("start"), json!(vec![json!({ "desc": descriptor })])],
            )
            .await
            .map_err(to_chain_err)?;
        let Some(unspents) = result.get("unspents").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };
        let mut utxos = Vec::with_capacity(unspents.len());
        for entry in unspents {
            let txid = entry
                .get("txid")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ChainServiceError::Generic("missing txid".to_string()))?
                .to_string();
            let vout = entry
                .get("vout")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| ChainServiceError::Generic("missing vout".to_string()))?;
            let amount_btc = entry
                .get("amount")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ChainServiceError::Generic("missing amount".to_string()))?;
            let value = (amount_btc * 100_000_000.0).round() as u64;
            let height = entry.get("height").and_then(|v| v.as_u64());
            utxos.push(Utxo {
                txid,
                vout: u32::try_from(vout)
                    .map_err(|_| ChainServiceError::Generic("vout overflow".to_string()))?,
                value,
                status: TxStatus {
                    confirmed: height.is_some(),
                    block_height: height.and_then(|h| u32::try_from(h).ok()),
                    block_time: None,
                },
            });
        }
        Ok(utxos)
    }

    async fn get_address_txos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        let parsed: Address<_> = address.parse().map_err(to_chain_err)?;
        let checked = parsed
            .require_network(bitcoin::Network::Regtest)
            .map_err(to_chain_err)?;
        let script_hex = checked.script_pubkey().to_hex_string();
        confirmed_txos_for_script(&self.bitcoind, &script_hex)
            .await
            .map_err(to_chain_err)
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        let parsed = Txid::from_str(&txid).map_err(to_chain_err)?;
        let hex: String = self
            .bitcoind
            .rpc("getrawtransaction", &[json!(parsed.to_string())])
            .await
            .map_err(to_chain_err)?;
        Ok(hex)
    }

    async fn get_outspend(&self, txid: String, vout: u32) -> Result<Outspend, ChainServiceError> {
        let parsed = Txid::from_str(&txid).map_err(to_chain_err)?;
        let target_txid = parsed.to_string();

        // bitcoind has no direct outspend RPC: scan mempool then blocks for a
        // transaction whose inputs consume (target_txid:vout).
        if let Some(spend) = find_spender_in_mempool(&self.bitcoind, &target_txid, vout)
            .await
            .map_err(to_chain_err)?
        {
            return Ok(spend);
        }
        if let Some(spend) = find_spender_in_blocks(&self.bitcoind, &target_txid, vout)
            .await
            .map_err(to_chain_err)?
        {
            return Ok(spend);
        }
        Ok(Outspend::Unspent)
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        let _: String = self
            .bitcoind
            .rpc("sendrawtransaction", &[json!(tx)])
            .await
            .map_err(to_chain_err)?;
        Ok(())
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        Ok(RecommendedFees {
            fastest_fee: 1,
            half_hour_fee: 1,
            hour_fee: 1,
            economy_fee: 1,
            minimum_fee: 1,
        })
    }
}

/// Every confirmed output ever paid to `script_hex`, spent or not. bitcoind has
/// no address index, so scan every block (as `find_spender_in_blocks` does),
/// matching each output's scriptPubKey against the target.
async fn confirmed_txos_for_script(bitcoind: &BitcoindRpc, script_hex: &str) -> Result<Vec<Utxo>> {
    let tip_info: Value = bitcoind.rpc("getblockchaininfo", &[]).await?;
    let tip_height = tip_info
        .get("blocks")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing blocks in getblockchaininfo"))?;

    let mut txos = Vec::new();
    for height in 0..=tip_height {
        let block_hash: String = bitcoind.rpc("getblockhash", &[json!(height)]).await?;
        let block: Value = bitcoind
            .rpc("getblock", &[json!(block_hash), json!(2)])
            .await?;
        let block_time = block.get("time").and_then(|v| v.as_u64());
        let Some(txs) = block.get("tx").and_then(|v| v.as_array()) else {
            continue;
        };
        for tx in txs {
            let Some(txid) = tx.get("txid").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(vouts) = tx.get("vout").and_then(|v| v.as_array()) else {
                continue;
            };
            for out in vouts {
                let matches = out
                    .get("scriptPubKey")
                    .and_then(|s| s.get("hex"))
                    .and_then(|h| h.as_str())
                    .is_some_and(|h| h == script_hex);
                if !matches {
                    continue;
                }
                let n = out.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
                let value_btc = out.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
                txos.push(Utxo {
                    txid: txid.to_string(),
                    vout: u32::try_from(n).unwrap_or(0),
                    value: (value_btc * 100_000_000.0).round() as u64,
                    status: TxStatus {
                        confirmed: true,
                        block_height: u32::try_from(height).ok(),
                        block_time,
                    },
                });
            }
        }
    }
    Ok(txos)
}

async fn find_spender_in_mempool(
    bitcoind: &BitcoindRpc,
    target_txid: &str,
    vout: u32,
) -> Result<Option<Outspend>> {
    let mempool: Vec<String> = bitcoind.rpc("getrawmempool", &[]).await?;
    for entry in mempool {
        let tx: Value = bitcoind
            .rpc("getrawtransaction", &[json!(entry), json!(true)])
            .await?;
        if let Some(vin) = match_input(&tx, target_txid, vout) {
            let spender_txid = tx
                .get("txid")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("mempool spender missing txid"))?
                .to_string();
            return Ok(Some(Outspend::Spent {
                txid: spender_txid,
                vin,
                status: TxStatus {
                    confirmed: false,
                    block_height: None,
                    block_time: None,
                },
            }));
        }
    }
    Ok(None)
}

async fn find_spender_in_blocks(
    bitcoind: &BitcoindRpc,
    target_txid: &str,
    vout: u32,
) -> Result<Option<Outspend>> {
    let tip_info: Value = bitcoind.rpc("getblockchaininfo", &[]).await?;
    let tip_height = tip_info
        .get("blocks")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing blocks in getblockchaininfo"))?;

    for height in (0..=tip_height).rev() {
        let block_hash: String = bitcoind.rpc("getblockhash", &[json!(height)]).await?;
        let block: Value = bitcoind
            .rpc("getblock", &[json!(block_hash), json!(2)])
            .await?;
        let Some(txs) = block.get("tx").and_then(|v| v.as_array()) else {
            continue;
        };
        let block_time = block.get("time").and_then(|v| v.as_u64());
        for tx in txs {
            if let Some(vin) = match_input(tx, target_txid, vout) {
                let spender_txid = tx
                    .get("txid")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("block spender missing txid"))?
                    .to_string();
                return Ok(Some(Outspend::Spent {
                    txid: spender_txid,
                    vin,
                    status: TxStatus {
                        confirmed: true,
                        block_height: u32::try_from(height).ok(),
                        block_time,
                    },
                }));
            }
        }
    }
    Ok(None)
}

fn match_input(tx: &Value, target_txid: &str, vout: u32) -> Option<u32> {
    let inputs = tx.get("vin")?.as_array()?;
    for (idx, input) in inputs.iter().enumerate() {
        let Some(in_txid) = input.get("txid").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(in_vout) = input.get("vout").and_then(|v| v.as_u64()) else {
            continue;
        };
        if in_txid == target_txid && u32::try_from(in_vout).ok() == Some(vout) {
            return u32::try_from(idx).ok();
        }
    }
    None
}
