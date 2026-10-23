//! Server-signed quotes: a MAC over a challenge's terms, carried in its
//! `extra`, so a server can later trust the terms the client echoes without
//! keeping a record of each challenge.

use bitcoin::hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256};
use serde_json::json;

use super::{
    error::{X402Error, X402ErrorReason, X402Result},
    jcs,
    network::LnbtcNetwork,
};

const DOMAIN: &str = "x402:exact:lnbtc:quote:v1";

/// The terms a quote commits to. The payment hash ties it to one invoice, so
/// it can't vouch for an invoice issued by anyone else.
pub struct QuoteTerms<'a> {
    pub network: LnbtcNetwork,
    pub amount_msat: u64,
    /// Lowercase hex of the `payTo` key.
    pub pay_to: &'a str,
    pub max_timeout_secs: u64,
    pub request_hash: [u8; 32],
    /// Lowercase hex.
    pub payment_hash: &'a str,
}

/// The quote for `terms` under `key`, as lowercase hex.
pub fn quote_mac(key: &[u8; 32], terms: &QuoteTerms) -> X402Result<String> {
    let message = json!({
        "domain": DOMAIN,
        "network": terms.network.id(),
        "amount": terms.amount_msat.to_string(),
        "payTo": terms.pay_to,
        "maxTimeoutSeconds": terms.max_timeout_secs,
        "requestHash": hex::encode(terms.request_hash),
        "paymentHash": terms.payment_hash,
    });
    let canonical = jcs::canonicalize(&message)
        .map_err(|e| X402Error::new(X402ErrorReason::InvalidQuote, e.0))?;
    let mut engine = HmacEngine::<sha256::Hash>::new(key);
    engine.input(canonical.as_bytes());
    Ok(hex::encode(
        Hmac::<sha256::Hash>::from_engine(engine).to_byte_array(),
    ))
}

/// Checks that `quote` is the quote for `terms` under `key`.
pub fn verify_quote(key: &[u8; 32], terms: &QuoteTerms, quote: Option<&str>) -> X402Result<()> {
    let expected = quote_mac(key, terms)?;
    let matches =
        quote.is_some_and(|quote| constant_time_eq(quote.as_bytes(), expected.as_bytes()));
    if !matches {
        return Err(X402Error::new(
            X402ErrorReason::InvalidQuote,
            "accepted terms are not a quote from this receiver",
        ));
    }
    Ok(())
}

/// Compares without an early exit, so the time taken doesn't reveal how much
/// of a forged quote was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(amount_msat: u64, payment_hash: &str) -> QuoteTerms<'_> {
        QuoteTerms {
            network: LnbtcNetwork::Regtest,
            amount_msat,
            pay_to: "02aa",
            max_timeout_secs: 300,
            request_hash: [7; 32],
            payment_hash,
        }
    }

    #[macros::test_all]
    fn verifies_its_own_quote_only() {
        let key = [1; 32];
        let quote = quote_mac(&key, &terms(21_000, "aa")).unwrap();
        assert!(verify_quote(&key, &terms(21_000, "aa"), Some(&quote)).is_ok());
        // Any changed term, another key, or no quote at all is refused.
        for (key, terms, quote) in [
            ([1; 32], terms(1_000, "aa"), Some(quote.as_str())),
            ([1; 32], terms(21_000, "bb"), Some(quote.as_str())),
            ([2; 32], terms(21_000, "aa"), Some(quote.as_str())),
            ([1; 32], terms(21_000, "aa"), None),
        ] {
            let err = verify_quote(&key, &terms, quote).unwrap_err();
            assert_eq!(err.reason, X402ErrorReason::InvalidQuote);
        }
    }
}
