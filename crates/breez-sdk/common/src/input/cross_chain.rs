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

/// Parsed representation of a cross-chain URI.
///
/// Supports:
/// - **EIP-681** (EVM): `ethereum:<address>[@<chain_id>][/transfer?address=<to>&uint256=<amount>]`
/// - **Solana**: `solana:<address>?amount=<amount>&spl-token=<mint>`
/// - **Tron**: `tron:<address>?amount=<amount>&token=<contract>`
///
/// Unknown query params are ignored.
pub struct ParsedCrossChainUri {
    pub scheme: String,
    /// The recipient address (for ERC-20 transfer URIs this is extracted from
    /// the `address=` query param, not the path position).
    pub address: String,
    /// Token contract / mint address parsed from the URI.
    /// - EVM: the contract in the path position of a `/transfer` call
    /// - Solana: `spl-token=` query param
    /// - Tron: `token=` query param
    pub contract_address: Option<String>,
    /// EIP-681 chain ID parsed from `@<chain_id>` suffix on the address.
    pub chain_id: Option<u64>,
    /// Amount parsed from `amount=` or `value=` (EIP-681) query param.
    pub amount: Option<u128>,
}

/// Attempt to parse a cross-chain URI. Supports:
///
/// - **EIP-681** (EVM): `ethereum:<addr>[@<chain_id>]?value=<wei>&...`
///   or ERC-20 transfer: `ethereum:<contract>[@<chain_id>]/transfer?address=<to>&uint256=<amount>`
/// - **Solana**: `solana:<addr>?amount=<amount>&spl-token=<mint>`
/// - **Tron**: `tron:<addr>?amount=<amount>&token=<contract>`
///
/// Returns `None` if the input is not a recognized cross-chain URI.
pub fn parse_cross_chain_uri(input: &str) -> Option<ParsedCrossChainUri> {
    let (scheme, rest) = input.split_once(':')?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }

    let (path_part, query_part) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };

    let params = parse_query_params(query_part);

    match scheme {
        "ethereum" => parse_evm_uri(scheme, path_part, &params),
        "solana" => Some(parse_solana_uri(scheme, path_part, &params)),
        "tron" => Some(parse_tron_uri(scheme, path_part, &params)),
        _ => None,
    }
}

type QueryParams = std::collections::HashMap<String, String>;

fn parse_query_params(query_part: Option<&str>) -> QueryParams {
    let mut params = QueryParams::new();
    if let Some(qs) = query_part {
        for pair in qs.split('&').filter(|s| !s.is_empty()) {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let decoded =
                super::percent_encode::decode(value).unwrap_or_else(|_| value.to_string());
            params.insert(key.to_string(), decoded);
        }
    }
    params
}

/// EIP-681: `ethereum:<addr>[@<chain_id>]?value=<wei>`
/// or ERC-20: `ethereum:<contract>[@<chain_id>]/transfer?address=<to>&uint256=<amount>`
fn parse_evm_uri(scheme: &str, path: &str, params: &QueryParams) -> Option<ParsedCrossChainUri> {
    // ERC-20 transfer: `<contract>/transfer?address=<to>&uint256=<amount>`
    if let Some((addr_or_contract, function)) = path.split_once('/') {
        if function == "transfer" {
            let (contract, chain_id) = parse_evm_address_and_chain_id(addr_or_contract);
            let recipient = params.get("address").cloned()?;
            let amount = params
                .get("uint256")
                .or(params.get("amount"))
                .and_then(|s| s.parse::<u128>().ok());
            return Some(ParsedCrossChainUri {
                scheme: scheme.to_string(),
                address: recipient,
                contract_address: Some(contract),
                chain_id,
                amount,
            });
        }
        return None; // Unknown function
    }

    // Simple send: `ethereum:<addr>[@<chain_id>]?value=<wei>`
    let (address, chain_id) = parse_evm_address_and_chain_id(path);
    let amount = params
        .get("value")
        .or(params.get("amount"))
        .and_then(|s| s.parse::<u128>().ok());
    Some(ParsedCrossChainUri {
        scheme: scheme.to_string(),
        address,
        contract_address: None,
        chain_id,
        amount,
    })
}

