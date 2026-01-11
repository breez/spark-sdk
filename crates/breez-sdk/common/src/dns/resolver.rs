use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use dnssec_prover::query::build_txt_proof_async;

use super::{DnsResolver, normalize_dns_name, parse_dns_name, verify_proof_and_extract_txt};

/// Default DNS resolver address (Cloudflare's public DNS)
const DEFAULT_RESOLVER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);

pub struct Resolver {
    resolver_addr: SocketAddr,
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            resolver_addr: DEFAULT_RESOLVER,
        }
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

        // Build the DNSSEC proof by querying the resolver
        let (proof, _ttl) = build_txt_proof_async(self.resolver_addr, &name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build DNSSEC proof: {}", e))?;

        verify_proof_and_extract_txt(&proof, &name)
    }
}
