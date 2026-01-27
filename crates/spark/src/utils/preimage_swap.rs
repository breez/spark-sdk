use std::sync::Arc;

use bitcoin::{
    hashes::{Hash, sha256},
    secp256k1::PublicKey,
};
use web_time::SystemTime;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::spark::{
            GetSigningCommitmentsRequest, InitiatePreimageSwapRequest,
            InitiatePreimageSwapResponse, InvoiceAmount, InvoiceAmountProof, StartTransferRequest,
            StartUserSignedTransferRequest, initiate_preimage_swap_request::Reason,
        },
    },
    services::{LeafKeyTweak, ServiceError, TransferId, map_signing_nonce_commitments},
    signer::Signer,
    utils::{
        refund::{SignRefundsParams, SignedRefundTransactions, sign_refunds},
        time::web_time_to_prost_timestamp,
    },
};

pub(crate) struct SwapNodesForPreimageRequest<'a> {
    pub transfer_id: &'a TransferId,
    pub leaves: &'a [LeafKeyTweak],
    pub receiver_pubkey: &'a PublicKey,
    pub payment_hash: &'a sha256::Hash,
    pub invoice_str: Option<&'a str>,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub is_inbound_payment: bool,
    pub transfer_request: Option<StartTransferRequest>,
    pub expiry_time: &'a SystemTime,
}

pub(crate) async fn swap_nodes_for_preimage(
    operator_pool: &Arc<OperatorPool>,
    signer: &Arc<dyn Signer>,
    network: Network,
    req: SwapNodesForPreimageRequest<'_>,
) -> Result<InitiatePreimageSwapResponse, ServiceError> {
    let SwapNodesForPreimageRequest {
        transfer_id,
        leaves,
        receiver_pubkey,
        payment_hash,
        invoice_str,
        amount_sats,
        fee_sats,
        is_inbound_payment,
        transfer_request,
        expiry_time,
    } = req;
    // get signing commitments
    let node_ids: Vec<String> = leaves
        .iter()
        .map(|l| l.node.id.clone().to_string())
        .collect();
    let signing_commitments = operator_pool
        .get_coordinator()
        .client
        .get_signing_commitments(GetSigningCommitmentsRequest { node_ids, count: 3 })
        .await?
        .signing_commitments
        .iter()
        .map(|sc| map_signing_nonce_commitments(&sc.signing_nonce_commitments))
        .collect::<Result<Vec<_>, _>>()?;

    let chunked_signing_commitments = signing_commitments.chunks(leaves.len()).collect::<Vec<_>>();

    if chunked_signing_commitments.len() != 3 {
        return Err(ServiceError::SSPswapError(
            "Not enough signing commitments returned".to_string(),
        ));
    }

    let cpfp_signing_commitments = chunked_signing_commitments[0].to_vec();
    let direct_signing_commitments = chunked_signing_commitments[1].to_vec();
    let direct_from_cpfp_signing_commitments = chunked_signing_commitments[2].to_vec();

    let SignedRefundTransactions {
        cpfp_signed_tx,
        direct_signed_tx,
        direct_from_cpfp_signed_tx,
    } = sign_refunds(SignRefundsParams {
        signer,
        leaves,
        cpfp_signing_commitments,
        direct_signing_commitments,
        direct_from_cpfp_signing_commitments,
        receiver_pubkey,
        payment_hash: None,
        network,
        cpfp_adaptor_public_key: None, // Preimage swaps don't use adaptor signatures
    })
    .await?;

    let reason = if is_inbound_payment {
        Reason::Receive
    } else {
        Reason::Send
    };

    // When a transfer request is provided, we do not send the direct signed txs
    let (direct_signed_tx, direct_from_cpfp_signed_tx) = if transfer_request.is_some() {
        (Vec::new(), Vec::new())
    } else {
        (direct_signed_tx, direct_from_cpfp_signed_tx)
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
        transfer: Some(StartUserSignedTransferRequest {
            transfer_id: transfer_id.to_string(),
            owner_identity_public_key: signer.get_identity_public_key().await?.serialize().to_vec(),
            receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
            expiry_time: Some(
                web_time_to_prost_timestamp(expiry_time)
                    .map_err(|_| ServiceError::Generic("Invalid expiry time".to_string()))?,
            ),
            leaves_to_send: cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_leaves_to_send: direct_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            direct_from_cpfp_leaves_to_send: direct_from_cpfp_signed_tx
                .iter()
                .map(|l| l.try_into())
                .collect::<Result<Vec<_>, _>>()?,
        }),
        receiver_identity_public_key: receiver_pubkey.serialize().to_vec(),
        fee_sats,
        transfer_request,
    };

    let response = operator_pool
        .get_coordinator()
        .client
        .initiate_preimage_swap_v3(request_data)
        .await?;
    Ok(response)
}
