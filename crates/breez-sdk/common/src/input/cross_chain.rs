//! Cross-chain address detection and URI parsing.
//!
//! This module lives in `breez-sdk-common` so it has **zero** provider
//! dependencies (no flashnet, no Orchestra client). It handles:
//!
//! * Detecting whether a bare string is an EVM / Solana / Tron address.
//! * Parsing canonical URIs of the form
//!   `ethereum:<addr>?chain=base&asset=usdc&amount=1000`.
//!
//! Chain and asset are passed through as plain strings — the SDK does not
//! maintain a curated enum of supported chains/assets. Route availability
//! is checked at prepare time via `get_cross_chain_routes()`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Address family + route pair types
// ---------------------------------------------------------------------------

/// Address family determines which chains a recipient address can belong to.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrossChainAddressFamily {
    /// Ethereum-compatible (checksummed or lowercase hex, 0x-prefixed, 20 bytes).
    Evm,
    /// Solana (base58, 32 bytes).
    Solana,
    /// Tron (base58check, `T` prefix).
    Tron,
}

impl CrossChainAddressFamily {
    /// The canonical URI scheme for this family.
    pub fn canonical_scheme(self) -> &'static str {
        match self {
            Self::Evm => "ethereum",
            Self::Solana => "solana",
            Self::Tron => "tron",
        }
    }

    /// Whether `chain` belongs to this address family.
    ///
    /// Solana and Tron match their exact chain name. EVM is the catch-all for
    /// everything else — any chain Orchestra returns that isn't Solana or Tron
    /// is assumed to use EVM-style addresses. This avoids hardcoding a list of
    /// EVM chains that would need updating every time a new one is added.
    pub fn matches_chain(self, chain: &str) -> bool {
        match self {
            Self::Solana => chain.eq_ignore_ascii_case("solana"),
            Self::Tron => chain.eq_ignore_ascii_case("tron"),
            Self::Evm => !Self::Solana.matches_chain(chain) && !Self::Tron.matches_chain(chain),
        }
    }
}

/// A single {chain, asset} pair available for cross-chain sends.
/// Returned by `get_cross_chain_routes()` for route discovery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CrossChainRoutePair {
    pub chain: String,
    pub asset: String,
    pub contract_address: Option<String>,
    pub decimals: u8,
    pub exact_out_eligible: bool,
}

// ---------------------------------------------------------------------------
// Lightweight parse result
// ---------------------------------------------------------------------------

/// Lightweight classification from the common-crate parser.
/// No route/pair information — that's populated by the SDK layer via
/// `get_cross_chain_routes()`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrossChainAddressInfo {
    /// The raw recipient address (e.g. `0xabc...`).
    pub address: String,
    /// Which address family this belongs to.
    pub family: CrossChainAddressFamily,
    /// Optional chain hint parsed from a URI `chain=` param (e.g. `"base"`).
    pub chain: Option<String>,
    /// Optional asset hint parsed from a URI `asset=` param (e.g. `"usdc"`).
    pub asset: Option<String>,
    /// Optional amount parsed from an `amount=` query param.
    pub amount: Option<u128>,
}

// ---------------------------------------------------------------------------
// Address detection
// ---------------------------------------------------------------------------

/// Detect the address family of a bare recipient string, returning `None`
/// if the string is not a recognizable cross-chain address.
pub fn detect_address_family(input: &str) -> Option<CrossChainAddressFamily> {
    let trimmed = input.trim();

    // EVM: 0x + 40 hex chars.
    if trimmed.len() == 42
        && trimmed.starts_with("0x")
        && trimmed[2..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some(CrossChainAddressFamily::Evm);
    }

    // Tron: T + 33 base58check chars (total 34). Keep this check before Solana so
    // that T-prefixed strings aren't mis-detected as Solana base58.
    if trimmed.len() == 34
        && trimmed.starts_with('T')
        && bitcoin::base58::decode_check(trimmed).is_ok()
    {
        return Some(CrossChainAddressFamily::Tron);
    }

    // Solana: base58 encoding of a 32-byte public key. Length is 43-44 chars
    // typically; we just try to decode and check the byte length.
    if (32..=44).contains(&trimmed.len())
        && bitcoin::base58::decode(trimmed).is_ok_and(|decoded| decoded.len() == 32)
    {
        return Some(CrossChainAddressFamily::Solana);
    }

    None
}

