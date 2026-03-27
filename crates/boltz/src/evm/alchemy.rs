use std::collections::HashMap;

use serde::Deserialize;

use platform_utils::http::HttpClient;

use crate::config::AlchemyConfig;
use crate::error::BoltzError;
use crate::evm::signing::{EvmSignature, EvmSigner};

/// Maximum polling attempts for `wallet_getCallsStatus`.
const MAX_POLL_ATTEMPTS: u32 = 60;

/// Polling interval in milliseconds.
const POLL_INTERVAL_MS: u64 = 1000;

/// A single call to submit via Alchemy gas abstraction.
#[derive(Debug, Clone)]
pub struct EvmCall {
    pub to: String,
    /// Omitted from JSON when `None` (matching web app behavior).
    pub value: Option<String>,
    /// Hex-encoded calldata with `0x` prefix. Omitted when `None`.
    pub data: Option<String>,
}

/// Result from a successfully submitted and confirmed sponsored call.
#[derive(Debug, Clone)]
pub struct AlchemyResult {
    pub tx_hash: String,
}

/// Alchemy EIP-7702 gas-sponsored transaction submission client.
pub struct AlchemyGasClient {
    rpc_url: String,
    gas_policy_id: String,
    http_client: Box<dyn HttpClient>,
    gas_signer: EvmSigner,
}

impl AlchemyGasClient {
    pub fn new(
        config: &AlchemyConfig,
        http_client: Box<dyn HttpClient>,
        gas_signer: EvmSigner,
    ) -> Self {
        Self {
            rpc_url: config.rpc_url(),
            gas_policy_id: config.gas_policy_id.clone(),
            http_client,
            gas_signer,
        }
    }

    /// Submit a bundle of calls via Alchemy gas abstraction.
    /// Handles first-time EIP-7702 delegation + `UserOp` signing.
    /// Polls until confirmed or timeout. Returns tx hash.
    pub async fn send_sponsored_calls(
        &self,
        calls: Vec<EvmCall>,
        chain_id: u64,
    ) -> Result<AlchemyResult, BoltzError> {
        // Step 1: wallet_prepareCalls
        let prepared = self.prepare_calls(&calls, chain_id).await?;

        // Step 2: Sign and send via wallet_sendPreparedCalls
        let call_id = self.sign_and_send(prepared).await?;

        // Step 3: Poll wallet_getCallsStatus until confirmed
        self.poll_status(&call_id).await
    }

    /// Step 1: `wallet_prepareCalls` — prepare calls for gas abstraction.
    async fn prepare_calls(
        &self,
        calls: &[EvmCall],
        chain_id: u64,
    ) -> Result<serde_json::Value, BoltzError> {
        let json_calls: Vec<serde_json::Value> = calls
            .iter()
            .map(|c| {
                let mut obj = serde_json::Map::new();
                obj.insert("to".to_string(), serde_json::json!(c.to));
                if let Some(v) = &c.value {
                    obj.insert("value".to_string(), serde_json::json!(v));
                }
                if let Some(d) = &c.data {
                    obj.insert("data".to_string(), serde_json::json!(d));
                }
                serde_json::Value::Object(obj)
            })
            .collect();

        let params = serde_json::json!([{
            "capabilities": {
                "paymasterService": { "policyId": self.gas_policy_id }
            },
            "calls": json_calls,
            "from": self.gas_signer.address_hex(),
            "chainId": format!("0x{:x}", chain_id)
        }]);

        self.rpc_call("wallet_prepareCalls", params).await
    }

    /// Step 2: Sign the prepared calls and send via `wallet_sendPreparedCalls`.
    async fn sign_and_send(
        &self,
        prepared: serde_json::Value,
    ) -> Result<String, BoltzError> {
        let signed = self.sign_prepared_response(&prepared)?;

        let result: SendPreparedCallsResponse = self
            .rpc_call("wallet_sendPreparedCalls", serde_json::json!([signed]))
            .await?;

        result
            .prepared_call_ids
            .into_iter()
            .next()
            .ok_or_else(|| BoltzError::Evm {
                reason: "wallet_sendPreparedCalls returned no call IDs".to_string(),
                tx_hash: None,
            })
    }

