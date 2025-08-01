use anyhow::Result;
use hickory_resolver::TokioResolver;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;

use super::DnsResolver;

pub struct Resolver {
    resolver: TokioResolver,
}

impl Resolver {
    pub fn new() -> Self {
        let mut opts = ResolverOpts::default();
        opts.validate = true;

        let resolver = TokioResolver::builder_with_config(
            ResolverConfig::default(),
            TokioConnectionProvider::default(),
        )
        .with_options(opts)
        .build();

        Self { resolver }
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

#[breez_sdk_macros::async_trait]
impl DnsResolver for Resolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>> {
        let txt_lookup = self.resolver.txt_lookup(dns_name).await?;
        let records: Vec<String> = txt_lookup
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        Ok(records)
    }
}
