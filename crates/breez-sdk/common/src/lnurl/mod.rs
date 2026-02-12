pub mod auth;
pub mod error;
pub mod pay;
pub mod withdraw;

use serde::{Deserialize, Serialize};

/// Contains the result of the entire LNURL interaction, as reported by the LNURL endpoint.
///
/// * `Ok` indicates the interaction with the endpoint was valid, and the endpoint
///  - started to pay the invoice asynchronously in the case of LNURL-withdraw,
///  - verified the client signature in the case of LNURL-auth
/// * `Error` indicates a generic issue the LNURL endpoint encountered, including a freetext
///   description of the reason.
///
/// Both cases are described in LUD-03 <https://github.com/lnurl/luds/blob/luds/03.md> & LUD-04: <https://github.com/lnurl/luds/blob/luds/04.md>
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
#[serde(tag = "status")]
pub enum LnurlCallbackStatus {
    /// On-wire format is: `{"status": "OK"}`
    Ok,
    /// On-wire format is: `{"status": "ERROR", "reason": "error details..."}`
    #[serde(rename = "ERROR")]
    ErrorStatus {
        #[serde(flatten)]
        error_details: LnurlErrorDetails,
    },
}

/// Wrapped in a [`LnUrlError`], this represents a LNURL-endpoint error.
#[derive(Clone, Deserialize, Debug, Serialize)]
pub struct LnurlErrorDetails {
    pub reason: String,
}

/// LUD-17 scheme prefixes that need to be converted to http(s) for bech32 encoding.
const LNURL_SCHEME_PREFIXES: [&str; 4] = ["lnurlp://", "lnurlw://", "lnurlc://", "keyauth://"];

/// Converts a LUD-17 scheme URL (e.g. `lnurlp://`, `lnurlw://`, `keyauth://`) to
/// its corresponding http(s) URL. Uses `http://` for `.onion` domains, `https://` otherwise.
fn normalize_lnurl_scheme(url: &str) -> String {
    for prefix in LNURL_SCHEME_PREFIXES {
        if let Some(rest) = url.strip_prefix(prefix) {
            let is_onion = rest
                .split(['/', ':', '?'])
                .next()
                .is_some_and(|host| host.to_ascii_lowercase().ends_with(".onion"));
            let scheme = if is_onion { "http://" } else { "https://" };
            return format!("{scheme}{rest}");
        }
    }
    url.to_string()
}

/// Encodes an lnurl as a bech32 string.
/// Handles LUD-17 scheme prefixes (lnurlp://, lnurlw://, lnurlc://, keyauth://)
/// by converting them to their corresponding http(s) URL before encoding.
pub fn encode_lnurl_to_bech32(lnurl: &str) -> Result<String, String> {
    let normalized = normalize_lnurl_scheme(lnurl);
    let hrp = bech32::Hrp::parse("lnurl").map_err(|e| e.to_string())?;
    let bech32_encoded =
        bech32::encode::<bech32::Bech32>(hrp, normalized.as_bytes()).map_err(|e| e.to_string())?;
    Ok(bech32_encoded.to_lowercase())
}

#[cfg(test)]
mod tests {
    use rand;
    use rand::distributions::{Alphanumeric, DistString};

    use super::{encode_lnurl_to_bech32, normalize_lnurl_scheme};

    pub fn rand_string(len: usize) -> String {
        Alphanumeric.sample_string(&mut rand::thread_rng(), len)
    }

    #[test]
    fn test_normalize_lnurl_scheme_lnurlp() {
        assert_eq!(
            normalize_lnurl_scheme("lnurlp://domain.com/lnurlp/user"),
            "https://domain.com/lnurlp/user"
        );
    }

    #[test]
    fn test_normalize_lnurl_scheme_lnurlw() {
        assert_eq!(
            normalize_lnurl_scheme("lnurlw://domain.com/lnurl-withdraw?k1=abc"),
            "https://domain.com/lnurl-withdraw?k1=abc"
        );
    }

    #[test]
    fn test_normalize_lnurl_scheme_keyauth() {
        assert_eq!(
            normalize_lnurl_scheme("keyauth://domain.com/lnurl-login?tag=login&k1=abc"),
            "https://domain.com/lnurl-login?tag=login&k1=abc"
        );
    }

    #[test]
    fn test_normalize_lnurl_scheme_onion() {
        assert_eq!(
            normalize_lnurl_scheme("lnurlp://example.onion/lnurlp/user"),
            "http://example.onion/lnurlp/user"
        );
    }

    #[test]
    fn test_normalize_lnurl_scheme_https_passthrough() {
        assert_eq!(
            normalize_lnurl_scheme("https://domain.com/lnurlp/user"),
            "https://domain.com/lnurlp/user"
        );
    }

    #[test]
    fn test_encode_lnurl_to_bech32_normalizes_lnurlp() {
        let from_lnurlp = encode_lnurl_to_bech32("lnurlp://domain.com/path").unwrap();
        let from_https = encode_lnurl_to_bech32("https://domain.com/path").unwrap();
        assert_eq!(from_lnurlp, from_https);
    }

    #[test]
    fn test_encode_lnurl_to_bech32_onion_uses_http() {
        let from_lnurlp = encode_lnurl_to_bech32("lnurlp://example.onion/path").unwrap();
        let from_http = encode_lnurl_to_bech32("http://example.onion/path").unwrap();
        assert_eq!(from_lnurlp, from_http);
    }
}
