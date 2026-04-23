//! `BitcoinChainService` backed by the local `BitcoindFixture`.
//!
//! Used by the `unilateral_exit` integration tests to drive `BreezSdk` against
//! the same regtest bitcoind that the spark-itest fixture runs.

use std::str::FromStr;

use anyhow::Result;
use bitcoin::{Address, Txid};
use breez_sdk_spark::{BitcoinChainService, ChainServiceError, RecommendedFees, TxStatus, Utxo};
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
