//! Request binding: the digest an invoice's description hash commits to.

use bitcoin::hashes::{Hash, sha256};
use serde_json::{Map, Value, json};

use super::{
    error::{X402Error, X402ErrorReason, X402Result},
    jcs,
};

pub const HTTP_PROFILE: &str = "http:1";
pub const MCP_PROFILE: &str = "mcp:1";
const DOMAIN_PREFIX: &str = "x402:exact:lnbtc:bolt11:";
const PAYMENT_SIGNATURE_HEADER_NAME: &str = "payment-signature";
const EXCLUDED_METADATA: [&str; 2] = ["x402/payment", "progressToken"];

/// An HTTP request as the client sends it or the server receives it.
pub struct HttpRequest<'a> {
    pub method: &'a str,
    /// Absolute target URL, exactly as sent (no normalization).
    pub url: &'a str,
    /// Content bytes after transfer decoding, before content decoding.
    pub body: &'a [u8],
    /// Field lines in arrival order. Names are matched case-insensitively.
    pub headers: &'a [(String, String)],
}

/// An MCP `tools/call` as the client sends it.
pub struct McpToolCall<'a> {
    /// The MCP server URI, taken from configuration: the server the client
    /// intends to call, or the server's own.
    pub server: &'a str,
    /// The call's `params` object (`name`, `arguments`, `_meta`).
    pub params: &'a Value,
}

/// Validated `extra.requestBindingParams` for a profile.
#[derive(Debug, PartialEq, Eq)]
pub enum BindingParams {
    Http {
        headers: Vec<String>,
    },
    Mcp {
        server: String,
        metadata: Vec<String>,
    },
}

impl BindingParams {
    /// Validates the parameters for `profile`. Unknown profiles, missing or
    /// extra members, and malformed values all fail.
    pub fn parse(profile: &str, params: &Value) -> X402Result<Self> {
        let Value::Object(map) = params else {
            return Err(binding_error("requestBindingParams is not an object"));
        };
        match profile {
            HTTP_PROFILE => {
                expect_members(map, &["headers"])?;
                let headers = string_array(&map["headers"], "headers")?;
                validate_bound_header_names(&headers)?;
                Ok(Self::Http { headers })
            }
            MCP_PROFILE => {
                expect_members(map, &["metadata", "server"])?;
                let Value::String(server) = &map["server"] else {
                    return Err(binding_error("server is not a string"));
                };
                validate_uri(server, false)?;
                let metadata = string_array(&map["metadata"], "metadata")?;
                validate_metadata_names(&metadata)?;
                Ok(Self::Mcp {
                    server: server.clone(),
                    metadata,
                })
            }
            other => Err(binding_error(format!("unsupported profile {other:?}"))),
        }
    }
}

/// Parses a `tools/call` `params` object from JSON text. Duplicate member
/// names are refused, since the request hash could not represent them
/// faithfully.
pub fn parse_tool_call_params(json: &str) -> X402Result<Value> {
    jcs::parse_strict(json).map_err(|e| binding_error(e.0))
}

/// Bound HTTP header names as the binding takes them: lowercase, sorted and
/// unique. A name that can't be bound is refused.
pub fn normalize_bound_header_names(names: &[String]) -> X402Result<Vec<String>> {
    let mut names: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    names.sort();
    names.dedup();
    validate_bound_header_names(&names)?;
    Ok(names)
}

/// Bound MCP `_meta` member names as the binding takes them: sorted in JCS
/// order and unique. A name that can't be bound is refused.
pub fn normalize_bound_metadata_names(names: &[String]) -> X402Result<Vec<String>> {
    let mut names = names.to_vec();
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    names.dedup();
    validate_metadata_names(&names)?;
    Ok(names)
}

/// Computes the `http:1` request hash.
pub fn http_request_hash(request: &HttpRequest, bound_headers: &[String]) -> X402Result<[u8; 32]> {
    Ok(sha256(http_description(request, bound_headers)?.as_bytes()))
}

/// Computes the `mcp:1` request hash.
pub fn mcp_request_hash(call: &McpToolCall, metadata: &[String]) -> X402Result<[u8; 32]> {
    Ok(sha256(mcp_description(call, metadata)?.as_bytes()))
}

/// The canonical `http:1` description whose SHA-256 is the request hash.
pub(crate) fn http_description(
    request: &HttpRequest,
    bound_headers: &[String],
) -> X402Result<String> {
    validate_method(request.method)?;
    validate_uri(request.url, true)?;
    validate_bound_header_names(bound_headers)?;
    let headers = bound_headers
        .iter()
        .map(|name| {
            let value_hash = match combined_field_value(request.headers, name)? {
                Some(value) => tagged_hash(1, value.as_bytes()),
                None => tagged_hash(0, &[]),
            };
            Ok(json!({ "name": name, "valueHash": value_hash }))
        })
        .collect::<X402Result<Vec<_>>>()?;
    let binding = json!({
        "domain": format!("{DOMAIN_PREFIX}{HTTP_PROFILE}"),
        "method": request.method,
        "url": request.url,
        "bodyHash": hex::encode(sha256(request.body)),
        "headers": headers,
    });
    jcs::canonicalize(&binding).map_err(|e| binding_error(e.0))
}

