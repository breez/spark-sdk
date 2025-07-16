use bitcoin::secp256k1::PublicKey;

use crate::{
    signer::Signer,
    ssp::{
        BitcoinNetwork, ClaimStaticDeposit, ClaimStaticDepositInput, CoopExitFeeEstimates,
        CurrencyAmount, LeavesSwapRequest, RequestCoopExitInput, RequestLeavesSwapInput,
        RequestLightningReceiveInput, RequestLightningSendInput, ServiceProviderConfig,
        StaticDepositQuote, Transfer,
        error::ServiceProviderResult,
        graphql::{CoopExitRequest, GraphQLClient, LightningReceiveRequest, LightningSendRequest},
    },
};

pub struct ServiceProvider<S> {
    identity_public_key: PublicKey,
    gql_client: GraphQLClient<S>,
}

impl<S> ServiceProvider<S>
where
    S: Signer,
{
    /// Create a new GraphQLClient with the given configuration and signer
    pub fn new(config: ServiceProviderConfig, signer: S) -> Self {
        Self {
            identity_public_key: config.identity_public_key,
            gql_client: GraphQLClient::new(config.into(), signer),
        }
    }

    pub fn identity_public_key(&self) -> PublicKey {
        self.identity_public_key
    }

    /// Get a swap fee estimate
    pub async fn get_swap_fee_estimate(
        &self,
        amount_sats: u64,
    ) -> ServiceProviderResult<CurrencyAmount> {
        Ok(self.gql_client.get_swap_fee_estimate(amount_sats).await?)
    }

    /// Get a lightning send fee estimate
    pub async fn get_lightning_send_fee_estimate(
        &self,
        encoded_invoice: &str,
        amount_sats: Option<u64>,
    ) -> ServiceProviderResult<CurrencyAmount> {
        Ok(self
            .gql_client
            .get_lightning_send_fee_estimate(encoded_invoice, amount_sats)
            .await?)
    }

    /// Get a coop exit fee estimate
    pub async fn get_coop_exit_fee_estimates(
        &self,
        leaf_external_ids: Vec<String>,
        withdrawal_address: &str,
    ) -> ServiceProviderResult<CoopExitFeeEstimates> {
        Ok(self
            .gql_client
            .get_coop_exit_fee_estimates(leaf_external_ids, withdrawal_address)
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

    /// Request leaves swap
    pub async fn request_leaves_swap(
        &self,
        input: RequestLeavesSwapInput,
    ) -> ServiceProviderResult<LeavesSwapRequest> {
        Ok(self.gql_client.request_leaves_swap(input).await?)
    }

    /// Complete a leaves swap
    pub async fn complete_leaves_swap(
        &self,
        adaptor_secret_key: &str,
        user_outbound_transfer_external_id: &str,
        leaves_swap_request_id: &str,
    ) -> ServiceProviderResult<LeavesSwapRequest> {
        Ok(self
            .gql_client
            .complete_leaves_swap(
                adaptor_secret_key,
                user_outbound_transfer_external_id,
                leaves_swap_request_id,
            )
            .await?)
    }

    /// Get claim deposit quote
    pub async fn get_claim_deposit_quote(
        &self,
        transaction_id: String,
        output_index: u32,
        network: BitcoinNetwork,
    ) -> ServiceProviderResult<StaticDepositQuote> {
        Ok(self
            .gql_client
            .get_claim_deposit_quote(transaction_id, output_index, network)
            .await?)
    }

    /// Get a lightning receive request by ID
    pub async fn get_lightning_receive_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<LightningReceiveRequest>> {
        Ok(self
            .gql_client
            .get_lightning_receive_request(request_id)
            .await?)
    }

    /// Get a lightning send request by ID
    pub async fn get_lightning_send_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<LightningSendRequest>> {
        Ok(self
            .gql_client
            .get_lightning_send_request(request_id)
            .await?)
    }

    /// Get a leaves swap request by ID
    pub async fn get_leaves_swap_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<LeavesSwapRequest>> {
        Ok(self.gql_client.get_leaves_swap_request(request_id).await?)
    }

    /// Get a cooperative exit request by ID
    pub async fn get_coop_exit_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<CoopExitRequest>> {
        Ok(self.gql_client.get_coop_exit_request(request_id).await?)
    }

    /// Claim static deposit
    pub async fn claim_static_deposit(
        &self,
        input: ClaimStaticDepositInput,
    ) -> ServiceProviderResult<ClaimStaticDeposit> {
        Ok(self.gql_client.claim_static_deposit(input).await?)
    }

    /// Get transfers by IDs
    pub async fn get_transfers(
        &self,
        transfer_spark_ids: Vec<&str>,
    ) -> ServiceProviderResult<Vec<Transfer>> {
        Ok(self.gql_client.get_transfers(transfer_spark_ids).await?)
    }

    /// Request regtest funds
    pub async fn request_regtest_funds(
        &self,
        amount_sats: u64,
        address: &str,
        faucet_username: &str,
        faucet_password: &str,
    ) -> ServiceProviderResult<String> {
        Ok(self
            .gql_client
            .request_regtest_funds(amount_sats, address, faucet_username, faucet_password)
            .await?)
    }
}
