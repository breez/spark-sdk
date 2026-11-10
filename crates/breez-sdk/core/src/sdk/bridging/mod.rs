//! Bridging: orders in which a payer on another chain or on Lightning pays a
//! recipient, with no funds moving through the SDK.

mod models;
mod orchestra;

pub use models::*;

use std::collections::HashSet;
use std::sync::Arc;

use breez_sdk_common::{
    breez_server::{BreezServer, PRODUCTION_BREEZSERVER_URL},
    buy::cashapp::{CashAppProvider, cash_app_lightning_address},
    fiat::FiatService,
    input::{CrossChainAddressFamily, detect_address_family, validate_lightning_address_format},
    lnurl::pay::validate_user_input,
};

use crate::{
    BitcoinNetwork, BridgeFromCashAppRequest, BridgeFromCashAppResponse, BridgeToCashAppRequest,
    BridgeToCashAppResponse, CrossChainAcceptedAsset, CrossChainAddressDetails, CrossChainConfig,
    CrossChainProvider, CrossChainReceiveInfo, CrossChainRoutePair, DeliveryMethod, FeePolicy,
    InputType, LightningAddressDetails, SparkAddressDetails, SparkAsset,
    cross_chain::{
        BreezServerOrchestraConfigResolver, CachedFiatService, CrossChainContext,
        CrossChainFeeMode, CrossChainProviderContext, CrossChainReceivePrepared,
        CrossChainSendPrepared, CrossChainService, DEFAULT_FIAT_CACHE_TTL, DepositDestination,
        OrchestraQuoter, convert_destination_amount_to_sats, delivery_method_from_wire,
        delivery_method_to_wire, fetch_btc_usd_rate, is_usd_stable_asset,
        parse_rfc3339_to_unix_seconds, rescale_decimals,
    },
    error::SdkError,
    utils::time::now_secs,
};

use self::orchestra::{bridge_status_response, is_unpaid_onramp_order, to_bridge_route_pair};
use super::SdkInputParser;
use super::helpers::{partner_id, validate_breez_api_key};
use super::payments::receive::{
    quote_cross_chain_receive, resolve_cross_chain_receive_params, validate_receive_destination,
};
use super::payments::validation::{
    known_token_contracts, resolve_direct_overpay_amount, resolve_slippage_bps,
    validate_address_family_against_route, validate_amount,
    validate_recipient_not_contract_address,
};

/// Creates bridges: orders that pay a recipient from another chain or over
/// Lightning, with no funds moving through the SDK. Get one with
/// [`new_bridging_client`], or from a connected SDK with
/// `get_bridging_client`. Only available on mainnet.
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct BridgingClient {
    cross_chain_config: Option<CrossChainConfig>,
    cross_chain_context: CrossChainContext,
    quoter: Arc<OrchestraQuoter>,
    input_parser: Arc<SdkInputParser>,
}

/// Builds a [`BridgingClient`] from an API key, with no seed or storage.
#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
pub async fn new_bridging_client(config: BridgingConfig) -> Result<Arc<BridgingClient>, SdkError> {
    validate_breez_api_key(&config.api_key)?;
    if let Some(proxy) = &config.proxy {
        proxy.validate()?;
    }
    if let Some(cross_chain_config) = &config.cross_chain_config {
        cross_chain_config.validate()?;
    }

    let user_agent = crate::default_user_agent();
    let transport_proxy = config.proxy.as_ref().map(platform_utils::ProxyConfig::from);
    let http_client =
        platform_utils::create_http_client_with_proxy(Some(&user_agent), transport_proxy.as_ref())
            .map_err(|e| {
                SdkError::InvalidInput(format!("Failed to build proxied HTTP client: {e}"))
            })?;
    let breez_server = Arc::new(
        BreezServer::new(
            PRODUCTION_BREEZSERVER_URL,
            None,
            &user_agent,
            transport_proxy.as_ref(),
        )
        .map_err(|e| SdkError::Generic(e.to_string()))?,
    );
    // Lightning-address hosts are chosen by remote parties, so their lookups
    // ride a client that revalidates every redirect hop, as in the SDK.
    let lnurl_http_client: Arc<dyn platform_utils::HttpClient> =
        platform_utils::create_http_client_with_redirect_filter(
            Some(&user_agent),
            transport_proxy.as_ref(),
            breez_sdk_common::lnurl::security::lnurl_redirect_filter(),
        )
        .map_err(|e| SdkError::Generic(format!("Failed to build LNURL HTTP client: {e}")))?;
    let input_parser = Arc::new(SdkInputParser::new(
        breez_sdk_common::dns::Resolver::with_proxy(transport_proxy.as_ref())
            .map_err(|e| SdkError::Generic(format!("Failed to build DNS resolver: {e}")))?,
        lnurl_http_client,
        None,
    ));
    let fiat_service: Arc<dyn FiatService> = Arc::new(CachedFiatService::new(
        Arc::clone(&breez_server) as Arc<dyn FiatService>,
        DEFAULT_FIAT_CACHE_TTL,
    ));
    let quoter = Arc::new(OrchestraQuoter::new(
        Arc::new(BreezServerOrchestraConfigResolver::new(breez_server)),
        partner_id(&config.api_key),
        Arc::clone(&fiat_service),
        http_client,
    ));
    Ok(Arc::new(BridgingClient::new(
        config.cross_chain_config,
        quoter,
        fiat_service,
        input_parser,
    )))
}

/// Settings for [`new_bridging_client`] with every optional field unset.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn default_bridging_config(api_key: String) -> BridgingConfig {
    BridgingConfig {
        api_key,
        proxy: None,
        cross_chain_config: None,
    }
}

impl BridgingClient {
    pub(crate) fn new(
        cross_chain_config: Option<CrossChainConfig>,
        quoter: Arc<OrchestraQuoter>,
        fiat_service: Arc<dyn FiatService>,
        input_parser: Arc<SdkInputParser>,
    ) -> Self {
        let mut cross_chain_context = CrossChainContext::new(fiat_service);
        cross_chain_context.insert(
            CrossChainProvider::Orchestra,
            Arc::clone(&quoter) as Arc<dyn CrossChainService>,
        );
        Self {
            cross_chain_config,
            cross_chain_context,
            quoter,
            input_parser,
        }
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl BridgingClient {
    /// Lists the routes that can pay `recipient`.
    pub async fn get_routes(
        &self,
        request: GetBridgeRoutesRequest,
    ) -> Result<GetBridgeRoutesResponse, SdkError> {
        let recipient = self.parse_recipient(&request.recipient).await?;
        let mut routes = match &recipient {
            BridgeRecipient::Spark(_) => {
                self.listed_routes(delivery_method_to_wire(DeliveryMethod::Spark), false)
                    .await?
            }
            BridgeRecipient::Lightning(_) => {
                self.listed_routes(delivery_method_to_wire(DeliveryMethod::Lightning), false)
                    .await?
            }
            BridgeRecipient::CrossChain(_) => self.routes_to_other_chain(&recipient).await?,
        };
        routes.retain(|route| route_pays(&recipient, route));
        let mut seen = HashSet::new();
        routes.retain(|route| seen.insert((route.source.clone(), route.destination.clone())));
        routes.sort_by(|a, b| {
            (
                &a.source.asset,
                &a.source.chain,
                &a.destination.asset,
                &a.destination.chain,
            )
                .cmp(&(
                    &b.source.asset,
                    &b.source.chain,
                    &b.destination.asset,
                    &b.destination.chain,
                ))
        });
        Ok(GetBridgeRoutesResponse { routes })
    }

    /// Creates a bridge and returns what the payer pays. Once paid, the
    /// provider delivers to the recipient, or refunds `refund_address` if it
    /// can't. Track it with [`Self::get_bridge_status`].
    pub async fn create_bridge(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        self.create_bridge_with(request, LightningInvoice::Quote)
            .await
    }

    /// Reads a bridge's status from the provider.
    pub async fn get_bridge_status(
        &self,
        request: GetBridgeStatusRequest,
    ) -> Result<GetBridgeStatusResponse, SdkError> {
        if request.bridge_id.trim().is_empty() {
            return Err(SdkError::InvalidInput("Bridge id is empty".to_string()));
        }
        let status_token = request
            .status_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty());
        let bridge_id = request.bridge_id.trim();
        // Only a bridge from Cash App is an order, created before payment.
        let order = if is_order_id(bridge_id) {
            self.quoter
                .find_order_by_id(bridge_id, status_token)
                .await?
                .filter(|order| !is_unpaid_onramp_order(order))
        } else {
            self.quoter.find_order(bridge_id, status_token).await?
        };
        Ok(bridge_status_response(order.as_ref()))
    }

    /// Pays a recipient on another chain from Cash App. Takes a route for the
    /// address that the payer funds over Lightning, and returns the bridge
    /// with the Cash App link as its `payment_request`. The provider allows
    /// these payments 10% slippage when they execute. With fees excluded,
    /// `max_slippage_bps` bounds how far the quoted delivery may fall short of
    /// `amount` when the bridge is created.
    pub async fn bridge_from_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        if delivery_method(&request.route.source) != Some(DeliveryMethod::Lightning) {
            return Err(SdkError::InvalidInput(
                "Cash App pays a recipient on another chain over Lightning".to_string(),
            ));
        }
        let mut response = self
            .create_bridge_with(request, LightningInvoice::CashApp)
            .await?;
        response.payment_request = CashAppProvider::build_url(&response.info.deposit_address);
        Ok(response)
    }

