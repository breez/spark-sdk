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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.generate_deposit_address(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_unused_deposit_addresses(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.finalize_deposit_tree_creation(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn start_transfer_v2(
        &self,
        req: StartTransferRequest,
    ) -> Result<StartTransferResponse> {
        debug!("Calling start_transfer with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.start_transfer_v2(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn claim_transfer(&self, req: ClaimTransferRequest) -> Result<ClaimTransferResponse> {
        debug!("Calling claim_transfer with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.claim_transfer(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_pending_transfers(
        &self,
        req: TransferFilter,
    ) -> Result<QueryTransfersResponse> {
        debug!("Querying pending transfers with filter: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_pending_transfers(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_all_transfers(&self, req: TransferFilter) -> Result<QueryTransfersResponse> {
        debug!("Calling query_all_transfers with filter: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_all_transfers(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn store_preimage_share_v2(&self, req: StorePreimageShareV2Request) -> Result<()> {
        debug!(
            "Calling store_preimage_share_v2 for payment_hash {}",
            hex::encode(&req.payment_hash)
        );
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.store_preimage_share_v2(req).await?) }
        })
        .await?;
        Ok(())
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn get_signing_commitments(
        &self,
        req: GetSigningCommitmentsRequest,
    ) -> Result<GetSigningCommitmentsResponse> {
        debug!("Calling get_signing_commitments with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.get_signing_commitments(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn cooperative_exit_v2(
        &self,
        req: CooperativeExitRequest,
    ) -> Result<CooperativeExitResponse> {
        debug!("Calling cooperative_exit_v2 with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.cooperative_exit_v2(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.initiate_preimage_swap_v3(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.provide_preimage(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.initiate_swap_primary_transfer(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn renew_leaf(
        &self,
        req: RenewLeafRequest,
        idempotency_key: Option<String>,
    ) -> Result<RenewLeafResponse> {
        debug!("Calling renew_leaf with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            let idempotency_key = idempotency_key.clone();
            async move {
                let mut request = Request::new(req);
                set_idempotency_key(request.metadata_mut(), idempotency_key)?;
                Ok(client.renew_leaf(request).await?)
            }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_nodes(&self, req: QueryNodesRequest) -> Result<QueryNodesResponse> {
        debug!("Calling query_nodes with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_nodes(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.freeze_tokens(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_token_outputs(
        &self,
        req: spark_token::QueryTokenOutputsRequest,
    ) -> Result<spark_token::QueryTokenOutputsResponse> {
        debug!("Calling query_token_outputs with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_token_outputs(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_token_metadata(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_token_transactions(
        &self,
        req: spark_token::QueryTokenTransactionsRequest,
    ) -> Result<spark_token::QueryTokenTransactionsResponse> {
        debug!("Calling query_token_transactions with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_token_transactions(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_spark_invoices(
        &self,
        req: QuerySparkInvoicesRequest,
    ) -> Result<QuerySparkInvoicesResponse> {
        debug!("Calling query_spark_invoices with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_spark_invoices(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_htlc(&self, req: QueryHtlcRequest) -> Result<QueryHtlcResponse> {
        debug!("Calling query_htlc with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_htlc(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn start_transaction(
        &self,
        req: StartTransactionRequest,
    ) -> Result<StartTransactionResponse> {
        debug!("Calling start_transaction with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.start_transaction(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn commit_transaction(
        &self,
        req: CommitTransactionRequest,
    ) -> Result<CommitTransactionResponse> {
        debug!("Calling commit_transaction with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.commit_transaction(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn broadcast_transaction(
        &self,
        req: BroadcastTransactionRequest,
    ) -> Result<BroadcastTransactionResponse> {
        debug!("Calling broadcast_transaction with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_token_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.broadcast_transaction(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.generate_static_deposit_address(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.rotate_static_deposit_address(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.query_static_deposit_addresses(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn get_utxos_for_identity(
        &self,
        req: GetUtxosForIdentityRequest,
    ) -> Result<GetUtxosForIdentityResponse> {
        debug!("Calling get_utxos_for_identity with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.get_utxos_for_identity(req).await?) }
        })
        .await
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
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.initiate_static_deposit_utxo_refund(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn subscribe_to_events(
        &self,
        req: SubscribeToEventsRequest,
    ) -> Result<tonic::codec::Streaming<SubscribeToEventsResponse>> {
        debug!("Calling subscribe_to_events with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.subscribe_to_events(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn update_wallet_setting(
        &self,
        req: UpdateWalletSettingRequest,
    ) -> Result<UpdateWalletSettingResponse> {
        debug!("Calling update_wallet_setting with request: {:?}", req);
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            let req = req.clone();
            async move { Ok(client.update_wallet_setting(req).await?) }
        })
        .await
    }

    #[instrument(level = "info", target = "spark::operator_rpc", skip_all, fields(operator_id = self.operator_id))]
    pub async fn query_wallet_setting(&self) -> Result<QueryWalletSettingResponse> {
        debug!("Calling query_wallet_setting");
        self.call_with_auth_retry(|interceptor| {
            let mut client = self.spark_service_client(interceptor);
            async move {
                Ok(client
                    .query_wallet_setting(QueryWalletSettingRequest {})
                    .await?)
            }
        })
        .await
    }

    /// Invokes a single unary or stream-opening RPC, retrying it once if the
    /// operator rejects it with `Unauthenticated`. The retry force-refreshes the
    /// session (re-minting the token, bypassing any cached one) so a
    /// stale-but-unexpired cached token self-heals instead of failing the call.
    ///
    /// `call` receives a freshly built [`HeaderInterceptor`] and must build its
    /// gRPC client from it, then issue exactly one request. It is invoked at most
    /// twice. For streaming RPCs only the stream open is covered; an auth failure
    /// surfacing while the stream is polled is not retried here.
    ///
    /// Concurrent refreshes for the same operator are not coalesced: several
    /// in-flight calls rejected at once each re-authenticate independently. This
    /// only happens when a token is server-rejected (rare, and self-correcting as
    /// each stores a fresh valid token), so the redundant round-trips do not
    /// justify a single-flight lock on the auth hot path.
    async fn call_with_auth_retry<T, F, Fut>(&self, call: F) -> Result<T>
    where
        F: Fn(HeaderInterceptor) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Response<T>>>,
    {
        let mut refreshed = false;
        loop {
            let interceptor = self.build_interceptor(refreshed).await?;
            match call(interceptor).await {
                Ok(response) => return Ok(response.into_inner()),
                Err(err) => {
                    if !refreshed && is_unauthenticated(&err) {
                        debug!(
                            "Operator returned Unauthenticated, refreshing session and retrying"
                        );
                        refreshed = true;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    fn spark_service_client(
        &self,
        interceptor: HeaderInterceptor,
    ) -> SparkServiceClient<InterceptedService<Transport, HeaderInterceptor>> {
        SparkServiceClient::with_interceptor(self.transport.clone(), interceptor)
    }

    fn spark_token_service_client(
        &self,
        interceptor: HeaderInterceptor,
    ) -> SparkTokenServiceClient<InterceptedService<Transport, HeaderInterceptor>> {
        SparkTokenServiceClient::with_interceptor(self.transport.clone(), interceptor)
    }

    async fn build_interceptor(&self, force_refresh: bool) -> Result<HeaderInterceptor> {
        let raw_headers = if force_refresh {
            self.header_provider.headers_refresh().await
        } else {
            self.header_provider.headers().await
        }
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

/// Whether an operator error is a gRPC `Unauthenticated` status, signalling the
/// session token was rejected and the call should be retried with a fresh one.
fn is_unauthenticated(err: &OperatorRpcError) -> bool {
    matches!(err, OperatorRpcError::Connection(status) if status.code() == tonic::Code::Unauthenticated)
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
