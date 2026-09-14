use std::sync::Arc;

use bitcoin::{
    hashes::{Hash, sha256},
    secp256k1::PublicKey,
};

use crate::{
    operator::{
        OperatorPool,
        rpc::spark::{
            InitiatePreimageSwapRequest, InitiatePreimageSwapResponse, InvoiceAmount,
            InvoiceAmountProof, StartTransferRequest, initiate_preimage_swap_request::Reason,
        },
    },
    services::ServiceError,
};

pub(crate) struct SwapNodesForPreimageRequest<'a> {
    pub receiver_pubkey: &'a PublicKey,
    pub payment_hash: &'a sha256::Hash,
    pub invoice_str: Option<&'a str>,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub is_inbound_payment: bool,
    pub transfer_request: StartTransferRequest,
}

pub(crate) async fn swap_nodes_for_preimage(
    operator_pool: &Arc<OperatorPool>,
    req: SwapNodesForPreimageRequest<'_>,
) -> Result<InitiatePreimageSwapResponse, ServiceError> {
    let SwapNodesForPreimageRequest {
        receiver_pubkey,
        payment_hash,
        invoice_str,
        amount_sats,
        fee_sats,
        is_inbound_payment,
        transfer_request,
    } = req;
    let reason = if is_inbound_payment {
        Reason::Receive
    } else {
        Reason::Send
    };

    let request_data = InitiatePreimageSwapRequest {
        payment_hash: payment_hash.to_byte_array().to_vec(),
        reason: reason as i32,
        invoice_amount: Some(InvoiceAmount {
            invoice_amount_proof: invoice_str.map(|i| InvoiceAmountProof {
                bolt11_invoice: i.to_string(),
            }),
            value_sats: amount_sats,
        }),
        receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
        fee_sats,
        transfer_request: Some(transfer_request),
    };

    let response = operator_pool
        .get_coordinator()
        .client
        .initiate_preimage_swap_v3(request_data)
        .await?;
    Ok(response)
}
