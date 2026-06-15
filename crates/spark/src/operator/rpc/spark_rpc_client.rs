use std::sync::Arc;

use super::error::Result;
use super::metadata::set_idempotency_key;
use super::spark::*;
use super::spark_token;
use crate::header_provider::HeaderProvider;
use crate::operator::rpc::OperatorRpcError;
use crate::operator::rpc::spark::query_nodes_request::Source;
use crate::operator::rpc::spark::spark_service_client::SparkServiceClient;
use crate::operator::rpc::spark_token::BroadcastTransactionRequest;
use crate::operator::rpc::spark_token::BroadcastTransactionResponse;
use crate::operator::rpc::spark_token::CommitTransactionRequest;
use crate::operator::rpc::spark_token::CommitTransactionResponse;
use crate::operator::rpc::spark_token::StartTransactionRequest;
use crate::operator::rpc::spark_token::StartTransactionResponse;
use crate::operator::rpc::spark_token::spark_token_service_client::SparkTokenServiceClient;
use crate::operator::rpc::transport::grpc_client::Transport;
use crate::utils::paging::{PagingFilter, PagingResult};
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tracing::{debug, instrument};

#[derive(Clone, Default)]
pub struct QueryNodesPaginatedRequest {
    pub include_parents: bool,
    pub network: i32,
    pub statuses: Vec<i32>,
    pub source: Option<Source>,
}

#[derive(Clone, Default)]
pub struct QueryAllTokenOutputsRequest {
    pub owner_public_keys: Vec<Vec<u8>>,
    pub issuer_public_keys: Vec<Vec<u8>>,
    pub token_identifiers: Vec<Vec<u8>>,
    pub network: i32,
}

#[derive(Clone)]
pub struct SparkRpcClient {
    transport: Transport,
    header_provider: Arc<dyn HeaderProvider>,
    /// Operator index in the pool (0..N). Surfaced as a span field by
    /// the per-method `#[instrument]` attributes on the
    /// `spark::operator_rpc` target, so a downstream subscriber can
    /// attribute a slow RPC to a specific operator.
    operator_id: usize,
}

