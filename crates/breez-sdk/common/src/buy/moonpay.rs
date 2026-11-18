use std::{
    net::{Ipv4Addr, Ipv6Addr},
    str::FromStr,
    sync::Arc,
};

use crate::{breez_server::BreezServer, grpc::SignUrlRequest};
use anyhow::Result;
use platform_utils::{HttpClient, time::Duration, tokio::time::timeout};
use tracing::{debug, warn};
use url::Url;

#[derive(Clone)]
struct MoonPayConfig {
    pub base_url: String,
    pub api_key: String,
    pub color_code: String,
    pub theme: String,
    pub redirect_url: String,
}

fn moonpay_config() -> MoonPayConfig {
    MoonPayConfig {
        base_url: String::from("https://buy.moonpay.io"),
        api_key: String::from("pk_live_Mx5g6bpD6Etd7T0bupthv7smoTNn2Vr"),
        color_code: String::from("#055DEB"),
        theme: String::from("light"),
        redirect_url: String::from("https://buy.moonpay.io/transaction_receipt?addFunds=true"),
    }
}

const CURRENCY_BTC: &str = "btc";
const CURRENCY_USDC_SOLANA: &str = "usdc_sol";

/// Decimal places `MoonPay` quotes `usdc_sol` amounts in.
pub const USDC_SOLANA_PRECISION: u32 = 2;

const IPV4_LOOKUP_URL: &str = "https://wimi4.breez.technology";
const IPV6_LOOKUP_URL: &str = "https://wimi6.breez.technology";
const IP_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

fn create_moonpay_url(
    currency_code: &str,
    wallet_address: &str,
    quote_currency_amount: Option<&String>,
    redirect_url: Option<&String>,
) -> Result<Url> {
    let config = moonpay_config();

    let mut url = Url::parse(&config.base_url)?;

    let redirect_url = redirect_url.unwrap_or(&config.redirect_url);

    // Build query params in the order defined by MoonPay's docs:
    // https://dev.moonpay.com/docs/ramps-sdk-buy-params
    let mut params: Vec<(&str, &str)> = vec![
        ("apiKey", &config.api_key),
        ("currencyCode", currency_code),
        ("walletAddress", wallet_address),
        ("colorCode", &config.color_code),
        ("theme", &config.theme),
    ];

    // A prefill the user can change: `MoonPay`'s `lockAmount` only applies to
    // a fiat `baseCurrencyAmount`.
    if let Some(quote_currency_amount) = quote_currency_amount {
        params.push(("quoteCurrencyAmount", quote_currency_amount));
    }

    params.push(("redirectURL", redirect_url));

    url.query_pairs_mut().extend_pairs(params);
    Ok(url)
}

pub struct MoonpayProvider {
    breez_server: Arc<BreezServer>,
    http_client: Arc<dyn HttpClient>,
    identity_pubkey: String,
}

async fn lookup_ip<T: FromStr>(http_client: &dyn HttpClient, url: &str) -> Option<T> {
    let response = match timeout(IP_LOOKUP_TIMEOUT, http_client.get(url.to_string(), None)).await {
        Ok(Ok(response)) if response.is_success() => response,
        Ok(Ok(response)) => {
            debug!("IP lookup at {url} returned status {}", response.status);
            return None;
        }
        Ok(Err(e)) => {
            debug!("IP lookup at {url} failed: {e}");
            return None;
        }
        Err(_) => {
            debug!("IP lookup at {url} timed out");
            return None;
        }
    };
    let ip = response.body.trim().parse().ok();
    if ip.is_none() {
        debug!("IP lookup at {url} returned an invalid address");
    }
    ip
}

/// Formats `amount` (base units at `decimals`) as a decimal string with
/// `precision` fractional digits, truncating any finer digits.
fn format_decimal_amount(amount: u128, decimals: u32, precision: u32) -> String {
    let scaled = match decimals.checked_sub(precision) {
        Some(shift) => amount
            .checked_div(10u128.saturating_pow(shift))
            .unwrap_or_default(),
        None => amount.saturating_mul(10u128.saturating_pow(precision.saturating_sub(decimals))),
    };
    if precision == 0 {
        return scaled.to_string();
    }
    let unit = 10u128.saturating_pow(precision);
    format!(
        "{}.{:0width$}",
        scaled.checked_div(unit).unwrap_or_default(),
        scaled.checked_rem(unit).unwrap_or_default(),
        width = precision as usize
    )
}

