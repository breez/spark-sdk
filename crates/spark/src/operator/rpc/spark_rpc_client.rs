use std::collections::HashMap;
use std::sync::Arc;

use super::auth::OperatorAuth;
use super::error::Result;
use super::spark::*;
use super::spark_token;
use crate::operator::rpc::OperatorRpcError;
use crate::operator::rpc::spark::query_nodes_request::Source;
use crate::operator::rpc::spark::spark_service_client::SparkServiceClient;
use crate::operator::rpc::spark_token::CommitTransactionRequest;
use crate::operator::rpc::spark_token::CommitTransactionResponse;
use crate::operator::rpc::spark_token::StartTransactionRequest;
use crate::operator::rpc::spark_token::StartTransactionResponse;
use crate::operator::rpc::spark_token::spark_token_service_client::SparkTokenServiceClient;
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::session_manager::Session;
use crate::session_manager::SessionManager;
use crate::signer::Signer;
use bitcoin::secp256k1::PublicKey;
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tracing::{debug, error};

pub struct QueryAllNodesRequest {
    pub include_parents: bool,
    pub network: Network,
    pub source: Option<Source>,
}

#[derive(Clone)]
pub struct SparkRpcClient {
    transport: Transport,
    auth: OperatorAuth,
    session_manager: Arc<dyn SessionManager>,
    identity_public_key: PublicKey,
}

impl SparkRpcClient {
    pub fn new(
        channel: Transport,
        signer: Arc<dyn Signer>,
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
        debug!(
            "Calling finalize_node_signatures_v2 with request: {:?}",
            req
        );
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
        debug!("Calling generate_deposit_address with request: {:?}", req);
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
        debug!(
            "Calling query_unused_deposit_addresses with request: {:?}",
            req
        );
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
        debug!(
            "Calling start_deposit_tree_creation with request: {:?}",
            req
        );
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
        debug!("Calling start_transfer with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .start_transfer_v2(req)
            .await?
            .into_inner())
    }

    pub async fn finalize_transfer_with_transfer_package(
        &self,
        req: FinalizeTransferWithTransferPackageRequest,
    ) -> Result<FinalizeTransferResponse> {
        debug!(
            "Calling finalize_transfer_with_transfer_package with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .finalize_transfer_with_transfer_package(req)
            .await?
            .into_inner())
    }

    pub async fn query_pending_transfers(
        &self,
        req: TransferFilter,
    ) -> Result<QueryTransfersResponse> {
        debug!("Querying pending transfers with filter: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_pending_transfers(req)
            .await?
            .into_inner())
    }

    pub async fn query_all_transfers(&self, req: TransferFilter) -> Result<QueryTransfersResponse> {
        debug!("Calling query_all_transfers with filter: {:?}", req);
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
        debug!("Calling claim_transfer_tweak_keys with request: {:?}", req);
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
        debug!(
            "Calling claim_transfer_sign_refunds_v2 with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .claim_transfer_sign_refunds_v2(req)
            .await?
            .into_inner())
    }

    pub async fn store_preimage_share(&self, req: StorePreimageShareRequest) -> Result<()> {
        debug!("Calling store_preimage_share with request: {:?}", req);
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
        debug!("Calling get_signing_commitments with request: {:?}", req);
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
        debug!("Calling cooperative_exit_v2 with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .cooperative_exit_v2(req)
            .await?
            .into_inner())
    }

    pub async fn initiate_preimage_swap_v3(
        &self,
        req: InitiatePreimageSwapRequest,
    ) -> Result<InitiatePreimageSwapResponse> {
        debug!("Calling initiate_preimage_swap_v3 with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .initiate_preimage_swap_v3(req)
            .await?
            .into_inner())
    }

    pub async fn provide_preimage(
        &self,
        req: ProvidePreimageRequest,
    ) -> Result<ProvidePreimageResponse> {
        debug!("Calling provide_preimage with request: {:?}", req);
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
        debug!("Calling start_leaf_swap_v2 with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .start_leaf_swap_v2(req)
            .await?
            .into_inner())
    }

    pub async fn renew_leaf(&self, req: RenewLeafRequest) -> Result<RenewLeafResponse> {
        debug!("Calling renew_leaf with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .renew_leaf(req)
            .await?
            .into_inner())
    }

    pub async fn get_signing_operator_list(&self) -> Result<GetSigningOperatorListResponse> {
        debug!("Calling get_signing_operator_list");
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
                statuses: vec![],
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
        debug!("Calling query_nodes with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_nodes(req)
            .await?
            .into_inner())
    }

    pub async fn query_balance(&self, req: QueryBalanceRequest) -> Result<QueryBalanceResponse> {
        debug!("Calling query_balance with request: {:?}", req);
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
        debug!("Calling query_user_signed_refunds with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_user_signed_refunds(req)
            .await?
            .into_inner())
    }

