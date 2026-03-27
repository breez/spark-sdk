use std::collections::HashMap;

use serde::Deserialize;

use platform_utils::http::HttpClient;

use crate::error::BoltzError;

/// Thin JSON-RPC wrapper over `platform_utils::HttpClient` for EVM read operations.
pub struct EvmProvider {
    rpc_url: String,
    http_client: Box<dyn HttpClient>,
}

/// Minimal transaction receipt from `eth_getTransactionReceipt`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxReceipt {
    pub transaction_hash: String,
    /// `"0x1"` = success, `"0x0"` = reverted.
    pub status: String,
    pub block_hash: String,
    pub block_number: String,
    pub gas_used: String,
}

impl TxReceipt {
    /// Returns true if the transaction succeeded (status == 0x1).
    pub fn is_success(&self) -> bool {
        self.status == "0x1"
    }
}

impl EvmProvider {
    pub fn new(rpc_url: String, http_client: Box<dyn HttpClient>) -> Self {
        Self {
            rpc_url,
            http_client,
        }
    }

    /// Execute a read-only contract call (`eth_call`).
    pub async fn eth_call(&self, to: &str, data: &[u8]) -> Result<Vec<u8>, BoltzError> {
        let result: String = self
            .rpc_request(
                "eth_call",
                serde_json::json!([
                    { "to": to, "data": format!("0x{}", hex::encode(data)) },
                    "latest"
                ]),
            )
            .await?;

        let clean = result.strip_prefix("0x").unwrap_or(&result);
        hex::decode(clean).map_err(|e| BoltzError::Evm {
            reason: format!("Failed to decode eth_call result: {e}"),
            tx_hash: None,
        })
    }

    /// Get a transaction receipt by hash.
    pub async fn eth_get_transaction_receipt(
        &self,
        tx_hash: &str,
    ) -> Result<Option<TxReceipt>, BoltzError> {
        let result: Option<TxReceipt> = self
            .rpc_request("eth_getTransactionReceipt", serde_json::json!([tx_hash]))
            .await?;
        Ok(result)
    }

    /// Get the chain ID.
    pub async fn eth_chain_id(&self) -> Result<u64, BoltzError> {
        let result: String = self
            .rpc_request("eth_chainId", serde_json::json!([]))
            .await?;
        parse_hex_u64(&result)
    }

    /// Get the latest block number.
    pub async fn eth_block_number(&self) -> Result<u64, BoltzError> {
        let result: String = self
            .rpc_request("eth_blockNumber", serde_json::json!([]))
            .await?;
        parse_hex_u64(&result)
    }

    /// Internal: send a JSON-RPC request and parse the result.
    async fn rpc_request<T: for<'a> Deserialize<'a>>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, BoltzError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let body = serde_json::to_string(&request).map_err(|e| BoltzError::Evm {
            reason: format!("Failed to serialize JSON-RPC request: {e}"),
            tx_hash: None,
        })?;

        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let response = self
            .http_client
            .post(self.rpc_url.clone(), Some(headers), Some(body))
            .await?;

        if !response.is_success() {
            return Err(BoltzError::Evm {
                reason: format!("RPC HTTP error {}: {}", response.status, response.body),
                tx_hash: None,
            });
        }

        let rpc_response: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|e| BoltzError::Evm {
                reason: format!(
                    "Failed to parse JSON-RPC response: {e} (body: {})",
                    response.body
                ),
                tx_hash: None,
            })?;

        if let Some(err) = rpc_response.get("error") {
            let code = err
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(BoltzError::Evm {
                reason: format!("JSON-RPC error {code}: {message}"),
                tx_hash: None,
            });
        }

        let result = rpc_response.get("result").ok_or_else(|| BoltzError::Evm {
            reason: "JSON-RPC response has no result".to_string(),
            tx_hash: None,
        })?;

        serde_json::from_value(result.clone()).map_err(|e| BoltzError::Evm {
            reason: format!("Failed to deserialize JSON-RPC result: {e}"),
            tx_hash: None,
        })
    }
}