    /// Pays a Cash App user from another chain. The recipient is their
    /// username, as `$alice`, or their `alice@cash.app` Lightning address, as
    /// given to [`Self::get_routes`].
    pub async fn bridge_to_cash_app(
        &self,
        request: CreateBridgeRequest,
    ) -> Result<CreateBridgeResponse, SdkError> {
        let Some(address) = cash_app_lightning_address(request.recipient.trim()) else {
            return Err(SdkError::InvalidInput(format!(
                "{} is not a Cash App username",
                request.recipient
            )));
        };
        self.create_bridge(CreateBridgeRequest {
            recipient: address,
            ..request
        })
        .await
    }
}

impl BridgingClient {
    /// Bridges from Cash App over a `CrossChainRoutePair` route. A failure
    /// before the swap is refunded to `refund_address`, a Lightning address.
    pub(crate) async fn legacy_bridge_from_cash_app(
        &self,
        request: BridgeFromCashAppRequest,
        refund_address: Option<&str>,
    ) -> Result<BridgeFromCashAppResponse, SdkError> {
        validate_amount(Some(request.amount))?;

        let BridgeFromCashAppRequest {
            address,
            route,
            amount,
            fee_policy,
            max_slippage_bps,
        } = request;

        // A Boltz reverse swap needs the SDK online to claim before the
        // payer's HTLC settles, so it can't back a link someone else pays
        // later. Orchestra orders are submitless and deliver without the SDK.
        if !matches!(route.provider, CrossChainProvider::Orchestra) {
            return Err(SdkError::InvalidInput(
                "Bridging from Cash App is only supported on Orchestra routes".to_string(),
            ));
        }

        // Cash App funds the deposit over Lightning. Fail fast before quoting if
        // the route can't be funded that way. Rejecting an empty
        // `delivery_methods` stops a hand-built route from reaching
        // `prepare` and committing provider state it can't fund.
        if !route_supports_delivery_method(&route, DeliveryMethod::Lightning) {
            return Err(SdkError::InvalidInput(
                "The selected route can't be funded over Lightning".to_string(),
            ));
        }

        // Validate the recipient before creating the order, using the same guards
        // as the send path.
        let InputType::CrossChainAddress(recipient) = self.parse(&address).await? else {
            return Err(SdkError::InvalidInput(
                "Recipient must be a cross-chain (EVM/Solana/Tron) address".to_string(),
            ));
        };
        self.validate_cross_chain_recipient(&recipient, &route)
            .await?;

        let funded = self
            .quote_lightning_funded(
                &recipient.address,
                &route,
                amount,
                fee_policy,
                max_slippage_bps,
                LightningInvoice::CashApp,
                refund_address,
            )
            .await?;

        let url = CashAppProvider::build_url(&funded.deposit_target);

        bridge_from_cash_app_response(funded.prepared, url, funded.amount_sats)
    }

    /// Bridges to a Cash App user over a `CrossChainRoutePair` route.
    pub(crate) async fn legacy_bridge_to_cash_app(
        &self,
        request: BridgeToCashAppRequest,
    ) -> Result<BridgeToCashAppResponse, SdkError> {
        validate_amount(Some(request.amount))?;

        let BridgeToCashAppRequest {
            recipient,
            route,
            amount,
            fee_policy,
            refund_address,
            max_slippage_bps,
        } = request;

        // The provider pays out after the deposit lands, with no wallet
        // online, which only Orchestra supports.
        if !matches!(route.provider, CrossChainProvider::Orchestra) {
            return Err(SdkError::InvalidInput(
                "Bridging to Cash App is only supported on Orchestra routes".to_string(),
            ));
        }
        if !route_supports_delivery_method(&route, DeliveryMethod::Lightning) {
            return Err(SdkError::InvalidInput(
                "The selected route can't deliver over Lightning".to_string(),
            ));
        }

        let Some(address) = cash_app_lightning_address(recipient.trim()) else {
            return Err(SdkError::InvalidInput(format!(
                "{recipient} is not a Cash App username"
            )));
        };
        // Resolved before quoting so an unknown account fails before the payer
        // funds anything.
        let Ok(InputType::LightningAddress(recipient)) = self.parse(&address).await else {
            return Err(SdkError::InvalidInput(format!(
                "{recipient} is not a reachable Cash App account"
            )));
        };

        let refund = self
            .validate_refund_address(Some(&refund_address), &route)
            .await?;

        let prepared = self
            .quote_external_funded(
                &route,
                amount,
                SparkAsset::Bitcoin,
                DeliveryMethod::Lightning,
                &recipient.address,
                refund.as_deref(),
                fee_policy,
                max_slippage_bps,
            )
            .await?;
        check_lightning_address_accepts(&recipient, prepared.info.expected_received_amount)?;

        Ok(BridgeToCashAppResponse {
            payment_request: prepared.payment_request,
            info: prepared.info,
        })
    }

    async fn parse(&self, input: &str) -> Result<InputType, SdkError> {
        Ok(self.input_parser.parse(input).await?.into())
    }

    /// Parses who a bridge pays. Anything a bridge can't pay fails with
    /// `InvalidInput`.
    async fn parse_recipient(&self, input: &str) -> Result<BridgeRecipient, SdkError> {
        match self.parse(input).await? {
            InputType::SparkAddress(details) => {
                validate_spark_recipient(&details)?;
                Ok(BridgeRecipient::Spark(details))
            }
            InputType::LightningAddress(details) => Ok(BridgeRecipient::Lightning(details)),
            InputType::CrossChainAddress(details) => Ok(BridgeRecipient::CrossChain(details)),
            _ => Err(SdkError::InvalidInput(
                "A bridge can only pay a Spark address, a Lightning address, or an \
                 EVM/Solana/Tron address"
                    .to_string(),
            )),
        }
    }

    async fn create_bridge_with(
        &self,
        request: CreateBridgeRequest,
        invoice: LightningInvoice,
    ) -> Result<CreateBridgeResponse, SdkError> {
        validate_amount(Some(request.amount))?;
        if !matches!(request.route.provider, CrossChainProvider::Orchestra) {
            return Err(SdkError::InvalidInput(
                "Bridging is only supported on Orchestra routes".to_string(),
            ));
        }
        let recipient = self.parse_recipient(&request.recipient).await?;
        self.create_bridge_for(&recipient, request, invoice).await
    }

    /// Creates a bridge to a parsed `recipient`.
    async fn create_bridge_for(
        &self,
        recipient: &BridgeRecipient,
        request: CreateBridgeRequest,
        invoice: LightningInvoice,
    ) -> Result<CreateBridgeResponse, SdkError> {
        let CreateBridgeRequest {
            route,
            amount,
            refund_address,
            fee_policy,
            max_slippage_bps,
            ..
        } = request;
        if !route_pays(recipient, &route) {
            return Err(SdkError::InvalidInput(
                "The route doesn't pay this recipient".to_string(),
            ));
        }
        match recipient {
            BridgeRecipient::Spark(details) => validate_spark_recipient(details)?,
            // Orchestra only pays out over Lightning with a refund address.
            BridgeRecipient::Lightning(_) if refund_address.is_none() => {
                return Err(SdkError::InvalidInput(
                    "A refund address is required to pay a Lightning address".to_string(),
                ));
            }
            BridgeRecipient::Lightning(_) | BridgeRecipient::CrossChain(_) => {}
        }
        // The payment request is built from the route's sides, so they must be
        // the provider's, not a hand-built or stale copy.
        let listed = self.listed_routes(&route.destination.chain, false).await?;
        if !is_listed(&listed, &route) {
            return Err(SdkError::InvalidInput(
                "The provider doesn't serve this route".to_string(),
            ));
        }

        match recipient {
            BridgeRecipient::CrossChain(details) => {
                self.create_bridge_to_other_chain(
                    details,
                    &route,
                    amount,
                    refund_address.as_deref(),
                    fee_policy,
                    max_slippage_bps,
                    invoice,
                )
                .await
            }
            BridgeRecipient::Spark(details) => {
                let destination = spark_asset(&route.destination);
                let source = external_pair(
                    &route,
                    &route.source,
                    Some((destination.clone(), DeliveryMethod::Spark)),
                );
                let refund = self
                    .validate_refund_address(refund_address.as_deref(), &source)
                    .await?;
                let prepared = self
                    .quote_external_funded(
                        &source,
                        source_units(&route, amount)?,
                        destination,
                        DeliveryMethod::Spark,
                        &details.address,
                        refund.as_deref(),
                        fee_policy,
                        max_slippage_bps,
                    )
                    .await?;
                Ok(create_bridge_response(prepared))
            }
            BridgeRecipient::Lightning(details) => {
                let source = external_pair(
                    &route,
                    &route.source,
                    Some((SparkAsset::Bitcoin, DeliveryMethod::Lightning)),
                );
                let refund = self
                    .validate_refund_address(refund_address.as_deref(), &source)
                    .await?;
                let prepared = self
                    .quote_external_funded(
                        &source,
                        amount,
                        SparkAsset::Bitcoin,
                        DeliveryMethod::Lightning,
                        &details.address,
                        refund.as_deref(),
                        fee_policy,
                        max_slippage_bps,
                    )
                    .await?;
                check_lightning_address_accepts(details, prepared.info.expected_received_amount)?;
                Ok(create_bridge_response(prepared))
            }
        }
    }

    fn default_slippage_bps(&self) -> Option<u32> {
        self.cross_chain_config
            .as_ref()
            .and_then(|c| c.default_slippage_bps)
    }