    /// Sign the response from `wallet_prepareCalls`.
    ///
    /// Two response shapes:
    /// - **First-time** (`type: "array"`): Two entries — authorization (raw sign) + `UserOp` (EIP-191)
    /// - **Subsequent** (`type: "user-operation-v070"`): Single `UserOp` (EIP-191)
    fn sign_prepared_response(
        &self,
        prepared: &serde_json::Value,
    ) -> Result<serde_json::Value, BoltzError> {
        let resp_type = prepared["type"]
            .as_str()
            .ok_or_else(|| BoltzError::Evm {
                reason: "prepareCalls response missing 'type' field".to_string(),
                tx_hash: None,
            })?;

        match resp_type {
            "array" => self.sign_first_time_response(prepared),
            "user-operation-v070" => self.sign_subsequent_response(prepared),
            other => Err(BoltzError::Evm {
                reason: format!("Unknown prepareCalls response type: {other}"),
                tx_hash: None,
            }),
        }
    }

    /// First-time response: array with [authorization, user-operation-v070].
    /// Entry 0: sign with `sign_raw_digest` (NO EIP-191 prefix).
    /// Entry 1: sign with `sign_message` (WITH EIP-191 prefix).
    fn sign_first_time_response(
        &self,
        prepared: &serde_json::Value,
    ) -> Result<serde_json::Value, BoltzError> {
        let data = prepared["data"]
            .as_array()
            .ok_or_else(|| BoltzError::Evm {
                reason: "First-time response 'data' is not an array".to_string(),
                tx_hash: None,
            })?;

        if data.len() < 2 {
            return Err(BoltzError::Evm {
                reason: format!("Expected 2 entries in first-time response, got {}", data.len()),
                tx_hash: None,
            });
        }

        let mut signed_entries = Vec::with_capacity(data.len());

        for (i, entry) in data.iter().enumerate() {
            let entry_type = entry["type"].as_str().unwrap_or("");
            let payload = extract_signing_payload(entry)?;
            let sig = if i == 0 && entry_type != "user-operation-v070" {
                // Authorization entry — raw ECDSA sign (no prefix)
                let digest = parse_payload_to_digest(&payload)?;
                self.gas_signer.sign_raw_digest(&digest)?
            } else {
                // UserOperation — EIP-191 personal_sign
                let bytes = parse_payload_to_bytes(&payload)?;
                self.gas_signer.sign_message(&bytes)?
            };

            signed_entries.push(attach_signature(entry, &sig));
        }

        Ok(serde_json::json!({
            "type": "array",
            "data": signed_entries
        }))
    }

    /// Subsequent response: single user-operation-v070.
    fn sign_subsequent_response(
        &self,
        prepared: &serde_json::Value,
    ) -> Result<serde_json::Value, BoltzError> {
        let payload = extract_signing_payload(prepared)?;
        let bytes = parse_payload_to_bytes(&payload)?;
        let sig = self.gas_signer.sign_message(&bytes)?;

        Ok(attach_signature(prepared, &sig))
    }

    /// Step 3: Poll `wallet_getCallsStatus` until confirmed or timeout.
    async fn poll_status(&self, call_id: &str) -> Result<AlchemyResult, BoltzError> {
        for attempt in 0..MAX_POLL_ATTEMPTS {
            let status: CallsStatusResponse = self
                .rpc_call(
                    "wallet_getCallsStatus",
                    serde_json::json!([call_id]),
                )
                .await?;

            if let Some(receipts) = &status.receipts
                && !receipts.is_empty()
            {
                let receipt = &receipts[0];

                // Check receipt status for safety (stricter than web app)
                if receipt.status.as_deref() == Some("0x0") {
                    return Err(BoltzError::Evm {
                        reason: "Transaction reverted".to_string(),
                        tx_hash: receipt.transaction_hash.clone(),
                    });
                }

                let tx_hash = receipt.transaction_hash.clone().ok_or_else(|| {
                    BoltzError::Evm {
                        reason: "Receipt missing transactionHash".to_string(),
                        tx_hash: None,
                    }
                })?;

                return Ok(AlchemyResult { tx_hash });
            }

            if attempt < MAX_POLL_ATTEMPTS - 1 {
                sleep_ms(POLL_INTERVAL_MS).await;
            }
        }

        Err(BoltzError::Evm {
            reason: format!("wallet_getCallsStatus timed out after {MAX_POLL_ATTEMPTS} attempts"),
            tx_hash: None,
        })
    }