/// Solana: `solana:<addr>?amount=<amount>&spl-token=<mint>`
fn parse_solana_uri(scheme: &str, path: &str, params: &QueryParams) -> ParsedCrossChainUri {
    ParsedCrossChainUri {
        scheme: scheme.to_string(),
        address: path.to_string(),
        contract_address: params.get("spl-token").cloned(),
        chain_id: None,
        amount: params.get("amount").and_then(|s| s.parse::<u128>().ok()),
    }
}

/// Tron: `tron:<addr>?amount=<amount>&token=<contract>`
fn parse_tron_uri(scheme: &str, path: &str, params: &QueryParams) -> ParsedCrossChainUri {
    ParsedCrossChainUri {
        scheme: scheme.to_string(),
        address: path.to_string(),
        contract_address: params.get("token").cloned(),
        chain_id: None,
        amount: params.get("amount").and_then(|s| s.parse::<u128>().ok()),
    }
}

/// Top-level entry point: attempt to classify `input` as a cross-chain
/// address (or canonical URI). Returns `None` if the input is not recognized.
///
/// This is a pure function with no network calls — route availability is
/// checked separately via `get_cross_chain_routes()`.
pub fn try_parse_cross_chain_address(
    input: &str,
) -> Option<super::models::CrossChainAddressDetails> {
    // Case 1: URI form.
    if let Some(parsed) = parse_cross_chain_uri(input) {
        let family = family_from_scheme(&parsed.scheme)?;
        // Sanity check: the recipient address must match the claimed family.
        if detect_address_family(&parsed.address) != Some(family) {
            return None;
        }
        return Some(super::models::CrossChainAddressDetails {
            address: parsed.address,
            address_family: family,
            contract_address: parsed.contract_address,
            chain_id: parsed.chain_id,
            amount: parsed.amount,
        });
    }

    // Case 2: bare address.
    let family = detect_address_family(input)?;
    Some(super::models::CrossChainAddressDetails {
        address: input.trim().to_string(),
        address_family: family,
        contract_address: None,
        chain_id: None,
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

/// Parse an EVM address that may have an `@<chain_id>` suffix (EIP-681).
/// Returns `(address_string, optional_chain_id)`.
fn parse_evm_address_and_chain_id(input: &str) -> (String, Option<u64>) {
    if let Some((addr, chain_id_str)) = input.split_once('@') {
        let chain_id = chain_id_str.parse::<u64>().ok();
        (addr.to_string(), chain_id)
    } else {
        (input.to_string(), None)
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

    // -- URI parsing: EVM (EIP-681) -----------------------------------------

    const EVM_ADDR: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    const USDC_CONTRACT: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

    #[test]
    fn evm_simple_send() {
        let parsed = parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}")).unwrap();
        assert_eq!(parsed.scheme, "ethereum");
        assert_eq!(parsed.address, EVM_ADDR);
        assert!(parsed.contract_address.is_none());
        assert!(parsed.chain_id.is_none());
        assert!(parsed.amount.is_none());
    }

    #[test]
    fn evm_simple_send_with_value() {
        let parsed = parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}?value=1000000")).unwrap();
        assert_eq!(parsed.amount, Some(1_000_000));
        assert!(parsed.contract_address.is_none());
    }

    #[test]
    fn evm_with_chain_id() {
        let parsed =
            parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}@8453?value=1000")).unwrap();
        assert_eq!(parsed.address, EVM_ADDR);
        assert_eq!(parsed.chain_id, Some(8453)); // Base chain ID
        assert_eq!(parsed.amount, Some(1000));
    }

    #[test]
    fn evm_erc20_transfer() {
        let uri = format!("ethereum:{USDC_CONTRACT}/transfer?address={EVM_ADDR}&uint256=1000000");
        let parsed = parse_cross_chain_uri(&uri).unwrap();
        assert_eq!(parsed.address, EVM_ADDR);
        assert_eq!(parsed.contract_address.as_deref(), Some(USDC_CONTRACT));
        assert_eq!(parsed.amount, Some(1_000_000));
        assert!(parsed.chain_id.is_none());
    }

    #[test]
    fn evm_erc20_transfer_with_chain_id() {
        let uri =
            format!("ethereum:{USDC_CONTRACT}@8453/transfer?address={EVM_ADDR}&uint256=5000000");
        let parsed = parse_cross_chain_uri(&uri).unwrap();
        assert_eq!(parsed.address, EVM_ADDR);
        assert_eq!(parsed.contract_address.as_deref(), Some(USDC_CONTRACT));
        assert_eq!(parsed.chain_id, Some(8453));
        assert_eq!(parsed.amount, Some(5_000_000));
    }

    #[test]
    fn evm_erc20_transfer_missing_address_returns_none() {
        // /transfer without address= query param should fail
        let uri = format!("ethereum:{USDC_CONTRACT}/transfer?uint256=1000");
        assert!(parse_cross_chain_uri(&uri).is_none());
    }

    #[test]
    fn evm_unknown_function_returns_none() {
        let uri = format!("ethereum:{EVM_ADDR}/approve?spender={USDC_CONTRACT}");
        assert!(parse_cross_chain_uri(&uri).is_none());
    }

    #[test]
    fn evm_amount_fallback_to_amount_param() {
        // `amount=` works as fallback when `value=` is absent
        let parsed = parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}?amount=500")).unwrap();
        assert_eq!(parsed.amount, Some(500));
    }

    #[test]
    fn evm_malformed_chain_id_ignored() {
        let parsed = parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}@notanumber")).unwrap();
        assert_eq!(parsed.address, EVM_ADDR);
        assert!(parsed.chain_id.is_none());
    }

    // -- URI parsing: Solana ------------------------------------------------

    #[test]
    fn solana_simple_send() {
        let parsed =
            parse_cross_chain_uri(&format!("solana:{SOLANA_ADDR}?amount=1000000")).unwrap();
        assert_eq!(parsed.scheme, "solana");
        assert_eq!(parsed.address, SOLANA_ADDR);
        assert_eq!(parsed.amount, Some(1_000_000));
        assert!(parsed.contract_address.is_none());
    }

    #[test]
    fn solana_spl_token() {
        let mint = SOLANA_ADDR; // reuse as a plausible mint address
        let recipient = "mvines9iiHiQTysrwkJjGf2gb9Ex9jXJX8ns3qwf2kN";
        let uri = format!("solana:{recipient}?amount=100&spl-token={mint}");
        let parsed = parse_cross_chain_uri(&uri).unwrap();
        assert_eq!(parsed.address, recipient);
        assert_eq!(parsed.contract_address.as_deref(), Some(mint));
        assert_eq!(parsed.amount, Some(100));
    }

    #[test]
    fn solana_no_query() {
        let parsed = parse_cross_chain_uri(&format!("solana:{SOLANA_ADDR}")).unwrap();
        assert_eq!(parsed.address, SOLANA_ADDR);
        assert!(parsed.contract_address.is_none());
        assert!(parsed.amount.is_none());
    }

    // -- URI parsing: Tron --------------------------------------------------

    #[test]
    fn tron_simple_send() {
        let parsed = parse_cross_chain_uri(&format!("tron:{TRON_ADDR}?amount=100")).unwrap();
        assert_eq!(parsed.scheme, "tron");
        assert_eq!(parsed.address, TRON_ADDR);
        assert_eq!(parsed.amount, Some(100));
        assert!(parsed.contract_address.is_none());
    }

    #[test]
    fn tron_trc20_token() {
        let trc20_contract = TRON_ADDR; // reuse as plausible contract
        let recipient = "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9";
        let uri = format!("tron:{recipient}?amount=500&token={trc20_contract}");
        let parsed = parse_cross_chain_uri(&uri).unwrap();
        assert_eq!(parsed.address, recipient);
        assert_eq!(parsed.contract_address.as_deref(), Some(trc20_contract));
        assert_eq!(parsed.amount, Some(500));
    }

    // -- URI parsing: general -----------------------------------------------

    #[test]
    fn uri_rejects_unknown_scheme() {
        assert!(parse_cross_chain_uri("bitcoin:bc1qxyz?amount=1").is_none());
        assert!(parse_cross_chain_uri("lightning:lnbc1...").is_none());
    }

    #[test]
    fn uri_malformed_amount_is_dropped() {
        let parsed =
            parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}?value=notanumber")).unwrap();
        assert!(parsed.amount.is_none());
    }

    #[test]
    fn uri_ignores_unknown_query_params() {
        let parsed =
            parse_cross_chain_uri(&format!("ethereum:{EVM_ADDR}?foo=bar&value=100")).unwrap();
        assert_eq!(parsed.amount, Some(100));
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
        let info = try_parse_cross_chain_address(EVM_ADDR).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Evm);
        assert_eq!(info.address, EVM_ADDR);
        assert!(info.contract_address.is_none());
        assert!(info.chain_id.is_none());
        assert!(info.amount.is_none());
    }

    #[test]
    fn try_parse_bare_solana_address() {
        let info = try_parse_cross_chain_address(SOLANA_ADDR).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Solana);
        assert!(info.contract_address.is_none());
    }

    #[test]
    fn try_parse_bare_tron_address() {
        let info = try_parse_cross_chain_address(TRON_ADDR).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Tron);
        assert!(info.contract_address.is_none());
    }

    #[test]
    fn try_parse_rejects_non_address() {
        assert!(try_parse_cross_chain_address("not-an-address").is_none());
    }

    #[test]
    fn try_parse_evm_erc20_uri() {
        let uri = format!("ethereum:{USDC_CONTRACT}/transfer?address={EVM_ADDR}&uint256=1000000");
        let info = try_parse_cross_chain_address(&uri).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Evm);
        assert_eq!(info.address, EVM_ADDR);
        assert_eq!(info.contract_address.as_deref(), Some(USDC_CONTRACT));
        assert_eq!(info.amount, Some(1_000_000));
    }

    #[test]
    fn try_parse_evm_with_chain_id() {
        let uri = format!("ethereum:{EVM_ADDR}@8453?value=5000");
        let info = try_parse_cross_chain_address(&uri).unwrap();
        assert_eq!(info.chain_id, Some(8453));
        assert_eq!(info.amount, Some(5000));
    }

    #[test]
    fn try_parse_solana_spl_token_uri() {
        let mint = SOLANA_ADDR;
        let recipient = "mvines9iiHiQTysrwkJjGf2gb9Ex9jXJX8ns3qwf2kN";
        let uri = format!("solana:{recipient}?spl-token={mint}&amount=100");
        let info = try_parse_cross_chain_address(&uri).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Solana);
        assert_eq!(info.address, recipient);
        assert_eq!(info.contract_address.as_deref(), Some(mint));
        assert_eq!(info.amount, Some(100));
    }

    #[test]
    fn try_parse_tron_trc20_uri() {
        let contract = TRON_ADDR;
        let recipient = "TN3W4H6rK2ce4vX9YnFQHwKENnHjoxb3m9";
        let uri = format!("tron:{recipient}?token={contract}&amount=200");
        let info = try_parse_cross_chain_address(&uri).unwrap();
        assert_eq!(info.address_family, CrossChainAddressFamily::Tron);
        assert_eq!(info.address, recipient);
        assert_eq!(info.contract_address.as_deref(), Some(contract));
        assert_eq!(info.amount, Some(200));
    }

    #[test]
    fn try_parse_uri_mismatched_scheme_and_address_returns_none() {
        assert!(
            try_parse_cross_chain_address("solana:0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")
                .is_none()
        );
    }
}
