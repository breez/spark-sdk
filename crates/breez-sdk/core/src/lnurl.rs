use bitcoin::hex::DisplayHex;
use lnurl_models::{
    CheckUsernameAvailableRequest, CheckUsernameAvailableResponse, ListMetadataResponse,
    RecoverLnurlPayRequest, RecoverLnurlPayResponse, RegisterLnurlPayRequest,
    RegisterLnurlPayResponse, TransferLnurlPayRequest, UnregisterLnurlPayRequest, signed_message,
};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use platform_utils::{ContentType, HttpClient, add_content_type_header};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

/// Headers `list_metadata` sends its credential in, keeping it out of the query
/// string and so out of proxy and access logs. The response carries preimages.
const SIGNATURE_HEADER: &str = "X-Breez-Signature";
const TIMESTAMP_HEADER: &str = "X-Breez-Timestamp";

/// A configured LNURL domain split into the two things a signed request needs.
///
/// Both come from one parse, so the domain the client signs cannot drift from
/// the one the request is addressed to.
#[derive(Clone)]
struct ParsedDomain {
    /// The URL every route is appended to: scheme, userinfo, authority, and any
    /// path prefix the deployment is mounted under.
    base_url: String,
    /// The authority a signed message names, which is what the server resolves
    /// from `Host`.
    signed_domain: String,
}

/// Splits a configured LNURL domain into the URL to call and the authority to
/// sign.
///
/// The authority has to be what the HTTP client will put in `Host`, since that
/// is what the server resolves and verifies the signature against: lowercased,
/// without userinfo, and without a port the scheme already implies. `Url`
/// applies all three while parsing, so the signed value is whatever it
/// normalized rather than a second implementation of the same rules.
fn parse_domain(configured: &str) -> ParsedDomain {
    let absolute = if configured.contains("://") {
        configured.to_string()
    } else {
        format!("https://{configured}")
    };
    let Ok(mut url) = url::Url::parse(&absolute) else {
        // Unparseable, so there is nothing to split. Passing it through leaves
        // the failure to the request, where the URL appears in the error.
        return ParsedDomain {
            base_url: configured.to_string(),
            signed_domain: configured.to_string(),
        };
    };

    // A query or fragment would swallow the route appended after it, and a
    // trailing slash would double the one the route already starts with.
    url.set_query(None);
    url.set_fragment(None);
    let base_url = url.as_str().trim_end_matches('/').to_string();

    let signed_domain = match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        // A scheme that carries no authority at all, such as `mailto:`.
        (None, _) => configured.to_ascii_lowercase(),
    };

    ParsedDomain {
        base_url,
        signed_domain,
    }
}

/// The authority a signed message names, given a configured LNURL domain.
pub(crate) fn signed_domain(configured: &str) -> String {
    parse_domain(configured).signed_domain
}

#[derive(Debug)]
pub enum LnurlServerError {
    InvalidApiKey,
    Network {
        statuscode: u16,
        message: Option<String>,
    },
    RequestFailure(String),
    SigningError(String),
}

