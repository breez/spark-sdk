use spark::operator::rpc::spark::{
    AdaptorPublicKeyPackage, CreateTreeRequest, CreateTreeResponse, FinalizeNodeSignaturesRequest,
    FinalizeNodeSignaturesResponse, GetUtxosForAddressRequest, GetUtxosForAddressResponse,
    PrepareTreeAddressRequest, PrepareTreeAddressResponse, QueryNodesRequest, QueryNodesResponse,
};
use spark::operator::rpc::{HeaderInterceptor, SparkRpcClient};
use tonic::service::interceptor::InterceptedService;

pub mod proto {
    #![allow(clippy::all, clippy::pedantic, dead_code)]
    pub mod spark_ssp_internal {
        tonic::include_proto!("spark_ssp_internal");
    }
    pub mod spark_internal {
        tonic::include_proto!("spark_internal");
    }
}

use proto::spark_internal::spark_internal_service_client::SparkInternalServiceClient;
use proto::spark_ssp_internal::spark_ssp_internal_service_client::SparkSspInternalServiceClient;
pub use proto::spark_ssp_internal::{
    ClaimInstantStaticDepositUtxoSwapRequest, ClaimInstantStaticDepositUtxoSwapResponse,
    CounterTransferRequest, InitiateStaticDepositUtxoSwapRequest,
    InitiateStaticDepositUtxoSwapResponse, ReserveInstantStaticDepositUtxoSwapRequest,
    ReserveInstantStaticDepositUtxoSwapResponse, ReturnStuckTransferRequest,
    ReturnStuckTransferResponse, SignStaticDepositSweepTxRequest, SignStaticDepositSweepTxResponse,
    SweepInput, sign_static_deposit_sweep_tx_response,
};

use spark::operator::rpc::OperatorRpcError;

type Result<T> = std::result::Result<T, OperatorRpcError>;

async fn with_auth_retry<T, F, Fut>(client: &SparkRpcClient, call: F) -> Result<T>
where
    F: Fn(HeaderInterceptor) -> Fut,
    Fut: std::future::Future<Output = std::result::Result<tonic::Response<T>, tonic::Status>>,
{
    let mut refreshed = false;
    loop {
        let interceptor = client
            .build_interceptor(refreshed)
            .await
            .map_err(|e| OperatorRpcError::Authentication(e.to_string()))?;
        match call(interceptor).await {
            Ok(response) => return Ok(response.into_inner()),
            Err(status) if !refreshed && status.code() == tonic::Code::Unauthenticated => {
                refreshed = true;
            }
            Err(status) => return Err(OperatorRpcError::Connection(Box::new(status))),
        }
    }
}

fn ssp_client(
    client: &SparkRpcClient,
    interceptor: HeaderInterceptor,
) -> SparkSspInternalServiceClient<
    InterceptedService<spark::operator::rpc::Transport, HeaderInterceptor>,
> {
    SparkSspInternalServiceClient::with_interceptor(client.transport().clone(), interceptor)
}

fn internal_client(
    client: &SparkRpcClient,
    interceptor: HeaderInterceptor,
) -> SparkInternalServiceClient<
    InterceptedService<spark::operator::rpc::Transport, HeaderInterceptor>,
> {
    SparkInternalServiceClient::with_interceptor(client.transport().clone(), interceptor)
}

pub async fn prepare_tree_address(
    client: &SparkRpcClient,
    req: PrepareTreeAddressRequest,
) -> Result<PrepareTreeAddressResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.prepare_tree_address(req).await }
    })
    .await
}

pub async fn create_tree(
    client: &SparkRpcClient,
    req: CreateTreeRequest,
) -> Result<CreateTreeResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.create_tree(req).await }
    })
    .await
}

pub async fn initiate_counter_transfer(
    client: &SparkRpcClient,
    req: CounterTransferRequest,
) -> Result<spark::operator::rpc::spark::StartTransferResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.initiate_counter_transfer(req).await }
    })
    .await
}

pub async fn return_stuck_transfer(
    client: &SparkRpcClient,
    req: ReturnStuckTransferRequest,
) -> Result<ReturnStuckTransferResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.return_stuck_transfer(req).await }
    })
    .await
}

pub async fn initiate_static_deposit_utxo_swap(
    client: &SparkRpcClient,
    req: InitiateStaticDepositUtxoSwapRequest,
) -> Result<InitiateStaticDepositUtxoSwapResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.initiate_static_deposit_utxo_swap(req).await }
    })
    .await
}

