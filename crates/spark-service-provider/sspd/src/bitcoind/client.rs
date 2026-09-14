use std::time::Duration;

use bitcoin::{
    Amount, Block, BlockHash, OutPoint, ScriptBuf, Transaction, TxOut,
    consensus::{deserialize, encode::serialize_hex},
    hashes::hex::FromHex,
};
use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use thiserror::Error;
use tracing::trace;

use crate::chain::{BroadcastError, ChainClient, ChainError};

use super::{
    EstimateSmartFeeResponse, GetBlockchainInfoResponse, GetMempoolInfoResponse, GetTxOutResponse,
    RpcError, RpcReply, RpcRequest, SubmitPackageResponse,
};

/// Bitcoin Core's error for broadcasting a transaction that has an output in the
/// UTXO set. Matched by code, since the message has changed between versions.
const RPC_VERIFY_ALREADY_IN_CHAIN: i64 = -27;

/// Bitcoin Core's invalid-parameter code, which `getblockhash` returns for a height
/// above the tip.
const RPC_INVALID_PARAMETER: i64 = -8;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct UnspentOutput {
    pub tx_out: TxOut,
    /// Zero while the output is only in the mempool.
    pub confirmations: u64,
}

pub struct ChainInfo {
    pub network: bitcoin::Network,
    pub pruned: bool,
}

#[derive(Debug)]
pub struct BitcoindClient {
    address: String,
    user: String,
    password: String,
    http: reqwest::Client,
}

#[derive(Debug, Error)]
pub(super) enum CallError {
    #[error("rpc error: {0:?}")]
    RpcError(RpcError),

    #[error("deserialize error: {0}")]
    Deserialize(serde_json::error::Error),

    #[error("{0}")]
    General(Box<dyn std::error::Error + Sync + Send>),
}

impl From<reqwest::Error> for CallError {
    fn from(value: reqwest::Error) -> Self {
        CallError::General(Box::new(value.without_url()))
    }
}

impl BitcoindClient {
    pub fn new(address: String, user: String, password: String) -> Result<Self, reqwest::Error> {
        // sspd has no proxy setting for this client to honour.
        #[allow(clippy::disallowed_methods)]
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            address,
            user,
            password,
            http,
        })
    }

    async fn call<TParams, TResponse>(
        &self,
        method: &str,
        params: TParams,
    ) -> Result<TResponse, CallError>
    where
        TParams: Serialize,
        TResponse: DeserializeOwned,
    {
        self.call_with_timeout(method, params, REQUEST_TIMEOUT)
            .await
    }

    async fn call_with_timeout<TParams, TResponse>(
        &self,
        method: &str,
        params: TParams,
        timeout: Duration,
    ) -> Result<TResponse, CallError>
    where
        TParams: Serialize,
        TResponse: DeserializeOwned,
    {
        trace!("calling {}", method);
        let reply: RpcReply = self
            .http
            .request(Method::POST, &self.address)
            .basic_auth(&self.user, Some(&self.password))
            .timeout(timeout)
            .json(&RpcRequest {
                jsonrpc: "1.0",
                id: "sspd",
                method: method.to_string(),
                params,
            })
            .send()
            .await?
            .json()
            .await?;
        match reply {
            RpcReply::Response { result } => Ok(serde_json::from_value(result)?),
            RpcReply::Error { error } => Err(CallError::RpcError(error)),
        }
    }

    pub async fn chain_info(&self) -> Result<ChainInfo, ChainError> {
        let info: GetBlockchainInfoResponse = self.call("getblockchaininfo", json!([])).await?;
        let network = bitcoin::Network::from_core_arg(&info.chain).map_err(|e| {
            ChainError::General(format!("unknown chain {}: {e}", info.chain).into())
        })?;
        Ok(ChainInfo {
            network,
            pruned: info.pruned,
        })
    }

    /// The output at `outpoint` while it is unspent, counting the mempool. `None`
    /// once it is spent, by a mempool transaction too, or if there is no such
    /// output. Unlike looking the transaction up, this needs no transaction index.
    pub async fn unspent_output(
        &self,
        outpoint: &OutPoint,
    ) -> Result<Option<UnspentOutput>, ChainError> {
        let resp: Option<GetTxOutResponse> = self
            .call(
                "gettxout",
                json!([outpoint.txid.to_string(), outpoint.vout, true]),
            )
            .await?;
        resp.map(|output| {
            Ok(UnspentOutput {
                tx_out: TxOut {
                    value: Amount::from_btc(output.value)
                        .map_err(|e| ChainError::General(Box::new(e)))?,
                    script_pubkey: ScriptBuf::from_hex(&output.script_pub_key.hex)
                        .map_err(|e| ChainError::General(Box::new(e)))?,
                },
                confirmations: output.confirmations,
            })
        })
        .transpose()
    }
}

