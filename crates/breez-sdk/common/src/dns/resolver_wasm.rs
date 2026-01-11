use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dnssec_prover::query::{ProofBuilder, QueryBuf};
use dnssec_prover::rr::Name;
use reqwest::Client;

use super::{normalize_dns_name, parse_dns_name, verify_proof_and_extract_txt, DnsResolver};

const DOH_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";

pub struct Resolver;

impl Resolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

#[macros::async_trait]
impl DnsResolver for Resolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>> {
        let dns_name = normalize_dns_name(dns_name);
        let name = parse_dns_name(&dns_name)?;

        // Build DNSSEC proof using DoH
        let proof = build_proof_doh(&name).await?;

        verify_proof_and_extract_txt(&proof, &name)
    }
}

/// Build a DNSSEC proof using DNS-over-HTTPS queries
async fn build_proof_doh(name: &Name) -> Result<Vec<u8>> {
    let client = Client::builder().build()?;

    // TXT record type = 16
    let (mut builder, initial_query) = ProofBuilder::new(name, 16);

    // Send initial query
    let mut pending_queries = vec![initial_query];

    while builder.awaiting_responses() {
        if pending_queries.is_empty() {
            anyhow::bail!("ProofBuilder awaiting responses but no queries to send");
        }

        // Process each pending query
        let mut new_queries = Vec::new();
        for query in pending_queries {
            // Send the query via DoH
            let response_bytes = send_doh_query(&client, query.as_ref()).await?;

            // Convert response bytes to QueryBuf
            let mut response_buf = QueryBuf::new_zeroed(0);
            response_buf.extend_from_slice(&response_bytes);

            // Process the response and collect new queries
            let queries = builder
                .process_response(&response_buf)
                .map_err(|e| anyhow!("Failed to process DNS response: {:?}", e))?;
            new_queries.extend(queries);
        }
        pending_queries = new_queries;
    }

    // Finish and return the proof
    let (proof, _ttl) = builder
        .finish_proof()
        .map_err(|e| anyhow!("Failed to finish DNSSEC proof: {:?}", e))?;

    Ok(proof)
}

/// Send a DNS query via DNS-over-HTTPS
async fn send_doh_query(client: &Client, query: &[u8]) -> Result<Vec<u8>> {
    // Base64url encode the query for GET request
    let encoded_query = URL_SAFE_NO_PAD.encode(query);

    let response = client
        .get(format!("{}?dns={}", DOH_ENDPOINT, encoded_query))
        .header("Accept", "application/dns-message")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    Ok(response.to_vec())
}
