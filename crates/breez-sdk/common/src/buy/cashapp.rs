const CASHAPP_LIGHTNING_BASE_URL: &str = "https://cash.app/launch/lightning/";

pub struct CashAppProvider;

impl CashAppProvider {
    /// Build a `CashApp` deep link URL from a bolt11 Lightning invoice.
    pub fn build_url(invoice: &str) -> String {
        format!("{CASHAPP_LIGHTNING_BASE_URL}{invoice}")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn test_cashapp_url_construction() {
        let invoice = "lnbc100n1p0abcde";
        let url = CashAppProvider::build_url(invoice);
        assert_eq!(url, format!("https://cash.app/launch/lightning/{invoice}"));
    }
}