    fn default_target_overpay_bps(&self) -> Option<u32> {
        self.cross_chain_config
            .as_ref()
            .and_then(|c| c.default_target_overpay_bps)
    }

    /// Checks a recipient on another chain against `route`, and that it isn't
    /// a token contract, which can never pass funds on.
    async fn validate_cross_chain_recipient(
        &self,
        recipient: &CrossChainAddressDetails,
        route: &CrossChainRoutePair,
    ) -> Result<(), SdkError> {
        let recipient_family: CrossChainAddressFamily = recipient.address_family.into();
        validate_address_family_against_route(recipient_family, route)?;
        let known_contracts = known_token_contracts(
            &self.cross_chain_context,
            &recipient.address,
            recipient_family,
            route,
        )
        .await;
        validate_recipient_not_contract_address(
            &recipient.address,
            recipient_family,
            &known_contracts,
        )
    }

    /// Parses the payer's refund address, when given, and checks it against
    /// the source `route`.
    async fn validate_refund_address(
        &self,
        refund_address: Option<&str>,
        route: &CrossChainRoutePair,
    ) -> Result<Option<String>, SdkError> {
        let Some(refund_address) = refund_address else {
            return Ok(None);
        };
        let InputType::CrossChainAddress(refund) = self.parse(refund_address).await? else {
            return Err(SdkError::InvalidInput(
                "Refund address must be a cross-chain (EVM/Solana/Tron) address".to_string(),
            ));
        };
        let refund_family: CrossChainAddressFamily = refund.address_family.into();
        validate_address_family_against_route(refund_family, route)?;
        let known_contracts = known_token_contracts(
            &self.cross_chain_context,
            &refund.address,
            refund_family,
            route,
        )
        .await;
        validate_recipient_not_contract_address(&refund.address, refund_family, &known_contracts)
            .map_err(|_| {
            SdkError::InvalidInput(
                "Refund address is a token contract. Funds refunded to it cannot be \
                     recovered. Use the payer's wallet address instead."
                    .to_string(),
            )
        })?;
        Ok(Some(refund.address))
    }

    /// Quotes an order the payer funds over Lightning, delivering `amount` of
    /// `route`'s asset to `recipient_address`, with an `invoice` of that kind.
    /// A failure before the swap is refunded to `refund_address`, which must
    /// be one [`Self::lightning_refund_address`] accepts.
    #[allow(clippy::too_many_arguments)]
    async fn quote_lightning_funded(
        &self,
        recipient_address: &str,
        route: &CrossChainRoutePair,
        amount: u128,
        fee_policy: Option<FeePolicy>,
        max_slippage_bps: Option<u32>,
        invoice: LightningInvoice,
        refund_address: Option<&str>,
    ) -> Result<LightningFunded, SdkError> {
        let slippage_bps = resolve_slippage_bps(max_slippage_bps, self.default_slippage_bps())?;
        let refund = self.lightning_refund_address(refund_address).await?;
        let refund_address = refund.as_ref().map(|r| r.address.as_str());

        // `amount` is in the destination asset's base units (the route's
        // `decimals`); for these USD-pegged stablecoins it equals the USD value
        // at parity. Convert it to the sats the provider's `prepare` expects
        // (for both fee modes) via the live rate.
        let btc_usd = fetch_btc_usd_rate(self.cross_chain_context.fiat_service().as_ref()).await?;
        let source_sats =
            convert_destination_amount_to_sats(amount, btc_usd, route.decimals.into())?;
        if source_sats == 0 {
            return Err(SdkError::InvalidInput(
                "Amount is too small to fund over Lightning".to_string(),
            ));
        }

        // Match the send path: on `FeesExcluded` pad the source so the recipient
        // lands at or above target despite provider slippage. The pad comes from
        // config (no per-request override on a Lightning-funded bridge).
        let fee_policy = fee_policy.unwrap_or_default();
        let overpay_bps = crate::cross_chain::resolve_target_overpay_bps(
            None,
            self.default_target_overpay_bps(),
        )?;
        let source_sats = resolve_direct_overpay_amount(source_sats, fee_policy, overpay_bps);

        let (prepared, order_id) = match invoice {
            LightningInvoice::Quote => {
                let prepared = self
                    .quoter
                    .quote_send(
                        recipient_address,
                        route,
                        source_sats,
                        DeliveryMethod::Lightning,
                        None,
                        slippage_bps,
                        fee_policy.into(),
                        refund_address,
                    )
                    .await?;
                (prepared, None)
            }
            LightningInvoice::CashApp => {
                let order = self
                    .quoter
                    .prepare_onramp_send(
                        recipient_address,
                        route,
                        source_sats,
                        slippage_bps,
                        fee_policy.into(),
                        refund_address,
                    )
                    .await?;
                (order.prepared, Some(order.order_id))
            }
        };

        let checked = match check_refund_outlives(refund.as_ref(), &prepared.expires_at) {
            Ok(()) => self.check_lightning_funded(&prepared).await,
            Err(e) => Err(e),
        };
        let (deposit_target, amount_sats) = checked.inspect_err(|e| {
            // The order exists and expires unpaid, but support may ask for it.
            if let Some(order_id) = &order_id {
                tracing::warn!("Bridging: discarding order {order_id}: {e}");
            }
        })?;

        Ok(LightningFunded {
            prepared,
            deposit_target,
            amount_sats,
            order_id,
        })
    }

    /// The BOLT11 the payer funds and its amount in sats, checked against the
    /// quote before the payer is sent to it.
    async fn check_lightning_funded(
        &self,
        prepared: &CrossChainSendPrepared,
    ) -> Result<(String, u64), SdkError> {
        let amount_sats = u64::try_from(prepared.amount_in).map_err(|_| {
            SdkError::Generic(format!(
                "Deposit amount {} sats exceeds u64::MAX",
                prepared.amount_in
            ))
        })?;
        // The payer funds the provider's invoice directly, and the provider
        // delivers once it is paid: there is no Spark transfer to make.
        let deposit_target = deposit_target(&prepared.provider_context);
        let parsed_target = self.parse(&deposit_target).await;
        check_deposit_target(&parsed_target, amount_sats)?;
        Ok((deposit_target, amount_sats))
    }

    /// The refund destination a Lightning-funded order takes for
    /// `refund_address`: a Lightning address, a `$cashtag` as its Cash App
    /// Lightning address, or an unexpired amountless mainnet BOLT11. Anything
    /// else fails with `InvalidInput`.
    async fn lightning_refund_address(
        &self,
        refund_address: Option<&str>,
    ) -> Result<Option<LightningRefund>, SdkError> {
        let Some(input) = refund_address.map(str::trim).filter(|a| !a.is_empty()) else {
            return Ok(None);
        };
        let parsed = if input.starts_with('$') || validate_lightning_address_format(input) {
            None
        } else {
            self.parse(input).await.ok()
        };
        accepted_lightning_refund(input, parsed.as_ref(), now_secs())
            .map(Some)
            .ok_or_else(|| {
                SdkError::InvalidInput(
                    "A Lightning refund address must be a Lightning address, a $cashtag or an \
                     unexpired amountless mainnet BOLT11 invoice"
                        .to_string(),
                )
            })
    }

