//! Domain-organized sub-object APIs for [`BreezClient`].
//!
//! Each sub-object groups related methods behind a dedicated UniFFI Object so
//! that every target language gets `client.payments().list(...)` style access
//! instead of 25+ flat methods on one type.
//!
//! The precedent is [`TokenIssuer`](crate::issuer::TokenIssuer), which already
//! uses this pattern.

use std::sync::Arc;

use crate::{
    CheckLightningAddressRequest, CheckMessageRequest, CheckMessageResponse,
    ClaimDepositRequest, ClaimDepositResponse, ClaimHtlcPaymentRequest,
    ClaimHtlcPaymentResponse, FetchConversionLimitsRequest, FetchConversionLimitsResponse,
    GetPaymentRequest, GetPaymentResponse, GetTokensMetadataRequest, GetTokensMetadataResponse,
    LightningAddressInfo, ListFiatCurrenciesResponse, ListFiatRatesResponse,
    ListPaymentsRequest, ListPaymentsResponse, ListUnclaimedDepositsRequest,
    ListUnclaimedDepositsResponse, LnurlAuthRequestDetails, LnurlCallbackStatus,
    LnurlWithdrawRequest, LnurlWithdrawResponse, OptimizationProgress,
    RecommendedFees,
    RefundDepositRequest, RefundDepositResponse, RegisterLightningAddressRequest,
    SignMessageRequest, SignMessageResponse, UpdateUserSettingsRequest, UserSettings,
    error::SdkError,
    events::EventListener,
    issuer::TokenIssuer,
};

use super::BreezClient;

// ---------------------------------------------------------------------------
// PaymentsApi
// ---------------------------------------------------------------------------

/// Domain sub-object for payment queries.
///
/// Access via [`BreezClient::payments()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct PaymentsApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl PaymentsApi {
    /// List payment history with optional filters and pagination.
    pub async fn list(
        &self,
        request: ListPaymentsRequest,
    ) -> Result<ListPaymentsResponse, SdkError> {
        self.sdk.list_payments(request).await
    }

    /// Get a single payment by ID.
    pub async fn get(&self, request: GetPaymentRequest) -> Result<GetPaymentResponse, SdkError> {
        self.sdk.get_payment(request).await
    }

    /// Claim an HTLC payment using its preimage.
    pub async fn claim_htlc(
        &self,
        request: ClaimHtlcPaymentRequest,
    ) -> Result<ClaimHtlcPaymentResponse, SdkError> {
        self.sdk.claim_htlc_payment(request).await
    }

}

// ---------------------------------------------------------------------------
// DepositsApi
// ---------------------------------------------------------------------------

/// Domain sub-object for on-chain deposit management.
///
/// Access via [`BreezClient::deposits()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct DepositsApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl DepositsApi {
    /// List unclaimed on-chain deposits.
    pub async fn list_unclaimed(
        &self,
        request: ListUnclaimedDepositsRequest,
    ) -> Result<ListUnclaimedDepositsResponse, SdkError> {
        self.sdk.list_unclaimed_deposits(request).await
    }

    /// Manually claim an on-chain deposit.
    pub async fn claim(
        &self,
        request: ClaimDepositRequest,
    ) -> Result<ClaimDepositResponse, SdkError> {
        self.sdk.claim_deposit(request).await
    }

    /// Refund a static deposit to a Bitcoin address.
    pub async fn refund(
        &self,
        request: RefundDepositRequest,
    ) -> Result<RefundDepositResponse, SdkError> {
        self.sdk.refund_deposit(request).await
    }

    /// Get the recommended BTC on-chain fees (useful for claim/refund decisions).
    pub async fn recommended_fees(&self) -> Result<RecommendedFees, SdkError> {
        self.sdk.recommended_fees().await
    }
}

// ---------------------------------------------------------------------------
// FiatCurrencyApi
// ---------------------------------------------------------------------------

/// Domain sub-object for fiat currency data.
///
/// Access via [`BreezClient::fiat()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct FiatCurrencyApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl FiatCurrencyApi {
    /// List supported fiat currencies.
    pub async fn currencies(&self) -> Result<ListFiatCurrenciesResponse, SdkError> {
        self.sdk.list_fiat_currencies().await
    }

    /// Get latest fiat exchange rates.
    pub async fn rates(&self) -> Result<ListFiatRatesResponse, SdkError> {
        self.sdk.list_fiat_rates().await
    }
}

// ---------------------------------------------------------------------------
// SettingsApi
// ---------------------------------------------------------------------------

/// Domain sub-object for user settings.
///
/// Access via [`BreezClient::user_settings()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SettingsApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl SettingsApi {
    /// Get the current user settings.
    pub async fn get(&self) -> Result<UserSettings, SdkError> {
        self.sdk.get_user_settings().await
    }

    /// Update user settings.
    pub async fn update(&self, request: UpdateUserSettingsRequest) -> Result<(), SdkError> {
        self.sdk.update_user_settings(request).await
    }
}

// ---------------------------------------------------------------------------
// OptimizationApi
// ---------------------------------------------------------------------------

/// Domain sub-object for leaf optimization.
///
/// Access via [`BreezClient::optimization()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct OptimizationApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl OptimizationApi {
    /// Start leaf optimization in the background.
    pub fn start(&self) {
        self.sdk.start_leaf_optimization();
    }

    /// Cancel the ongoing leaf optimization and wait for it to stop.
    pub async fn cancel(&self) -> Result<(), SdkError> {
        self.sdk.cancel_leaf_optimization().await
    }

    /// Get the current optimization progress snapshot.
    pub fn progress(&self) -> OptimizationProgress {
        self.sdk.get_leaf_optimization_progress()
    }
}

