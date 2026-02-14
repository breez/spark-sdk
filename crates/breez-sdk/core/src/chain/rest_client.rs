use bitcoin::{Address, address::NetworkUnchecked};
use platform_utils::{
    ContentType, HttpClient, HttpError, HttpResponse, add_basic_auth_header,
    add_content_type_header,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio_with_wasm::alias as tokio;
use tracing::info;

use crate::chain::RecommendedFees;
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

#[derive(Serialize, Deserialize, Clone)]
struct TxInfo {
    txid: String,
    status: super::TxStatus,
}

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
    client: Box<dyn HttpClient>,
    max_retries: usize,
    basic_auth: Option<BasicAuth>,
    api_type: ChainApiType,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ChainApiType {
    Esplora,
    MempoolSpace,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MempoolSpaceRecommendedFeesResponse {
    fastest_fee: f64,
    half_hour_fee: f64,
    hour_fee: f64,
    economy_fee: f64,
    minimum_fee: f64,
}

impl From<MempoolSpaceRecommendedFeesResponse> for RecommendedFees {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from(response: MempoolSpaceRecommendedFeesResponse) -> Self {
        Self {
            fastest_fee: response.fastest_fee.ceil() as u64,
            half_hour_fee: response.half_hour_fee.ceil() as u64,
            hour_fee: response.hour_fee.ceil() as u64,
            economy_fee: response.economy_fee.ceil() as u64,
            minimum_fee: response.minimum_fee.ceil() as u64,
        }
    }
}

impl RestClientChainService {
    pub fn new(
        base_url: String,
        network: Network,
        max_retries: usize,
        http_client: Box<dyn HttpClient>,
        basic_auth: Option<BasicAuth>,
        api_type: ChainApiType,
    ) -> Self {
        Self {
            base_url,
            network,
            client: http_client,
            max_retries,
            basic_auth,
            api_type,
        }
    }

    async fn get_response_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ChainServiceError> {
        let url = format!("{}{}", self.base_url, path);
        info!("Fetching response json from {}", url);
        let (response, _) = self.get_with_retry(&url, self.client.as_ref()).await?;

        let response: T = serde_json::from_str(&response)
            .map_err(|e| ChainServiceError::Generic(e.to_string()))?;

        Ok(response)
    }

    async fn get_response_text(&self, path: &str) -> Result<String, ChainServiceError> {
        let url = format!("{}{}", self.base_url, path);
        info!("Fetching response text from {}", url);
        let (response, _) = self.get_with_retry(&url, self.client.as_ref()).await?;
        Ok(response)
    }

    async fn get_with_retry(
        &self,
        url: &str,
        client: &dyn HttpClient,
    ) -> Result<(String, u16), ChainServiceError> {
        let mut delay = BASE_BACKOFF_MILLIS;
        let mut attempts = 0;

        loop {
            let mut headers = HashMap::new();
            if let Some(basic_auth) = &self.basic_auth {
                add_basic_auth_header(&mut headers, &basic_auth.username, &basic_auth.password);
            }

            let HttpResponse { body, status } = client.get(url.to_string(), Some(headers)).await?;
            match status {
                status if attempts < self.max_retries && is_status_retryable(status) => {
                    tokio::time::sleep(delay).await;
                    attempts = attempts.saturating_add(1);
                    delay = delay.saturating_mul(2);
                }
                _ => {
                    if !(200..300).contains(&status) {
                        return Err(HttpError::Status { status, body }.into());
                    }
                    return Ok((body, status));
                }
            }
        }
    }

    async fn post(&self, url: &str, body: Option<String>) -> Result<String, ChainServiceError> {
        let mut headers: HashMap<String, String> = HashMap::new();
        add_content_type_header(&mut headers, ContentType::TextPlain);
        if let Some(basic_auth) = &self.basic_auth {
            add_basic_auth_header(&mut headers, &basic_auth.username, &basic_auth.password);
        }
        info!(
            "Posting to {} with body {} and headers {:?}",
            url,
            body.clone().unwrap_or_default(),
            headers
        );
        let HttpResponse { body, status } = self
            .client
            .post(url.to_string(), Some(headers), body)
            .await?;
        if !(200..300).contains(&status) {
            return Err(HttpError::Status { status, body }.into());
        }

        Ok(body)
    }

    async fn recommended_fees_esplora(&self) -> Result<RecommendedFees, ChainServiceError> {
        let fee_map = self
            .get_response_json::<HashMap<u16, f64>>("/fee-estimates")
            .await?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let get_fees = |block: &u16| fee_map.get(block).map_or(0, |fee| fee.ceil() as u64);

        Ok(RecommendedFees {
            fastest_fee: get_fees(&1),
            half_hour_fee: get_fees(&3),
            hour_fee: get_fees(&6),
            economy_fee: get_fees(&25),
            minimum_fee: get_fees(&1008),
        })
    }

    async fn recommended_fees_mempool_space(&self) -> Result<RecommendedFees, ChainServiceError> {
        let response = self
            .get_response_json::<MempoolSpaceRecommendedFeesResponse>("/v1/fees/recommended")
            .await?;
        Ok(response.into())
    }
}

#[macros::async_trait]
impl BitcoinChainService for RestClientChainService {
    async fn get_address_utxos(&self, address: String) -> Result<Vec<Utxo>, ChainServiceError> {
        let address = address
            .parse::<Address<NetworkUnchecked>>()?
            .require_network(self.network.into())?;

        let utxos = self
            .get_response_json::<Vec<Utxo>>(format!("/address/{address}/utxo").as_str())
            .await?;

        Ok(utxos)
    }

    async fn get_transaction_status(
        &self,
        txid: String,
    ) -> Result<super::TxStatus, ChainServiceError> {
        let tx_info = self
            .get_response_json::<TxInfo>(format!("/tx/{txid}").as_str())
            .await?;
        Ok(tx_info.status)
    }

    async fn get_transaction_hex(&self, txid: String) -> Result<String, ChainServiceError> {
        let tx = self
            .get_response_text(format!("/tx/{txid}/hex").as_str())
            .await?;
        Ok(tx)
    }

    async fn broadcast_transaction(&self, tx: String) -> Result<(), ChainServiceError> {
        let url = format!("{}{}", self.base_url, "/tx");
        self.post(&url, Some(tx)).await?;
        Ok(())
    }

    async fn recommended_fees(&self) -> Result<RecommendedFees, ChainServiceError> {
        match self.api_type {
            ChainApiType::Esplora => self.recommended_fees_esplora().await,
            ChainApiType::MempoolSpace => self.recommended_fees_mempool_space().await,
        }
    }
}

fn is_status_retryable(status: u16) -> bool {
    RETRYABLE_ERROR_CODES.contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Network;

    use macros::async_test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[cfg(test)]
    use breez_sdk_common::test_utils::mock_rest_client::{MockResponse, MockRestClient};

    #[async_test_all]
    async fn test_get_address_utxos() {
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

        let mock = MockRestClient::new();
        mock.add_response(MockResponse::new(200, mock_response.to_string()));

        // Create the service with the mock server URL
        let service = RestClientChainService::new(
            "http://localhost:8080".to_string(),
            Network::Mainnet,
            3,
            Box::new(mock),
            None,
            ChainApiType::Esplora,
        );

        // Call the method under test
        let mut result = service
            .get_address_utxos("1wiz18xYmhRX6xStj2b9t1rwWX4GKUgpv".to_string())
            .await
            .unwrap();

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
        assert_eq!(result[0].status.block_height, Some(726_892));

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
}
