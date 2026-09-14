use serde::Deserialize;

#[derive(Deserialize)]
pub struct EstimateSmartFeeResponse {
    /// BTC/kvB. Absent when the node has no estimate for the target.
    pub feerate: Option<f64>,
}

#[derive(Deserialize)]
pub struct GetMempoolInfoResponse {
    /// BTC/kvB.
    pub mempoolminfee: f64,
}

#[derive(Deserialize)]
pub struct GetBlockchainInfoResponse {
    pub chain: String,
    pub pruned: bool,
}

#[derive(Deserialize)]
pub struct SubmitPackageResponse {
    /// "success" once every transaction of the package is in the mempool, whether
    /// or not it was already, otherwise the package's reject reason.
    pub package_msg: String,
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
