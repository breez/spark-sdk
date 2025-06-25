use crate::{
    Network,
    signer::Signer,
    ssp::{
        ServiceProviderOptions,
        error::ServiceProviderResult,
        graphql::{
            CoopExitFeeEstimatesOutput, CoopExitRequest, GraphQLClient,
            LeavesSwapFeeEstimateOutput, LightningReceiveRequest, LightningSendFeeEstimateOutput,
            LightningSendRequest, RequestCoopExitInput, RequestLightningReceiveInput,
            RequestLightningSendInput, StaticDepositQuoteInput, StaticDepositQuoteOutput,
        },
    },
};

pub struct ServiceProvider<S>
where
    S: Signer,
{
    gql_client: GraphQLClient<S>,
}

impl<S> ServiceProvider<S>
where
    S: Signer,
{
    /// Create a new GraphQLClient with the given options
    pub fn new(options: ServiceProviderOptions, network: Network, signer: S) -> Self {
        Self {
            gql_client: GraphQLClient::new(options.into(), network, signer),
        }
    }

    /// Get a swap fee estimate
    pub async fn get_swap_fee_estimate(
        &self,
        amount_sats: u64,
    ) -> ServiceProviderResult<LeavesSwapFeeEstimateOutput> {
        Ok(self.gql_client.get_swap_fee_estimate(amount_sats).await?)
    }

    /// Get a lightning send fee estimate
    pub async fn get_lightning_send_fee_estimate(
        &self,
        encoded_invoice: &str,
    ) -> ServiceProviderResult<LightningSendFeeEstimateOutput> {
        Ok(self
            .gql_client
            .get_lightning_send_fee_estimate(encoded_invoice)
            .await?)
    }

    /// Get a coop exit fee estimate
    pub async fn get_coop_exit_fee_estimate(
        &self,
        leaf_external_ids: Vec<String>,
        withdrawal_address: &str,
    ) -> ServiceProviderResult<CoopExitFeeEstimatesOutput> {
        Ok(self
            .gql_client
            .get_coop_exit_fee_estimate(leaf_external_ids, withdrawal_address)
            .await?)
    }

    /// Complete a cooperative exit
    pub async fn complete_coop_exit(
        &self,
        user_outbound_transfer_external_id: &str,
        coop_exit_request_id: &str,
    ) -> ServiceProviderResult<CoopExitRequest> {
        Ok(self
            .gql_client
            .complete_coop_exit(user_outbound_transfer_external_id, coop_exit_request_id)
            .await?)
    }

    /// Request a cooperative exit
    pub async fn request_coop_exit(
        &self,
        input: RequestCoopExitInput,
    ) -> ServiceProviderResult<CoopExitRequest> {
        Ok(self.gql_client.request_coop_exit(input).await?)
    }

    /// Request lightning receive
    pub async fn request_lightning_receive(
        &self,
        input: RequestLightningReceiveInput,
    ) -> ServiceProviderResult<LightningReceiveRequest> {
        Ok(self.gql_client.request_lightning_receive(input).await?)
    }

    /// Request lightning send
    pub async fn request_lightning_send(
        &self,
        input: RequestLightningSendInput,
    ) -> ServiceProviderResult<LightningSendRequest> {
        Ok(self.gql_client.request_lightning_send(input).await?)
    }

    /// Get claim deposit quote
    pub async fn get_claim_deposit_quote(
        &self,
        input: StaticDepositQuoteInput,
    ) -> ServiceProviderResult<StaticDepositQuoteOutput> {
        Ok(self.gql_client.get_claim_deposit_quote(input).await?)
    }

    /// Get a lightning receive request by ID
    pub async fn get_lightning_receive_request(
        &self,
        id: &str,
    ) -> ServiceProviderResult<Option<LightningReceiveRequest>> {
        Ok(self.gql_client.get_lightning_receive_request(id).await?)
    }

    /// Get a lightning send request by ID
    pub async fn get_lightning_send_request(
        &self,
        id: &str,
    ) -> ServiceProviderResult<Option<LightningSendRequest>> {
        Ok(self.gql_client.get_lightning_send_request(id).await?)
    }

    /// Get a cooperative exit request by ID
    pub async fn get_coop_exit_request(
        &self,
        id: &str,
    ) -> ServiceProviderResult<Option<CoopExitRequest>> {
        Ok(self.gql_client.get_coop_exit_request(id).await?)
    }
}