// ---------------------------------------------------------------------------
// URI parsing
// ---------------------------------------------------------------------------

/// Parsed representation of a canonical cross-chain URI of the form
/// `<scheme>:<address>?chain=<chain>&asset=<asset>&amount=<u128>&label=<text>`.
/// Unknown query params are ignored.
pub struct ParsedCrossChainUri<'a> {
    pub scheme: &'a str,
    pub address: String,
    pub chain: Option<String>,
    pub asset: Option<String>,
    pub amount: Option<u128>,
}

/// Attempt to parse a canonical cross-chain URI. Returns `None` if `input` is
/// not of the expected form (scheme, colon, address, optional query string).
pub fn parse_cross_chain_uri(input: &str) -> Option<ParsedCrossChainUri<'_>> {
    let (scheme, rest) = input.split_once(':')?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }
    // Only recognize the three canonical schemes we emit ourselves. This avoids
    // accidentally swallowing other URI-like inputs (e.g. `lightning:...`).
    if !matches!(scheme, "ethereum" | "solana" | "tron") {
        return None;
    }

    let (addr_part, query_part) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };

    let mut chain = None;
    let mut asset = None;
    let mut amount = None;

    if let Some(qs) = query_part {
        for pair in qs.split('&').filter(|s| !s.is_empty()) {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let decoded =
                super::percent_encode::decode(value).unwrap_or_else(|_| value.to_string());
            match key {
                "chain" => chain = Some(decoded),
                "asset" => asset = Some(decoded),
                "amount" => amount = decoded.parse::<u128>().ok(),
                _ => {}
            }
        }
    }

    Some(ParsedCrossChainUri {
        scheme,
        address: addr_part.to_string(),
        chain,
        asset,
        amount,
    })
}

