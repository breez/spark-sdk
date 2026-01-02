use std::sync::Arc;

use super::BuyBitcoinProviderApi;
use crate::{breez_server::BreezServer, grpc::SignUrlRequest};
use anyhow::Result;
use url::Url;

#[derive(Clone)]
struct MoonPayConfig {
    pub base_url: String,
    pub api_key: String,
    pub currency_code: String,
    pub color_code: String,
    pub theme: String,
    pub lock_amount: String,
    pub redirect_url: String,
}

fn moonpay_config() -> MoonPayConfig {
    MoonPayConfig {
        base_url: String::from("https://buy.moonpay.io"),
        api_key: String::from("pk_live_Mx5g6bpD6Etd7T0bupthv7smoTNn2Vr"),
        currency_code: String::from("btc"),
        color_code: String::from("#055DEB"),
        theme: String::from("light"),
        lock_amount: String::from("true"),
        redirect_url: String::from("https://buy.moonpay.io/transaction_receipt?addFunds=true"),
    }
}

fn create_moonpay_url(
    wallet_address: String,
    quote_currency_amount: Option<String>,
    redirect_url: Option<String>,
) -> Result<Url> {
    let config = moonpay_config();

    // Build query params in the order defined by MoonPay's docs:
    // https://dev.moonpay.com/docs/ramps-sdk-buy-params
    let mut params = vec![
        ("apiKey", config.api_key),
        ("currencyCode", config.currency_code),
        ("walletAddress", wallet_address),
        ("colorCode", config.color_code),
        ("theme", config.theme),
    ];

    // Only lock the amount when a specific amount is requested
    if let Some(quote_currency_amount) = quote_currency_amount {
        params.extend(vec![
            ("quoteCurrencyAmount", quote_currency_amount),
            ("lockAmount", config.lock_amount),
        ]);
    }

    // redirectURL comes after the conditional lockAmount params
    params.push(("redirectURL", redirect_url.unwrap_or(config.redirect_url)));

    let url = Url::parse_with_params(&config.base_url, params)?;
    Ok(url)
}

pub struct MoonpayProvider {
    breez_server: Arc<BreezServer>,
}

impl MoonpayProvider {
    pub fn new(breez_server: Arc<BreezServer>) -> Self {
        Self { breez_server }
    }
}

#[macros::async_trait]
impl BuyBitcoinProviderApi for MoonpayProvider {
    async fn buy_bitcoin(
        &self,
        address: String,
        locked_amount_sat: Option<u64>,
        redirect_url: Option<String>,
    ) -> Result<String> {
        let config = moonpay_config();
        #[allow(clippy::cast_precision_loss)]
        let url = create_moonpay_url(
            address,
            locked_amount_sat.map(|amount| format!("{:.8}", amount as f64 / 100_000_000.0)),
            redirect_url,
        )?;
        let mut signer = self.breez_server.get_signer_client().await;
        let signed_url = signer
            .sign_url(SignUrlRequest {
                base_url: config.base_url.clone(),
                query_string: format!("?{}", url.query().unwrap()),
            })
            .await?
            .into_inner()
            .full_url;
        Ok(signed_url)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use macros::async_test_all;
    use std::collections::HashMap;

    use crate::buy::moonpay::{create_moonpay_url, moonpay_config};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[async_test_all]
    async fn test_sign_moonpay_url() -> Result<(), Box<dyn std::error::Error>> {
        let wallet_address = "a wallet address".to_string();
        let quote_amount = "a quote amount".to_string();
        let config = moonpay_config();

        let url = create_moonpay_url(wallet_address.clone(), Some(quote_amount.clone()), None)?;

        let query_pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(url.host_str(), Some("buy.moonpay.io"));
        assert_eq!(url.path(), "/");
        assert_eq!(query_pairs.get("apiKey"), Some(&config.api_key));
        assert_eq!(query_pairs.get("currencyCode"), Some(&config.currency_code));
        assert_eq!(query_pairs.get("colorCode"), Some(&config.color_code));
        assert_eq!(query_pairs.get("theme"), Some(&config.theme));
        assert_eq!(query_pairs.get("redirectURL"), Some(&config.redirect_url));
        assert_eq!(query_pairs.get("lockAmount"), Some(&config.lock_amount));
        assert_eq!(query_pairs.get("walletAddress"), Some(&wallet_address));
        assert_eq!(query_pairs.get("quoteCurrencyAmount"), Some(&quote_amount),);
        Ok(())
    }

    #[async_test_all]
    async fn test_sign_moonpay_url_with_redirect() -> Result<(), Box<dyn std::error::Error>> {
        let wallet_address = "a wallet address".to_string();
        let quote_amount = "a quote amount".to_string();
        let redirect_url = "https://test.moonpay.url/receipt".to_string();
        let config = moonpay_config();

        let url = create_moonpay_url(
            wallet_address.clone(),
            Some(quote_amount.clone()),
            Some(redirect_url.clone()),
        )?;

        let query_pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(url.host_str(), Some("buy.moonpay.io"));
        assert_eq!(url.path(), "/");
        assert_eq!(query_pairs.get("apiKey"), Some(&config.api_key));
        assert_eq!(query_pairs.get("currencyCode"), Some(&config.currency_code));
        assert_eq!(query_pairs.get("colorCode"), Some(&config.color_code));
        assert_eq!(query_pairs.get("theme"), Some(&config.theme));
        assert_eq!(query_pairs.get("redirectURL"), Some(&redirect_url));
        assert_eq!(query_pairs.get("lockAmount"), Some(&config.lock_amount));
        assert_eq!(query_pairs.get("walletAddress"), Some(&wallet_address));
        assert_eq!(query_pairs.get("quoteCurrencyAmount"), Some(&quote_amount),);
        Ok(())
    }
}