    /// Quotes an order the payer funds with a deposit on `route`'s chain,
    /// delivering `destination` to `recipient` over `delivery_method`.
    #[allow(clippy::too_many_arguments)]
    async fn quote_external_funded(
        &self,
        route: &CrossChainRoutePair,
        amount: u128,
        destination: SparkAsset,
        delivery_method: DeliveryMethod,
        recipient: &str,
        refund_address: Option<&str>,
        fee_policy: Option<FeePolicy>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainReceivePrepared, SdkError> {
        let params = resolve_cross_chain_receive_params(
            &self.cross_chain_context,
            self.cross_chain_config.as_ref(),
            route,
            amount,
            delivery_method,
            fee_policy.map(Into::into),
            max_slippage_bps,
            None,
        )?;
        let destination = validate_receive_destination(route, destination)?;
        quote_cross_chain_receive(
            &self.cross_chain_context,
            route,
            amount,
            &destination,
            &params,
            recipient,
            refund_address,
        )
        .await
    }

    /// Creates a bridge to a recipient on another chain, funded over Lightning
    /// or by a stablecoin deposit, as `route` says.
    #[allow(clippy::too_many_arguments)]
    async fn create_bridge_to_other_chain(
        &self,
        recipient: &CrossChainAddressDetails,
        route: &BridgeRoutePair,
        amount: u128,
        refund_address: Option<&str>,
        fee_policy: Option<FeePolicy>,
        max_slippage_bps: Option<u32>,
        invoice: LightningInvoice,
    ) -> Result<CreateBridgeResponse, SdkError> {
        if delivery_method(&route.source) == Some(DeliveryMethod::Lightning) {
            let destination = external_pair(
                route,
                &route.destination,
                Some((SparkAsset::Bitcoin, DeliveryMethod::Lightning)),
            );
            self.validate_cross_chain_recipient(recipient, &destination)
                .await?;
            let funded = self
                .quote_lightning_funded(
                    &recipient.address,
                    &destination,
                    amount,
                    fee_policy,
                    max_slippage_bps,
                    invoice,
                    refund_address,
                )
                .await?;
            return lightning_funded_bridge_response(funded);
        }

        let source = external_pair(route, &route.source, None);
        let refund = self
            .validate_refund_address(refund_address, &source)
            .await?;
        self.validate_cross_chain_recipient(
            recipient,
            &external_pair(route, &route.destination, None),
        )
        .await?;
        let prepared = self
            .quote_chain_to_chain(
                route,
                &source,
                amount,
                &recipient.address,
                refund.as_deref(),
                fee_policy,
                max_slippage_bps,
            )
            .await?;
        Ok(create_bridge_response(prepared))
    }

    /// Quotes an order the payer funds with a stablecoin deposit on
    /// `source`'s chain, delivering `route`'s destination stablecoin to
    /// `recipient`. `amount` is in destination units.
    #[allow(clippy::too_many_arguments)]
    async fn quote_chain_to_chain(
        &self,
        route: &BridgeRoutePair,
        source: &CrossChainRoutePair,
        amount: u128,
        recipient: &str,
        refund_address: Option<&str>,
        fee_policy: Option<FeePolicy>,
        max_slippage_bps: Option<u32>,
    ) -> Result<CrossChainReceivePrepared, SdkError> {
        let slippage_bps = resolve_slippage_bps(max_slippage_bps, self.default_slippage_bps())?;
        let overpay_bps = crate::cross_chain::resolve_target_overpay_bps(
            None,
            self.default_target_overpay_bps(),
        )?;
        let fee_mode: CrossChainFeeMode = fee_policy.unwrap_or_default().into();
        // Fees included, `amount` is the deposit, at parity in the source asset.
        let provider_amount = match fee_mode {
            CrossChainFeeMode::FeesExcluded => amount,
            CrossChainFeeMode::FeesIncluded => source_units(route, amount)?,
        };
        if provider_amount == 0 {
            return Err(SdkError::InvalidInput(
                "Amount is too small for this route".to_string(),
            ));
        }
        let destination = DepositDestination {
            chain: route.destination.chain.clone(),
            asset: route.destination.asset.clone(),
            decimals: route.destination.decimals.into(),
            token_identifier: None,
        };
        let quote = self
            .quoter
            .quote_deposit(
                source,
                &destination,
                route.limits.as_ref(),
                recipient,
                provider_amount,
                slippage_bps,
                fee_mode,
                overpay_bps,
                refund_address,
            )
            .await?;
        Ok(quote.prepared)
    }

    /// The routes the provider serves out of `chain` when `is_send`, into it
    /// otherwise.
    async fn listed_routes(
        &self,
        chain: &str,
        is_send: bool,
    ) -> Result<Vec<BridgeRoutePair>, SdkError> {
        let routes = self.quoter.route_listing(chain, is_send).await?;
        Ok(routes.iter().map(to_bridge_route_pair).collect())
    }

    /// Routes to a recipient on another chain: funded over Lightning, or by a
    /// deposit into one of the chains Lightning-funded routes reach.
    async fn routes_to_other_chain(
        &self,
        recipient: &BridgeRecipient,
    ) -> Result<Vec<BridgeRoutePair>, SdkError> {
        let mut routes = self
            .listed_routes(delivery_method_to_wire(DeliveryMethod::Lightning), true)
            .await?;
        routes.retain(|route| route_pays(recipient, route));
        let mut chains: Vec<String> = routes.iter().map(|r| r.destination.chain.clone()).collect();
        chains.sort();
        chains.dedup();
        let listings =
            futures::future::join_all(chains.iter().map(|chain| self.listed_routes(chain, false)))
                .await;
        for (chain, listing) in chains.iter().zip(listings) {
            match listing {
                Ok(listed) => routes.extend(listed),
                Err(e) => tracing::warn!("Bridging: listing the routes into {chain} failed: {e}"),
            }
        }
        Ok(routes)
    }
}

/// Who a bridge pays.
#[derive(Debug, Clone)]
enum BridgeRecipient {
    /// Paid in Bitcoin or a Spark token.
    Spark(SparkAddressDetails),
    /// Paid in Bitcoin over Lightning.
    Lightning(LightningAddressDetails),
    /// Paid in a USD stablecoin.
    CrossChain(CrossChainAddressDetails),
}

/// Whether `route` pays `recipient`, moving only USD stablecoins and Bitcoin.
/// A Spark or Lightning recipient is paid from a stablecoin deposit on another
/// chain. A recipient on another chain is paid from one too, or over
/// Lightning.
fn route_pays(recipient: &BridgeRecipient, route: &BridgeRoutePair) -> bool {
    let (source, destination) = (&route.source, &route.destination);
    match recipient {
        BridgeRecipient::Spark(_) => {
            is_deposit(source)
                && delivery_method(destination) == Some(DeliveryMethod::Spark)
                && (is_bitcoin(destination)
                    || (destination.contract_address.is_some()
                        && is_usd_stable_asset(&destination.asset)))
        }
        BridgeRecipient::Lightning(_) => {
            is_deposit(source) && delivery_method(destination) == Some(DeliveryMethod::Lightning)
        }
        BridgeRecipient::CrossChain(details) => {
            (is_deposit(source) || delivery_method(source) == Some(DeliveryMethod::Lightning))
                && pays_address(destination, details)
        }
    }
}

/// Whether `listed` has a route with `route`'s sides.
fn is_listed(listed: &[BridgeRoutePair], route: &BridgeRoutePair) -> bool {
    listed
        .iter()
        .any(|r| r.source == route.source && r.destination == route.destination)
}

/// The delivery method `side` is on, or `None` for another chain. Lightning
/// only carries Bitcoin, while Spark also carries tokens.
fn delivery_method(side: &BridgeRoutePairSide) -> Option<DeliveryMethod> {
    delivery_method_from_wire(&side.chain)
}

fn is_bitcoin(side: &BridgeRoutePairSide) -> bool {
    side.asset.eq_ignore_ascii_case("BTC")
}

/// A stablecoin on another chain whose token contract names its address
/// family, so a refund address can be checked against it.
fn is_deposit(side: &BridgeRoutePairSide) -> bool {
    delivery_method(side).is_none()
        && is_usd_stable_asset(&side.asset)
        && side
            .contract_address
            .as_deref()
            .and_then(detect_address_family)
            .is_some()
}

/// Whether `side` delivers a stablecoin `recipient` can hold: on a chain of
/// its address family, and the token and chain it names, if any.
fn pays_address(side: &BridgeRoutePairSide, recipient: &CrossChainAddressDetails) -> bool {
    let family: CrossChainAddressFamily = recipient.address_family.into();
    delivery_method(side).is_none()
        && is_usd_stable_asset(&side.asset)
        && family.matches_chain(&side.chain, side.contract_address.as_deref())
        && recipient.contract_address.as_deref().is_none_or(|wanted| {
            side.contract_address
                .as_deref()
                .is_some_and(|contract| family.addresses_equal(contract, wanted))
        })
        && recipient.chain_id.is_none_or(|wanted| {
            side.chain_id
                .as_deref()
                .and_then(|id| id.parse::<u64>().ok())
                == Some(wanted)
        })
}

/// The Spark asset a route pays a Spark recipient.
fn spark_asset(side: &BridgeRoutePairSide) -> SparkAsset {
    match &side.contract_address {
        Some(token_identifier) if !is_bitcoin(side) => SparkAsset::Token {
            token_identifier: token_identifier.clone(),
        },
        _ => SparkAsset::Bitcoin,
    }
}

/// `amount` in source units. A stablecoin destination is rescaled at parity,
/// and any other amount is already in source units.
fn source_units(route: &BridgeRoutePair, amount: u128) -> Result<u128, SdkError> {
    if is_bitcoin(&route.destination) {
        return Ok(amount);
    }
    rescale_decimals(
        amount,
        route.destination.decimals.into(),
        route.source.decimals.into(),
    )
}

/// `side` as the external side of a [`CrossChainRoutePair`], for the quoting
/// and checks written against one. `local` is the Spark asset the route moves
/// and the rail it moves on.
fn external_pair(
    route: &BridgeRoutePair,
    side: &BridgeRoutePairSide,
    local: Option<(SparkAsset, DeliveryMethod)>,
) -> CrossChainRoutePair {
    let (accepted_assets, delivery_methods) = match local {
        Some((asset, delivery_method)) => (
            vec![CrossChainAcceptedAsset {
                asset,
                limits: route.limits.clone(),
            }],
            vec![delivery_method],
        ),
        None => (Vec::new(), Vec::new()),
    };
    CrossChainRoutePair {
        provider: route.provider,
        chain: side.chain.clone(),
        chain_id: side.chain_id.clone(),
        asset: side.asset.clone(),
        contract_address: side.contract_address.clone(),
        decimals: side.decimals,
        exact_out_eligible: false,
        accepted_assets,
        delivery_methods,
    }
}

/// Rejects a Spark address on a network other than mainnet.
fn validate_spark_recipient(details: &crate::SparkAddressDetails) -> Result<(), SdkError> {
    if details.network != BitcoinNetwork::Bitcoin {
        return Err(SdkError::InvalidInput(
            "The Spark address is not a mainnet address".to_string(),
        ));
    }
    Ok(())
}

/// Checks the Lightning address accepts `expected_sats`, the amount the
/// provider will pay it.
fn check_lightning_address_accepts(
    recipient: &LightningAddressDetails,
    expected_sats: u128,
) -> Result<(), SdkError> {
    let amount_msat = u64::try_from(expected_sats)
        .ok()
        .and_then(|sats| sats.checked_mul(1000))
        .ok_or_else(|| SdkError::InvalidInput("Amount is too large".to_string()))?;
    validate_user_input(
        amount_msat,
        &None,
        recipient.pay_request.min_sendable,
        recipient.pay_request.max_sendable,
        0,
    )
    .map_err(|e| SdkError::InvalidInput(e.to_string()))
}

fn create_bridge_response(prepared: CrossChainReceivePrepared) -> CreateBridgeResponse {
    CreateBridgeResponse {
        bridge_id: prepared.info.quote_id.clone(),
        status_token: prepared.quote_read_token,
        payment_request: prepared.payment_request,
        info: prepared.info,
    }
}

/// The response for a bridge the payer funds over Lightning: the deposit is
/// the provider's BOLT11 for `amount_sats`, and the recipient receives the
/// route's stablecoin. The bridge is the order when there is one, otherwise
/// the quote.
fn lightning_funded_bridge_response(
    funded: LightningFunded,
) -> Result<CreateBridgeResponse, SdkError> {
    let LightningFunded {
        prepared,
        deposit_target,
        amount_sats,
        order_id,
    } = funded;
    let CrossChainProviderContext::Orchestra { quote_id, .. } = prepared.provider_context else {
        return Err(SdkError::Generic(
            "Bridging is only supported on Orchestra routes".to_string(),
        ));
    };
    Ok(CreateBridgeResponse {
        bridge_id: order_id.unwrap_or_else(|| quote_id.clone()),
        status_token: prepared.quote_read_token,
        payment_request: deposit_target.clone(),
        info: CrossChainReceiveInfo {
            quote_id,
            deposit_address: deposit_target,
            deposit_amount: u128::from(amount_sats),
            expected_received_amount: prepared.estimated_out,
            destination_asset: prepared.pair.asset,
            token_identifier: None,
            service_fee_amount: prepared.service_fee_amount,
            service_fee_asset: prepared.service_fee_asset,
            service_fee_asset_decimals: prepared.service_fee_asset_decimals,
            expires_at: parse_rfc3339_to_unix_seconds(&prepared.expires_at)?,
        },
    })
}

/// Where a bridge the payer funds over Lightning gets its invoice.
#[derive(Debug, Clone, Copy)]
enum LightningInvoice {
    /// A quote's invoice.
    Quote,
    /// An onramp order's invoice.
    CashApp,
}

/// A bridge the payer funds over Lightning, before it is reported.
struct LightningFunded {
    prepared: CrossChainSendPrepared,
    /// The BOLT11 invoice the payer pays.
    deposit_target: String,
    amount_sats: u64,
    /// The order, when the invoice is an onramp order's.
    order_id: Option<String>,
}

/// Where a Lightning-funded bridge refunds a failure before the swap.
#[derive(Debug, PartialEq)]
struct LightningRefund {
    address: String,
    /// When the refund can no longer be paid, for an invoice.
    expires_at: Option<u64>,
}

/// What [`BridgingClient::lightning_refund_address`] accepts for `input`,
/// which parsed as `parsed` when it is neither a cashtag nor a Lightning
/// address.
fn accepted_lightning_refund(
    input: &str,
    parsed: Option<&InputType>,
    now: u64,
) -> Option<LightningRefund> {
    let address = |address: String| LightningRefund {
        address,
        expires_at: None,
    };
    if input.starts_with('$') {
        return cash_app_lightning_address(input).map(address);
    }
    if validate_lightning_address_format(input) {
        return Some(address(input.to_string()));
    }
    match parsed {
        // Orchestra refuses an invoice with an amount: the refund sets it.
        Some(InputType::Bolt11Invoice(details))
            if details.network == BitcoinNetwork::Bitcoin
                && details.amount_msat.is_none()
                && details.timestamp.saturating_add(details.expiry) > now =>
        {
            Some(LightningRefund {
                address: details.invoice.bolt11.clone(),
                expires_at: Some(details.timestamp.saturating_add(details.expiry)),
            })
        }
        _ => None,
    }
}

/// Errors if `refund` is an invoice that expires before the bridge, which
/// can be paid until `bridge_expires_at`: a failure after a late payment
/// could not be refunded.
fn check_refund_outlives(
    refund: Option<&LightningRefund>,
    bridge_expires_at: &str,
) -> Result<(), SdkError> {
    let Some(refund_expires_at) = refund.and_then(|r| r.expires_at) else {
        return Ok(());
    };
    if refund_expires_at < parse_rfc3339_to_unix_seconds(bridge_expires_at)? {
        return Err(SdkError::InvalidInput(
            "The refund invoice expires before the bridge does".to_string(),
        ));
    }
    Ok(())
}

/// Whether a bridge id names an order rather than a quote. Orchestra's API
/// documents order ids as `ord_...` (`GET /v1/explorer/operations/{id}`), and
/// its quote ids start with `q_`.
fn is_order_id(bridge_id: &str) -> bool {
    bridge_id.starts_with("ord_")
}

/// Whether `route` advertises `required` as a fundable delivery method. An
/// empty `delivery_methods` (e.g. a hand-built route) matches nothing and is
/// rejected.
fn route_supports_delivery_method(route: &CrossChainRoutePair, required: DeliveryMethod) -> bool {
    route.delivery_methods.contains(&required)
}

/// Verifies the provider's deposit `target` is a BOLT11 invoice requesting
/// exactly `expected_sats`, the Lightning payment request the payer funds. A
/// wrong type, wrong amount, or unparseable target indicates a provider bug.
pub(super) fn check_deposit_target(
    parsed_target: &Result<InputType, SdkError>,
    expected_sats: u64,
) -> Result<(), SdkError> {
    let Ok(InputType::Bolt11Invoice(details)) = parsed_target else {
        return Err(SdkError::Generic(
            "The provider returned a deposit target that is not a valid Lightning \
             payment request"
                .to_string(),
        ));
    };
    // The invoice must request the quoted deposit: a mismatch would have the
    // payer fund the wrong amount.
    if details.amount_msat.map(u128::from) != Some(u128::from(expected_sats) * 1000) {
        return Err(SdkError::Generic(format!(
            "The provider's deposit invoice does not request the quoted \
             {expected_sats} sats"
        )));
    }
    Ok(())
}

/// The BOLT11 an external payer funds, read from the provider context
/// (Orchestra `deposit_address` or Boltz `invoice`).
fn deposit_target(context: &CrossChainProviderContext) -> String {
    match context {
        CrossChainProviderContext::Orchestra {
            deposit_address, ..
        } => deposit_address.clone(),
        CrossChainProviderContext::Boltz { invoice, .. } => invoice.clone(),
    }
}

/// Maps a [`CrossChainSendPrepared`] + funding `url` to a
/// [`BridgeFromCashAppResponse`].
///
/// `estimated_out` is in the destination `asset`'s base units. The fee mirrors
/// the cross-chain send response: `service_fee_amount` in `service_fee_asset`
/// base units, where `None` means sats (Boltz denominates its fee in sats;
/// Orchestra in the stablecoin).
fn bridge_from_cash_app_response(
    prepared: CrossChainSendPrepared,
    url: String,
    amount_sats: u64,
) -> Result<BridgeFromCashAppResponse, SdkError> {
    Ok(BridgeFromCashAppResponse {
        url,
        amount_sats,
        estimated_out: prepared.estimated_out,
        asset: prepared.pair.asset,
        service_fee_amount: prepared.service_fee_amount,
        service_fee_asset: prepared.service_fee_asset,
        service_fee_asset_decimals: prepared.service_fee_asset_decimals,
        expires_at: parse_rfc3339_to_unix_seconds(&prepared.expires_at)?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use breez_sdk_common::buy::cashapp::CashAppProvider;
    use macros::{async_test_all, test_all};

    use super::{
        BridgeRecipient, BridgeRoutePair, BridgeRoutePairSide, BridgeStatus, BridgingClient,
        CreateBridgeRequest, GetBridgeStatusRequest, LightningFunded, LightningInvoice,
        LightningRefund, accepted_lightning_refund, bridge_from_cash_app_response,
        check_lightning_address_accepts, check_refund_outlives, deposit_target, external_pair,
        is_listed, is_order_id, lightning_funded_bridge_response, route_pays,
        route_supports_delivery_method, source_units, validate_spark_recipient,
    };
    use crate::cross_chain::{CrossChainProviderContext, CrossChainSendPrepared};
    use crate::{
        BitcoinNetwork, Bolt11Invoice, Bolt11InvoiceDetails, CrossChainAddressDetails,
        CrossChainAddressFamily, CrossChainFeeMode, CrossChainProvider, CrossChainRouteLimits,
        CrossChainRoutePair, DeliveryMethod, InputType, LightningAddressDetails,
        LnurlPayRequestDetails, PaymentRequestSource, SdkError, SparkAddressDetails, SparkAsset,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn orchestra_context() -> CrossChainProviderContext {
        CrossChainProviderContext::Orchestra {
            quote_id: "q1".to_string(),
            deposit_address: "lnbc100u1pxyz".to_string(),
            deposit_amount: 5000,
        }
    }

    fn route_with_delivery_methods(methods: Vec<DeliveryMethod>) -> CrossChainRoutePair {
        CrossChainRoutePair {
            provider: CrossChainProvider::Orchestra,
            chain: "base".to_string(),
            chain_id: Some("8453".to_string()),
            asset: "USDC".to_string(),
            contract_address: None,
            decimals: 6,
            exact_out_eligible: false,
            accepted_assets: vec![],
            delivery_methods: methods,
        }
    }

    #[test_all]
    fn route_supports_delivery_method_checks_membership() {
        let lightning_only = route_with_delivery_methods(vec![DeliveryMethod::Lightning]);
        // Cash App (Lightning) is supported; MoonPay (Bitcoin) is not.
        assert!(route_supports_delivery_method(
            &lightning_only,
            DeliveryMethod::Lightning
        ));
        assert!(!route_supports_delivery_method(
            &lightning_only,
            DeliveryMethod::Bitcoin
        ));

        let both =
            route_with_delivery_methods(vec![DeliveryMethod::Lightning, DeliveryMethod::Bitcoin]);
        assert!(route_supports_delivery_method(
            &both,
            DeliveryMethod::Bitcoin
        ));

        // An empty list matches nothing: a route must advertise its rails, so a
        // hand-built route can't slip a bad rail through to `prepare`.
        let empty = route_with_delivery_methods(vec![]);
        assert!(!route_supports_delivery_method(
            &empty,
            DeliveryMethod::Bitcoin
        ));
    }

    fn prepared(provider_context: CrossChainProviderContext) -> CrossChainSendPrepared {
        CrossChainSendPrepared {
            amount_in: 5000,
            asset_amount_in: 6_500_000,
            estimated_out: 6_450_000,
            fee_amount: 50_000,
            service_fee_amount: 47_684,
            service_fee_asset: Some("USDC".to_string()),
            service_fee_asset_decimals: Some(6),
            source_transfer_fee_sats: 0,
            fee_mode: CrossChainFeeMode::FeesExcluded,
            expires_at: "2026-07-25T08:09:26.770Z".to_string(),
            pair: CrossChainRoutePair {
                provider: CrossChainProvider::Orchestra,
                chain: "base".to_string(),
                chain_id: Some("8453".to_string()),
                asset: "USDC".to_string(),
                contract_address: None,
                decimals: 6,
                exact_out_eligible: false,
                accepted_assets: vec![],
                delivery_methods: vec![],
            },
            recipient_address: "0xabc".to_string(),
            token_identifier: None,
            provider_context,
            quote_read_token: None,
        }
    }

    #[test_all]
    fn deposit_target_reads_orchestra_address_and_boltz_invoice() {
        assert_eq!(deposit_target(&orchestra_context()), "lnbc100u1pxyz");
        assert_eq!(
            deposit_target(&CrossChainProviderContext::Boltz {
                swap_id: "s1".to_string(),
                invoice: "lnbc200u1pboltz".to_string(),
                invoice_amount_sats: 5000,
                max_slippage_bps: 100,
            }),
            "lnbc200u1pboltz"
        );
    }

    #[test_all]
    fn cashapp_url_built_from_deposit_target() {
        let url = CashAppProvider::build_url(&deposit_target(&orchestra_context()));
        assert_eq!(url, "https://cash.app/launch/lightning/lnbc100u1pxyz");
    }

    #[test_all]
    fn orchestra_response_maps_stablecoin_fee() {
        let resp = bridge_from_cash_app_response(
            prepared(orchestra_context()),
            "https://x".to_string(),
            5000,
        )
        .unwrap();
        assert_eq!(resp.url, "https://x");
        assert_eq!(resp.expires_at, 1_784_966_966);
        assert_eq!(resp.amount_sats, 5000);
        assert_eq!(resp.estimated_out, 6_450_000);
        assert_eq!(resp.asset, "USDC");
        // Orchestra denominates its service fee in the stablecoin.
        assert_eq!(resp.service_fee_amount, 47_684);
        assert_eq!(resp.service_fee_asset.as_deref(), Some("USDC"));
        assert_eq!(resp.service_fee_asset_decimals, Some(6));
    }

    #[test_all]
    fn boltz_response_reports_native_sats_fee() {
        let mut prepared = prepared(CrossChainProviderContext::Boltz {
            swap_id: "s1".to_string(),
            invoice: "lnbc200u1pboltz".to_string(),
            invoice_amount_sats: 5000,
            max_slippage_bps: 100,
        });
        // Boltz denominates its service fee in sats (service_fee_asset = None).
        prepared.service_fee_amount = 17;
        prepared.service_fee_asset = None;
        prepared.service_fee_asset_decimals = None;
        let resp = bridge_from_cash_app_response(prepared, "https://x".to_string(), 5000).unwrap();
        assert_eq!(resp.asset, "USDC");
        assert_eq!(resp.service_fee_amount, 17);
        assert_eq!(resp.service_fee_asset, None);
        assert_eq!(resp.service_fee_asset_decimals, None);
    }

    fn spark_address(network: BitcoinNetwork) -> SparkAddressDetails {
        SparkAddressDetails {
            address: "spark1merchant".to_string(),
            identity_public_key: "02".to_string(),
            network,
            source: PaymentRequestSource::default(),
        }
    }

    fn lightning_address(min_sendable: u64, max_sendable: u64) -> LightningAddressDetails {
        LightningAddressDetails {
            address: "alice@cash.app".to_string(),
            pay_request: LnurlPayRequestDetails {
                callback: "https://cash.app/callback".to_string(),
                min_sendable,
                max_sendable,
                metadata_str: "[]".to_string(),
                comment_allowed: 0,
                domain: "cash.app".to_string(),
                url: "https://cash.app/.well-known/lnurlp/alice".to_string(),
                address: Some("alice@cash.app".to_string()),
                allows_nostr: None,
                nostr_pubkey: None,
            },
        }
    }

    #[test_all]
    fn spark_recipient_must_be_on_mainnet() {
        assert!(validate_spark_recipient(&spark_address(BitcoinNetwork::Bitcoin)).is_ok());
        assert!(matches!(
            validate_spark_recipient(&spark_address(BitcoinNetwork::Regtest)),
            Err(SdkError::InvalidInput(_))
        ));
    }

    #[test_all]
    fn lightning_address_bounds_are_checked_in_msat() {
        let recipient = lightning_address(1_000_000, 100_000_000);
        assert!(check_lightning_address_accepts(&recipient, 1_000).is_ok());
        assert!(check_lightning_address_accepts(&recipient, 100_000).is_ok());
        assert!(check_lightning_address_accepts(&recipient, 999).is_err());
        assert!(check_lightning_address_accepts(&recipient, 100_001).is_err());
    }

    /// A client whose provider calls would fail, for checks that must reject
    /// a request before anything is quoted.
    fn offline_client() -> BridgingClient {
        struct Unreachable;

        #[macros::async_trait]
        impl platform_utils::HttpClient for Unreachable {
            async fn get(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                panic!("unexpected GET {url}")
            }
            async fn post(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
                _: Option<String>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                panic!("unexpected POST {url}")
            }
            async fn delete(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
                _: Option<String>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                panic!("unexpected DELETE {url}")
            }
        }

        struct NoConfig;

        #[macros::async_trait]
        impl flashnet::OrchestraConfigResolver for NoConfig {
            async fn resolve(&self) -> Result<flashnet::OrchestraConfig, flashnet::FlashnetError> {
                panic!("unexpected Orchestra config lookup")
            }
        }

        client_with(Arc::new(Unreachable), Arc::new(NoConfig))
    }

    fn client_with(
        http: Arc<dyn platform_utils::HttpClient>,
        config: Arc<dyn flashnet::OrchestraConfigResolver>,
    ) -> BridgingClient {
        struct NoFiat;

        #[macros::async_trait]
        impl breez_sdk_common::fiat::FiatService for NoFiat {
            async fn fetch_fiat_currencies(
                &self,
            ) -> Result<
                Vec<breez_sdk_common::fiat::FiatCurrency>,
                breez_sdk_common::error::ServiceConnectivityError,
            > {
                panic!("unexpected fiat lookup")
            }
            async fn fetch_fiat_rates(
                &self,
            ) -> Result<
                Vec<breez_sdk_common::fiat::Rate>,
                breez_sdk_common::error::ServiceConnectivityError,
            > {
                panic!("unexpected fiat lookup")
            }
        }

        let fiat: Arc<dyn breez_sdk_common::fiat::FiatService> = Arc::new(NoFiat);
        let quoter = Arc::new(crate::cross_chain::OrchestraQuoter::new(
            config,
            None,
            Arc::clone(&fiat),
            Arc::clone(&http),
        ));
        let input_parser = Arc::new(crate::sdk::SdkInputParser::new(
            breez_sdk_common::dns::Resolver::with_proxy(None).unwrap(),
            http,
            None,
        ));
        BridgingClient::new(None, quoter, fiat, input_parser)
    }

    #[test_all]
    fn a_lightning_refund_takes_a_lightning_address_cashtag_or_amountless_invoice() {
        let invoice = |amount_msat: Option<u64>, timestamp: u64| {
            InputType::Bolt11Invoice(Bolt11InvoiceDetails {
                amount_msat,
                description: None,
                description_hash: None,
                expiry: 3600,
                invoice: Bolt11Invoice {
                    bolt11: "lnbc1refund".to_string(),
                    source: PaymentRequestSource::default(),
                },
                min_final_cltv_expiry_delta: 144,
                network: BitcoinNetwork::Bitcoin,
                payee_pubkey: String::new(),
                payment_hash: String::new(),
                payment_secret: String::new(),
                routing_hints: vec![],
                timestamp,
            })
        };
        let now = 1_000_000;
        let accepted = |input: &str, parsed: Option<InputType>| {
            accepted_lightning_refund(input, parsed.as_ref(), now).map(|r| r.address)
        };

        assert_eq!(
            accepted("alice@example.com", None).as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(accepted("$alice", None).as_deref(), Some("alice@cash.app"));
        assert_eq!(accepted("$not-a-tag", None), None);
        assert_eq!(
            accepted("lightning:lnbc1refund", Some(invoice(None, now))).as_deref(),
            Some("lnbc1refund")
        );
        // The refund sets the amount, and an expired invoice can't take it.
        assert_eq!(accepted("lnbc1", Some(invoice(Some(1_000), now))), None);
        assert_eq!(accepted("lnbc1", Some(invoice(None, now - 3600))), None);
        // Bridges run on mainnet, where a refund can't pay a test network.
        let mut testnet = invoice(None, now);
        if let InputType::Bolt11Invoice(details) = &mut testnet {
            details.network = BitcoinNetwork::Testnet3;
        }
        assert_eq!(accepted("lntb1", Some(testnet)), None);
        assert_eq!(accepted("0xabc", None), None);
        assert_eq!(
            accepted_lightning_refund("lnbc1", Some(&invoice(None, now)), now)
                .and_then(|r| r.expires_at),
            Some(now + 3600)
        );
    }

    #[test_all]
    fn a_refund_invoice_must_outlive_the_bridge() {
        let bridge_expires_at = "2026-11-10T12:00:00Z";
        let bridge_secs = super::parse_rfc3339_to_unix_seconds(bridge_expires_at).unwrap();
        let invoice = |expires_at| LightningRefund {
            address: "lnbc1refund".to_string(),
            expires_at: Some(expires_at),
        };
        let address = LightningRefund {
            address: "alice@cash.app".to_string(),
            expires_at: None,
        };
        assert!(check_refund_outlives(None, bridge_expires_at).is_ok());
        assert!(check_refund_outlives(Some(&address), bridge_expires_at).is_ok());
        assert!(check_refund_outlives(Some(&invoice(bridge_secs)), bridge_expires_at).is_ok());
        assert_invalid_input(
            check_refund_outlives(
                Some(&invoice(bridge_secs.checked_sub(1).unwrap())),
                bridge_expires_at,
            ),
            "expires before the bridge",
        );
    }

    /// A client whose Orchestra knows one Lightning-funded order, `ord_1`, with
    /// `status`. Its explorer names the source chain and `/status` doesn't.
    fn explorer_client(status: &'static str) -> BridgingClient {
        struct Explorer(&'static str);

        #[macros::async_trait]
        impl platform_utils::HttpClient for Explorer {
            async fn get(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                let mut operation = serde_json::json!({
                    "id": "ord_1",
                    "status": self.0,
                    "destinationChain": "base",
                    "createdAt": "2026-11-10T10:00:00Z",
                    "updatedAt": "2026-11-10T10:00:00Z",
                });
                let body = if url.contains("/v1/orchestration/status?id=ord_1") {
                    serde_json::json!({ "order": operation, "stages": [] })
                } else {
                    assert!(url.ends_with("/v1/explorer/operations?q=ord_1"), "{url}");
                    operation["sourceChain"] = "lightning".into();
                    serde_json::json!({
                        "operations": [{ "operation": operation, "partner": null, "refs": [] }],
                        "nextOffset": null,
                    })
                };
                Ok(platform_utils::HttpResponse {
                    status: 200,
                    body: body.to_string(),
                    headers: std::collections::HashMap::new(),
                })
            }
            async fn post(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
                _: Option<String>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                panic!("unexpected POST {url}")
            }
            async fn delete(
                &self,
                url: String,
                _: Option<std::collections::HashMap<String, String>>,
                _: Option<String>,
            ) -> Result<platform_utils::HttpResponse, platform_utils::HttpError> {
                panic!("unexpected DELETE {url}")
            }
        }

        struct FixedConfig;

        #[macros::async_trait]
        impl flashnet::OrchestraConfigResolver for FixedConfig {
            async fn resolve(&self) -> Result<flashnet::OrchestraConfig, flashnet::FlashnetError> {
                Ok(flashnet::OrchestraConfig {
                    base_url: "https://orchestra.test".to_string(),
                    api_key: "key".to_string(),
                })
            }
        }

        client_with(Arc::new(Explorer(status)), Arc::new(FixedConfig))
    }

    #[async_test_all]
    async fn an_unpaid_cash_app_bridge_is_created_until_its_order_expires() {
        for (order_status, expected) in [
            ("processing", BridgeStatus::Created),
            ("expired", BridgeStatus::Expired),
            ("swapping", BridgeStatus::Pending),
        ] {
            for status_token in [None, Some("rt_order".to_string())] {
                let response = explorer_client(order_status)
                    .get_bridge_status(GetBridgeStatusRequest {
                        bridge_id: "ord_1".to_string(),
                        status_token: status_token.clone(),
                    })
                    .await
                    .unwrap();
                assert_eq!(response.status, expected, "{order_status} {status_token:?}");
            }
        }
    }

    fn create_request(recipient: &str, route: BridgeRoutePair) -> CreateBridgeRequest {
        CreateBridgeRequest {
            recipient: recipient.to_string(),
            route,
            amount: 10_000_000,
            refund_address: None,
            fee_policy: None,
            max_slippage_bps: None,
        }
    }

    fn assert_invalid_input<T: std::fmt::Debug>(result: Result<T, SdkError>, contains: &str) {
        match result {
            Err(SdkError::InvalidInput(message)) => {
                assert!(message.contains(contains), "{message}");
            }
            other => panic!("expected InvalidInput containing {contains:?}, got {other:?}"),
        }
    }

    fn base_address() -> CrossChainAddressDetails {
        CrossChainAddressDetails {
            address: "0x0000000000000000000000000000000000000001".to_string(),
            address_family: CrossChainAddressFamily::Evm,
            contract_address: None,
            chain_id: None,
            amount: None,
        }
    }

    const BASE_USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";

    fn side(chain: &str, asset: &str, contract: Option<&str>, decimals: u8) -> BridgeRoutePairSide {
        BridgeRoutePairSide {
            chain: chain.to_string(),
            chain_id: None,
            asset: asset.to_string(),
            contract_address: contract.map(str::to_string),
            decimals,
        }
    }

    fn base_usdc() -> BridgeRoutePairSide {
        BridgeRoutePairSide {
            chain_id: Some("8453".to_string()),
            ..side("base", "USDC", Some(BASE_USDC), 6)
        }
    }

    fn base_usdt() -> BridgeRoutePairSide {
        BridgeRoutePairSide {
            chain_id: Some("8453".to_string()),
            ..side(
                "base",
                "USDT",
                Some("0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2"),
                6,
            )
        }
    }

    fn tron_usdt() -> BridgeRoutePairSide {
        side(
            "tron",
            "USDT",
            Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            6,
        )
    }

    fn solana_usdc() -> BridgeRoutePairSide {
        side(
            "solana",
            "USDC",
            Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            6,
        )
    }

    fn base_weth() -> BridgeRoutePairSide {
        side(
            "base",
            "WETH",
            Some("0x4200000000000000000000000000000000000006"),
            18,
        )
    }

    fn spark_btc() -> BridgeRoutePairSide {
        side("spark", "BTC", None, 8)
    }

    fn spark_usdb() -> BridgeRoutePairSide {
        side("spark", "USDB", Some("btkn1usdb"), 6)
    }

    fn lightning_btc() -> BridgeRoutePairSide {
        side("lightning", "BTC", None, 8)
    }

    fn pair(source: BridgeRoutePairSide, destination: BridgeRoutePairSide) -> BridgeRoutePair {
        BridgeRoutePair {
            provider: CrossChainProvider::Orchestra,
            source,
            destination,
            limits: None,
        }
    }

    fn spark_recipient() -> BridgeRecipient {
        BridgeRecipient::Spark(spark_address(BitcoinNetwork::Bitcoin))
    }

    fn lightning_recipient() -> BridgeRecipient {
        BridgeRecipient::Lightning(lightning_address(1_000, 1_000_000))
    }

    fn base_recipient() -> BridgeRecipient {
        BridgeRecipient::CrossChain(base_address())
    }

    #[test_all]
    fn route_pays_each_kind_of_recipient() {
        let (spark, lightning, base) = (spark_recipient(), lightning_recipient(), base_recipient());

        assert!(route_pays(&spark, &pair(base_usdc(), spark_btc())));
        assert!(route_pays(&spark, &pair(tron_usdt(), spark_usdb())));
        assert!(!route_pays(&spark, &pair(base_usdc(), lightning_btc())));
        assert!(!route_pays(&spark, &pair(lightning_btc(), spark_btc())));

        assert!(route_pays(&lightning, &pair(base_usdc(), lightning_btc())));
        assert!(!route_pays(&lightning, &pair(base_usdc(), spark_btc())));

        assert!(route_pays(&base, &pair(lightning_btc(), base_usdc())));
        assert!(route_pays(&base, &pair(tron_usdt(), base_usdc())));
        assert!(route_pays(&base, &pair(base_usdt(), base_usdc())));
        // A payer on Spark sends from their wallet instead.
        assert!(!route_pays(&base, &pair(spark_btc(), base_usdc())));
        assert!(!route_pays(&base, &pair(tron_usdt(), solana_usdc())));
    }

    #[test_all]
    fn route_pays_only_in_usd_stablecoins_and_bitcoin() {
        let (spark, base) = (spark_recipient(), base_recipient());
        assert!(!route_pays(&spark, &pair(base_weth(), spark_btc())));
        assert!(!route_pays(&base, &pair(tron_usdt(), base_weth())));
        assert!(!route_pays(
            &spark,
            &pair(side("bitcoin", "BTC", None, 8), spark_btc())
        ));
        // A deposit whose chain can't be told from its token takes no refund
        // address that could be checked.
        assert!(!route_pays(
            &spark,
            &pair(side("ton", "USDT", None, 6), spark_btc())
        ));
    }

    #[test_all]
    fn route_pays_the_token_and_chain_the_address_names() {
        let usdc = BridgeRecipient::CrossChain(CrossChainAddressDetails {
            contract_address: Some(BASE_USDC.to_lowercase()),
            ..base_address()
        });
        assert!(route_pays(&usdc, &pair(tron_usdt(), base_usdc())));
        assert!(!route_pays(&usdc, &pair(tron_usdt(), base_usdt())));

        let on_base = BridgeRecipient::CrossChain(CrossChainAddressDetails {
            chain_id: Some(8453),
            ..base_address()
        });
        assert!(route_pays(&on_base, &pair(tron_usdt(), base_usdc())));
        let on_arbitrum = BridgeRecipient::CrossChain(CrossChainAddressDetails {
            chain_id: Some(42161),
            ..base_address()
        });
        assert!(!route_pays(&on_arbitrum, &pair(tron_usdt(), base_usdc())));
    }

    #[test_all]
    fn is_listed_matches_both_sides_exactly() {
        let listed = vec![pair(tron_usdt(), base_usdc())];
        assert!(is_listed(&listed, &pair(tron_usdt(), base_usdc())));
        let wrong_chain = BridgeRoutePairSide {
            chain_id: Some("1".to_string()),
            ..base_usdc()
        };
        assert!(!is_listed(&listed, &pair(tron_usdt(), wrong_chain)));
        let wrong_token = BridgeRoutePairSide {
            contract_address: Some("TOther".to_string()),
            ..tron_usdt()
        };
        assert!(!is_listed(&listed, &pair(wrong_token, base_usdc())));
    }

    #[test_all]
    fn source_units_rescales_a_stablecoin_destination() {
        let bsc_usdt = side(
            "bsc",
            "USDT",
            Some("0x55d398326f99059fF775485246999027B3197955"),
            18,
        );
        assert_eq!(
            source_units(&pair(bsc_usdt.clone(), base_usdc()), 10_000_000).unwrap(),
            10_000_000_000_000_000_000
        );
        assert_eq!(
            source_units(&pair(bsc_usdt, lightning_btc()), 10_000_000_000_000_000_000).unwrap(),
            10_000_000_000_000_000_000
        );
    }

    #[test_all]
    fn external_pair_moves_the_local_asset_within_the_route_limits() {
        let usdb = SparkAsset::Token {
            token_identifier: "btkn1usdb".to_string(),
        };
        let route = BridgeRoutePair {
            limits: Some(CrossChainRouteLimits {
                min_amount: None,
                max_amount: None,
                min_usd_cents: Some(200),
                max_usd_cents: None,
            }),
            ..pair(tron_usdt(), spark_usdb())
        };
        let external = external_pair(
            &route,
            &route.source,
            Some((usdb.clone(), DeliveryMethod::Spark)),
        );
        assert_eq!(external.chain, "tron");
        assert_eq!(
            external.contract_address.as_deref(),
            Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")
        );
        assert_eq!(
            external.limits_for(&usdb).and_then(|l| l.min_usd_cents),
            Some(200)
        );
        assert_eq!(external.delivery_methods, vec![DeliveryMethod::Spark]);

        let bare = external_pair(&route, &route.source, None);
        assert!(bare.accepted_assets.is_empty() && bare.delivery_methods.is_empty());
    }

    #[async_test_all]
    async fn create_bridge_rejects_a_route_that_does_not_pay_the_recipient() {
        let client = offline_client();
        for (recipient, route) in [
            (spark_recipient(), pair(base_usdc(), lightning_btc())),
            (lightning_recipient(), pair(base_usdc(), spark_btc())),
            (base_recipient(), pair(spark_btc(), base_usdc())),
            (base_recipient(), pair(tron_usdt(), base_weth())),
        ] {
            assert_invalid_input(
                client
                    .create_bridge_for(
                        &recipient,
                        create_request("", route),
                        LightningInvoice::Quote,
                    )
                    .await,
                "doesn't pay this recipient",
            );
        }
        // An address on another chain parses without a lookup.
        assert_invalid_input(
            client
                .create_bridge(create_request(
                    "0x0000000000000000000000000000000000000001",
                    pair(spark_btc(), base_usdc()),
                ))
                .await,
            "doesn't pay this recipient",
        );
    }

    #[async_test_all]
    async fn create_bridge_requires_a_refund_address_to_pay_a_lightning_address() {
        let result = offline_client()
            .create_bridge_for(
                &lightning_recipient(),
                create_request("", pair(base_usdc(), lightning_btc())),
                LightningInvoice::Quote,
            )
            .await;
        assert_invalid_input(result, "refund address is required");
    }

    #[async_test_all]
    async fn a_refund_address_is_optional_otherwise() {
        let client = offline_client();
        let route = external_pair(&pair(tron_usdt(), base_usdc()), &tron_usdt(), None);
        assert_eq!(
            client.validate_refund_address(None, &route).await.unwrap(),
            None
        );
    }

    #[async_test_all]
    async fn create_bridge_rejects_a_non_mainnet_spark_recipient() {
        let result = offline_client()
            .create_bridge_for(
                &BridgeRecipient::Spark(spark_address(BitcoinNetwork::Regtest)),
                create_request("", pair(base_usdc(), spark_btc())),
                LightningInvoice::Quote,
            )
            .await;
        assert_invalid_input(result, "not a mainnet address");
    }

    #[async_test_all]
    async fn create_bridge_rejects_a_zero_amount_and_other_providers_before_any_lookup() {
        // The offline client fails the test on a lookup of the address.
        let client = offline_client();
        let route = pair(base_usdc(), lightning_btc());

        let mut request = create_request("alice@example.com", route.clone());
        request.amount = 0;
        assert_invalid_input(client.create_bridge(request).await, "greater than 0");

        let mut route = route;
        route.provider = CrossChainProvider::Boltz;
        assert_invalid_input(
            client
                .create_bridge(create_request("alice@example.com", route))
                .await,
            "only supported on Orchestra",
        );
    }

    #[async_test_all]
    async fn cash_app_bridges_take_only_cash_app_routes_and_recipients() {
        let client = offline_client();
        assert_invalid_input(
            client
                .bridge_from_cash_app(create_request(
                    "0x0000000000000000000000000000000000000001",
                    pair(tron_usdt(), base_usdc()),
                ))
                .await,
            "over Lightning",
        );

        for recipient in [
            "alice@example.com",
            "sp1pgss8s9wq7kafdwppyynpr0xzx7v9x8nyx0",
        ] {
            assert_invalid_input(
                client
                    .bridge_to_cash_app(create_request(
                        recipient,
                        pair(base_usdc(), lightning_btc()),
                    ))
                    .await,
                "not a Cash App username",
            );
        }
        // A Cash App user, however written, is bridged as any other recipient.
        for recipient in ["alice", "$alice", "alice@cash.app"] {
            let mut request = create_request(recipient, pair(base_usdc(), lightning_btc()));
            request.amount = 0;
            assert_invalid_input(client.bridge_to_cash_app(request).await, "greater than 0");
        }
    }

    #[async_test_all]
    async fn a_lightning_bridge_rejects_a_refund_address_it_cannot_refund() {
        let client = offline_client();
        assert_eq!(client.lightning_refund_address(None).await.unwrap(), None);
        assert_eq!(
            client.lightning_refund_address(Some("  ")).await.unwrap(),
            None
        );
        assert_eq!(
            client
                .lightning_refund_address(Some(" $alice "))
                .await
                .unwrap()
                .map(|r| r.address)
                .as_deref(),
            Some("alice@cash.app")
        );
        assert_invalid_input(
            client
                .lightning_refund_address(Some("0x0000000000000000000000000000000000000001"))
                .await,
            "Lightning refund address",
        );
    }

    #[test_all]
    fn lightning_funded_response_describes_the_invoice_and_the_stablecoin() {
        let response = lightning_funded_bridge_response(LightningFunded {
            prepared: prepared(orchestra_context()),
            deposit_target: "lnbc100u1pxyz".to_string(),
            amount_sats: 5000,
            order_id: None,
        })
        .unwrap();
        assert_eq!(response.bridge_id, "q1");
        assert_eq!(response.payment_request, "lnbc100u1pxyz");
        assert_eq!(response.info.deposit_address, "lnbc100u1pxyz");
        assert_eq!(response.info.deposit_amount, 5000);
        assert_eq!(response.info.expected_received_amount, 6_450_000);
        assert_eq!(response.info.destination_asset, "USDC");
        assert_eq!(response.info.token_identifier, None);
        assert_eq!(response.info.service_fee_asset.as_deref(), Some("USDC"));
        assert_eq!(response.info.expires_at, 1_784_966_966);
    }

    #[test_all]
    fn an_onramp_bridge_is_identified_by_its_order() {
        let response = lightning_funded_bridge_response(LightningFunded {
            prepared: prepared(orchestra_context()),
            deposit_target: "lnbc100u1pxyz".to_string(),
            amount_sats: 5000,
            order_id: Some("ord_1".to_string()),
        })
        .unwrap();
        assert_eq!(response.bridge_id, "ord_1");
        assert_eq!(response.info.quote_id, "q1");
        assert!(is_order_id(&response.bridge_id));
        assert!(!is_order_id("q1"));
    }
}
