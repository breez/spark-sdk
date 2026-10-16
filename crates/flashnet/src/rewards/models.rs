use serde::{Deserialize, Serialize};

/// The share of an endorser's rewards that endorsements can allocate in total.
pub const MAX_RATIO_BPS: u16 = 10_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Endorsements {
    pub endorser: String,
    pub endorsements: Vec<Endorsement>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Endorsement {
    /// Recipient pubkey, compressed hex.
    pub to: String,
    pub ratio_bps: u16,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpsertEndorsementRequest {
    pub endorser: String,
    pub to: String,
    pub ratio_bps: u16,
    pub timestamp_nonce: String,
    pub signature: String,
}
