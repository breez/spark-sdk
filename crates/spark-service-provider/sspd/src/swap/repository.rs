#[derive(Debug, Clone)]
pub struct SwapRecord {
    pub id: String,
    pub user_identity_public_key: Vec<u8>,
    pub user_transfer_id: String,
    pub counter_transfer_id: String,
    pub reservation_id: String,
    pub total_amount_sats: i64,
    pub target_amount_sats: i64,
    pub fee_sats: i64,
}

#[derive(Debug, Clone)]
pub struct SwapDetail {
    pub swap: SwapRecord,
    pub outbound: Vec<SwapLeaf>,
    pub inbound: Vec<SwapLeaf>,
}

#[derive(Debug, Clone)]
pub struct SwapLeaf {
    pub leaf_id: String,
    pub value_sats: i64,
}