/// The canonical `mcp:1` description whose SHA-256 is the request hash.
pub(crate) fn mcp_description(call: &McpToolCall, metadata: &[String]) -> X402Result<String> {
    validate_uri(call.server, false)?;
    validate_metadata_names(metadata)?;
    let Value::Object(params) = call.params else {
        return Err(binding_error("params is not an object"));
    };
    let name = match params.get("name") {
        Some(Value::String(name)) if !name.is_empty() => name,
        _ => return Err(binding_error("params.name is not a non-empty string")),
    };
    let arguments = match params.get("arguments") {
        None => Value::Object(Map::new()),
        Some(Value::Object(arguments)) => Value::Object(arguments.clone()),
        Some(_) => return Err(binding_error("params.arguments is not an object")),
    };
    let meta = match params.get("_meta") {
        None => Map::new(),
        Some(Value::Object(meta)) => meta.clone(),
        Some(_) => return Err(binding_error("params._meta is not an object")),
    };
    let metadata = metadata
        .iter()
        .map(|name| {
            let value_hash = match meta.get(name) {
                Some(value) => {
                    let canonical = jcs::canonicalize(value).map_err(|e| binding_error(e.0))?;
                    tagged_hash(1, canonical.as_bytes())
                }
                None => tagged_hash(0, &[]),
            };
            Ok(json!({ "name": name, "valueHash": value_hash }))
        })
        .collect::<X402Result<Vec<_>>>()?;
    let binding = json!({
        "domain": format!("{DOMAIN_PREFIX}{MCP_PROFILE}"),
        "server": call.server,
        "method": "tools/call",
        "name": name,
        "arguments": arguments,
        "metadata": metadata,
    });
    jcs::canonicalize(&binding).map_err(|e| binding_error(e.0))
}

fn binding_error(detail: impl Into<String>) -> X402Error {
    X402Error::new(X402ErrorReason::RequestBinding, detail)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    sha256::Hash::hash(bytes).to_byte_array()
}

/// SHA-256 of `tag || bytes`, hex encoded. Tag 1 marks a present value and tag
/// 0 an absent one, so an empty value and a missing one hash differently.
fn tagged_hash(tag: u8, bytes: &[u8]) -> String {
    let mut input = Vec::with_capacity(bytes.len().saturating_add(1));
    input.push(tag);
    input.extend_from_slice(bytes);
    hex::encode(sha256(&input))
}

