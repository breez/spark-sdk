use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use dnssec_prover::query::build_txt_proof_async;
use dnssec_prover::rr::{Name, RR};
use dnssec_prover::ser::parse_rr_stream;
use dnssec_prover::validation::verify_rr_stream;

use super::DnsResolver;

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
        // Parse the domain name
        let name = Name::try_from(dns_name.as_str())
            .map_err(|()| anyhow::anyhow!("Invalid DNS name: {}", dns_name))?;

        // Build the DNSSEC proof by querying the resolver
        let (proof, _ttl) = build_txt_proof_async(self.resolver_addr, &name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build DNSSEC proof: {}", e))?;

        // Parse the proof into resource records
        let rrs =
            parse_rr_stream(&proof).map_err(|()| anyhow::anyhow!("Failed to parse DNS proof"))?;

        // Verify the DNSSEC chain
        let verified = verify_rr_stream(&rrs)
            .map_err(|e| anyhow::anyhow!("DNSSEC verification failed: {:?}", e))?;

        // Check that the proof is currently valid
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if now < verified.valid_from || now > verified.expires {
            anyhow::bail!(
                "DNSSEC proof is not currently valid (valid from {} to {}, current time {})",
                verified.valid_from,
                verified.expires,
                now
            );
        }

        // Extract TXT records from verified records
        let txt_records: Vec<String> = verified
            .verified_rrs
            .into_iter()
            .filter_map(|rr| {
                if let RR::Txt(txt) = rr {
                    Some(String::from_utf8_lossy(&txt.data.as_vec()).into_owned())
                } else {
                    None
                }
            })
            .collect();

        Ok(txt_records)
    }
}
