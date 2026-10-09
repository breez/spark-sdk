const CASHAPP_LIGHTNING_BASE_URL: &str = "https://cash.app/launch/lightning/";
const CASHAPP_LIGHTNING_ADDRESS_DOMAIN: &str = "cash.app";
const CASHTAG_MAX_LEN: usize = 20;

pub struct CashAppProvider;

impl CashAppProvider {
    /// Build a `CashApp` deep link URL from a bolt11 Lightning invoice.
    pub fn build_url(invoice: &str) -> String {
        format!("{CASHAPP_LIGHTNING_BASE_URL}{invoice}")
    }
}

/// The Lightning address of a Cash App user, given as `alice`, `$alice` or
/// `alice@cash.app`: `alice@cash.app`. Returns `None` if `input` is not a valid
/// username (1 to 20 letters or digits, at least one of them a letter).
pub fn cash_app_lightning_address(input: &str) -> Option<String> {
    let username = input.strip_prefix('$').unwrap_or(input);
    let username = match username.split_once('@') {
        Some((user, domain)) if domain.eq_ignore_ascii_case(CASHAPP_LIGHTNING_ADDRESS_DOMAIN) => {
            user
        }
        Some(_) => return None,
        None => username,
    };
    let valid = (1..=CASHTAG_MAX_LEN).contains(&username.len())
        && username.chars().all(|c| c.is_ascii_alphanumeric())
        && username.chars().any(|c| c.is_ascii_alphabetic());
    valid.then(|| format!("{username}@{CASHAPP_LIGHTNING_ADDRESS_DOMAIN}"))
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

    #[test]
    fn test_cash_app_lightning_address() {
        let address = |input| cash_app_lightning_address(input);
        assert_eq!(address("alice").as_deref(), Some("alice@cash.app"));
        assert_eq!(address("$alice").as_deref(), Some("alice@cash.app"));
        assert_eq!(address("$Alice42").as_deref(), Some("Alice42@cash.app"));
        assert_eq!(address("a").as_deref(), Some("a@cash.app"));
        assert_eq!(address("alice@cash.app").as_deref(), Some("alice@cash.app"));
        assert_eq!(address("alice@Cash.App").as_deref(), Some("alice@cash.app"));
        assert_eq!(
            address("abcdefghijklmnopqrst").as_deref(),
            Some("abcdefghijklmnopqrst@cash.app")
        );

        for invalid in [
            "",
            "$",
            "$$alice",
            "12345",
            "$12345",
            "abcdefghijklmnopqrstu",
            "al ice",
            "al-ice",
            "al_ice",
            "alice@example.com",
            "@cash.app",
            "ålice",
        ] {
            assert_eq!(address(invalid), None, "{invalid}");
        }
    }
}
