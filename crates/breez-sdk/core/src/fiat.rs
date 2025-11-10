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
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FiatCurrency {
    pub id: String,
    pub info: CurrencyInfo,
}

impl From<breez_sdk_common::fiat::FiatCurrency> for FiatCurrency {
    fn from(value: breez_sdk_common::fiat::FiatCurrency) -> Self {
        FiatCurrency {
            id: value.id,
            info: value.info.into(),
        }
    }
}

impl From<FiatCurrency> for breez_sdk_common::fiat::FiatCurrency {
    fn from(value: FiatCurrency) -> Self {
        breez_sdk_common::fiat::FiatCurrency {
            id: value.id,
            info: value.info.into(),
        }
    }
}

/// Details about a supported currency in the fiat rate feed
#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl From<breez_sdk_common::fiat::CurrencyInfo> for CurrencyInfo {
    fn from(value: breez_sdk_common::fiat::CurrencyInfo) -> Self {
        CurrencyInfo {
            name: value.name,
            fraction_size: value.fraction_size,
            spacing: value.spacing,
            symbol: value.symbol.map(From::from),
            uniq_symbol: value.uniq_symbol.map(From::from),
            localized_name: value.localized_name.into_iter().map(From::from).collect(),
            locale_overrides: value.locale_overrides.into_iter().map(From::from).collect(),
        }
    }
}

impl From<CurrencyInfo> for breez_sdk_common::fiat::CurrencyInfo {
    fn from(value: CurrencyInfo) -> Self {
        breez_sdk_common::fiat::CurrencyInfo {
            name: value.name,
            fraction_size: value.fraction_size,
            spacing: value.spacing,
            symbol: value.symbol.map(From::from),
            uniq_symbol: value.uniq_symbol.map(From::from),
            localized_name: value.localized_name.into_iter().map(From::from).collect(),
            locale_overrides: value.locale_overrides.into_iter().map(From::from).collect(),
        }
    }
}

/// Localized name of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocalizedName {
    pub locale: String,
    pub name: String,
}

impl From<breez_sdk_common::fiat::LocalizedName> for LocalizedName {
    fn from(value: breez_sdk_common::fiat::LocalizedName) -> Self {
        LocalizedName {
            locale: value.locale,
            name: value.name,
        }
    }
}

impl From<LocalizedName> for breez_sdk_common::fiat::LocalizedName {
    fn from(value: LocalizedName) -> Self {
        breez_sdk_common::fiat::LocalizedName {
            locale: value.locale,
            name: value.name,
        }
    }
}

/// Locale-specific settings for the representation of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocaleOverrides {
    pub locale: String,
    pub spacing: Option<u32>,
    pub symbol: Symbol,
}

impl From<breez_sdk_common::fiat::LocaleOverrides> for LocaleOverrides {
    fn from(value: breez_sdk_common::fiat::LocaleOverrides) -> Self {
        LocaleOverrides {
            locale: value.locale,
            spacing: value.spacing,
            symbol: value.symbol.into(),
        }
    }
}

impl From<LocaleOverrides> for breez_sdk_common::fiat::LocaleOverrides {
    fn from(value: LocaleOverrides) -> Self {
        breez_sdk_common::fiat::LocaleOverrides {
            locale: value.locale,
            spacing: value.spacing,
            symbol: value.symbol.into(),
        }
    }
}

/// Denominator in an exchange rate
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Rate {
    pub coin: String,
    pub value: f64,
}

impl From<breez_sdk_common::fiat::Rate> for Rate {
    fn from(value: breez_sdk_common::fiat::Rate) -> Self {
        Rate {
            coin: value.coin,
            value: value.value,
        }
    }
}

impl From<Rate> for breez_sdk_common::fiat::Rate {
    fn from(value: Rate) -> Self {
        breez_sdk_common::fiat::Rate {
            coin: value.coin,
            value: value.value,
        }
    }
}

/// Settings for the symbol representation of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Symbol {
    pub grapheme: Option<String>,
    pub template: Option<String>,
    pub rtl: Option<bool>,
    pub position: Option<u32>,
}

impl From<breez_sdk_common::fiat::Symbol> for Symbol {
    fn from(value: breez_sdk_common::fiat::Symbol) -> Self {
        Symbol {
            grapheme: value.grapheme,
            template: value.template,
            rtl: value.rtl,
            position: value.position,
        }
    }
}

impl From<Symbol> for breez_sdk_common::fiat::Symbol {
    fn from(value: Symbol) -> Self {
        breez_sdk_common::fiat::Symbol {
            grapheme: value.grapheme,
            template: value.template,
            rtl: value.rtl,
            position: value.position,
        }
    }
}