impl MoonpayProvider {
    pub fn new(
        breez_server: Arc<BreezServer>,
        http_client: Arc<dyn HttpClient>,
        identity_pubkey: String,
    ) -> Self {
        Self {
            breez_server,
            http_client,
            identity_pubkey,
        }
    }

    pub async fn buy_bitcoin(
        &self,
        address: String,
        amount_sat: Option<u64>,
        redirect_url: Option<String>,
    ) -> Result<String> {
        #[allow(clippy::cast_precision_loss)]
        let url = create_moonpay_url(
            CURRENCY_BTC,
            &address,
            amount_sat
                .map(|amount| format!("{:.8}", amount as f64 / 100_000_000.0))
                .as_ref(),
            redirect_url.as_ref(),
        )?;
        self.sign(&url).await
    }

    /// Builds a signed URL buying `amount` USDC on Solana, in base units at
    /// `decimals`, delivered to `address`. `amount` must be whole cents
    /// (see [`USDC_SOLANA_PRECISION`]).
    pub async fn buy_usdc_solana(
        &self,
        address: String,
        amount: u128,
        decimals: u32,
        redirect_url: Option<String>,
    ) -> Result<String> {
        let url = create_moonpay_url(
            CURRENCY_USDC_SOLANA,
            &address,
            Some(&format_decimal_amount(
                amount,
                decimals,
                USDC_SOLANA_PRECISION,
            )),
            redirect_url.as_ref(),
        )?;
        self.sign(&url).await
    }

