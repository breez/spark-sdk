use std::collections::HashMap;
use std::sync::Arc;

use super::auth::OperatorAuth;
use super::error::Result;
use super::spark::*;
use super::spark_token;
use crate::operator::OperatorSession;
use crate::operator::SessionManager;
use crate::operator::rpc::OperatorRpcError;
use crate::operator::rpc::spark::query_nodes_request::Source;
use crate::operator::rpc::spark::spark_service_client::SparkServiceClient;
use crate::operator::rpc::spark_token::CommitTransactionRequest;
use crate::operator::rpc::spark_token::CommitTransactionResponse;
use crate::operator::rpc::spark_token::StartTransactionRequest;
use crate::operator::rpc::spark_token::StartTransactionResponse;
use crate::operator::rpc::spark_token::spark_token_service_client::SparkTokenServiceClient;
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::signer::Signer;
use bitcoin::secp256k1::PublicKey;
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tracing::error;
use tracing::trace;

pub struct QueryAllNodesRequest {
    pub include_parents: bool,
    pub network: Network,
    pub source: Option<Source>,
}

#[derive(Clone)]
pub struct SparkRpcClient<S> {
    transport: Transport,
    auth: OperatorAuth<S>,
    session_manager: Arc<dyn SessionManager>,
    identity_public_key: PublicKey,
}

