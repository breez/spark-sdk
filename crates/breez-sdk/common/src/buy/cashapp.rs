use super::BuyBitcoinProviderApi;
use anyhow::Result;

const CASHAPP_LIGHTNING_BASE_URL: &str = "https://cash.app/launch/lightning/";

#[derive(Default)]
pub struct CashAppProvider;

impl CashAppProvider {
    pub fn new() -> Self {
        Self
    }
}

#[macros::async_trait]
impl BuyBitcoinProviderApi for CashAppProvider {
    async fn buy_bitcoin(
        &self,
        invoice: String,
        _locked_amount_sat: Option<u64>,
        _redirect_url: Option<String>,
    ) -> Result<String> {
        Ok(format!("{CASHAPP_LIGHTNING_BASE_URL}{invoice}"))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use macros::async_test_all;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[async_test_all]
    async fn test_cashapp_url_construction() -> Result<(), Box<dyn std::error::Error>> {
        let provider = CashAppProvider::new();
        let invoice = "lnbc100n1p0abcde".to_string();
        let url = provider.buy_bitcoin(invoice.clone(), None, None).await?;
        assert_eq!(url, format!("https://cash.app/launch/lightning/{invoice}"));
        Ok(())
    }

    #[async_test_all]
    async fn test_cashapp_url_ignores_extra_params() -> Result<(), Box<dyn std::error::Error>> {
        let provider = CashAppProvider::new();
        let invoice = "lnbc100n1p0abcde".to_string();
        let url = provider
            .buy_bitcoin(
                invoice.clone(),
                Some(100_000),
                Some("https://example.com".to_string()),
            )
            .await?;
        assert_eq!(url, format!("https://cash.app/launch/lightning/{invoice}"));
        Ok(())
    }
}
