use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use dnssec_prover::query::build_txt_proof_async;
use platform_utils::ProxyConfig;
use reqwest::Client;

use super::{DnsResolver, doh, normalize_dns_name, parse_dns_name, verify_proof_and_extract_txt};

/// Default DNS resolver address (Cloudflare's public DNS)
const DEFAULT_RESOLVER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);

/// How queries reach a resolver.
enum Transport {
    /// Plain DNS straight to `SocketAddr`.
    Plain(SocketAddr),
    /// DNS-over-HTTPS, so the query rides the same proxied TLS path as every
    /// other request instead of escaping over UDP.
    Doh(Client),
}

pub struct Resolver {
    transport: Transport,
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            transport: Transport::Plain(DEFAULT_RESOLVER),
        }
    }

    /// A resolver that respects `proxy`.
    ///
    /// With a proxy set this switches to `DoH`: plain DNS is UDP, which a SOCKS5
    /// proxy does not carry, so the query would bypass the tunnel and reveal
    /// the name being looked up. Without one, this is [`Self::new`].
    pub fn with_proxy(proxy: Option<&ProxyConfig>) -> Result<Self> {
        match proxy {
            Some(proxy) => Ok(Self {
                transport: Transport::Doh(doh::build_client(Some(proxy))?),
            }),
            None => Ok(Self::new()),
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

        let proof = match &self.transport {
            Transport::Plain(addr) => {
                let (proof, _ttl) = build_txt_proof_async(*addr, &name)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to build DNSSEC proof: {e}"))?;
                proof
            }
            Transport::Doh(client) => doh::build_proof(client, &name).await?,
        };

        verify_proof_and_extract_txt(&proof, &name)
    }
}