fn parse_hex_u64(s: &str) -> Result<u64, BoltzError> {
    let clean = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(clean, 16).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to parse hex u64 '{s}': {e}"),
        tx_hash: None,
    })
}

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;
    use platform_utils::http::{HttpError, HttpResponse};
    use std::sync::{Arc, Mutex};

    /// Mock HTTP client that returns canned responses.
    struct MockHttpClient {
        responses: Arc<Mutex<Vec<HttpResponse>>>,
    }

    impl MockHttpClient {
        fn new(responses: Vec<HttpResponse>) -> Self {
            // Reverse so we can pop from the back
            let mut r = responses;
            r.reverse();
            Self {
                responses: Arc::new(Mutex::new(r)),
            }
        }
    }

    #[macros::async_trait]
    impl HttpClient for MockHttpClient {
        async fn get(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
        ) -> Result<HttpResponse, HttpError> {
            unimplemented!()
        }

        async fn post(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            let mut responses = self.responses.lock().unwrap();
            Ok(responses.pop().expect("no more mock responses"))
        }

        async fn delete(
            &self,
            _url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            unimplemented!()
        }
    }

    fn rpc_success(result: &serde_json::Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": result
            })
            .to_string(),
        }
    }

    fn rpc_error(code: i64, message: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": { "code": code, "message": message }
            })
            .to_string(),
        }
    }

    #[tokio::test]
    async fn test_eth_chain_id() {
        let client = MockHttpClient::new(vec![rpc_success(&serde_json::json!("0xa4b1"))]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let chain_id = provider.eth_chain_id().await.unwrap();
        assert_eq!(chain_id, 42161); // 0xa4b1 = Arbitrum
    }

    #[tokio::test]
    async fn test_eth_block_number() {
        let client = MockHttpClient::new(vec![rpc_success(&serde_json::json!("0x1234"))]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let block = provider.eth_block_number().await.unwrap();
        assert_eq!(block, 0x1234);
    }

    #[tokio::test]
    async fn test_eth_call() {
        // Return ABI-encoded uint256(6) — 32 bytes, value 6
        let hex_result = format!("0x{}", "00".repeat(31) + "06");
        let client = MockHttpClient::new(vec![rpc_success(&serde_json::json!(hex_result))]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let data = hex::decode("54fd4d50").unwrap(); // version() selector
        let result = provider
            .eth_call("0x6398B76DF91C5eBe9f488e3656658E79284dDc0F", &data)
            .await
            .unwrap();

        assert_eq!(result.len(), 32);
        assert_eq!(result[31], 6);
    }

    #[tokio::test]
    async fn test_eth_get_transaction_receipt_found() {
        let receipt_json = serde_json::json!({
            "transactionHash": "0xabc123",
            "status": "0x1",
            "blockHash": "0xblock",
            "blockNumber": "0x100",
            "gasUsed": "0x5208"
        });
        let client = MockHttpClient::new(vec![rpc_success(&receipt_json)]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let receipt = provider
            .eth_get_transaction_receipt("0xabc123")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.transaction_hash, "0xabc123");
        assert!(receipt.is_success());
    }

    #[tokio::test]
    async fn test_eth_get_transaction_receipt_not_found() {
        let client = MockHttpClient::new(vec![rpc_success(&serde_json::json!(null))]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let receipt = provider
            .eth_get_transaction_receipt("0xnonexistent")
            .await
            .unwrap();
        assert!(receipt.is_none());
    }

    #[tokio::test]
    async fn test_rpc_error_response() {
        let client = MockHttpClient::new(vec![rpc_error(-32601, "Method not found")]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let result = provider.eth_chain_id().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Method not found"));
    }

    #[tokio::test]
    async fn test_http_error_response() {
        let client = MockHttpClient::new(vec![HttpResponse {
            status: 500,
            body: "Internal Server Error".to_string(),
        }]);
        let provider = EvmProvider::new("http://localhost:8545".to_string(), Box::new(client));

        let result = provider.eth_chain_id().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_hex_u64() {
        assert_eq!(parse_hex_u64("0xa4b1").unwrap(), 42161);
        assert_eq!(parse_hex_u64("0x0").unwrap(), 0);
        assert_eq!(parse_hex_u64("0x1").unwrap(), 1);
        assert_eq!(parse_hex_u64("ff").unwrap(), 255);
    }

    #[test]
    fn test_tx_receipt_is_success() {
        let receipt = TxReceipt {
            transaction_hash: "0x".to_string(),
            status: "0x1".to_string(),
            block_hash: "0x".to_string(),
            block_number: "0x1".to_string(),
            gas_used: "0x0".to_string(),
        };
        assert!(receipt.is_success());

        let reverted = TxReceipt {
            status: "0x0".to_string(),
            ..receipt
        };
        assert!(!reverted.is_success());
    }
}
