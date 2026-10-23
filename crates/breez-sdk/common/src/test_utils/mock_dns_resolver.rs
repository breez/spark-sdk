use anyhow::Result;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tracing::debug;

use crate::dns::DnsResolver;

#[derive(Clone, Default)]
pub struct MockDnsResolver {
    responses: Arc<Mutex<VecDeque<Vec<String>>>>,
    queried_names: Arc<Mutex<Vec<String>>>,
}

impl MockDnsResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_response(&self, response: Vec<String>) -> &Self {
        debug!("Push response: {response:?}");
        let mut responses = self.responses.lock().unwrap();
        responses.push_back(response);
        self
    }

    /// The names of all TXT lookups so far, in order.
    pub fn queried_names(&self) -> Vec<String> {
        self.queried_names.lock().unwrap().clone()
    }
}

#[macros::async_trait]
impl DnsResolver for MockDnsResolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>> {
        self.queried_names.lock().unwrap().push(dns_name.clone());
        let mut responses = self.responses.lock().unwrap();
        let response = responses
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("No response available for DNS lookup"))?;
        debug!("Pop TXT response for {dns_name}: {response:?}");

        Ok(response)
    }
}