    async fn sign(&self, url: &Url) -> Result<String> {
        let config = moonpay_config();
        let (ipv4, ipv6) = futures::future::join(
            lookup_ip::<Ipv4Addr>(self.http_client.as_ref(), IPV4_LOOKUP_URL),
            lookup_ip::<Ipv6Addr>(self.http_client.as_ref(), IPV6_LOOKUP_URL),
        )
        .await;
        if ipv4.is_none() && ipv6.is_none() {
            warn!("Signing MoonPay URL without a client IP address");
        }
        let mut signer = self.breez_server.get_signer_client().await?;
        let signed_url = signer
            .sign_url(SignUrlRequest {
                base_url: config.base_url.clone(),
                query_string: format!("?{}", url.query().unwrap()),
                ipv4: ipv4.map(|ip| ip.to_string()).unwrap_or_default(),
                ipv6: ipv6.map(|ip| ip.to_string()).unwrap_or_default(),
                pubkey: self.identity_pubkey.clone(),
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
    use std::{
        collections::HashMap,
        net::{Ipv4Addr, Ipv6Addr},
    };

    use crate::{
        buy::moonpay::{
            CURRENCY_BTC, CURRENCY_USDC_SOLANA, create_moonpay_url, format_decimal_amount,
            lookup_ip, moonpay_config,
        },
        test_utils::mock_rest_client::{MockResponse, MockRestClient},
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[async_test_all]
    async fn test_sign_moonpay_url() -> Result<(), Box<dyn std::error::Error>> {
        let wallet_address = "a wallet address".to_string();
        let quote_amount = "a quote amount".to_string();
        let config = moonpay_config();

        let url = create_moonpay_url(CURRENCY_BTC, &wallet_address, Some(&quote_amount), None)?;

        let query_pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(url.host_str(), Some("buy.moonpay.io"));
        assert_eq!(url.path(), "/");
        assert_eq!(query_pairs.get("apiKey"), Some(&config.api_key));
        assert_eq!(
            query_pairs.get("currencyCode").map(String::as_str),
            Some(CURRENCY_BTC)
        );
        assert_eq!(query_pairs.get("colorCode"), Some(&config.color_code));
        assert_eq!(query_pairs.get("theme"), Some(&config.theme));
        assert_eq!(query_pairs.get("redirectURL"), Some(&config.redirect_url));
        assert!(!query_pairs.contains_key("lockAmount"));
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
            CURRENCY_BTC,
            &wallet_address,
            Some(&quote_amount),
            Some(&redirect_url),
        )?;

        let query_pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(url.host_str(), Some("buy.moonpay.io"));
        assert_eq!(url.path(), "/");
        assert_eq!(query_pairs.get("apiKey"), Some(&config.api_key));
        assert_eq!(
            query_pairs.get("currencyCode").map(String::as_str),
            Some(CURRENCY_BTC)
        );
        assert_eq!(query_pairs.get("colorCode"), Some(&config.color_code));
        assert_eq!(query_pairs.get("theme"), Some(&config.theme));
        assert_eq!(query_pairs.get("redirectURL"), Some(&redirect_url));
        assert!(!query_pairs.contains_key("lockAmount"));
        assert_eq!(query_pairs.get("walletAddress"), Some(&wallet_address));
        assert_eq!(query_pairs.get("quoteCurrencyAmount"), Some(&quote_amount),);
        Ok(())
    }

    #[async_test_all]
    async fn test_sign_moonpay_url_usdc_solana() -> Result<(), Box<dyn std::error::Error>> {
        let wallet_address = "a solana address".to_string();
        let quote_amount = "12.34".to_string();

        let url = create_moonpay_url(
            CURRENCY_USDC_SOLANA,
            &wallet_address,
            Some(&quote_amount),
            None,
        )?;

        let query_pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
        assert_eq!(
            query_pairs.get("currencyCode").map(String::as_str),
            Some(CURRENCY_USDC_SOLANA)
        );
        assert_eq!(query_pairs.get("walletAddress"), Some(&wallet_address));
        assert_eq!(query_pairs.get("quoteCurrencyAmount"), Some(&quote_amount));
        Ok(())
    }

    async fn lookup_with<T: std::str::FromStr>(response: Option<MockResponse>) -> Option<T> {
        let client = MockRestClient::new();
        if let Some(response) = response {
            client.add_response(response);
        }
        lookup_ip(&client, "https://ip.lookup").await
    }

    #[async_test_all]
    async fn test_lookup_ip() {
        assert_eq!(
            lookup_with::<Ipv4Addr>(Some(MockResponse::new(200, "203.0.113.7\n".to_string())))
                .await,
            Some(Ipv4Addr::new(203, 0, 113, 7))
        );
        assert_eq!(
            lookup_with::<Ipv6Addr>(Some(MockResponse::new(200, "2001:db8::1".to_string()))).await,
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[async_test_all]
    async fn test_lookup_ip_unavailable() {
        assert_eq!(lookup_with::<Ipv4Addr>(None).await, None);
        assert_eq!(
            lookup_with::<Ipv4Addr>(Some(MockResponse::new(503, "203.0.113.7".to_string()))).await,
            None
        );
        assert_eq!(
            lookup_with::<Ipv4Addr>(Some(MockResponse::new(200, "<html></html>".to_string())))
                .await,
            None
        );
        assert_eq!(
            lookup_with::<Ipv4Addr>(Some(MockResponse::new(200, "2001:db8::1".to_string()))).await,
            None
        );
        assert_eq!(
            lookup_with::<Ipv6Addr>(Some(MockResponse::new(200, "203.0.113.7".to_string()))).await,
            None
        );
    }

    #[async_test_all]
    async fn test_format_decimal_amount() {
        assert_eq!(format_decimal_amount(12_340_000, 6, 2), "12.34");
        assert_eq!(format_decimal_amount(5_000_000, 6, 2), "5.00");
        assert_eq!(format_decimal_amount(12_345_678, 6, 2), "12.34");
        assert_eq!(format_decimal_amount(7, 6, 2), "0.00");
        assert_eq!(format_decimal_amount(1_234, 2, 2), "12.34");
        assert_eq!(format_decimal_amount(5, 0, 2), "5.00");
        assert_eq!(format_decimal_amount(1_299, 2, 0), "12");
    }
}
