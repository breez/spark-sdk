use std::collections::HashMap;

use maybe_sync::{MaybeSend, MaybeSync};
use serde::{Deserialize, Serialize};

use crate::{
    breez_server::BreezServer,
    error::{ServiceConnectivityError, ServiceConnectivityErrorKind},
    grpc::RatesRequest,
    with_connection_retry,
};

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

/// Trait covering fiat-related functionality
#[breez_sdk_macros::async_trait]
pub trait FiatAPI: MaybeSend + MaybeSync {
    /// List all supported fiat currencies for which there is a known exchange rate.
    async fn fetch_fiat_currencies(&self) -> Result<Vec<FiatCurrency>, ServiceConnectivityError>;

    /// Get the live rates from the server.
    async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError>;
}

fn convert_to_fiat_currency_with_id(id: String, info: CurrencyInfo) -> FiatCurrency {
    FiatCurrency { id, info }
}

#[breez_sdk_macros::async_trait]
impl FiatAPI for BreezServer {
    async fn fetch_fiat_currencies(&self) -> Result<Vec<FiatCurrency>, ServiceConnectivityError> {
        let known_rates = self.fetch_fiat_rates().await?;
        let known_rates_currencies = known_rates
            .iter()
            .map(|r| r.coin.clone())
            .collect::<Vec<String>>();

        let data = include_str!("../assets/json/currencies.json");
        let fiat_currency_map: HashMap<String, CurrencyInfo> =
            serde_json::from_str(data).map_err(|e| {
                ServiceConnectivityError::new(
                    ServiceConnectivityErrorKind::Json,
                    format!("failed to load embedded fiat currencies: {:?}", e),
                )
            })?;
        let mut fiat_currency_list: Vec<FiatCurrency> = Vec::new();
        for (key, value) in fiat_currency_map {
            if known_rates_currencies.contains(&key) {
                fiat_currency_list.push(convert_to_fiat_currency_with_id(key, value));
            }
        }
        fiat_currency_list.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        Ok(fiat_currency_list)
    }

    async fn fetch_fiat_rates(&self) -> Result<Vec<Rate>, ServiceConnectivityError> {
        let mut client = self.get_information_client().await;

        let request = RatesRequest {};
        let response = with_connection_retry!(client.rates(request.clone()))
            .await
            .map_err(|e| {
                ServiceConnectivityError::new(
                    ServiceConnectivityErrorKind::Other,
                    format!("(Breez: {e:?}) Failed to fetch fiat rates"),
                )
            })?;

        let mut rates = response.into_inner().rates;
        rates.sort_by(|a, b| a.coin.cmp(&b.coin));
        Ok(rates
            .into_iter()
            .map(|r| Rate {
                coin: r.coin,
                value: r.value,
            })
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

/// Localized name of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocalizedName {
    pub locale: String,
    pub name: String,
}

/// Locale-specific settings for the representation of a currency
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocaleOverrides {
    pub locale: String,
    pub spacing: Option<u32>,
    pub symbol: Symbol,
}

/// Denominator in an exchange rate
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Rate {
    pub coin: String,
    pub value: f64,
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