    /// Internal: send a JSON-RPC request to Alchemy.
    async fn rpc_call<T: for<'a> Deserialize<'a>>(
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
            reason: format!("Failed to serialize Alchemy request: {e}"),
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
                reason: format!("Alchemy HTTP error {}: {}", response.status, response.body),
                tx_hash: None,
            });
        }

        let rpc_response: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|e| BoltzError::Evm {
                reason: format!(
                    "Failed to parse Alchemy response: {e} (body: {})",
                    response.body
                ),
                tx_hash: None,
            })?;

        if let Some(err) = rpc_response.get("error") {
            let code = err.get("code").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(BoltzError::Evm {
                reason: format!("Alchemy RPC error {code}: {message}"),
                tx_hash: None,
            });
        }

        let result = rpc_response.get("result").ok_or_else(|| BoltzError::Evm {
            reason: "Alchemy response has no result".to_string(),
            tx_hash: None,
        })?;

        serde_json::from_value(result.clone()).map_err(|e| BoltzError::Evm {
            reason: format!("Failed to deserialize Alchemy result: {e}"),
            tx_hash: None,
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Extract the signing payload from a prepareCalls entry.
/// The payload is in `data.hash` (a hex string).
fn extract_signing_payload(entry: &serde_json::Value) -> Result<String, BoltzError> {
    // Try data.hash first (UserOp), then just hash at top level
    entry["data"]["hash"]
        .as_str()
        .or_else(|| entry["hash"].as_str())
        .map(String::from)
        .ok_or_else(|| BoltzError::Evm {
            reason: format!("No signing payload found in entry: {entry}"),
            tx_hash: None,
        })
}

/// Parse a hex-encoded payload into a 32-byte digest (for raw signing).
fn parse_payload_to_digest(payload: &str) -> Result<[u8; 32], BoltzError> {
    let bytes = parse_payload_to_bytes(payload)?;
    if bytes.len() != 32 {
        return Err(BoltzError::Signing(format!(
            "Expected 32-byte digest, got {} bytes",
            bytes.len()
        )));
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&bytes);
    Ok(digest)
}

/// Parse a hex-encoded payload into raw bytes (for EIP-191 signing).
fn parse_payload_to_bytes(payload: &str) -> Result<Vec<u8>, BoltzError> {
    let clean = payload.strip_prefix("0x").unwrap_or(payload);
    hex::decode(clean).map_err(|e| BoltzError::Signing(format!("Invalid hex payload: {e}")))
}

/// Attach a signature to a prepared calls entry.
fn attach_signature(entry: &serde_json::Value, sig: &EvmSignature) -> serde_json::Value {
    let sig_hex = format!(
        "0x{}{}{}",
        hex::encode(sig.r),
        hex::encode(sig.s),
        hex::encode([sig.v])
    );

    let mut result = entry.clone();
    result["signature"] = serde_json::json!({
        "type": "secp256k1",
        "data": sig_hex
    });
    result
}

/// Encode an `EvmSignature` as a 65-byte hex string (r || s || v) with `0x` prefix.
pub fn signature_to_hex(sig: &EvmSignature) -> String {
    format!(
        "0x{}{}{}",
        hex::encode(sig.r),
        hex::encode(sig.s),
        hex::encode([sig.v])
    )
}

/// Platform-aware sleep.
async fn sleep_ms(ms: u64) {
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    {
        // On WASM, use a simple JS timeout via gloo_timers or similar.
        // For now, yield back to the event loop.
        let _ = ms;
        futures_util::future::ready(()).await;
    }
}

// ─── Serde Types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendPreparedCallsResponse {
    prepared_call_ids: Vec<String>,
}

#[derive(Deserialize)]
struct CallsStatusResponse {
    #[serde(default)]
    receipts: Option<Vec<CallsStatusReceipt>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallsStatusReceipt {
    #[serde(default)]
    transaction_hash: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::EvmKeyManager;
    use platform_utils::http::{HttpError, HttpResponse};
    use std::sync::{Arc, Mutex};

    const TEST_SEED_HEX: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";

    fn test_signer() -> EvmSigner {
        let seed = hex::decode(TEST_SEED_HEX).unwrap();
        let manager = EvmKeyManager::from_seed(&seed).unwrap();
        let key_pair = manager.derive_gas_signer(42161).unwrap();
        EvmSigner::new(&key_pair, 42161)
    }

    struct MockAlchemyHttpClient {
        responses: Arc<Mutex<Vec<HttpResponse>>>,
    }

    impl MockAlchemyHttpClient {
        fn new(responses: Vec<HttpResponse>) -> Self {
            let mut r = responses;
            r.reverse();
            Self {
                responses: Arc::new(Mutex::new(r)),
            }
        }
    }

    #[macros::async_trait]
    impl HttpClient for MockAlchemyHttpClient {
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

    fn alchemy_rpc_success(result: serde_json::Value) -> HttpResponse {
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

    #[test]
    fn test_signature_to_hex() {
        let sig = EvmSignature {
            v: 27,
            r: [1u8; 32],
            s: [2u8; 32],
        };
        let hex_str = signature_to_hex(&sig);
        assert!(hex_str.starts_with("0x"));
        assert_eq!(hex_str.len(), 132); // 0x + 64 + 64 + 2
        assert!(hex_str.ends_with("1b")); // v=27 = 0x1b
    }

    #[test]
    fn test_attach_signature() {
        let entry = serde_json::json!({
            "type": "user-operation-v070",
            "data": { "hash": "0xabcd" }
        });
        let sig = EvmSignature {
            v: 28,
            r: [0xaa; 32],
            s: [0xbb; 32],
        };
        let signed = attach_signature(&entry, &sig);
        assert!(signed["signature"]["type"].as_str() == Some("secp256k1"));
        let sig_data = signed["signature"]["data"].as_str().unwrap();
        assert!(sig_data.starts_with("0x"));
        assert!(sig_data.ends_with("1c")); // v=28 = 0x1c
    }

    #[test]
    fn test_parse_payload_to_digest() {
        let hex_digest = format!("0x{}", hex::encode([42u8; 32]));
        let digest = parse_payload_to_digest(&hex_digest).unwrap();
        assert_eq!(digest, [42u8; 32]);
    }

    #[test]
    fn test_parse_payload_to_digest_wrong_length() {
        let result = parse_payload_to_digest("0xabcd");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_signing_payload() {
        // UserOp style: data.hash
        let entry = serde_json::json!({
            "type": "user-operation-v070",
            "data": { "hash": "0xdeadbeef" }
        });
        assert_eq!(extract_signing_payload(&entry).unwrap(), "0xdeadbeef");

        // Top-level hash
        let entry2 = serde_json::json!({
            "type": "authorization",
            "hash": "0xcafebabe"
        });
        assert_eq!(extract_signing_payload(&entry2).unwrap(), "0xcafebabe");
    }

    #[test]
    fn test_sign_subsequent_response() {
        let config = AlchemyConfig {
            api_key: "test".to_string(),
            gas_policy_id: "policy123".to_string(),
        };
        let signer = test_signer();
        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(vec![])),
            signer,
        );

        // Subsequent response: single UserOp
        let prepared = serde_json::json!({
            "type": "user-operation-v070",
            "data": {
                "hash": format!("0x{}", hex::encode([1u8; 32]))
            },
            "chainId": "0xa4b1"
        });

        let signed = client.sign_prepared_response(&prepared).unwrap();
        assert!(signed["signature"]["type"].as_str() == Some("secp256k1"));
        assert!(signed["signature"]["data"].as_str().unwrap().starts_with("0x"));
    }

    #[test]
    fn test_sign_first_time_response() {
        let config = AlchemyConfig {
            api_key: "test".to_string(),
            gas_policy_id: "policy123".to_string(),
        };
        let signer = test_signer();
        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(vec![])),
            signer,
        );

        // First-time response: array with authorization + UserOp
        let prepared = serde_json::json!({
            "type": "array",
            "data": [
                {
                    "type": "authorization",
                    "data": {
                        "hash": format!("0x{}", hex::encode([2u8; 32]))
                    },
                    "chainId": "0xa4b1"
                },
                {
                    "type": "user-operation-v070",
                    "data": {
                        "hash": format!("0x{}", hex::encode([3u8; 32]))
                    },
                    "chainId": "0xa4b1"
                }
            ]
        });

        let signed = client.sign_prepared_response(&prepared).unwrap();
        assert_eq!(signed["type"], "array");

        let data = signed["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);

        // Both entries should have signatures
        for entry in data {
            assert!(entry["signature"]["type"].as_str() == Some("secp256k1"));
        }
    }

    #[tokio::test]
    async fn test_full_sponsored_call_flow() {
        let config = AlchemyConfig {
            api_key: "test_key".to_string(),
            gas_policy_id: "policy_id".to_string(),
        };
        let signer = test_signer();

        // Mock responses: prepareCalls -> sendPreparedCalls -> getCallsStatus (confirmed)
        let responses = vec![
            // 1. wallet_prepareCalls -> subsequent-style response
            alchemy_rpc_success(serde_json::json!({
                "type": "user-operation-v070",
                "data": {
                    "hash": format!("0x{}", hex::encode([1u8; 32]))
                },
                "chainId": "0xa4b1"
            })),
            // 2. wallet_sendPreparedCalls
            alchemy_rpc_success(serde_json::json!({
                "preparedCallIds": ["call_123"]
            })),
            // 3. wallet_getCallsStatus (confirmed)
            alchemy_rpc_success(serde_json::json!({
                "status": 1,
                "receipts": [{
                    "transactionHash": "0xabc123",
                    "status": "0x1",
                    "blockHash": "0xblock",
                    "blockNumber": "0x100",
                    "gasUsed": "0x5208"
                }]
            })),
        ];

        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(responses)),
            signer,
        );

        let result = client
            .send_sponsored_calls(
                vec![EvmCall {
                    to: "0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61".to_string(),
                    value: None,
                    data: Some("0xdeadbeef".to_string()),
                }],
                42161,
            )
            .await
            .unwrap();

        assert_eq!(result.tx_hash, "0xabc123");
    }

    #[tokio::test]
    async fn test_sponsored_call_reverted() {
        let config = AlchemyConfig {
            api_key: "test_key".to_string(),
            gas_policy_id: "policy_id".to_string(),
        };
        let signer = test_signer();

        let responses = vec![
            alchemy_rpc_success(serde_json::json!({
                "type": "user-operation-v070",
                "data": {
                    "hash": format!("0x{}", hex::encode([1u8; 32]))
                },
                "chainId": "0xa4b1"
            })),
            alchemy_rpc_success(serde_json::json!({
                "preparedCallIds": ["call_456"]
            })),
            alchemy_rpc_success(serde_json::json!({
                "status": 1,
                "receipts": [{
                    "transactionHash": "0xfailed",
                    "status": "0x0",
                    "blockHash": "0xblock",
                    "blockNumber": "0x100",
                    "gasUsed": "0x5208"
                }]
            })),
        ];

        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(responses)),
            signer,
        );

        let result = client
            .send_sponsored_calls(
                vec![EvmCall {
                    to: "0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61".to_string(),
                    value: None,
                    data: Some("0xdeadbeef".to_string()),
                }],
                42161,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("reverted"));
    }

    #[test]
    fn test_evm_call_json_structure() {
        // Verify that EvmCall produces the expected JSON when value/data are None
        let call = EvmCall {
            to: "0xabc".to_string(),
            value: None,
            data: None,
        };
        let mut obj = serde_json::Map::new();
        obj.insert("to".to_string(), serde_json::json!(call.to));
        if let Some(v) = &call.value {
            obj.insert("value".to_string(), serde_json::json!(v));
        }
        if let Some(d) = &call.data {
            obj.insert("data".to_string(), serde_json::json!(d));
        }
        let json = serde_json::Value::Object(obj);

        // value and data should be absent when None
        assert!(json.get("value").is_none());
        assert!(json.get("data").is_none());
        assert_eq!(json["to"], "0xabc");
    }
}
