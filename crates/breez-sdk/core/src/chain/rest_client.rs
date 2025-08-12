use std::{sync::OnceLock, time::Duration};

use bitcoin::{Address, address::NetworkUnchecked};
use reqwest::Response;
use tracing::info;

use crate::{
    Network,
    chain::{ChainServiceError, Utxo},
};

use super::BitcoinChainService;

pub const RETRYABLE_ERROR_CODES: [u16; 3] = [
    429, // TOO_MANY_REQUESTS
    500, // INTERNAL_SERVER_ERROR
    503, // SERVICE_UNAVAILABLE
];

/// Base backoff in milliseconds.
const BASE_BACKOFF_MILLIS: Duration = Duration::from_millis(256);

pub struct BasicAuth {
    username: String,
    password: String,
}

impl BasicAuth {
    pub fn new(username: String, password: String) -> Self {
        Self { username, password }
    }
}

pub struct RestClientChainService {
    base_url: String,
    network: Network,
    client: OnceLock<reqwest::Client>,
    max_retries: usize,
    basic_auth: Option<BasicAuth>,
}

impl RestClientChainService {
    pub fn new(
        base_url: String,
        network: Network,
        max_retries: usize,
        basic_auth: Option<BasicAuth>,
    ) -> Self {
        Self {
            base_url,
            network,
            client: OnceLock::new(),
            max_retries,
            basic_auth,
        }
    }

    fn get_client(&self) -> Result<&reqwest::Client, ChainServiceError> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }

        let client = reqwest::ClientBuilder::new()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let client = self.client.get_or_init(|| client);
        Ok(client)
    }

    async fn get_response_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ChainServiceError> {
        let url = format!("{}{}", self.base_url, path);
        info!("Fetching response json from {}", url);
        let response = self.get_with_retry(&url, self.get_client()?).await?;

        if !response.status().is_success() {
            return Err(ChainServiceError::HttpError {
                status: response.status().as_u16(),
                message: response.text().await?,
            });
        }

        response
            .json::<T>()
            .await
            .map_err(|e| ChainServiceError::GenericError(e.to_string()))
    }

    async fn get_response_text(&self, path: &str) -> Result<String, ChainServiceError> {
        let url = format!("{}{}", self.base_url, path);
        info!("Fetching response text from {}", url);
        let response = self.get_with_retry(&url, self.get_client()?).await?;

        if !response.status().is_success() {
            return Err(ChainServiceError::HttpError {
                status: response.status().as_u16(),
                message: response.text().await?,
            });
        }

        response
            .text()
            .await
            .map_err(|e| ChainServiceError::GenericError(e.to_string()))
    }

    async fn get_with_retry(
        &self,
        url: &str,
        client: &reqwest::Client,
    ) -> Result<Response, reqwest::Error> {
        let mut delay = BASE_BACKOFF_MILLIS;
        let mut attempts = 0;

        loop {
            let mut request = client.get(url);
            if let Some(basic_auth) = &self.basic_auth {
                request = request.basic_auth(&basic_auth.username, Some(&basic_auth.password));
            }
            match request.send().await? {
                resp if attempts < self.max_retries && is_status_retryable(resp.status()) => {
                    tokio::time::sleep(delay).await;
                    attempts += 1;
                    delay *= 2;
                }
                resp => return Ok(resp),
            }
        }
    }
}

#[async_trait::async_trait]
impl BitcoinChainService for RestClientChainService {
    async fn get_address_utxos(&self, address: &str) -> Result<Vec<Utxo>, ChainServiceError> {
        let address = address
            .parse::<Address<NetworkUnchecked>>()?
            .require_network(self.network.clone().try_into()?)?;

        let utxos = self
            .get_response_json::<Vec<Utxo>>(format!("/address/{address}/utxo").as_str())
            .await?;

        Ok(utxos)
    }

    async fn get_transaction_hex(&self, txid: &str) -> Result<String, ChainServiceError> {
        let tx = self
            .get_response_text(format!("/tx/{txid}/hex").as_str())
            .await?;
        Ok(tx)
    }
}

impl From<reqwest::Error> for ChainServiceError {
    fn from(value: reqwest::Error) -> Self {
        ChainServiceError::GenericError(value.to_string())
    }
}