// ---------------------------------------------------------------------------
// LightningAddressApi
// ---------------------------------------------------------------------------

/// Domain sub-object for Lightning Address management.
///
/// Access via [`BreezClient::lightning_address()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct LightningAddressApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl LightningAddressApi {
    /// Check if a Lightning Address username is available.
    pub async fn is_available(
        &self,
        request: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        self.sdk.check_lightning_address_available(request).await
    }

    /// Get the currently registered Lightning Address, if any.
    pub async fn get(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        self.sdk.get_lightning_address().await
    }

    /// Register or update a Lightning Address.
    pub async fn register(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        self.sdk.register_lightning_address(request).await
    }

    /// Delete the registered Lightning Address.
    pub async fn delete(&self) -> Result<(), SdkError> {
        self.sdk.delete_lightning_address().await
    }
}

// ---------------------------------------------------------------------------
// LnurlApi
// ---------------------------------------------------------------------------

/// Domain sub-object for LNURL operations.
///
/// Access via [`BreezClient::lnurl()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct LnurlApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl LnurlApi {
    /// Execute an LNURL-withdraw.
    pub async fn withdraw(
        &self,
        request: LnurlWithdrawRequest,
    ) -> Result<LnurlWithdrawResponse, SdkError> {
        self.sdk.lnurl_withdraw(request).await
    }

    /// Perform LNURL-auth (login).
    pub async fn auth(
        &self,
        request_data: LnurlAuthRequestDetails,
    ) -> Result<LnurlCallbackStatus, SdkError> {
        self.sdk.lnurl_auth(request_data).await
    }
}

// ---------------------------------------------------------------------------
// EventsApi
// ---------------------------------------------------------------------------

/// Domain sub-object for event listener management.
///
/// Access via [`BreezClient::events()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct EventsApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl EventsApi {
    /// Register an event listener. Returns a listener ID.
    pub async fn add(&self, listener: Box<dyn EventListener>) -> String {
        self.sdk.add_event_listener(listener).await
    }

    /// Remove a previously registered event listener. Returns whether it was found.
    pub async fn remove(&self, id: &str) -> bool {
        self.sdk.remove_event_listener(id).await
    }
}

// ---------------------------------------------------------------------------
// TokensApi
// ---------------------------------------------------------------------------

/// Domain sub-object for token queries.
///
/// Access via [`BreezClient::tokens()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct TokensApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl TokensApi {
    /// Get metadata for the given token identifiers.
    pub async fn metadata(
        &self,
        request: GetTokensMetadataRequest,
    ) -> Result<GetTokensMetadataResponse, SdkError> {
        self.sdk.get_tokens_metadata(request).await
    }

    /// Get the token issuer instance for managing token issuance.
    pub fn issuer(&self) -> TokenIssuer {
        self.sdk.get_token_issuer()
    }

    /// Fetch the conversion limits between Bitcoin and a token.
    pub async fn fetch_conversion_limits(
        &self,
        request: FetchConversionLimitsRequest,
    ) -> Result<FetchConversionLimitsResponse, SdkError> {
        self.sdk.fetch_conversion_limits(request).await
    }
}

// ---------------------------------------------------------------------------
// MessageApi
// ---------------------------------------------------------------------------

/// Domain sub-object for message signing and verification.
///
/// Access via [`BreezClient::message()`].
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct MessageApi {
    sdk: Arc<BreezClient>,
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl MessageApi {
    /// Sign a message with the wallet's identity key.
    pub async fn sign(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResponse, SdkError> {
        self.sdk.sign_message(request).await
    }

    /// Verify a message signature against a public key.
    pub async fn check(
        &self,
        request: CheckMessageRequest,
    ) -> Result<CheckMessageResponse, SdkError> {
        self.sdk.check_message(request).await
    }
}

// ---------------------------------------------------------------------------
// Accessor methods on BreezClient
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl BreezClient {
    /// Access payment query operations.
    pub fn payments(&self) -> Arc<PaymentsApi> {
        Arc::new(PaymentsApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access on-chain deposit operations.
    pub fn deposits(&self) -> Arc<DepositsApi> {
        Arc::new(DepositsApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access fiat currency data.
    pub fn fiat(&self) -> Arc<FiatCurrencyApi> {
        Arc::new(FiatCurrencyApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access user settings.
    pub fn user_settings(&self) -> Arc<SettingsApi> {
        Arc::new(SettingsApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access leaf optimization controls.
    pub fn optimization(&self) -> Arc<OptimizationApi> {
        Arc::new(OptimizationApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access Lightning Address management.
    pub fn lightning_address(&self) -> Arc<LightningAddressApi> {
        Arc::new(LightningAddressApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access LNURL operations (withdraw, auth).
    pub fn lnurl(&self) -> Arc<LnurlApi> {
        Arc::new(LnurlApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access event listener management.
    pub fn events(&self) -> Arc<EventsApi> {
        Arc::new(EventsApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access token queries and issuer.
    pub fn tokens(&self) -> Arc<TokensApi> {
        Arc::new(TokensApi {
            sdk: Arc::new(self.clone()),
        })
    }

    /// Access message signing and verification.
    pub fn message(&self) -> Arc<MessageApi> {
        Arc::new(MessageApi {
            sdk: Arc::new(self.clone()),
        })
    }
}