impl<S> SparkRpcClient<S>
where
    S: Signer,
{
    pub fn new(
        channel: Transport,
        signer: Arc<S>,
        identity_public_key: PublicKey,
        session_manager: Arc<dyn SessionManager>,
    ) -> Self {
        Self {
            transport: channel.clone(),
            auth: OperatorAuth::new(channel, signer),
            session_manager,
            identity_public_key,
        }
    }

    pub async fn finalize_node_signatures_v2(
        &self,
        req: FinalizeNodeSignaturesRequest,
    ) -> Result<FinalizeNodeSignaturesResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .finalize_node_signatures_v2(req)
            .await?
            .into_inner())
    }

    pub async fn generate_deposit_address(
        &self,
        req: GenerateDepositAddressRequest,
    ) -> Result<GenerateDepositAddressResponse> {
        Ok(self
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
            .spark_service_client()
            .await?
            .start_deposit_tree_creation(req)
            .await?
            .into_inner())
    }

    pub async fn start_transfer_v2(
        &self,
        req: StartTransferRequest,
    ) -> Result<StartTransferResponse> {
        trace!("Calling start_transfer with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .start_transfer_v2(req)
            .await?
            .into_inner())
    }

    pub async fn finalize_transfer(
        &self,
        req: FinalizeTransferWithTransferPackageRequest,
    ) -> Result<FinalizeTransferResponse> {
        Ok(self
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
            .spark_service_client()
            .await?
            .query_pending_transfers(req)
            .await?
            .into_inner())
    }

    pub async fn query_all_transfers(&self, req: TransferFilter) -> Result<QueryTransfersResponse> {
        Ok(self
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
        self.spark_service_client()
            .await?
            .claim_transfer_tweak_keys(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn claim_transfer_sign_refunds_v2(
        &self,
        req: ClaimTransferSignRefundsRequest,
    ) -> Result<ClaimTransferSignRefundsResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .claim_transfer_sign_refunds_v2(req)
            .await?
            .into_inner())
    }

    pub async fn store_preimage_share(&self, req: StorePreimageShareRequest) -> Result<()> {
        self.spark_service_client()
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
            .spark_service_client()
            .await?
            .get_signing_commitments(req)
            .await?
            .into_inner())
    }

    pub async fn cooperative_exit_v2(
        &self,
        req: CooperativeExitRequest,
    ) -> Result<CooperativeExitResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .cooperative_exit_v2(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_preimage_swap_v2(
        &self,
        req: InitiatePreimageSwapRequest,
    ) -> Result<InitiatePreimageSwapResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .initiate_preimage_swap_v2(req)
            .await?
            .into_inner())
    }

    pub async fn provide_preimage(
        &self,
        req: ProvidePreimageRequest,
    ) -> Result<ProvidePreimageResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .provide_preimage(req)
            .await?
            .into_inner())
    }

    pub async fn start_leaf_swap_v2(
        &self,
        req: StartTransferRequest,
    ) -> Result<StartTransferResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .start_leaf_swap_v2(req)
            .await?
            .into_inner())
    }

    pub async fn counter_leaf_swap_v2(
        &self,
        req: CounterLeafSwapRequest,
    ) -> Result<CounterLeafSwapResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .counter_leaf_swap_v2(req)
            .await?
            .into_inner())
    }

    pub async fn refresh_timelock_v2(
        &self,
        req: RefreshTimelockRequest,
    ) -> Result<RefreshTimelockResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .refresh_timelock_v2(req)
            .await?
            .into_inner())
    }

    pub async fn extend_leaf_v2(&self, req: ExtendLeafRequest) -> Result<ExtendLeafResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .extend_leaf_v2(req)
            .await?
            .into_inner())
    }

    pub async fn get_signing_operator_list(&self) -> Result<GetSigningOperatorListResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .get_signing_operator_list(())
            .await?
            .into_inner())
    }

    // TODO: move this to an upper layer where we can handle paging for all queries where it makes sense
    pub async fn query_nodes_all_pages(
        &self,
        req: QueryAllNodesRequest,
    ) -> Result<QueryNodesResponse> {
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
            .spark_service_client()
            .await?
            .query_nodes_by_value(req)
            .await?
            .into_inner())
    }

    pub async fn query_balance(&self, req: QueryBalanceRequest) -> Result<QueryBalanceResponse> {
        Ok(self
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
        self.spark_service_client()
            .await?
            .finalize_token_transaction(req)
            .await?
            .into_inner();
        Ok(())
    }

    pub async fn freeze_tokens(
        &self,
        req: spark_token::FreezeTokensRequest,
    ) -> Result<spark_token::FreezeTokensResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .freeze_tokens(req)
            .await?
            .into_inner())
    }

    pub async fn query_token_outputs(
        &self,
        req: spark_token::QueryTokenOutputsRequest,
    ) -> Result<spark_token::QueryTokenOutputsResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .query_token_outputs(req)
            .await?
            .into_inner())
    }

    pub async fn query_token_metadata(
        &self,
        req: spark_token::QueryTokenMetadataRequest,
    ) -> Result<spark_token::QueryTokenMetadataResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .query_token_metadata(req)
            .await?
            .into_inner())
    }

    pub async fn query_token_transactions(
        &self,
        req: spark_token::QueryTokenTransactionsRequest,
    ) -> Result<spark_token::QueryTokenTransactionsResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .query_token_transactions(req)
            .await?
            .into_inner())
    }

    pub async fn start_transaction(
        &self,
        req: StartTransactionRequest,
    ) -> Result<StartTransactionResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .start_transaction(req)
            .await?
            .into_inner())
    }

    pub async fn commit_transaction(
        &self,
        req: CommitTransactionRequest,
    ) -> Result<CommitTransactionResponse> {
        Ok(self
            .spark_token_service_client()
            .await?
            .commit_transaction(req)
            .await?
            .into_inner())
    }

    pub async fn return_lightning_payment(&self, req: ReturnLightningPaymentRequest) -> Result<()> {
        self.spark_service_client()
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
            .spark_service_client()
            .await?
            .query_static_deposit_addresses(req)
            .await?
            .into_inner())
    }

    pub async fn get_utxos_for_address(
        &self,
        req: GetUtxosForAddressRequest,
    ) -> Result<GetUtxosForAddressResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .get_utxos_for_address(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_static_deposit_utxo_refund(
        &self,
        req: InitiateStaticDepositUtxoRefundRequest,
    ) -> Result<InitiateStaticDepositUtxoRefundResponse> {
        Ok(self
            .spark_service_client()
            .await?
            .initiate_static_deposit_utxo_refund(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_utxo_swap(
        &self,
        req: InitiateUtxoSwapRequest,
    ) -> Result<InitiateUtxoSwapResponse> {
        // TODO: update to drop use of deprecated initiate_utxo_swap call
        #[allow(deprecated)]
        Ok(self
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
            .spark_service_client()
            .await?
            .subscribe_to_events(req)
            .await?
            .into_inner())
    }

    async fn spark_service_client(
        &self,
    ) -> Result<SparkServiceClient<InterceptedService<Transport, OperationSessionInterceptor>>>
    {
        let session = self.get_session_interceptor().await?;
        Ok(SparkServiceClient::with_interceptor(
            self.transport.clone(),
            session,
        ))
    }

    async fn spark_token_service_client(
        &self,
    ) -> Result<SparkTokenServiceClient<InterceptedService<Transport, OperationSessionInterceptor>>>
    {
        let session = self.get_session_interceptor().await?;
        Ok(SparkTokenServiceClient::with_interceptor(
            self.transport.clone(),
            session,
        ))
    }

    async fn get_session_interceptor(&self) -> Result<OperationSessionInterceptor> {
        let current_session = self
            .session_manager
            .get_session(&self.identity_public_key)
            .await;
        let valid_session = match current_session {
            Ok(session) => self.auth.get_authenticated_session(Some(session)).await,
            Err(e) => {
                error!("Failed to get session from session manager: {}", e);
                self.auth.get_authenticated_session(None).await
            }
        }?;
        self.session_manager
            .set_session(&self.identity_public_key, valid_session.clone())
            .await?;
        valid_session.try_into()
    }
}

impl TryFrom<OperatorSession> for OperationSessionInterceptor {
    type Error = OperatorRpcError;

    fn try_from(session: OperatorSession) -> std::result::Result<Self, Self::Error> {
        Ok(OperationSessionInterceptor {
            token: session.token.parse().map_err(|_| {
                OperatorRpcError::Authentication("Invalid session token".to_string())
            })?,
        })
    }
}

#[derive(Clone, Debug)]
struct OperationSessionInterceptor {
    token: MetadataValue<Ascii>,
}

impl Interceptor for OperationSessionInterceptor {
    fn call(&mut self, mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
        req.metadata_mut()
            .insert("authorization", self.token.clone());
        Ok(req)
    }
}