impl SparkRpcClient {
    pub fn new(
        channel: Transport,
        header_provider: Arc<dyn HeaderProvider>,
        operator_id: usize,
    ) -> Self {
        Self {
            transport: channel,
            header_provider,
            operator_id,
        }
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn finalize_deposit_tree_creation(
        &self,
        req: FinalizeDepositTreeCreationRequest,
    ) -> Result<FinalizeDepositTreeCreationResponse> {
        debug!(
            "Calling finalize_deposit_tree_creation with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .finalize_deposit_tree_creation(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn claim_transfer(&self, req: ClaimTransferRequest) -> Result<ClaimTransferResponse> {
        debug!("Calling claim_transfer with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .claim_transfer(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_all_transfers(&self, req: TransferFilter) -> Result<QueryTransfersResponse> {
        debug!("Calling query_all_transfers with filter: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_all_transfers(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn store_preimage_share_v2(&self, req: StorePreimageShareV2Request) -> Result<()> {
        debug!(
            "Calling store_preimage_share_v2 for payment_hash {}",
            hex::encode(&req.payment_hash)
        );
        self.spark_service_client()
            .await?
            .store_preimage_share_v2(req)
            .await?
            .into_inner();
        Ok(())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn initiate_preimage_swap_v3(
        &self,
        req: InitiatePreimageSwapRequest,
    ) -> Result<InitiatePreimageSwapResponse> {
        debug!(
            "Calling initiate_preimage_swap_v3 for payment_hash {}",
            hex::encode(&req.payment_hash)
        );
        Ok(self
            .spark_service_client()
            .await?
            .initiate_preimage_swap_v3(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn provide_preimage(
        &self,
        req: ProvidePreimageRequest,
    ) -> Result<ProvidePreimageResponse> {
        debug!(
            "Calling provide_preimage for payment_hash {}",
            hex::encode(&req.payment_hash)
        );
        Ok(self
            .spark_service_client()
            .await?
            .provide_preimage(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn initiate_swap_primary_transfer(
        &self,
        req: InitiateSwapPrimaryTransferRequest,
    ) -> Result<InitiateSwapPrimaryTransferResponse> {
        debug!(
            "Calling initiate_swap_primary_transfer with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .initiate_swap_primary_transfer(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn renew_leaf(
        &self,
        req: RenewLeafRequest,
        idempotency_key: Option<String>,
    ) -> Result<RenewLeafResponse> {
        debug!("Calling renew_leaf with request: {:?}", req);
        let mut request = Request::new(req);
        set_idempotency_key(request.metadata_mut(), idempotency_key)?;
        Ok(self
            .spark_service_client()
            .await?
            .renew_leaf(request)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_nodes(&self, req: QueryNodesRequest) -> Result<QueryNodesResponse> {
        debug!("Calling query_nodes with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_nodes(req)
            .await?
            .into_inner())
    }

    /// Paginated version of query_nodes
    ///
    /// If `req.paging` is `Some`, returns a single page according to the filter.
    /// If `req.paging` is `None`, fetches all pages automatically.
    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_nodes_paginated(
        &self,
        req: QueryNodesPaginatedRequest,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<(String, TreeNode)>> {
        match paging {
            Some(paging) => self.query_nodes_paginated_inner(&req, paging.clone()).await,
            None => {
                crate::utils::paging::pager(
                    |pf| self.query_nodes_paginated_inner(&req, pf),
                    PagingFilter::default(),
                )
                .await
            }
        }
    }

    async fn query_nodes_paginated_inner(
        &self,
        req: &QueryNodesPaginatedRequest,
        pf: PagingFilter,
    ) -> Result<PagingResult<(String, TreeNode)>> {
        let response = self
            .query_nodes(QueryNodesRequest {
                source: req.source.clone(),
                include_parents: req.include_parents,
                limit: pf.limit as i64,
                offset: pf.offset as i64,
                network: req.network,
                statuses: vec![],
            })
            .await?;

        // Convert HashMap response to Vec for PagingResult
        let items: Vec<_> = response.nodes.into_iter().collect();
        let has_next = items.len() == pf.limit as usize;

        Ok(PagingResult {
            items,
            next: if has_next { Some(pf.next()) } else { None },
        })
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    /// Query all token outputs by automatically fetching all pages.
    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_all_token_outputs(
        &self,
        req: QueryAllTokenOutputsRequest,
    ) -> Result<Vec<spark_token::OutputWithPreviousTransactionData>> {
        let mut all_items = Vec::new();
        let mut current_cursor: Option<String> = None;

        loop {
            let response = self
                .query_token_outputs(spark_token::QueryTokenOutputsRequest {
                    owner_public_keys: req.owner_public_keys.clone(),
                    issuer_public_keys: req.issuer_public_keys.clone(),
                    token_identifiers: req.token_identifiers.clone(),
                    network: req.network,
                    page_request: Some(PageRequest {
                        cursor: current_cursor.unwrap_or_default(),
                        ..Default::default()
                    }),
                })
                .await?;

            let items = response.outputs_with_previous_transaction_data;

            if items.is_empty() {
                break;
            }

            all_items.extend(items);

            // Check if there's a next page
            match response.page_response {
                Some(page_resp) if page_resp.has_next_page => {
                    current_cursor = Some(page_resp.next_cursor);
                }
                _ => break,
            }
        }

        Ok(all_items)
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_htlc(&self, req: QueryHtlcRequest) -> Result<QueryHtlcResponse> {
        debug!("Calling query_htlc with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .query_htlc(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn broadcast_transaction(
        &self,
        req: BroadcastTransactionRequest,
    ) -> Result<BroadcastTransactionResponse> {
        debug!("Calling broadcast_transaction with request: {:?}", req);
        Ok(self
            .spark_token_service_client()
            .await?
            .broadcast_transaction(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn generate_static_deposit_address(
        &self,
        req: GenerateStaticDepositAddressRequest,
    ) -> Result<GenerateStaticDepositAddressResponse> {
        debug!(
            "Calling generate_static_deposit_address with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .generate_static_deposit_address(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn rotate_static_deposit_address(
        &self,
        req: RotateStaticDepositAddressRequest,
    ) -> Result<RotateStaticDepositAddressResponse> {
        debug!(
            "Calling rotate_static_deposit_address with request: {:?}",
            req
        );
        Ok(self
            .spark_service_client()
            .await?
            .rotate_static_deposit_address(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn get_utxos_for_identity(
        &self,
        req: GetUtxosForIdentityRequest,
    ) -> Result<GetUtxosForIdentityResponse> {
        debug!("Calling get_utxos_for_identity with request: {:?}", req);
        Ok(self
            .spark_service_client()
            .await?
            .get_utxos_for_identity(req)
            .await?
            .into_inner())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
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
    ) -> Result<SparkServiceClient<InterceptedService<Transport, HeaderInterceptor>>> {
        let interceptor = self.build_interceptor().await?;
        Ok(SparkServiceClient::with_interceptor(
            self.transport.clone(),
            interceptor,
        ))
    }

    async fn spark_token_service_client(
        &self,
    ) -> Result<SparkTokenServiceClient<InterceptedService<Transport, HeaderInterceptor>>> {
        let interceptor = self.build_interceptor().await?;
        Ok(SparkTokenServiceClient::with_interceptor(
            self.transport.clone(),
            interceptor,
        ))
    }

    async fn build_interceptor(&self) -> Result<HeaderInterceptor> {
        let raw_headers = self
            .header_provider
            .headers()
            .await
            .map_err(|e| OperatorRpcError::Authentication(e.to_string()))?;
        let mut headers = Vec::with_capacity(raw_headers.len());
        for (key, value) in raw_headers {
            let metadata_key = key
                .parse::<tonic::metadata::MetadataKey<Ascii>>()
                .map_err(|_| OperatorRpcError::Generic(format!("Invalid metadata key: {key}")))?;
            let metadata_value = value.parse::<MetadataValue<Ascii>>().map_err(|_| {
                OperatorRpcError::Generic(format!("Invalid metadata value for key: {key}"))
            })?;
            headers.push((metadata_key, metadata_value));
        }
        Ok(HeaderInterceptor { headers })
    }
}

/// Tonic [`Interceptor`] adapter that stamps a snapshot of HTTP-style headers
/// onto outgoing gRPC requests as metadata. The actual header values come from
/// a [`HeaderProvider`] resolved at the moment the gRPC client is built.
#[derive(Clone, Debug)]
pub struct HeaderInterceptor {
    headers: Vec<(tonic::metadata::MetadataKey<Ascii>, MetadataValue<Ascii>)>,
}

impl Interceptor for HeaderInterceptor {
    fn call(&mut self, mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
        for (key, value) in &self.headers {
            req.metadata_mut().insert(key, value.clone());
        }
        Ok(req)
    }
}