fn expect_members(map: &Map<String, Value>, expected: &[&str]) -> X402Result<()> {
    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
    keys.sort_unstable();
    if keys != expected {
        return Err(binding_error(format!(
            "requestBindingParams members are {keys:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

fn string_array(value: &Value, field: &str) -> X402Result<Vec<String>> {
    let Value::Array(items) = value else {
        return Err(binding_error(format!("{field} is not an array")));
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(s) => Ok(s.clone()),
            _ => Err(binding_error(format!("{field} contains a non-string"))),
        })
        .collect()
}

/// Bound header names are lowercase field-name tokens, strictly ascending in
/// ASCII byte order, and never the payment header itself.
fn validate_bound_header_names(names: &[String]) -> X402Result<()> {
    for name in names {
        if name.is_empty() || !name.bytes().all(|b| is_tchar(b) && !b.is_ascii_uppercase()) {
            return Err(binding_error(format!(
                "bound header {name:?} is not a lowercase token"
            )));
        }
        if name == PAYMENT_SIGNATURE_HEADER_NAME {
            return Err(binding_error("payment-signature cannot be bound"));
        }
    }
    if names.windows(2).any(|w| w[0].as_bytes() >= w[1].as_bytes()) {
        return Err(binding_error("bound headers are not sorted and unique"));
    }
    Ok(())
}

/// Bound metadata names are non-empty, strictly ascending in JCS (UTF-16)
/// order, and exclude the payment and progress members.
fn validate_metadata_names(names: &[String]) -> X402Result<()> {
    for name in names {
        if name.is_empty() {
            return Err(binding_error("bound metadata name is empty"));
        }
        if EXCLUDED_METADATA.contains(&name.as_str()) {
            return Err(binding_error(format!("{name} cannot be bound")));
        }
    }
    if names
        .windows(2)
        .any(|w| w[0].encode_utf16().cmp(w[1].encode_utf16()).is_ge())
    {
        return Err(binding_error("bound metadata is not sorted and unique"));
    }
    Ok(())
}

/// Combines a field's lines as RFC 9421 section 2.1 does: each value trimmed
/// of surrounding whitespace, then joined with ", ". Only visible ASCII,
/// space and tab are accepted.
fn combined_field_value(lines: &[(String, String)], name: &str) -> X402Result<Option<String>> {
    let mut values = Vec::new();
    for (line_name, value) in lines {
        if !line_name.eq_ignore_ascii_case(name) {
            continue;
        }
        if !value
            .bytes()
            .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
        {
            return Err(binding_error(format!(
                "header {name} has an unsupported value"
            )));
        }
        values.push(value.trim_matches([' ', '\t']));
    }
    Ok((!values.is_empty()).then(|| values.join(", ")))
}

fn validate_method(method: &str) -> X402Result<()> {
    if method.is_empty() || !method.bytes().all(is_tchar) {
        return Err(binding_error(format!("method {method:?} is not a token")));
    }
    Ok(())
}

/// RFC 9110 `tchar`.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Validates an absolute URI in ASCII syntax without user information or a
/// fragment. `http_only` also requires an `http` or `https` URL with a host.
/// The URI is checked, never normalized: the hash covers it as written.
fn validate_uri(uri: &str, http_only: bool) -> X402Result<()> {
    let invalid = |why: &str| binding_error(format!("URI {uri:?} {why}"));
    let bytes = uri.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let valid = bytes
                .get(i.saturating_add(1)..i.saturating_add(3))
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
            if !valid {
                return Err(invalid("has a malformed percent escape"));
            }
            i = i.saturating_add(3);
            continue;
        }
        if b == b'#' {
            return Err(invalid("has a fragment"));
        }
        if !(b.is_ascii_alphanumeric() || b"-._~:/?[]@!$&'()*+,;=".contains(&b)) {
            return Err(invalid("has a character outside URI syntax"));
        }
        i = i.saturating_add(1);
    }

    let (scheme, rest) = uri
        .split_once(':')
        .ok_or_else(|| invalid("is not absolute"))?;
    let scheme_valid = scheme
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
    if !scheme_valid {
        return Err(invalid("is not absolute"));
    }
    if http_only && scheme != "http" && scheme != "https" {
        return Err(invalid("is not an http or https URL"));
    }

    let brackets = |part: &str| part.contains(['[', ']']);
    let Some(after_slashes) = rest.strip_prefix("//") else {
        if http_only {
            return Err(invalid("has no authority"));
        }
        if brackets(rest) {
            return Err(invalid("has brackets outside an IP literal"));
        }
        return Ok(());
    };
    let authority_end = after_slashes
        .find(['/', '?'])
        .unwrap_or(after_slashes.len());
    let (authority, path_and_query) = after_slashes.split_at(authority_end);
    if brackets(path_and_query) {
        return Err(invalid("has brackets outside an IP literal"));
    }
    if authority.contains('@') {
        return Err(invalid("has user information"));
    }
    let (host, port) = if let Some(literal) = authority.strip_prefix('[') {
        let (host, after) = literal
            .split_once(']')
            .ok_or_else(|| invalid("has an unterminated IP literal"))?;
        if !is_ip_literal(host) {
            return Err(invalid("has a malformed IP literal"));
        }
        match after {
            "" => (host, None),
            _ => (
                host,
                Some(
                    after
                        .strip_prefix(':')
                        .ok_or_else(|| invalid("has a malformed authority"))?,
                ),
            ),
        }
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.contains(['[', ']']) {
            return Err(invalid("has brackets outside an IP literal"));
        }
        (host, port)
    };
    if http_only && host.is_empty() {
        return Err(invalid("has no host"));
    }
    // RFC 3986 allows an empty port. Port 0 is syntactically allowed but
    // cannot be served, so it is refused.
    if let Some(port) = port.filter(|port| !port.is_empty()) {
        let valid =
            port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok_and(|p| p > 0);
        if !valid {
            return Err(invalid("has an invalid port"));
        }
    }
    Ok(())
}

/// RFC 3986 `IP-literal` contents: an IPv6 address or `IPvFuture`.
fn is_ip_literal(host: &str) -> bool {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    let Some(future) = host.strip_prefix(['v', 'V']) else {
        return false;
    };
    let Some((version, address)) = future.split_once('.') else {
        return false;
    };
    !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_hexdigit())
        && !address.is_empty()
        && address
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::validate_uri;

    #[macros::test_all]
    fn normalizes_bound_names() {
        let names = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            super::normalize_bound_header_names(&names(&["Authorization", "accept", "Accept"]))
                .unwrap(),
            names(&["accept", "authorization"])
        );
        assert!(super::normalize_bound_header_names(&names(&["payment-signature"])).is_err());
        assert_eq!(
            super::normalize_bound_metadata_names(&names(&["traceId", "b", "traceId"])).unwrap(),
            names(&["b", "traceId"])
        );
    }

    #[macros::test_all]
    fn validates_the_authority_as_rfc_3986_does() {
        for uri in [
            "https://api.example.com/a",
            "https://api.example.com:/a",
            "https://[::1]:8443/a",
            "https://[v1.fe80::a+en1]/a",
        ] {
            assert!(validate_uri(uri, true).is_ok(), "{uri} was refused");
        }
        for uri in [
            "https://a]b/x",
            "https://[zz]/x",
            "https://[]/x",
            "https://api.example.com:0/a",
            "https://api.example.com:65536/a",
            "https://api.example.com/a[1]",
            "https://api.example.com/?a[]=1",
            "https://[::1]/?a[]=1",
        ] {
            assert!(validate_uri(uri, true).is_err(), "{uri} was accepted");
        }
        assert!(validate_uri("urn:a[1]", false).is_err());
    }
}
