use crate::operator_rpc::auth::OperatorAuth;
use crate::operator_rpc::error::Result;
use crate::signer::Signer;
use spark_protos::spark::{
    GenerateDepositAddressRequest, GenerateDepositAddressResponse,
    QueryUnusedDepositAddressesRequest, QueryUnusedDepositAddressesResponse,
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

    pub async fn generate_deposit_address(
        &self,
        req: GenerateDepositAddressRequest,
    ) -> Result<GenerateDepositAddressResponse> {
        Ok(self
            .auth
            .spark_service_client()?
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
            .spark_service_client()?
            .query_unused_deposit_addresses(req)
            .await?
            .into_inner())
    }
}