pub async fn reserve_instant_static_deposit_utxo_swap(
    client: &SparkRpcClient,
    req: ReserveInstantStaticDepositUtxoSwapRequest,
) -> Result<ReserveInstantStaticDepositUtxoSwapResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.reserve_instant_static_deposit_utxo_swap(req).await }
    })
    .await
}

pub async fn claim_instant_static_deposit_utxo_swap(
    client: &SparkRpcClient,
    req: ClaimInstantStaticDepositUtxoSwapRequest,
) -> Result<ClaimInstantStaticDepositUtxoSwapResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.claim_instant_static_deposit_utxo_swap(req).await }
    })
    .await
}

pub async fn sign_static_deposit_sweep_tx(
    client: &SparkRpcClient,
    req: SignStaticDepositSweepTxRequest,
) -> Result<SignStaticDepositSweepTxResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = ssp_client(client, interceptor);
        let req = req.clone();
        async move { c.sign_static_deposit_sweep_tx(req).await }
    })
    .await
}

/// Unlike the public query, returns nodes regardless of their wallet's privacy
/// setting.
pub async fn query_nodes_internal(
    client: &SparkRpcClient,
    req: QueryNodesRequest,
) -> Result<QueryNodesResponse> {
    with_auth_retry(client, |interceptor| {
        let mut c = internal_client(client, interceptor);
        let req = req.clone();
        async move { c.query_nodes(req).await }
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn counter_transfer(
    transfer_service: &spark::services::TransferService,
    client: &SparkRpcClient,
    transfer_id: &spark::services::TransferId,
    leaf_key_tweaks: &[spark::services::LeafKeyTweak],
    receiver_public_key: &bitcoin::secp256k1::PublicKey,
    primary_transfer_id: &spark::services::TransferId,
    cpfp_adaptor_public_key: &bitcoin::secp256k1::PublicKey,
    expiry_time: Option<std::time::SystemTime>,
) -> std::result::Result<spark::services::Transfer, spark::services::ServiceError> {
    let mut prepared = transfer_service
        .prepare_transfer_request(
            transfer_id,
            leaf_key_tweaks,
            receiver_public_key,
            None,
            expiry_time,
            Some(cpfp_adaptor_public_key),
        )
        .await?;

    // The operators reject a Swap V3 transfer that carries direct transactions.
    if let Some(transfer_package) = prepared.transfer_request.transfer_package.as_mut() {
        transfer_package.direct_leaves_to_send.clear();
        transfer_package.direct_from_cpfp_leaves_to_send.clear();
    }

    let transfer = initiate_counter_transfer(
        client,
        CounterTransferRequest {
            transfer: Some(prepared.transfer_request),
            adaptor_public_keys: Some(AdaptorPublicKeyPackage {
                adaptor_public_key: cpfp_adaptor_public_key.serialize().to_vec(),
                ..Default::default()
            }),
            primary_transfer_id: primary_transfer_id.to_string(),
        },
    )
    .await
    .map_err(|e| spark::services::ServiceError::ServiceConnectionError(Box::new(e)))?
    .transfer
    .ok_or_else(|| {
        spark::services::ServiceError::Generic(
            "No transfer from operator for counter swap".to_string(),
        )
    })?;

    transfer.try_into()
}

pub async fn get_utxos_for_address(
    client: &SparkRpcClient,
    req: GetUtxosForAddressRequest,
) -> Result<GetUtxosForAddressResponse> {
    use spark::operator::rpc::spark::spark_service_client::SparkServiceClient;
    with_auth_retry(client, |interceptor| {
        let mut c = SparkServiceClient::with_interceptor(client.transport().clone(), interceptor);
        let req = req.clone();
        async move { c.get_utxos_for_address(req).await }
    })
    .await
}

pub async fn finalize_node_signatures_v2(
    client: &SparkRpcClient,
    req: FinalizeNodeSignaturesRequest,
) -> Result<FinalizeNodeSignaturesResponse> {
    use spark::operator::rpc::spark::spark_service_client::SparkServiceClient;
    with_auth_retry(client, |interceptor| {
        let mut c = SparkServiceClient::with_interceptor(client.transport().clone(), interceptor);
        let req = req.clone();
        async move { c.finalize_node_signatures_v2(req).await }
    })
    .await
}
