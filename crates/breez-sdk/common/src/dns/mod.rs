#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    path = "resolver_wasm.rs"
)]
mod resolver;

use anyhow::Result;
pub use resolver::Resolver;

#[breez_sdk_macros::async_trait]
pub trait DnsResolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>>;
}
