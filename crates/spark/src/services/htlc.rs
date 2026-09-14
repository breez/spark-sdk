use std::sync::Arc;

use bitcoin::hashes::Hash as _;
use bitcoin::hashes::sha256::Hash;
use bitcoin::secp256k1::PublicKey;
use platform_utils::time::SystemTime;

use crate::address::SparkAddress;
use crate::operator::rpc as operator_rpc;
use crate::operator::rpc::spark::ProvidePreimageRequest;
use crate::services::models::convert_page;
use crate::services::{Preimage, Transfer, TransferId, TransferObserver};
use crate::utils::preimage_swap::{SwapNodesForPreimageRequest, swap_nodes_for_preimage};
use crate::{
    Network,
    operator::{OperatorPool, rpc::spark::QueryHtlcRequest},
    services::{
        LeafKeyTweak, PreimageRequestWithTransfer, QueryHtlcFilter, ServiceError, TransferService,
    },
    signer::SparkSigner,
    utils::paging::{PagingFilter, PagingResult, pager},
};

pub struct HtlcService {
    operator_pool: Arc<OperatorPool>,
    network: Network,
    spark_signer: Arc<dyn SparkSigner>,
    transfer_service: Arc<TransferService>,
    transfer_observer: Option<Arc<dyn TransferObserver>>,
}

impl HtlcService {
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        network: Network,
        spark_signer: Arc<dyn SparkSigner>,
        transfer_service: Arc<TransferService>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
    ) -> Self {
        HtlcService {
            operator_pool,
            network,
            spark_signer,
            transfer_service,
            transfer_observer,
        }
    }

    pub async fn create_htlc(
        &self,
        leaves: &[LeafKeyTweak],
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
            let identity_public_key = &self.spark_signer.get_identity_public_key().await?;
            if identity_public_key != receiver_id {
                let receiver_address = SparkAddress::new(*receiver_id, self.network, None);
                let amount_sats: u64 = leaves.iter().map(|l| l.node.value).sum();
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

        let prepared_transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &unwrapped_transfer_id,
                leaves,
                receiver_id,
                Some(payment_hash),
                Some(expiry_time),
                None, // No adaptor public key for HTLC transfers
            )
            .await?;

        let amount_sats = leaves.iter().map(|l| l.node.value).sum();

        let transfer: Transfer = match swap_nodes_for_preimage(
            &self.operator_pool,
            SwapNodesForPreimageRequest {
                receiver_pubkey: receiver_id,
                payment_hash,
                invoice_str: None,
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: false,
                transfer_request: prepared_transfer_request.transfer_request,
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
                    .recover_committed_transfer(&unwrapped_transfer_id, e)
                    .await?
            }
        };

        Ok(transfer)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn receive_htlc_with_tweaks(
        &self,
        leaf_key_tweaks: Vec<LeafKeyTweak>,
        receiver_id: &PublicKey,
        payment_hash: &Hash,
        invoice: &str,
        amount_sats: u64,
        expiry_time: SystemTime,
        transfer_id: Option<TransferId>,
    ) -> Result<(Transfer, Option<Preimage>), ServiceError> {
        let unwrapped_transfer_id = match &transfer_id {
            Some(transfer_id) => transfer_id.clone(),
            None => TransferId::generate(),
        };

        let prepared_transfer_request = self
            .transfer_service
            .prepare_transfer_request(
                &unwrapped_transfer_id,
                &leaf_key_tweaks,
                receiver_id,
                // Plain P2TR refunds paying the receiver, not hash-locked ones: the
                // operators validate a receive swap's refunds against the receiver's
                // identity key and reject HTLC outputs (only a send's refunds are
                // hash-locked). The fronted leaves are backed by the transfer's
                // `expiry_time` instead, which returns them if no preimage follows.
                None,
                Some(expiry_time),
                None, // No adaptor public key for HTLC transfers
            )
            .await?;

        match swap_nodes_for_preimage(
            &self.operator_pool,
            SwapNodesForPreimageRequest {
                receiver_pubkey: receiver_id,
                payment_hash,
                invoice_str: Some(invoice),
                amount_sats,
                fee_sats: 0,
                is_inbound_payment: true,
                transfer_request: prepared_transfer_request.transfer_request,
            },
        )
        .await
        {
            Ok(response) => {
                let transfer: Transfer = response
                    .transfer
                    .ok_or(ServiceError::SSPswapError(
                        "Swap response did not contain a transfer".to_string(),
                    ))?
                    .try_into()?;
                // Empty preimage means the share was not present (a HODL invoice):
                // the transfer is committed but the preimage must be provided later.
                let preimage = if response.preimage.is_empty() {
                    None
                } else {
                    let preimage = Preimage::try_from(response.preimage)?;
                    if preimage.compute_hash() != *payment_hash {
                        return Err(ServiceError::SSPswapError(
                            "preimage does not match the payment hash".to_string(),
                        ));
                    }
                    Some(preimage)
                };
                Ok((transfer, preimage))
            }
            // A failed swap may still have committed; recover it and let the caller
            // learn the preimage via `query_htlc`.
            Err(e) => {
                let transfer = self
                    .transfer_service
                    .recover_committed_transfer(&unwrapped_transfer_id, e)
                    .await?;
                Ok((transfer, None))
            }
        }
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
                    .spark_signer
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
        let payment_hashes = decode_payment_hashes(&filter.payment_hashes)?;

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
            items: convert_page(response.preimage_requests, "preimage request")?,
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

fn decode_payment_hashes(payment_hashes: &[String]) -> Result<Vec<Vec<u8>>, ServiceError> {
    payment_hashes
        .iter()
        .map(|h| hex::decode(h).map_err(|_| ServiceError::InvalidPaymentHash(h.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::decode_payment_hashes;
    use crate::services::ServiceError;

    #[test]
    fn decodes_hex_payment_hashes() {
        let hash = "a".repeat(64);
        assert_eq!(
            decode_payment_hashes(std::slice::from_ref(&hash)).unwrap(),
            vec![hex::decode(&hash).unwrap()]
        );
    }

    #[test]
    fn rejects_a_non_hex_payment_hash() {
        assert!(matches!(
            decode_payment_hashes(&["zz".to_string()]),
            Err(ServiceError::InvalidPaymentHash(h)) if h == "zz"
        ));
    }

    #[test]
    fn rejects_an_odd_length_payment_hash() {
        assert!(matches!(
            decode_payment_hashes(&["abc".to_string()]),
            Err(ServiceError::InvalidPaymentHash(_))
        ));
    }
}