/// Top-level entry point: attempt to classify `input` as a cross-chain
/// address (or canonical URI). Returns `None` if the input is not recognized.
///
/// This is a pure function with no network calls — route availability is
/// checked separately via `get_cross_chain_routes()`.
pub fn try_parse_cross_chain_address(input: &str) -> Option<CrossChainAddressInfo> {
    // Case 1: canonical URI form.
    if let Some(parsed) = parse_cross_chain_uri(input) {
        let family = family_from_scheme(parsed.scheme)?;
        // Sanity check the address matches the claimed family.
        if detect_address_family(&parsed.address) != Some(family) {
            return None;
        }
        return Some(CrossChainAddressInfo {
            address: parsed.address,
            family,
            chain: parsed.chain,
            asset: parsed.asset,
            amount: parsed.amount,
        });
    }

    // Case 2: bare address.
    let family = detect_address_family(input)?;
    let address = input.trim().to_string();
    Some(CrossChainAddressInfo {
        address,
        family,
        chain: None,
        asset: None,
        amount: None,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn family_from_scheme(scheme: &str) -> Option<CrossChainAddressFamily> {
    match scheme {
        "ethereum" => Some(CrossChainAddressFamily::Evm),
        "solana" => Some(CrossChainAddressFamily::Solana),
        "tron" => Some(CrossChainAddressFamily::Tron),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Address detection --------------------------------------------------

    #[test]
    fn detects_evm_address() {
        assert_eq!(
            detect_address_family("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            Some(CrossChainAddressFamily::Evm)
        );
    }

    #[test]
    fn rejects_short_evm_address() {
        assert_eq!(detect_address_family("0xdeadbeef"), None);
    }

    #[test]
    fn accepts_lowercase_evm_address() {
        assert_eq!(
            detect_address_family("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"),
            Some(CrossChainAddressFamily::Evm)
        );
    }

    #[test]
    fn rejects_evm_with_non_hex() {
        assert_eq!(
            detect_address_family("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA0291Z"),
            None
        );
    }

    #[test]
    fn rejects_evm_missing_prefix() {
        assert_eq!(
            detect_address_family("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            None
        );
    }

    const SOLANA_ADDR: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    #[test]
    fn detects_solana_address() {
        assert_eq!(
            detect_address_family(SOLANA_ADDR),
            Some(CrossChainAddressFamily::Solana)
        );
    }

    #[test]
    fn rejects_non_base58_solana_string() {
        assert_eq!(
            detect_address_family("0PjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            None
        );
    }

    const TRON_ADDR: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

    #[test]
    fn detects_tron_address() {
        assert_eq!(
            detect_address_family(TRON_ADDR),
            Some(CrossChainAddressFamily::Tron)
        );
    }

    #[test]
    fn rejects_tron_with_bad_checksum() {
        let mut bad = TRON_ADDR.to_string();
        bad.pop();
        bad.push('X');
        assert_eq!(detect_address_family(&bad), None);
    }

    // -- URI parsing --------------------------------------------------------

    #[test]
    fn parses_canonical_uri() {
        let parsed = parse_cross_chain_uri(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&asset=usdc&amount=1000",
        )
        .unwrap();
        assert_eq!(parsed.scheme, "ethereum");
        assert_eq!(parsed.chain.as_deref(), Some("base"));
        assert_eq!(parsed.asset.as_deref(), Some("usdc"));
        assert_eq!(parsed.amount, Some(1000));
    }

    #[test]
    fn uri_without_query_has_no_pair() {
        let parsed =
            parse_cross_chain_uri("ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913").unwrap();
        assert!(parsed.chain.is_none());
        assert!(parsed.asset.is_none());
    }

    #[test]
    fn parses_solana_uri_with_chain_asset() {
        let input = format!("solana:{SOLANA_ADDR}?chain=solana&asset=usdc");
        let parsed = parse_cross_chain_uri(&input).unwrap();
        assert_eq!(parsed.scheme, "solana");
        assert_eq!(parsed.address, SOLANA_ADDR);
        assert_eq!(parsed.chain.as_deref(), Some("solana"));
        assert_eq!(parsed.asset.as_deref(), Some("usdc"));
    }

    #[test]
    fn parses_tron_uri() {
        let input = format!("tron:{TRON_ADDR}?chain=tron&asset=usdt");
        let parsed = parse_cross_chain_uri(&input).unwrap();
        assert_eq!(parsed.scheme, "tron");
        assert_eq!(parsed.address, TRON_ADDR);
        assert_eq!(parsed.chain.as_deref(), Some("tron"));
        assert_eq!(parsed.asset.as_deref(), Some("usdt"));
    }

    #[test]
    fn uri_with_amount_and_label() {
        let parsed = parse_cross_chain_uri(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&asset=usdc&amount=100000",
        )
        .unwrap();
        assert_eq!(parsed.amount, Some(100_000));
    }

    #[test]
    fn uri_rejects_unknown_scheme() {
        assert!(parse_cross_chain_uri("bitcoin:bc1qxyz?amount=1").is_none());
        assert!(parse_cross_chain_uri("lightning:lnbc1...").is_none());
    }

    #[test]
    fn uri_ignores_unknown_query_params() {
        let parsed = parse_cross_chain_uri(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&foo=bar&asset=usdc",
        )
        .unwrap();
        assert_eq!(parsed.chain.as_deref(), Some("base"));
        assert_eq!(parsed.asset.as_deref(), Some("usdc"));
    }

    #[test]
    fn uri_malformed_amount_is_dropped() {
        let parsed = parse_cross_chain_uri(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?amount=notanumber",
        )
        .unwrap();
        assert!(parsed.amount.is_none());
    }

    // -- canonical_scheme / matches_chain -------------------------------------

    #[test]
    fn canonical_scheme_evm() {
        assert_eq!(CrossChainAddressFamily::Evm.canonical_scheme(), "ethereum");
    }

    #[test]
    fn canonical_scheme_solana() {
        assert_eq!(CrossChainAddressFamily::Solana.canonical_scheme(), "solana");
    }

    #[test]
    fn canonical_scheme_tron() {
        assert_eq!(CrossChainAddressFamily::Tron.canonical_scheme(), "tron");
    }

    #[test]
    fn matches_chain_evm_known() {
        let evm = CrossChainAddressFamily::Evm;
        assert!(evm.matches_chain("ethereum"));
        assert!(evm.matches_chain("base"));
        assert!(evm.matches_chain("arbitrum"));
        assert!(evm.matches_chain("optimism"));
        assert!(evm.matches_chain("polygon"));
    }

    #[test]
    fn matches_chain_evm_unknown_chain_is_evm() {
        // Any new chain Orchestra adds that isn't Solana/Tron is assumed EVM.
        let evm = CrossChainAddressFamily::Evm;
        assert!(evm.matches_chain("avalanche"));
        assert!(evm.matches_chain("zksync"));
        assert!(evm.matches_chain("fantom"));
    }

    #[test]
    fn matches_chain_evm_excludes_solana_and_tron() {
        let evm = CrossChainAddressFamily::Evm;
        assert!(!evm.matches_chain("solana"));
        assert!(!evm.matches_chain("tron"));
    }

    #[test]
    fn matches_chain_solana() {
        assert!(CrossChainAddressFamily::Solana.matches_chain("solana"));
        assert!(CrossChainAddressFamily::Solana.matches_chain("Solana"));
        assert!(!CrossChainAddressFamily::Solana.matches_chain("ethereum"));
        assert!(!CrossChainAddressFamily::Solana.matches_chain("tron"));
    }

    #[test]
    fn matches_chain_tron() {
        assert!(CrossChainAddressFamily::Tron.matches_chain("tron"));
        assert!(CrossChainAddressFamily::Tron.matches_chain("TRON"));
        assert!(!CrossChainAddressFamily::Tron.matches_chain("ethereum"));
        assert!(!CrossChainAddressFamily::Tron.matches_chain("solana"));
    }

    // -- try_parse_cross_chain_address ----------------------------------------

    #[test]
    fn try_parse_bare_evm_address() {
        let info =
            try_parse_cross_chain_address("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913").unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Evm);
        assert!(info.chain.is_none());
        assert!(info.asset.is_none());
    }

    #[test]
    fn try_parse_uri_with_chain_asset() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&asset=usdc",
        )
        .unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Evm);
        assert_eq!(info.chain.as_deref(), Some("base"));
        assert_eq!(info.asset.as_deref(), Some("usdc"));
    }

    #[test]
    fn try_parse_uri_unknown_chain_passes_through() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=avalanche&asset=usdc",
        )
        .unwrap();
        // avalanche passes through as a string now (not rejected)
        assert_eq!(info.chain.as_deref(), Some("avalanche"));
        assert_eq!(info.asset.as_deref(), Some("usdc"));
    }

    #[test]
    fn try_parse_uri_unknown_asset_passes_through() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&asset=eth",
        )
        .unwrap();
        assert_eq!(info.chain.as_deref(), Some("base"));
        assert_eq!(info.asset.as_deref(), Some("eth"));
    }

    #[test]
    fn try_parse_rejects_non_address() {
        assert!(try_parse_cross_chain_address("not-an-address").is_none());
    }

    #[test]
    fn try_parse_bare_solana_address() {
        let info = try_parse_cross_chain_address(SOLANA_ADDR).unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Solana);
        assert!(info.chain.is_none());
        assert!(info.asset.is_none());
    }

    #[test]
    fn try_parse_bare_tron_address() {
        let info = try_parse_cross_chain_address(TRON_ADDR).unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Tron);
        assert!(info.chain.is_none());
    }

    #[test]
    fn try_parse_uri_with_only_chain_hint() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base",
        )
        .unwrap();
        assert_eq!(info.chain.as_deref(), Some("base"));
        assert!(info.asset.is_none());
    }

    #[test]
    fn try_parse_uri_with_only_asset_hint() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?asset=usdt",
        )
        .unwrap();
        assert!(info.chain.is_none());
        assert_eq!(info.asset.as_deref(), Some("usdt"));
    }

    #[test]
    fn try_parse_uri_with_amount() {
        let info = try_parse_cross_chain_address(
            "ethereum:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913?chain=base&asset=usdc&amount=50000",
        )
        .unwrap();
        assert_eq!(info.amount, Some(50000));
    }

    #[test]
    fn try_parse_solana_uri() {
        let input = format!("solana:{SOLANA_ADDR}?chain=solana&asset=usdc");
        let info = try_parse_cross_chain_address(&input).unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Solana);
        assert_eq!(info.chain.as_deref(), Some("solana"));
        assert_eq!(info.asset.as_deref(), Some("usdc"));
    }

    #[test]
    fn try_parse_tron_uri_usdt() {
        let input = format!("tron:{TRON_ADDR}?chain=tron&asset=usdt");
        let info = try_parse_cross_chain_address(&input).unwrap();
        assert_eq!(info.family, CrossChainAddressFamily::Tron);
        assert_eq!(info.chain.as_deref(), Some("tron"));
        assert_eq!(info.asset.as_deref(), Some("usdt"));
    }

    #[test]
    fn try_parse_uri_mismatched_scheme_and_address_returns_none() {
        assert!(
            try_parse_cross_chain_address("solana:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
                .is_none()
        );
    }
}