    pub async fn freeze_tokens(
        &self,
        req: spark_token::FreezeTokensRequest,
    ) -> Result<spark_token::FreezeTokensResponse> {
        debug!("Calling freeze_tokens with request: {:?}", req);
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
        debug!("Calling query_token_outputs with request: {:?}", req);
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
        debug!("Calling query_token_metadata with request: {:?}", req);
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
        debug!("Calling query_token_transactions with request: {:?}", req);
        Ok(self
            .spark_token_service_client()
            .await?
            .query_token_transactions(req)
            .await?
            .into_inner())
    }

    pub async fn query_spark_invoices(
        &self,
        req: QuerySparkInvoicesRequest,
    ) -> Result<QuerySparkInvoicesResponse> {
        debug!("Calling query_spark_invoices with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_spark_invoices(req)
            .await?
            .into_inner())
    }

    pub async fn start_transaction(
        &self,
        req: StartTransactionRequest,
    ) -> Result<StartTransactionResponse> {
        debug!("Calling start_transaction with request: {:?}", req);
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
        debug!("Calling commit_transaction with request: {:?}", req);
        Ok(self
            .spark_token_service_client()
            .await?
            .commit_transaction(req)
            .await?
            .into_inner())
    }

    pub async fn query_static_deposit_addresses(
        &self,
        req: QueryStaticDepositAddressesRequest,
    ) -> Result<QueryStaticDepositAddressesResponse> {
        debug!(
            "Calling query_static_deposit_addresses with request: {:?}",
            req
        );
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
        debug!("Calling get_utxos_for_address with request: {:?}", req);
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
        debug!(
            "Calling initiate_static_deposit_utxo_refund with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .initiate_static_deposit_utxo_refund(req)
            .await?
            .into_inner())
    }

    pub async fn subscribe_to_events(
        &self,
        req: SubscribeToEventsRequest,
    ) -> Result<tonic::codec::Streaming<SubscribeToEventsResponse>> {
        debug!("Calling subscribe_to_events with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .subscribe_to_events(req)
            .await?
            .into_inner())
    }

    pub async fn update_wallet_setting(
        &self,
        req: UpdateWalletSettingRequest,
    ) -> Result<UpdateWalletSettingResponse> {
        debug!("Calling update_wallet_setting with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .update_wallet_setting(req)
            .await?
            .into_inner())
    }

    pub async fn query_wallet_setting(&self) -> Result<QueryWalletSettingResponse> {
        debug!("Calling query_wallet_setting");
        Ok(self
            .spark_service_client()
            .await?
            .query_wallet_setting(QueryWalletSettingRequest {})
            .await?
            .into_inner())
    }

    async fn spark_service_client(
        &self,
    ) -> Result<SparkServiceClient<InterceptedService<Transport, OperatorSessionInterceptor>>> {
        let session = self.get_session_interceptor().await?;
        Ok(SparkServiceClient::with_interceptor(
            self.transport.clone(),
            session,
        ))
    }

    async fn spark_token_service_client(
        &self,
    ) -> Result<SparkTokenServiceClient<InterceptedService<Transport, OperatorSessionInterceptor>>>
    {
        let session = self.get_session_interceptor().await?;
        Ok(SparkTokenServiceClient::with_interceptor(
            self.transport.clone(),
            session,
        ))
    }

    async fn get_session_interceptor(&self) -> Result<OperatorSessionInterceptor> {
        let current_session = self
            .session_manager
            .get_session(&self.identity_public_key)
            .await;
        let valid_session = match current_session {
            Ok(session) => self.auth.get_authenticated_session(Some(session)).await,
            Err(e) => {
                match e {
                    crate::session_manager::SessionManagerError::NotFound => {
                        debug!("Operator session not found, authenticating")
                    }
                    crate::session_manager::SessionManagerError::Generic(e) => {
                        error!("Failed to get operator session from session manager: {}", e)
                    }
                };
                self.auth.get_authenticated_session(None).await
            }
        }?;
        self.session_manager
            .set_session(&self.identity_public_key, valid_session.clone())
            .await?;
        valid_session.try_into()
    }
}

impl TryFrom<Session> for OperatorSessionInterceptor {
    type Error = OperatorRpcError;

    fn try_from(session: Session) -> std::result::Result<Self, Self::Error> {
        Ok(OperatorSessionInterceptor {
            token: session.token.parse().map_err(|_| {
                OperatorRpcError::Authentication("Invalid session token".to_string())
            })?,
        })
    }
}

#[derive(Clone, Debug)]
struct OperatorSessionInterceptor {
    token: MetadataValue<Ascii>,
}

impl Interceptor for OperatorSessionInterceptor {
    fn call(&mut self, mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
        req.metadata_mut()
            .insert("authorization", self.token.clone());
        Ok(req)
    }
}
