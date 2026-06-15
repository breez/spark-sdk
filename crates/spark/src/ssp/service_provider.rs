use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use platform_utils::{HttpClient, create_http_client};
use tracing::instrument;

use crate::{
    default_user_agent,
    header_provider::{CombinedHeaderProvider, HeaderProvider},
    session_store::SessionStore,
    signer::SparkSigner,
    ssp::{
        BitcoinNetwork, ClaimStaticDeposit, ClaimStaticDepositInput, CoopExitFeeQuote,
        CurrencyAmount, LeavesSwapRequest, RequestCoopExitInput, RequestLightningReceiveInput,
        RequestLightningSendInput, RequestSwapInput, ServiceProviderConfig,
        SparkWalletWebhookEventType, SspAuthHeaderProvider, SspTransfer, StaticDepositQuote,
        WebhookEntry,
        error::ServiceProviderResult,
        graphql::{CoopExitRequest, GraphQLClient, LightningReceiveRequest, LightningSendRequest},
    },
};

pub struct ServiceProvider {
    identity_public_key: PublicKey,
    gql_client: GraphQLClient,
}

impl ServiceProvider {
    /// Create a new SSP service provider.
    ///
    /// Internally builds a [`SspAuthHeaderProvider`] backed by the supplied
    /// `signer` and `session_store`. If `extra_header_provider` is set, the
    /// auth provider's headers are combined with it on every request — used,
    /// for example, to attach the Breez partner JWT alongside the SSP session
    /// token.
    pub fn new(
        config: ServiceProviderConfig,
        spark_signer: Arc<dyn SparkSigner>,
        session_store: Arc<dyn SessionStore>,
        extra_header_provider: Option<Arc<dyn HeaderProvider>>,
    ) -> Self {
        let user_agent = config.user_agent.clone().unwrap_or_else(default_user_agent);
        let http_client = create_http_client(Some(&user_agent));
        Self::new_with_client(
            config,
            spark_signer,
            session_store,
            extra_header_provider,
            http_client,
        )
    }

    /// Like [`ServiceProvider::new`], but uses a shared HTTP client so the
    /// underlying `reqwest::Client` (and its connection pool) is reused across
    /// SDK instances. The same client is used for both the GraphQL service and
    /// the SSP auth challenge-response, so a single connection pool serves all
    /// SSP traffic.
    pub fn new_with_client(
        config: ServiceProviderConfig,
        spark_signer: Arc<dyn SparkSigner>,
        session_store: Arc<dyn SessionStore>,
        extra_header_provider: Option<Arc<dyn HeaderProvider>>,
        http_client: Arc<dyn HttpClient>,
    ) -> Self {
        let header_provider = Self::build_header_provider(
            &config,
            Arc::clone(&http_client),
            spark_signer,
            session_store,
            extra_header_provider,
        );
        Self {
            identity_public_key: config.identity_public_key,
            gql_client: GraphQLClient::new_with_client(config.into(), header_provider, http_client),
        }
    }

    fn build_header_provider(
        config: &ServiceProviderConfig,
        http_client: Arc<dyn HttpClient>,
        spark_signer: Arc<dyn SparkSigner>,
        session_store: Arc<dyn SessionStore>,
        extra_header_provider: Option<Arc<dyn HeaderProvider>>,
    ) -> Arc<dyn HeaderProvider> {
        let auth_provider: Arc<dyn HeaderProvider> = Arc::new(SspAuthHeaderProvider::new(
            &config.base_url,
            config.schema_endpoint.as_deref(),
            http_client,
            spark_signer,
            session_store,
            config.identity_public_key,
        ));
        match extra_header_provider {
            Some(extra) => Arc::new(CombinedHeaderProvider::new(vec![auth_provider, extra])),
            None => auth_provider,
        }
    }

    pub fn identity_public_key(&self) -> PublicKey {
        self.identity_public_key
    }

    /// Get a swap fee estimate
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn get_swap_fee_estimate(
        &self,
        amount_sats: u64,
    ) -> ServiceProviderResult<CurrencyAmount> {
        Ok(self.gql_client.get_swap_fee_estimate(amount_sats).await?)
    }

    /// Get a lightning send fee estimate
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
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

    /// Get a coop exit fee quote
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn get_coop_exit_fee_quote(
        &self,
        leaf_external_ids: Vec<String>,
        withdrawal_address: &str,
    ) -> ServiceProviderResult<CoopExitFeeQuote> {
        Ok(self
            .gql_client
            .get_coop_exit_fee_quote(leaf_external_ids, withdrawal_address)
            .await?)
    }

    /// Complete a cooperative exit
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
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
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn request_coop_exit(
        &self,
        input: RequestCoopExitInput,
    ) -> ServiceProviderResult<CoopExitRequest> {
        Ok(self.gql_client.request_coop_exit(input).await?)
    }

    /// Request lightning receive
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn request_lightning_receive(
        &self,
        input: RequestLightningReceiveInput,
    ) -> ServiceProviderResult<LightningReceiveRequest> {
        Ok(self.gql_client.request_lightning_receive(input).await?)
    }

    /// Request lightning send
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn request_lightning_send(
        &self,
        input: RequestLightningSendInput,
    ) -> ServiceProviderResult<LightningSendRequest> {
        Ok(self.gql_client.request_lightning_send(input).await?)
    }

    /// Request swap (v3)
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn request_swap(
        &self,
        input: RequestSwapInput,
    ) -> ServiceProviderResult<LeavesSwapRequest> {
        Ok(self.gql_client.request_swap(input).await?)
    }

    /// Get claim deposit quote
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
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
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
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
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
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
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn get_leaves_swap_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<LeavesSwapRequest>> {
        Ok(self.gql_client.get_leaves_swap_request(request_id).await?)
    }

    /// Get a cooperative exit request by ID
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn get_coop_exit_request(
        &self,
        request_id: &str,
    ) -> ServiceProviderResult<Option<CoopExitRequest>> {
        Ok(self.gql_client.get_coop_exit_request(request_id).await?)
    }

    /// Claim static deposit
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn claim_static_deposit(
        &self,
        input: ClaimStaticDepositInput,
    ) -> ServiceProviderResult<ClaimStaticDeposit> {
        Ok(self.gql_client.claim_static_deposit(input).await?)
    }

    /// Get transfers by IDs
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn get_transfers(
        &self,
        transfer_spark_ids: Vec<String>,
    ) -> ServiceProviderResult<Vec<SspTransfer>> {
        Ok(self.gql_client.get_transfers(transfer_spark_ids).await?)
    }

    /// Register a wallet webhook with the SSP
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn register_wallet_webhook(
        &self,
        url: &str,
        secret: &str,
        event_types: Vec<SparkWalletWebhookEventType>,
    ) -> ServiceProviderResult<String> {
        Ok(self
            .gql_client
            .register_wallet_webhook(url, secret, event_types)
            .await?)
    }

    /// Delete a wallet webhook from the SSP
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn delete_wallet_webhook(&self, webhook_id: &str) -> ServiceProviderResult<bool> {
        Ok(self.gql_client.delete_wallet_webhook(webhook_id).await?)
    }

    /// List wallet webhooks from the SSP
    #[instrument(level = "info", target = "spark::ssp", skip_all)]
    pub async fn list_wallet_webhooks(&self) -> ServiceProviderResult<Vec<WebhookEntry>> {
        Ok(self.gql_client.list_wallet_webhooks().await?)
    }
}
