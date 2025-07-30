use anyhow::Result;
use std::{collections::VecDeque, sync::Mutex};

use crate::dns::DnsResolver;

#[derive(Default)]
pub struct MockDnsResolver {
    responses: Mutex<VecDeque<Vec<String>>>,
}

impl MockDnsResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_response(&self, response: Vec<String>) -> &Self {
        println!("Push response: {response:?}");
        let mut responses = self.responses.lock().unwrap();
        responses.push_back(response);
        self
    }
}

#[breez_sdk_macros::async_trait]
impl DnsResolver for MockDnsResolver {
    async fn txt_lookup(&self, dns_name: String) -> Result<Vec<String>> {
        let mut responses = self.responses.lock().unwrap();
        let response = responses
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("No response available for DNS lookup"))?;
        println!("Pop TXT response for {dns_name}: {response:?}");

        Ok(response)
    }
}