impl std::fmt::Display for LnurlServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LnurlServerError::InvalidApiKey => write!(f, "Invalid API key"),
            LnurlServerError::Network {
                statuscode,
                message,
            } => {
                write!(f, "Network error (status {statuscode}): {message:?}")
            }
            LnurlServerError::RequestFailure(msg) => write!(f, "Request failure: {msg}"),
            LnurlServerError::SigningError(msg) => write!(f, "Signing error: {msg}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegisterLightningAddressRequest {
    pub username: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct TransferLightningAddressRequest {
    pub username: String,
    pub description: String,
    /// Hex-encoded secp256k1 compressed public key of the current owner.
    pub from_pubkey: String,
    /// Hex-encoded DER ECDSA signature by the current owner over
    /// [`signed_message::transfer_from`].
    pub from_signature: String,
    /// Seconds since the Unix epoch, taken from the current owner's
    /// authorization. Both signatures cover it, so both are bound to the same
    /// window.
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct UnregisterLightningAddressRequest {
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct ListMetadataRequest {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub updated_after: Option<i64>,
}

#[macros::async_trait]
pub trait LnurlServerClient: Send + Sync {
    fn domain(&self) -> &str;
    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError>;
    async fn recover_lightning_address(
        &self,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError>;
    async fn register_lightning_address(
        &self,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError>;
    async fn transfer_lightning_address(
        &self,
        request: &TransferLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError>;
    async fn unregister_lightning_address(
        &self,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError>;
    async fn list_metadata(
        &self,
        request: &ListMetadataRequest,
    ) -> Result<ListMetadataResponse, LnurlServerError>;
}

/// Default `LnurlServerClient` implementation using `HttpClient` abstraction.
pub struct DefaultLnurlServerClient {
    http_client: Arc<dyn HttpClient>,
    domain: String,
    /// The configured domain as a URL to call and an authority to sign, split
    /// once here so no request can address one domain and sign another.
    parsed_domain: ParsedDomain,
    api_key: Option<String>,
    wallet: Arc<spark_wallet::SparkWallet>,
}

impl DefaultLnurlServerClient {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        domain: String,
        api_key: Option<String>,
        wallet: Arc<spark_wallet::SparkWallet>,
    ) -> Self {
        Self {
            http_client,
            parsed_domain: parse_domain(&domain),
            domain,
            api_key,
            wallet,
        }
    }

    /// The URL every route is appended to.
    fn base_url(&self) -> &str {
        &self.parsed_domain.base_url
    }

    /// The authority every signed message names.
    fn signed_domain(&self) -> &str {
        &self.parsed_domain.signed_domain
    }

    /// Get common headers for all requests (User-Agent and Authorization).
    fn get_common_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("User-Agent".to_string(), "breez-sdk-spark".to_string());
        if let Some(api_key) = &self.api_key {
            headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
        }
        headers
    }

    /// Get headers for POST/DELETE requests (includes Content-Type).
    fn get_post_headers(&self) -> HashMap<String, String> {
        let mut headers = self.get_common_headers();
        add_content_type_header(&mut headers, ContentType::Json);
        headers
    }

    /// Seconds since the Unix epoch, the unit every signed message and every
    /// `timestamp` field on the wire uses.
    fn now() -> Result<u64, LnurlServerError> {
        Ok(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| LnurlServerError::SigningError("invalid systemtime".to_string()))?
            .as_secs())
    }

    /// Sign one canonical message, hex-encoding the DER signature the way the
    /// server parses it.
    async fn sign(&self, message: &str) -> Result<String, LnurlServerError> {
        let signature = self
            .wallet
            .sign_message(message)
            .await
            .map_err(|e| LnurlServerError::SigningError(e.to_string()))?;
        Ok(signature.serialize_der().to_lower_hex_string())
    }

    /// Handle response status and parse JSON
    fn handle_response<T: serde::de::DeserializeOwned>(
        status: u16,
        body: &str,
    ) -> Result<T, LnurlServerError> {
        match status {
            401 => Err(LnurlServerError::InvalidApiKey),
            s if (200..300).contains(&s) => serde_json::from_str(body).map_err(|e| {
                LnurlServerError::RequestFailure(format!(
                    "failed to deserialize response json: {e}"
                ))
            }),
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(body.to_string()),
            }),
        }
    }
}

#[macros::async_trait]
impl LnurlServerClient for DefaultLnurlServerClient {
    fn domain(&self) -> &str {
        &self.domain
    }

    async fn check_username_available(&self, username: &str) -> Result<bool, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let timestamp = Self::now()?;
        let signature = self
            .sign(&signed_message::available(
                self.signed_domain(),
                &pubkey.to_string(),
                username,
                timestamp,
            ))
            .await?;

        // Signed so the answer is this wallet's own: a username it gave up is
        // held for it and available to no one else, which the unsigned route
        // cannot say without naming who holds a name.
        let request = CheckUsernameAvailableRequest {
            username: username.to_string(),
            signature,
            timestamp,
        };
        let url = format!("{}/lnurlpay/{}/available", self.base_url(), pubkey);
        let body = serde_json::to_string(&request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let result: CheckUsernameAvailableResponse =
            Self::handle_response(response.status, &response.body)?;
        Ok(result.available)
    }

    async fn recover_lightning_address(
        &self,
    ) -> Result<Option<RecoverLnurlPayResponse>, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let timestamp = Self::now()?;
        let signature = self
            .sign(&signed_message::recover(
                self.signed_domain(),
                &pubkey.to_string(),
                timestamp,
            ))
            .await?;

        let request = RecoverLnurlPayRequest {
            signature,
            timestamp,
        };
        let url = format!("{}/lnurlpay/{}/recover", self.base_url(), pubkey);
        let body = serde_json::to_string(&request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        match response.status {
            401 => Err(LnurlServerError::InvalidApiKey),
            404 => Ok(None),
            s if (200..300).contains(&s) => {
                let result = serde_json::from_str(&response.body).map_err(|e| {
                    LnurlServerError::RequestFailure(format!(
                        "failed to deserialize response json: {e}"
                    ))
                })?;
                Ok(Some(result))
            }
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(response.body),
            }),
        }
    }

    async fn register_lightning_address(
        &self,
        request: &RegisterLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let timestamp = Self::now()?;
        let signature = self
            .sign(&signed_message::register(
                self.signed_domain(),
                &request.username,
                &request.description,
                timestamp,
            ))
            .await?;
        let api_request = RegisterLnurlPayRequest {
            username: request.username.clone(),
            description: request.description.clone(),
            signature,
            timestamp,
        };
        let url = format!("{}/lnurlpay/{}", self.base_url(), pubkey);
        let body = serde_json::to_string(&api_request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        Self::handle_response(response.status, &response.body)
    }

    async fn transfer_lightning_address(
        &self,
        request: &TransferLightningAddressRequest,
    ) -> Result<RegisterLnurlPayResponse, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        // Signed with the current owner's timestamp, not a fresh one: the two
        // signatures authorize one transfer and must name the same instant.
        let to_signature = self
            .sign(&signed_message::transfer_to(
                self.signed_domain(),
                &request.username,
                &request.from_pubkey,
                &pubkey.to_string(),
                &request.description,
                request.timestamp,
            ))
            .await?;
        let api_request = TransferLnurlPayRequest {
            username: request.username.clone(),
            description: request.description.clone(),
            from_pubkey: request.from_pubkey.clone(),
            from_signature: request.from_signature.clone(),
            to_signature,
            timestamp: Some(request.timestamp),
        };
        let url = format!("{}/lnurlpay/{}/transfer", self.base_url(), pubkey);
        let body = serde_json::to_string(&api_request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .post(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        Self::handle_response(response.status, &response.body)
    }

    async fn unregister_lightning_address(
        &self,
        request: &UnregisterLightningAddressRequest,
    ) -> Result<(), LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let timestamp = Self::now()?;
        let signature = self
            .sign(&signed_message::unregister(
                self.signed_domain(),
                &request.username,
                timestamp,
            ))
            .await?;

        let api_request = UnregisterLnurlPayRequest {
            username: request.username.clone(),
            signature,
            timestamp,
        };

        let url = format!("{}/lnurlpay/{}", self.base_url(), pubkey);
        let body = serde_json::to_string(&api_request)
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        let response = self
            .http_client
            .delete(url, Some(self.get_post_headers()), Some(body))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        match response.status {
            401 => Err(LnurlServerError::InvalidApiKey),
            404 => Ok(()),
            s if (200..300).contains(&s) => Ok(()),
            other => Err(LnurlServerError::Network {
                statuscode: other,
                message: Some(response.body),
            }),
        }
    }

    async fn list_metadata(
        &self,
        request: &ListMetadataRequest,
    ) -> Result<ListMetadataResponse, LnurlServerError> {
        let pubkey = self.wallet.get_identity_public_key();

        let timestamp = Self::now()?;
        let signature = self
            .sign(&signed_message::metadata(
                self.signed_domain(),
                &pubkey.to_string(),
                timestamp,
            ))
            .await?;

        let mut url = format!("{}/lnurlpay/{pubkey}/metadata", self.base_url());
        let mut separator = '?';
        for (name, value) in [
            ("offset", request.offset.map(|v| v.to_string())),
            ("limit", request.limit.map(|v| v.to_string())),
            (
                "updated_after",
                request.updated_after.map(|v| v.to_string()),
            ),
        ] {
            if let Some(value) = value {
                let _ = write!(url, "{separator}{name}={value}");
                separator = '&';
            }
        }

        // Headers, never the query string: the response carries preimages, and a
        // GET's query string lands in proxy and access logs. Costs a CORS
        // preflight round trip on wasm web, which the router already allows.
        let mut headers = self.get_common_headers();
        headers.insert(SIGNATURE_HEADER.to_string(), signature);
        headers.insert(TIMESTAMP_HEADER.to_string(), timestamp.to_string());

        let response = self
            .http_client
            .get(url, Some(headers))
            .await
            .map_err(|e| LnurlServerError::RequestFailure(e.to_string()))?;

        Self::handle_response(response.status, &response.body)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_domain;

    /// The URL a request is addressed to and the authority its signature names,
    /// for every shape a configured domain arrives in. Checked as a pair: the
    /// signature only verifies if the domain matches the `Host` the request URL
    /// resolves to, so one being right is worth nothing without the other.
    ///
    /// The URL is the parse's normalized form, so a configured host may come
    /// back lowercased and without a port its scheme implies. Both address the
    /// same server and produce the same `Host`.
    #[test]
    fn a_configured_domain_yields_a_request_url_and_the_authority_it_resolves_to() {
        for (configured, base_url, signed_domain) in [
            (
                "https://lnurl.breez.technology",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            (
                "lnurl.breez.technology",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            (
                "http://localhost:8080",
                "http://localhost:8080",
                "localhost:8080",
            ),
            // The server lowercases the authority it resolves, and so does the
            // `Host` the HTTP client derives from the URL, so this must too.
            (
                "https://LNURL.Breez.Technology",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            // A port the scheme already implies is absent from `Host`, so it
            // must be absent from the signed authority too.
            (
                "https://lnurl.breez.technology:443",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            ("http://localhost:80", "http://localhost", "localhost"),
            // A non-default port does reach `Host`.
            (
                "https://lnurl.breez.technology:8443",
                "https://lnurl.breez.technology:8443",
                "lnurl.breez.technology:8443",
            ),
            // Only the scheme's own default is redundant.
            (
                "https://lnurl.breez.technology:80",
                "https://lnurl.breez.technology:80",
                "lnurl.breez.technology:80",
            ),
            // A deployment mounted under a path prefix: the routes hang off it,
            // and none of it is part of the authority.
            (
                "https://lnurl.breez.technology/base",
                "https://lnurl.breez.technology/base",
                "lnurl.breez.technology",
            ),
            // A trailing slash would double the one every route starts with.
            (
                "lnurl.breez.technology/",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            // A query or fragment would swallow the route appended to it.
            (
                "https://lnurl.breez.technology?a=1",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            (
                "https://lnurl.breez.technology#frag",
                "https://lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            // Userinfo stays in the URL and out of the signed authority.
            (
                "https://user@lnurl.breez.technology",
                "https://user@lnurl.breez.technology",
                "lnurl.breez.technology",
            ),
            (
                "https://user:pw@lnurl.breez.technology:8443/base?a=1",
                "https://user:pw@lnurl.breez.technology:8443/base",
                "lnurl.breez.technology:8443",
            ),
            // A bracketed IPv6 literal, with and without a port to strip.
            ("http://[::1]", "http://[::1]", "[::1]"),
            ("https://[::1]:443", "https://[::1]", "[::1]"),
            ("http://[::1]:8080", "http://[::1]:8080", "[::1]:8080"),
        ] {
            let parsed = parse_domain(configured);
            assert_eq!(parsed.base_url, base_url, "base_url: {configured}");
            assert_eq!(
                parsed.signed_domain, signed_domain,
                "signed_domain: {configured}"
            );
        }
    }
}
