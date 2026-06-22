//! Test helper: rebuild the signer authorization request for a prepared gated
//! send from its transfer context.
//!
//! This lives in the test crate (not on the SDK) so the public SDK surface
//! stays minimal. It uses the test-only `BreezSdk::spark_wallet` accessor plus
//! the spark-wallet request builder and the FFI-type conversion.

use anyhow::{Context, Result};
use breez_sdk_spark::signer::ExternalPrepareTransferRequest;
use breez_sdk_spark::{BreezSdk, TransferContext};
use spark_wallet::{TransferId, TreeNodeId};

/// Builds the exact `prepare_transfer` payload `send_payment` will issue for the
/// given transfer context: what a caller hands to its signer to authorize the
/// send out of band before resuming it.
pub async fn build_transfer_authorization_request(
    sdk: &BreezSdk,
    context: TransferContext,
) -> Result<ExternalPrepareTransferRequest> {
    let transfer_id: TransferId = context
        .transfer_id
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid transfer id in transfer context: {e}"))?;
    let leaf_ids = context
        .leaf_ids
        .iter()
        .map(|id| id.parse::<TreeNodeId>())
        .collect::<Result<Vec<_>, String>>()
        .map_err(|e| anyhow::anyhow!("invalid leaf id in transfer context: {e}"))?;
    let request = sdk
        .spark_wallet()
        .build_lightning_send_prepare_transfer_request(&transfer_id, &leaf_ids)
        .await
        .context("building the prepare_transfer request")?;
    ExternalPrepareTransferRequest::from_prepare_transfer_request(&request)
        .context("converting to ExternalPrepareTransferRequest")
}