fn is_status_retryable(status: reqwest::StatusCode) -> bool {
    RETRYABLE_ERROR_CODES.contains(&status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;
    use mockito::Server;

    #[tokio::test]
    async fn test_get_address_utxos() {
        // Create a mock server
        let mut server = Server::new_async().await;

        // Mock JSON response from the actual API call
        let mock_response = r#"[
            {
                "txid": "277bbdc3557f163810feea810bf390ed90724ec75de779ab181b865292bb1dc1",
                "vout": 3,
                "status": {
                    "confirmed": true,
                    "block_height": 725850,
                    "block_hash": "00000000000000000002d5aace1354d3f5420fcabf4e931f1c4c7ae9c0b405f8",
                    "block_time": 1646382740
                },
                "value": 24201
            },
            {
                "txid": "3a3774433c15d8c1791806d25043335c2a53e5c0ed19517defa4dba9d0b2019f",
                "vout": 0,
                "status": {
                    "confirmed": true,
                    "block_height": 840719,
                    "block_hash": "0000000000000000000170deaa4ccf2de2f1c94346dfef40318d0a7c5178ffd3",
                    "block_time": 1713994081
                },
                "value": 30236
            },
            {
                "txid": "5f2712d4ab1c9aa09c82c28e881724dc3c8c85cbbe71692e593f3911296d40fd",
                "vout": 74,
                "status": {
                    "confirmed": true,
                    "block_height": 726892,
                    "block_hash": "0000000000000000000841798eb13e9230c11f508121e6e1ba25fff3ad3bc448",
                    "block_time": 1647033214
                },
                "value": 5155
            },
            {
                "txid": "7cb4410874b99055fda468dbca45b20ed910909641b46d9fb86869d560c462de",
                "vout": 0,
                "status": {
                    "confirmed": true,
                    "block_height": 857808,
                    "block_hash": "0000000000000000000286598ae217ea4e5b3c63359f3fe105106556182cb926",
                    "block_time": 1724272387
                },
                "value": 6127
            },
            {
                "txid": "4654a83d953c68ba2c50473a80921bb4e1f01d428b18c65ff0128920865cc314",
                "vout": 126,
                "status": {
                    "confirmed": true,
                    "block_height": 748177,
                    "block_hash": "00000000000000000004a65956b7e99b3fcdfb1c01a9dfe5d6d43618427116be",
                    "block_time": 1659763398
                },
                "value": 22190
            }
        ]"#;

        // Mock the API endpoint
        let mock = server
            .mock("GET", "/address/1wiz18xYmhRX6xStj2b9t1rwWX4GKUgpv/utxo")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        // Create the service with the mock server URL
        let service = RestClientChainService::new(server.url(), Network::Mainnet, 3, None);

        // Call the method under test
        let mut result = service
            .get_address_utxos("1wiz18xYmhRX6xStj2b9t1rwWX4GKUgpv")
            .await
            .unwrap();

        // Verify the mock was called
        mock.assert_async().await;

        // Sort results by value for consistent testing
        result.sort_by(|a, b| a.value.cmp(&b.value));

        // Verify we got the expected number of UTXOs
        assert_eq!(result.len(), 5);

        // Verify the UTXOs are correctly parsed and sorted by value
        assert_eq!(result[0].value, 5155); // Smallest value
        assert_eq!(
            result[0].txid,
            "5f2712d4ab1c9aa09c82c28e881724dc3c8c85cbbe71692e593f3911296d40fd"
        );
        assert_eq!(result[0].vout, 74);
        assert!(result[0].status.confirmed);
        assert_eq!(result[0].status.block_height, Some(726892));

        assert_eq!(result[1].value, 6127);
        assert_eq!(
            result[1].txid,
            "7cb4410874b99055fda468dbca45b20ed910909641b46d9fb86869d560c462de"
        );

        assert_eq!(result[2].value, 22190);
        assert_eq!(result[3].value, 24201);
        assert_eq!(result[4].value, 30236); // Largest value

        // Verify all UTXOs are confirmed
        for utxo in &result {
            assert!(utxo.status.confirmed);
            assert!(utxo.status.block_height.is_some());
            assert!(utxo.status.block_time.is_some());
        }
    }

    #[tokio::test]
    async fn test_get_address_utxos_invalid_address() {
        let server = Server::new_async().await;

        let service = RestClientChainService::new(server.url(), Network::Mainnet, 3, None);

        // Test with invalid address format
        let result = service.get_address_utxos("invalid_address_format").await;

        // Should return an error for invalid address
        assert!(result.is_err());
    }
}
