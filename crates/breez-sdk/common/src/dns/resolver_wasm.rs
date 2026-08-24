use anyhow::{Result, anyhow};
use platform_utils::ProxyConfig;
use reqwest::Client;

use super::{DnsResolver, doh, normalize_dns_name, parse_dns_name, verify_proof_and_extract_txt};

pub struct Resolver {
    /// A failed build is carried rather than unwrapped: this type is
    /// constructed during `connect`, where a panic aborts the wasm module.
    client: Result<Client, String>,
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            client: doh::build_client(None).map_err(|e| e.to_string()),
        }
    }

    /// `proxy` must be `None`: browser fetch offers no proxy control, so the
    /// lookup cannot be tunnelled and must not silently go direct.
    pub fn with_proxy(proxy: Option<&ProxyConfig>) -> Result<Self> {
        if proxy.is_some() {
            return Err(anyhow!(
                "a SOCKS5 proxy cannot be honoured on WASM: fetch exposes no proxy control"
            ));
        }
        Ok(Self {
            client: Ok(doh::build_client(None)?),
        })
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

        let client = self
            .client
            .as_ref()
            .map_err(|e| anyhow!("Failed to build DoH client: {e}"))?;
        let proof = doh::build_proof(client, &name).await?;

        verify_proof_and_extract_txt(&proof, &name)
    }
}
