use std::sync::Arc;

use bitcoin::hashes::Hash as _;
use bitcoin::hashes::sha256::Hash;
use bitcoin::secp256k1::PublicKey;
use web_time::SystemTime;

use crate::address::SparkAddress;
use crate::operator::rpc as operator_rpc;
use crate::operator::rpc::spark::ProvidePreimageRequest;
use crate::services::{Preimage, Transfer, TransferId, TransferObserver};
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
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl HtlcService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        network: Network,
        signer: Arc<dyn Signer>,
        transfer_service: Arc<TransferService>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        HtlcService {
            operator_pool,
            network,
            signer,
            transfer_service,
            transfer_observer,
        }
    }

    pub async fn create_htlc(
        &self,
        leaves: Vec<TreeNode>,
        receiver_id: &PublicKey,
        payment_hash: &Hash,
        expiry_time: SystemTime,
        transfer_id: Option<TransferId>,
    ) -> Result<Transfer, ServiceError> {
        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        if let Some(transfer_observer) = &self.transfer_observer {
            let identity_public_key = &self.signer.get_identity_public_key().await?;
            if identity_public_key != receiver_id {
                let receiver_address = SparkAddress::new(*receiver_id, self.network, None);
                let amount_sats: u64 = leaves.iter().map(|l| l.value).sum();
                transfer_observer
                    .before_send_transfer(
                        &unwrapped_transfer_id,
                        &receiver_address.to_address_string().map_err(|_| {
                            ServiceError::Generic("Failed to get pay request".to_string())
                        })?,
                        amount_sats,
                    )
                    .await?;
            }
        }

        let leaf_key_tweaks = prepare_leaf_key_tweaks_to_send(&self.signer, leaves, None).await?;

        let prepared_transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &unwrapped_transfer_id,
                &leaf_key_tweaks,
                receiver_id,
                Default::default(),
                Some(payment_hash),
                Some(expiry_time),
                None, // No adaptor public key for HTLC transfers
            )
            .await?;

        let amount_sats = leaf_key_tweaks.iter().map(|l| l.node.value).sum();

        let transfer: Transfer = match swap_nodes_for_preimage(
            &self.operator_pool,
            &self.signer,
            self.network,
            SwapNodesForPreimageRequest {
                transfer_id: &unwrapped_transfer_id,
                leaves: &leaf_key_tweaks,
                receiver_pubkey: receiver_id,
                payment_hash,
                invoice_str: None,
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: false,
                transfer_request: Some(prepared_transfer_request.transfer_request),
                expiry_time: &expiry_time,
            },
        )
        .await
        {
            Ok(response) => response
                .transfer
                .ok_or(ServiceError::SSPswapError(
                    "Swap response did not contain a transfer".to_string(),
                ))?
                .try_into()?,
            Err(e) => {
                self.transfer_service
                    .recover_transfer_on_rpc_connection_error(&unwrapped_transfer_id, e)
                    .await?
            }
        };

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
                identity_public_key: self
                    .signer
                    .get_identity_public_key()
                    .await?
                    .serialize()
                    .to_vec(),
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
                match_role: filter.match_role.into(),
                payment_hashes,
                transfer_ids: filter.transfer_ids,
                identity_public_key: filter.identity_public_key.serialize().to_vec(),
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
