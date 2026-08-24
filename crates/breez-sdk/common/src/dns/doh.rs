//! DNS-over-HTTPS transport for the `DNSSEC` proof builder.
//!
//! Used on WASM, where raw DNS sockets don't exist, and on native whenever a
//! proxy is configured: a plain DNS query would go out over UDP, outside the
//! SOCKS5 tunnel, and leak exactly the hostname the proxy is meant to hide.
//!
//! This talks to `reqwest` directly rather than through
//! [`HttpClient`](platform_utils::HttpClient) because `DoH` responses are binary
//! wire-format DNS, and that trait carries bodies as `String`. The proxy is
//! applied here so the exception can't become a leak.

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dnssec_prover::query::{ProofBuilder, QueryBuf};
use dnssec_prover::rr::Name;
use platform_utils::ProxyConfig;
use reqwest::Client;

const DOH_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";

/// TXT record type, per RFC 1035.
const RR_TYPE_TXT: u16 = 16;

/// Builds the `DoH` client, routing it through `proxy` when one is set.
///
/// `proxy` is unreachable on WASM: `reqwest` has no proxy support there, and
/// the WASM resolver rejects a proxy before calling this.
#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    expect(unused_variables)
)]
pub(super) fn build_client(proxy: Option<&ProxyConfig>) -> Result<Client> {
    // DoH responses are binary DNS wire format, which `HttpClient` cannot
    // carry, so this builds its own client and applies the proxy itself.
    #[allow(clippy::disallowed_methods)]
    let builder = Client::builder();
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    let builder = match proxy {
        Some(proxy) => builder.proxy(reqwest::Proxy::all(proxy.reqwest_url())?),
        None => builder,
    };
    Ok(builder.build()?)
}

/// Builds a `DNSSEC` proof for `name`'s TXT records over `DoH`.
///
/// The proof is verified by the caller, so a hostile resolver can withhold an
/// answer but cannot forge one.
pub(super) async fn build_proof(client: &Client, name: &Name) -> Result<Vec<u8>> {
    let (mut builder, initial_query) = ProofBuilder::new(name, RR_TYPE_TXT);
    let mut pending_queries = vec![initial_query];

    while builder.awaiting_responses() {
        if pending_queries.is_empty() {
            anyhow::bail!("ProofBuilder awaiting responses but no queries to send");
        }

        let mut new_queries = Vec::new();
        for query in pending_queries {
            let response_bytes = send_query(client, query.as_ref()).await?;

            let mut response_buf = QueryBuf::new_zeroed(0);
            response_buf.extend_from_slice(&response_bytes);

            let queries = builder
                .process_response(&response_buf)
                .map_err(|e| anyhow!("Failed to process DNS response: {e:?}"))?;
            new_queries.extend(queries);
        }
        pending_queries = new_queries;
    }

    let (proof, _ttl) = builder
        .finish_proof()
        .map_err(|e| anyhow!("Failed to finish DNSSEC proof: {e:?}"))?;

    Ok(proof)
}

async fn send_query(client: &Client, query: &[u8]) -> Result<Vec<u8>> {
    let encoded_query = URL_SAFE_NO_PAD.encode(query);

    let response = client
        .get(format!("{DOH_ENDPOINT}?dns={encoded_query}"))
        .header("Accept", "application/dns-message")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    Ok(response.to_vec())
}