#[async_trait::async_trait]
impl ChainClient for BitcoindClient {
    async fn broadcast_tx(&self, tx: Transaction) -> Result<(), BroadcastError> {
        let hex = serialize_hex(&tx);
        trace!(tx = hex, "broadcasting tx");
        let _txid: Value = self.call("sendrawtransaction", json!([hex])).await?;
        Ok(())
    }

    async fn broadcast_package(&self, txs: &[Transaction]) -> Result<(), BroadcastError> {
        let hexes: Vec<String> = txs.iter().map(serialize_hex).collect();
        let response: SubmitPackageResponse = self.call("submitpackage", json!([hexes])).await?;
        if response.package_msg == "success" {
            Ok(())
        } else {
            Err(BroadcastError::UnknownError(response.package_msg))
        }
    }

    async fn estimate_fee_rate(&self, conf_target: u32) -> Result<u64, ChainError> {
        let estimate: EstimateSmartFeeResponse = self
            .call("estimatesmartfee", json!([conf_target.clamp(1, 1008)]))
            .await?;
        // A node without enough fee history answers with no `feerate`; the least
        // it relays then is the rate a transaction has to pay to be accepted.
        let btc_per_kvb = if let Some(feerate) = estimate.feerate {
            feerate
        } else {
            let info: GetMempoolInfoResponse = self.call("getmempoolinfo", json!([])).await?;
            info.mempoolminfee
        };
        // 1 BTC/kvB == 25_000_000 sat/kw.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let sat_per_kw = (btc_per_kvb * 25_000_000.0).ceil() as u64;
        Ok(sat_per_kw)
    }

    async fn get_blockheight(&self) -> Result<u64, ChainError> {
        Ok(self.call("getblockcount", json!([])).await?)
    }

    async fn get_block_hash(&self, height: u64) -> Result<Option<BlockHash>, ChainError> {
        match self
            .call::<_, String>("getblockhash", json!([height]))
            .await
        {
            Ok(hash) => Ok(Some(hash.parse()?)),
            Err(CallError::RpcError(e)) if e.code == RPC_INVALID_PARAMETER => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_block(&self, hash: &BlockHash) -> Result<Block, ChainError> {
        let hex: String = self.call("getblock", json!([hash.to_string(), 0])).await?;
        let raw: Vec<u8> = FromHex::from_hex(&hex)?;
        Ok(deserialize(&raw)?)
    }

    async fn wait_for_block_height(
        &self,
        height: u64,
        timeout: Duration,
    ) -> Result<(), ChainError> {
        let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let _tip: Value = self
            .call_with_timeout(
                "waitforblockheight",
                json!([height, timeout_ms]),
                timeout.saturating_add(REQUEST_TIMEOUT),
            )
            .await?;
        Ok(())
    }
}

impl From<bitcoin::consensus::encode::Error> for ChainError {
    fn from(value: bitcoin::consensus::encode::Error) -> Self {
        ChainError::General(Box::new(value))
    }
}

impl From<CallError> for ChainError {
    fn from(value: CallError) -> Self {
        match value {
            CallError::RpcError(e) => ChainError::General(e.message.into()),
            CallError::Deserialize(e) => ChainError::General(Box::new(e)),
            CallError::General(e) => ChainError::General(e),
        }
    }
}

impl From<CallError> for BroadcastError {
    fn from(value: CallError) -> Self {
        match value {
            CallError::RpcError(rpc_error) => {
                let msg = &rpc_error.message;
                if rpc_error.code == RPC_VERIFY_ALREADY_IN_CHAIN
                    || msg.contains("txn-already-known")
                    || msg.contains("txn-already-in-mempool")
                {
                    BroadcastError::AlreadyKnown
                } else {
                    BroadcastError::UnknownError(rpc_error.message)
                }
            }
            CallError::Deserialize(_) | CallError::General(_) => {
                BroadcastError::Chain(value.into())
            }
        }
    }
}

impl From<serde_json::error::Error> for CallError {
    fn from(value: serde_json::error::Error) -> Self {
        CallError::Deserialize(value)
    }
}

impl From<bitcoin::address::FromScriptError> for ChainError {
    fn from(value: bitcoin::address::FromScriptError) -> Self {
        ChainError::General(Box::new(value))
    }
}

impl From<bitcoin::hashes::hex::HexToArrayError> for ChainError {
    fn from(value: bitcoin::hashes::hex::HexToArrayError) -> Self {
        ChainError::General(Box::new(value))
    }
}

impl From<bitcoin::hashes::hex::HexToBytesError> for ChainError {
    fn from(value: bitcoin::hashes::hex::HexToBytesError) -> Self {
        ChainError::General(Box::new(value))
    }
}
