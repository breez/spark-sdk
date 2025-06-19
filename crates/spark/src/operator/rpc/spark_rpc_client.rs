use super::auth::OperatorAuth;
use super::error::Result;
use crate::signer::Signer;
use spark_protos::spark::{
    GenerateDepositAddressRequest, GenerateDepositAddressResponse,
    QueryUnusedDepositAddressesRequest, QueryUnusedDepositAddressesResponse,
    StartDepositTreeCreationRequest, StartDepositTreeCreationResponse,
};
use tonic::transport::Channel;

pub struct SparkRpcClient<S>
where
    S: Signer,
{
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
        req: spark_protos::spark::FinalizeNodeSignaturesRequest,
    ) -> Result<spark_protos::spark::FinalizeNodeSignaturesResponse> {
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
}
