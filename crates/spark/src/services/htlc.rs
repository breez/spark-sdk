use std::sync::Arc;

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::PublicKey;
use web_time::SystemTime;

use crate::operator::rpc as operator_rpc;
use crate::operator::rpc::spark::ProvidePreimageRequest;
use crate::services::{Preimage, Transfer, TransferId};
use crate::tree::TreeNode;
use crate::utils::leaf_key_tweak::prepare_leaf_key_tweaks_to_send;
use crate::utils::preimage_swap::{SwapNodesForPreimageRequest, swap_nodes_for_preimage};
use crate::{
    Network,
    operator::{OperatorPool, rpc::spark::QueryHtlcRequest},
    services::{PreimageRequestWithTransfer, QueryHtlcFilter, ServiceError, TransferService},
    signer::Signer,
    utils::paging::{PagingFilter, PagingResult, pager},
};

pub struct HtlcService {
    operator_pool: Arc<OperatorPool>,
    network: Network,
    signer: Arc<dyn Signer>,
    transfer_service: Arc<TransferService>,
}

impl HtlcService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        network: Network,
        signer: Arc<dyn Signer>,
        transfer_service: Arc<TransferService>,
    ) -> Self {
        HtlcService {
            operator_pool,
            network,
            signer,
            transfer_service,
        }
    }

    pub async fn create_htlc(
        &self,
        leaves: Vec<TreeNode>,
        receiver_id: &PublicKey,
        preimage: &Preimage,
        expiry_time: SystemTime,
    ) -> Result<Transfer, ServiceError> {
        let transfer_id = TransferId::generate();

        // TODO: run transfer observer method

        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(&self.signer, leaves, None)?;

        let payment_hash = preimage.compute_hash();

        let transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &transfer_id,
                &leaf_key_tweaks,
                receiver_id,
                Default::default(),
                Some(&payment_hash),
                Some(expiry_time),
            )
            .await?;

        let amount_sats = leaf_key_tweaks.iter().map(|l| l.node.value).sum();

        let transfer: Transfer = swap_nodes_for_preimage(
            &self.operator_pool,
            &self.signer,
            self.network,
            SwapNodesForPreimageRequest {
                transfer_id: &transfer_id,
                leaves: &leaf_key_tweaks,
                receiver_pubkey: receiver_id,
                payment_hash: &payment_hash,
                invoice_str: None,
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: false,
                transfer_request: Some(transfer_request),
                expiry_time: &expiry_time,
            },
        )
        .await?
        .transfer
        .ok_or(ServiceError::SSPswapError(
            "Swap response did not contain a transfer".to_string(),
        ))?
        .try_into()?;

        Ok(transfer)
    }

    /// Provides a preimage to the operator to claim an HTLC.
    pub async fn provide_preimage(&self, preimage: &Preimage) -> Result<Transfer, ServiceError> {
        let payment_hash = preimage.compute_hash();

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .provide_preimage(ProvidePreimageRequest {
                payment_hash: payment_hash.to_byte_array().to_vec(),
                preimage: preimage.to_vec(),
                identity_public_key: self.signer.get_identity_public_key()?.serialize().to_vec(),
            })
            .await?;

        let Some(transfer) = response.transfer else {
            return Err(ServiceError::Generic(
                "ProvidePreimageResponse did not contain a transfer".to_string(),
            ));
        };

        Transfer::try_from(transfer)
    }

    async fn query_htlc_inner(
        &self,
        filter: QueryHtlcFilter,
        paging: PagingFilter,
    ) -> Result<PagingResult<PreimageRequestWithTransfer>, ServiceError> {
        let payment_hashes = filter
            .payment_hashes
            .iter()
            .map(|h| {
                hex::decode(h)
                    .map_err(|_| ServiceError::InvalidPaymentHash(h.to_string()))
                    .unwrap()
            })
            .collect();

        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_htlc(QueryHtlcRequest {
                payment_hashes,
                identity_public_key: self.signer.get_identity_public_key()?.serialize().to_vec(),
                status: filter
                    .status
                    .map(|s| operator_rpc::spark::PreimageRequestStatus::from(s).into()),
                limit: paging.limit as i64,
                offset: paging.offset as i64,
            })
            .await?;

        Ok(PagingResult {
            items: response
                .preimage_requests
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<PreimageRequestWithTransfer>, _>>()?,
            next: paging.next_from_offset(response.offset),
        })
    }

    pub async fn query_htlc(
        &self,
        filter: QueryHtlcFilter,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<PreimageRequestWithTransfer>, ServiceError> {
        let transactions = match paging {
            Some(paging) => self.query_htlc_inner(filter, paging).await?,
            None => {
                pager(
                    |p| self.query_htlc_inner(filter.clone(), p),
                    PagingFilter::default(),
                )
                .await?
            }
        };
        Ok(transactions)
    }
}
