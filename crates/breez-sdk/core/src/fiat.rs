use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ServiceConnectivityError;

/// Trait covering fiat-related functionality
#[cfg_attr(feature = "uniffi", uniffi::export(with_foreign))]
#[macros::async_trait]
pub trait FiatService: Send + Sync {
    /// List all supported fiat currencies for which there is a known exchange rate.
    async fn fetch_fiat_currencies(&self) -> Result<Vec<FiatCurrency>, ServiceConnectivityError>;

    /// Get the live rates from the server.
    async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError>;
}

pub(crate) struct FiatServiceWrapper {
    inner: Arc<dyn FiatService>,
}

impl FiatServiceWrapper {
    pub fn new(inner: Arc<dyn FiatService>) -> Self {
        FiatServiceWrapper { inner }
    }
}

#[macros::async_trait]
impl breez_sdk_common::fiat::FiatService for FiatServiceWrapper {
    async fn fetch_fiat_currencies(
        &self,
    ) -> Result<
        Vec<breez_sdk_common::fiat::FiatCurrency>,
        breez_sdk_common::error::ServiceConnectivityError,
    > {
        Ok(self
            .inner
            .fetch_fiat_currencies()
            .await?
            .into_iter()
            .map(From::from)
            .collect())
    }

    async fn fetch_fiat_rates(
        &self,
    ) -> Result<Vec<breez_sdk_common::fiat::Rate>, breez_sdk_common::error::ServiceConnectivityError>
    {
        Ok(self
            .inner
            .fetch_fiat_rates()
            .await?
            .into_iter()
            .map(From::from)
            .collect())
    }
}

/// Wrapper around the [`CurrencyInfo`] of a fiat currency
#[derive(Clone, Debug, Serialize, Deserialize)]
#[macros::derive_from(breez_sdk_common::fiat::FiatCurrency)]
#[macros::derive_into(breez_sdk_common::fiat::FiatCurrency)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FiatCurrency {
    pub id: String,
    pub info: CurrencyInfo,
}

/// Details about a supported currency in the fiat rate feed
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::fiat::CurrencyInfo)]
#[macros::derive_into(breez_sdk_common::fiat::CurrencyInfo)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CurrencyInfo {
    pub name: String,
    pub fraction_size: u32,
    pub spacing: Option<u32>,
    pub symbol: Option<Symbol>,
    pub uniq_symbol: Option<Symbol>,
    #[serde(default)]
    pub localized_name: Vec<LocalizedName>,
    #[serde(default)]
    pub locale_overrides: Vec<LocaleOverrides>,
}

/// Localized name of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::fiat::LocalizedName)]
#[macros::derive_into(breez_sdk_common::fiat::LocalizedName)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocalizedName {
    pub locale: String,
    pub name: String,
}

/// Locale-specific settings for the representation of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::fiat::LocaleOverrides)]
#[macros::derive_into(breez_sdk_common::fiat::LocaleOverrides)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocaleOverrides {
    pub locale: String,
    pub spacing: Option<u32>,
    pub symbol: Symbol,
}

/// Denominator in an exchange rate
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::fiat::Rate)]
#[macros::derive_into(breez_sdk_common::fiat::Rate)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Rate {
    pub coin: String,
    pub value: f64,
}

/// Settings for the symbol representation of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[macros::derive_from(breez_sdk_common::fiat::Symbol)]
#[macros::derive_into(breez_sdk_common::fiat::Symbol)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Symbol {
    pub grapheme: Option<String>,
    pub template: Option<String>,
    pub rtl: Option<bool>,
    pub position: Option<u32>,
}
