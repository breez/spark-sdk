use std::collections::HashMap;

use super::auth::OperatorAuth;
use super::error::Result;
use super::spark::*;
use crate::operator::rpc::spark::query_nodes_request::Source;
use crate::signer::Signer;
use tonic::transport::Channel;
use tracing::trace;

pub struct QueryAllNodesRequest {
    pub include_parents: bool,
    pub network: Network,
    pub source: Option<Source>,
}

pub struct SparkRpcClient<S> {
    auth: OperatorAuth<S>,
}

impl<S> SparkRpcClient<S>
where
    S: Signer,
{
    pub fn new(channel: Channel, signer: S) -> Self {
        Self {
            auth: OperatorAuth::new(channel, signer),
        }
    }

    pub async fn finalize_node_signatures(
        &self,
        req: FinalizeNodeSignaturesRequest,
    ) -> Result<FinalizeNodeSignaturesResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .finalize_node_signatures(req)
            .await?
            .into_inner())
    }

    pub async fn generate_deposit_address(
        &self,
        req: GenerateDepositAddressRequest,
    ) -> Result<GenerateDepositAddressResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .generate_deposit_address(req)
            .await?
            .into_inner())
    }

    pub async fn query_unused_deposit_addresses(
        &self,
        req: QueryUnusedDepositAddressesRequest,
    ) -> Result<QueryUnusedDepositAddressesResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_unused_deposit_addresses(req)
            .await?
            .into_inner())
    }

    pub async fn start_deposit_tree_creation(
        &self,
        req: StartDepositTreeCreationRequest,
    ) -> Result<StartDepositTreeCreationResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .start_deposit_tree_creation(req)
            .await?
            .into_inner())
    }

    pub async fn start_transfer(&self, req: StartTransferRequest) -> Result<StartTransferResponse> {
        trace!("Calling start_transfer with request: {:?}", req);
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .start_transfer(req)
            .await?
            .into_inner())
    }

    pub async fn finalize_transfer(
        &self,
        req: FinalizeTransferWithTransferPackageRequest,
    ) -> Result<FinalizeTransferResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .finalize_transfer_with_transfer_package(req)
            .await?
            .into_inner())
    }

    pub async fn cancel_transfer(
        &self,
        req: CancelTransferRequest,
    ) -> Result<CancelTransferResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .cancel_transfer(req)
            .await?
            .into_inner())
    }

    pub async fn query_pending_transfers(
        &self,
        req: TransferFilter,
    ) -> Result<QueryTransfersResponse> {
        trace!("Querying pending transfers with filter: {:?}", req);
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_pending_transfers(req)
            .await?
            .into_inner())
    }

    pub async fn query_all_transfers(&self, req: TransferFilter) -> Result<QueryTransfersResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_all_transfers(req)
            .await?
            .into_inner())
    }

    pub async fn claim_transfer_tweak_keys(
        &self,
        req: ClaimTransferTweakKeysRequest,
    ) -> Result<()> {
        self.auth
            .spark_service_client()
            .await?
            .claim_transfer_tweak_keys(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn claim_transfer_sign_refunds(
        &self,
        req: ClaimTransferSignRefundsRequest,
    ) -> Result<ClaimTransferSignRefundsResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .claim_transfer_sign_refunds(req)
            .await?
            .into_inner())
    }

    pub async fn store_preimage_share(&self, req: StorePreimageShareRequest) -> Result<()> {
        self.auth
            .spark_service_client()
            .await?
            .store_preimage_share(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn get_signing_commitments(
        &self,
        req: GetSigningCommitmentsRequest,
    ) -> Result<GetSigningCommitmentsResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .get_signing_commitments(req)
            .await?
            .into_inner())
    }

    pub async fn cooperative_exit(
        &self,
        req: CooperativeExitRequest,
    ) -> Result<CooperativeExitResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .cooperative_exit(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_preimage_swap(
        &self,
        req: InitiatePreimageSwapRequest,
    ) -> Result<InitiatePreimageSwapResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .initiate_preimage_swap(req)
            .await?
            .into_inner())
    }

    pub async fn provide_preimage(
        &self,
        req: ProvidePreimageRequest,
    ) -> Result<ProvidePreimageResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .provide_preimage(req)
            .await?
            .into_inner())
    }

    pub async fn start_leaf_swap(
        &self,
        req: StartTransferRequest,
    ) -> Result<StartTransferResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .start_leaf_swap(req)
            .await?
            .into_inner())
    }

    pub async fn counter_leaf_swap(
        &self,
        req: CounterLeafSwapRequest,
    ) -> Result<CounterLeafSwapResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .counter_leaf_swap(req)
            .await?
            .into_inner())
    }

    pub async fn refresh_timelock(
        &self,
        req: RefreshTimelockRequest,
    ) -> Result<RefreshTimelockResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .refresh_timelock(req)
            .await?
            .into_inner())
    }

    pub async fn extend_leaf(&self, req: ExtendLeafRequest) -> Result<ExtendLeafResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .extend_leaf(req)
            .await?
            .into_inner())
    }

    pub async fn prepare_tree_address(
        &self,
        req: PrepareTreeAddressRequest,
    ) -> Result<PrepareTreeAddressResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .prepare_tree_address(req)
            .await?
            .into_inner())
    }

    pub async fn create_tree(&self, req: CreateTreeRequest) -> Result<CreateTreeResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .create_tree(req)
            .await?
            .into_inner())
    }

    pub async fn get_signing_operator_list(&self) -> Result<GetSigningOperatorListResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .get_signing_operator_list(())
            .await?
            .into_inner())
    }

    // TODO: move this to an upper layer where we can handle paging for all queries where it makes sense
    pub async fn query_all_nodes(&self, req: QueryAllNodesRequest) -> Result<QueryNodesResponse> {
        let mut aggregated_nodes: HashMap<String, TreeNode> = HashMap::new();
        let page_size = 100;
        let mut offset = 0;

        loop {
            let query_request = QueryNodesRequest {
                source: req.source.clone(),
                include_parents: req.include_parents,
                limit: page_size,
                offset,
                network: req.network as i32,
            };

            let response = self.query_nodes(query_request).await?;

            // Check if we received fewer nodes than requested (this is the last page)
            let received = response.nodes.len() as i64;

            // Merge nodes from this page, deduplicating by node id
            for (node_id, node) in response.nodes {
                aggregated_nodes.insert(node_id, node);
            }
            if received < page_size {
                return Ok(QueryNodesResponse {
                    nodes: aggregated_nodes,
                    offset: response.offset,
                });
            }

            offset += page_size;
        }
    }

    pub async fn query_nodes(&self, req: QueryNodesRequest) -> Result<QueryNodesResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_nodes(req)
            .await?
            .into_inner())
    }

    pub async fn query_nodes_distribution(
        &self,
        req: QueryNodesDistributionRequest,
    ) -> Result<QueryNodesDistributionResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_nodes_distribution(req)
            .await?
            .into_inner())
    }

    pub async fn query_nodes_by_value(
        &self,
        req: QueryNodesByValueRequest,
    ) -> Result<QueryNodesByValueResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_nodes_by_value(req)
            .await?
            .into_inner())
    }

    pub async fn query_balance(&self, req: QueryBalanceRequest) -> Result<QueryBalanceResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_balance(req)
            .await?
            .into_inner())
    }

    pub async fn query_user_signed_refunds(
        &self,
        req: QueryUserSignedRefundsRequest,
    ) -> Result<QueryUserSignedRefundsResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_user_signed_refunds(req)
            .await?
            .into_inner())
    }

    pub async fn start_token_transaction(
        &self,
        req: StartTokenTransactionRequest,
    ) -> Result<StartTokenTransactionResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .start_token_transaction(req)
            .await?
            .into_inner())
    }

    pub async fn sign_token_transaction(
        &self,
        req: SignTokenTransactionRequest,
    ) -> Result<SignTokenTransactionResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .sign_token_transaction(req)
            .await?
            .into_inner())
    }

    pub async fn finalize_token_transaction(
        &self,
        req: FinalizeTokenTransactionRequest,
    ) -> Result<()> {
        self.auth
            .spark_service_client()
            .await?
            .finalize_token_transaction(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn freeze_tokens(&self, req: FreezeTokensRequest) -> Result<FreezeTokensResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .freeze_tokens(req)
            .await?
            .into_inner())
    }

    pub async fn query_token_outputs(
        &self,
        req: QueryTokenOutputsRequest,
    ) -> Result<QueryTokenOutputsResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_token_outputs(req)
            .await?
            .into_inner())
    }

    pub async fn query_token_transactions(
        &self,
        req: QueryTokenTransactionsRequest,
    ) -> Result<QueryTokenTransactionsResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_token_transactions(req)
            .await?
            .into_inner())
    }

    pub async fn return_lightning_payment(&self, req: ReturnLightningPaymentRequest) -> Result<()> {
        self.auth
            .spark_service_client()
            .await?
            .return_lightning_payment(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn query_static_deposit_addresses(
        &self,
        req: QueryStaticDepositAddressesRequest,
    ) -> Result<QueryStaticDepositAddressesResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .query_static_deposit_addresses(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_utxo_swap(
        &self,
        req: InitiateUtxoSwapRequest,
    ) -> Result<InitiateUtxoSwapResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .initiate_utxo_swap(req)
            .await?
            .into_inner())
    }

    pub async fn exit_single_node_trees(
        &self,
        req: ExitSingleNodeTreesRequest,
    ) -> Result<ExitSingleNodeTreesResponse> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .exit_single_node_trees(req)
            .await?
            .into_inner())
    }

    pub async fn subscribe_to_events(
        &self,
        req: SubscribeToEventsRequest,
    ) -> Result<tonic::codec::Streaming<SubscribeToEventsResponse>> {
        Ok(self
            .auth
            .spark_service_client()
            .await?
            .subscribe_to_events(req)
            .await?
            .into_inner())
    }
}
