//! `BitcoinChainService` backed by the local `BitcoindFixture`.
//!
//! Used by the `unilateral_exit` integration tests to drive `BreezSdk` against
//! the same regtest bitcoind that the spark-itest fixture runs.

use std::str::FromStr;

use anyhow::Result;
use bitcoin::{Address, Txid};
use breez_sdk_spark::{
    BitcoinChainService, ChainServiceError, Outspend, RecommendedFees, TxStatus, Utxo,
};
use serde_json::{Value, json};
use spark_itest::fixtures::setup::TestFixtures;

pub struct LocalBitcoindChainService {
    fixtures: std::sync::Arc<TestFixtures>,
}

impl LocalBitcoindChainService {
    #[must_use]
    pub fn new(fixtures: std::sync::Arc<TestFixtures>) -> Self {
        Self { fixtures }
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
            .fixtures
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
            .fixtures
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

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        let parsed = Txid::from_str(&txid).map_err(to_chain_err)?;
        let hex: String = self
            .fixtures
            .bitcoind
            .rpc("getrawtransaction", &[json!(parsed.to_string())])
            .await
            .map_err(to_chain_err)?;
        Ok(hex)
    }

    async fn get_outspend(&self, txid: String, vout: u32) -> Result<Outspend, ChainServiceError> {
        let parsed = Txid::from_str(&txid).map_err(to_chain_err)?;
        let target_txid = parsed.to_string();

        // bitcoind has no direct outspend RPC. Scan mempool then blocks for
        // any transaction whose inputs consume (target_txid:vout). If found,
        // return its details; otherwise report the output as unspent.
        if let Some(spend) = find_spender_in_mempool(&self.fixtures, &target_txid, vout)
            .await
            .map_err(to_chain_err)?
        {
            return Ok(spend);
        }
        if let Some(spend) = find_spender_in_blocks(&self.fixtures, &target_txid, vout)
            .await
            .map_err(to_chain_err)?
        {
            return Ok(spend);
        }
        Ok(Outspend {
            spent: false,
            txid: None,
            vin: None,
            status: None,
        })
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        let _: String = self
            .fixtures
            .bitcoind
            .rpc("sendrawtransaction", &[json!(tx)])
            .await
            .map_err(to_chain_err)?;
        Ok(())
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        // Not exercised by unilateral_exit tests.
        Ok(RecommendedFees {
            fastest_fee: 1,
            half_hour_fee: 1,
            hour_fee: 1,
            economy_fee: 1,
            minimum_fee: 1,
        })
    }
}

async fn find_spender_in_mempool(
    fixtures: &TestFixtures,
    target_txid: &str,
    vout: u32,
) -> Result<Option<Outspend>> {
    let mempool: Vec<String> = fixtures.bitcoind.rpc("getrawmempool", &[]).await?;
    for entry in mempool {
        let tx: Value = fixtures
            .bitcoind
            .rpc("getrawtransaction", &[json!(entry), json!(true)])
            .await?;
        if let Some(vin) = match_input(&tx, target_txid, vout) {
            return Ok(Some(Outspend {
                spent: true,
                txid: tx
                    .get("txid")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                vin: Some(vin),
                status: Some(TxStatus {
                    confirmed: false,
                    block_height: None,
                    block_time: None,
                }),
            }));
        }
    }
    Ok(None)
}

async fn find_spender_in_blocks(
    fixtures: &TestFixtures,
    target_txid: &str,
    vout: u32,
) -> Result<Option<Outspend>> {
    let tip_info: Value = fixtures.bitcoind.rpc("getblockchaininfo", &[]).await?;
    let tip_height = tip_info
        .get("blocks")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing blocks in getblockchaininfo"))?;

    for height in (0..=tip_height).rev() {
        let block_hash: String = fixtures
            .bitcoind
            .rpc("getblockhash", &[json!(height)])
            .await?;
        let block: Value = fixtures
            .bitcoind
            .rpc("getblock", &[json!(block_hash), json!(2)])
            .await?;
        let Some(txs) = block.get("tx").and_then(|v| v.as_array()) else {
            continue;
        };
        let block_time = block.get("time").and_then(|v| v.as_u64());
        for tx in txs {
            if let Some(vin) = match_input(tx, target_txid, vout) {
                return Ok(Some(Outspend {
                    spent: true,
                    txid: tx
                        .get("txid")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string),
                    vin: Some(vin),
                    status: Some(TxStatus {
                        confirmed: true,
                        block_height: u32::try_from(height).ok(),
                        block_time,
                    }),
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
