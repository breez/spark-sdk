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
use platform_utils::{ProxyConfig, REQUEST_TIMEOUT, read_capped_bytes};
use reqwest::Client;
use std::time::Duration;

const DOH_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";

/// Maximum `DoH` response the resolver will buffer.
///
/// A DNS message cannot exceed 65535 bytes, so this only refuses a resolver
/// answering with something that is not one.
const MAX_DOH_RESPONSE_BYTES: usize = 64 * 1024;

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
            let response_bytes = send_query(client, DOH_ENDPOINT, query.as_ref()).await?;

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

async fn send_query(client: &Client, endpoint: &str, query: &[u8]) -> Result<Vec<u8>> {
    let encoded_query = URL_SAFE_NO_PAD.encode(query);

    // The deadline goes on the request, not the client: WASM's `ClientBuilder`
    // has no timeout knob, while its `RequestBuilder` arms an `AbortController`.
    let response = client
        .get(format!("{endpoint}?dns={encoded_query}"))
        .header("Accept", "application/dns-message")
        .timeout(Duration::from_secs(REQUEST_TIMEOUT))
        .send()
        .await?
        .error_for_status()?;

    Ok(read_capped_bytes(response, MAX_DOH_RESPONSE_BYTES).await?)
}

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{MAX_DOH_RESPONSE_BYTES, build_client, send_query};

    /// Size of each body write the one-shot server makes.
    const BLOCK: usize = 16 * 1024;

    /// Serves one response of `body_len` filler bytes, chunked so there is no
    /// `Content-Length` for the cap to short-circuit on, and returns its URL.
    async fn serve_once(body_len: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}/dns-query", listener.local_addr().expect("addr"));

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                match socket.read(&mut byte).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => request.push(byte[0]),
                }
            }
            let head = "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\n\
                        Transfer-Encoding: chunked\r\n\r\n";
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let mut sent = 0usize;
            while sent < body_len {
                let len = BLOCK.min(body_len.saturating_sub(sent));
                let mut frame = format!("{len:x}\r\n").into_bytes();
                frame.extend_from_slice(&vec![b'a'; len]);
                frame.extend_from_slice(b"\r\n");
                if socket.write_all(&frame).await.is_err() {
                    return;
                }
                sent = sent.saturating_add(len);
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });

        url
    }

    #[tokio::test]
    async fn refuses_an_oversized_resolver_response() {
        let url = serve_once(MAX_DOH_RESPONSE_BYTES.saturating_mul(4)).await;
        let client = build_client(None).expect("client");

        let err = send_query(&client, &url, b"query")
            .await
            .expect_err("an oversized answer should be refused");
        assert!(
            format!("{err:?}").contains("byte limit"),
            "expected the body-limit error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn reads_a_response_within_the_cap() {
        let url = serve_once(1024).await;
        let client = build_client(None).expect("client");

        let body = send_query(&client, &url, b"query").await.expect("accepted");
        assert_eq!(body.len(), 1024);
    }
}
