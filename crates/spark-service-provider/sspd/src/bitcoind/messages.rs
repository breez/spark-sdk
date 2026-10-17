use std::collections::HashMap;

use bitcoin::Amount;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct EstimateSmartFeeResponse {
    /// The fee per kvB. Absent when bitcoind has no estimate for the target.
    #[serde(default, with = "bitcoin::amount::serde::as_btc::opt")]
    pub feerate: Option<Amount>,
}

#[derive(Deserialize)]
pub struct GetMempoolInfoResponse {
    /// The fee per kvB.
    #[serde(with = "bitcoin::amount::serde::as_btc")]
    pub mempoolminfee: Amount,
}

#[derive(Deserialize)]
pub struct GetBlockchainInfoResponse {
    pub chain: String,
    pub pruned: bool,
    pub blocks: u64,
    pub headers: u64,
    pub initialblockdownload: bool,
}

#[derive(Deserialize)]
pub struct GetBlockHeaderResponse {
    pub height: u64,
}

#[derive(Deserialize)]
pub struct GetTxSpendingPrevoutResponse {
    pub txid: String,
    pub vout: u32,
    pub spendingtxid: Option<String>,
    /// Absent while the spending transaction is unconfirmed.
    pub blockhash: Option<String>,
}

#[derive(Deserialize)]
pub struct SubmitPackageResponse {
    /// "success" once every transaction of the package is in the mempool, whether
    /// or not it was already, otherwise the package's reject reason.
    pub package_msg: String,
    /// By wtxid. Here bitcoind gives its reason for refusing a transaction.
    #[serde(rename = "tx-results", default)]
    pub tx_results: HashMap<String, SubmitPackageTxResult>,
}

#[derive(Deserialize)]
pub struct SubmitPackageTxResult {
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct GetTxOutResponse {
    pub confirmations: u64,
    /// BTC.
    pub value: f64,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: GetTxOutScriptPubKey,
}

#[derive(Deserialize)]
pub struct GetTxOutScriptPubKey {
    pub hex: String,
}
