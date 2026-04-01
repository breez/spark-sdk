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
pub(crate) struct EvmCall {
    pub to: String,
    /// Omitted from JSON when `None` (matching web app behavior).
    pub value: Option<String>,
    /// Hex-encoded calldata with `0x` prefix. Omitted when `None`.
    pub data: Option<String>,
}

/// Result from a successfully submitted and confirmed sponsored call.
#[derive(Debug, Clone)]
pub(crate) struct AlchemyResult {
    pub tx_hash: String,
}

/// Alchemy EIP-7702 gas-sponsored transaction submission client.
///
/// Request and response JSON is built with `serde_json::json!()` / `Value`
/// rather than typed structs. This is intentional: Alchemy responses are
/// polymorphic (first-time EIP-7702 vs subsequent `UserOp` flows return
/// different shapes with decoy fields), and the signing flow needs to
/// extract specific fields, strip others (`signatureRequest`), and forward
/// the rest as-is — all easier with `Value` than with a full typed schema.
pub(crate) struct AlchemyGasClient {
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
    async fn sign_and_send(&self, prepared: serde_json::Value) -> Result<String, BoltzError> {
        tracing::debug!(
            prepared = %serde_json::to_string_pretty(&prepared).unwrap_or_default(),
            "wallet_prepareCalls response"
        );

        let signed = self.sign_prepared_response(&prepared)?;

        tracing::debug!(
            signed = %serde_json::to_string_pretty(&signed).unwrap_or_default(),
            "Signed payload for wallet_sendPreparedCalls"
        );

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
        let resp_type = prepared["type"].as_str().ok_or_else(|| BoltzError::Evm {
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
    /// Authorization: sign `signatureRequest.rawPayload` with raw ECDSA (no prefix).
    /// `UserOp`: sign `signatureRequest.data.raw` with EIP-191.
    fn sign_first_time_response(
        &self,
        prepared: &serde_json::Value,
    ) -> Result<serde_json::Value, BoltzError> {
        let data = prepared["data"].as_array().ok_or_else(|| BoltzError::Evm {
            reason: "First-time response 'data' is not an array".to_string(),
            tx_hash: None,
        })?;

        if data.len() < 2 {
            return Err(BoltzError::Evm {
                reason: format!(
                    "Expected 2 entries in first-time response, got {}",
                    data.len()
                ),
                tx_hash: None,
            });
        }

        // Entry 0: authorization — raw ECDSA sign of signatureRequest.rawPayload
        let auth_entry = &data[0];
        let auth_payload = extract_raw_payload(auth_entry)?;
        let auth_digest = parse_payload_to_digest(&auth_payload)?;
        let auth_sig = self.gas_signer.sign_raw_digest(&auth_digest)?;

        // Entry 1: user-operation — EIP-191 sign of signatureRequest.data.raw
        let uo_entry = &data[1];
        let uo_payload = extract_data_raw(uo_entry)?;
        let uo_bytes = parse_payload_to_bytes(&uo_payload)?;
        let uo_sig = self.gas_signer.sign_message(&uo_bytes)?;

        Ok(serde_json::json!({
            "type": "array",
            "data": [
                attach_signature(auth_entry, &auth_sig),
                attach_signature(uo_entry, &uo_sig),
            ]
        }))
    }

    /// Subsequent response: single user-operation-v070.
    /// Sign `signatureRequest.data.raw` with EIP-191.
    fn sign_subsequent_response(
        &self,
        prepared: &serde_json::Value,
    ) -> Result<serde_json::Value, BoltzError> {
        let payload = extract_data_raw(prepared)?;
        let bytes = parse_payload_to_bytes(&payload)?;
        let sig = self.gas_signer.sign_message(&bytes)?;

        Ok(attach_signature(prepared, &sig))
    }

    /// Step 3: Poll `wallet_getCallsStatus` until confirmed or timeout.
    async fn poll_status(&self, call_id: &str) -> Result<AlchemyResult, BoltzError> {
        for attempt in 0..MAX_POLL_ATTEMPTS {
            let status: CallsStatusResponse = self
                .rpc_call("wallet_getCallsStatus", serde_json::json!([call_id]))
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

                let tx_hash = receipt
                    .transaction_hash
                    .clone()
                    .ok_or_else(|| BoltzError::Evm {
                        reason: "Receipt missing transactionHash".to_string(),
                        tx_hash: None,
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
            let code = err
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
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

/// Extract `signatureRequest.rawPayload` (authorization entries).
fn extract_raw_payload(entry: &serde_json::Value) -> Result<String, BoltzError> {
    entry["signatureRequest"]["rawPayload"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| BoltzError::Evm {
            reason: format!("Missing signatureRequest.rawPayload in entry: {entry}"),
            tx_hash: None,
        })
}

/// Extract `signatureRequest.data.raw` (`UserOp` entries).
fn extract_data_raw(entry: &serde_json::Value) -> Result<String, BoltzError> {
    entry["signatureRequest"]["data"]["raw"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| BoltzError::Evm {
            reason: format!("Missing signatureRequest.data.raw in entry: {entry}"),
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

/// Build a signed entry from a prepared calls entry.
/// Only includes the fields Alchemy expects: `type`, `data`, `chainId`, `signature`.
/// Mirrors the web app's `SignedEntry` shape — `signatureRequest` is NOT forwarded.
fn attach_signature(entry: &serde_json::Value, sig: &EvmSignature) -> serde_json::Value {
    let sig_hex = format!(
        "0x{}{}{}",
        hex::encode(sig.r),
        hex::encode(sig.s),
        hex::encode([sig.v])
    );

    serde_json::json!({
        "type": entry["type"],
        "data": entry["data"],
        "chainId": entry["chainId"],
        "signature": {
            "type": "secp256k1",
            "data": sig_hex
        }
    })
}

async fn sleep_ms(ms: u64) {
    platform_utils::tokio::time::sleep(platform_utils::time::Duration::from_millis(ms)).await;
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

#[cfg(all(test, not(all(target_family = "wasm", target_os = "unknown"))))]
mod tests {
    use super::*;
    use crate::keys::EvmKeyManager;
    use platform_utils::http::{HttpError, HttpResponse};

    /// Encode an `EvmSignature` as a 65-byte hex string (r || s || v) with `0x` prefix.
    fn signature_to_hex(sig: &EvmSignature) -> String {
        format!(
            "0x{}{}{}",
            hex::encode(sig.r),
            hex::encode(sig.s),
            hex::encode([sig.v])
        )
    }
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

    fn alchemy_rpc_success(result: &serde_json::Value) -> HttpResponse {
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
    fn test_extract_raw_payload() {
        let entry = serde_json::json!({
            "type": "authorization",
            "signatureRequest": {
                "rawPayload": "0xabcd1234",
                "type": "eip7702Auth"
            }
        });
        assert_eq!(extract_raw_payload(&entry).unwrap(), "0xabcd1234");
        assert!(extract_raw_payload(&serde_json::json!({})).is_err());
    }

    #[test]
    fn test_extract_data_raw() {
        let entry = serde_json::json!({
            "type": "user-operation-v070",
            "signatureRequest": {
                "data": { "raw": "0xdeadbeef" },
                "rawPayload": "0x_WRONG_do_not_use_this"
            }
        });
        // Must use data.raw, NOT rawPayload, even when both are present
        assert_eq!(extract_data_raw(&entry).unwrap(), "0xdeadbeef");
        assert!(extract_data_raw(&serde_json::json!({})).is_err());
    }

    #[test]
    #[allow(clippy::similar_names)]
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
            "data": { "sender": "0x1234" },
            "signatureRequest": {
                "data": { "raw": format!("0x{}", hex::encode([1u8; 32])) }
            },
            "chainId": "0xa4b1"
        });

        let signed = client.sign_prepared_response(&prepared).unwrap();
        assert!(signed["signature"]["type"].as_str() == Some("secp256k1"));
        assert!(
            signed["signature"]["data"]
                .as_str()
                .unwrap()
                .starts_with("0x")
        );
    }

    #[test]
    #[allow(clippy::similar_names)]
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
                    "data": { "address": "0x1234", "nonce": "0x0" },
                    "signatureRequest": {
                        "rawPayload": format!("0x{}", hex::encode([2u8; 32])),
                        "type": "eip7702Auth"
                    },
                    "chainId": "0xa4b1"
                },
                {
                    "type": "user-operation-v070",
                    "data": { "sender": "0x5678" },
                    "signatureRequest": {
                        "data": { "raw": format!("0x{}", hex::encode([3u8; 32])) }
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
            alchemy_rpc_success(&serde_json::json!({
                "type": "user-operation-v070",
                "data": { "sender": "0x1234" },
                "signatureRequest": {
                    "data": { "raw": format!("0x{}", hex::encode([1u8; 32])) }
                },
                "chainId": "0xa4b1"
            })),
            // 2. wallet_sendPreparedCalls
            alchemy_rpc_success(&serde_json::json!({
                "preparedCallIds": ["call_123"]
            })),
            // 3. wallet_getCallsStatus (confirmed)
            alchemy_rpc_success(&serde_json::json!({
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
            alchemy_rpc_success(&serde_json::json!({
                "type": "user-operation-v070",
                "data": { "sender": "0x1234" },
                "signatureRequest": {
                    "data": { "raw": format!("0x{}", hex::encode([1u8; 32])) }
                },
                "chainId": "0xa4b1"
            })),
            alchemy_rpc_success(&serde_json::json!({
                "preparedCallIds": ["call_456"]
            })),
            alchemy_rpc_success(&serde_json::json!({
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

    // ─── Cross-validated test vectors ───────────────────────────────
    // These signatures were generated by `test_vectors/generate_signing.mjs`
    // using ethers v6.16.0 — the same library the Boltz web app uses.
    // Any mismatch here means our signing diverges from the web app.

    #[test]
    fn test_raw_ecdsa_matches_ethers() {
        // Vector 1: signingKey.sign(payload).serialized
        let signer = test_signer();
        let payload = "0x0101010101010101010101010101010101010101010101010101010101010101";
        let digest = parse_payload_to_digest(payload).unwrap();
        let sig = signer.sign_raw_digest(&digest).unwrap();
        let sig_hex = signature_to_hex(&sig);
        assert_eq!(
            sig_hex,
            "0xdbdb4d2abb3cae4ece51ccbe3989aaf8d39d385d3844ce75aa7fe9b93c8295731d47a5dd8df538e573714ea4f4ad3f4848787552b70edb4fdfa08315cddf6c7a1b",
            "Raw ECDSA signature must match ethers signingKey.sign().serialized"
        );
    }

    #[test]
    fn test_eip191_sign_message_matches_ethers() {
        // Vector 2: wallet.signMessage(getBytes(payload))
        let signer = test_signer();
        let payload = "0x0202020202020202020202020202020202020202020202020202020202020202";
        let bytes = parse_payload_to_bytes(payload).unwrap();
        let sig = signer.sign_message(&bytes).unwrap();
        let sig_hex = signature_to_hex(&sig);
        assert_eq!(
            sig_hex,
            "0x8a664e40364a638a70cb1e0ba0e6c39d24e9de39d68868598935b6f7c1d9ef0156bcffe603d274f403389de6afe6471c46a5ac5968279a61239ac2d380b5406f1c",
            "EIP-191 signature must match ethers wallet.signMessage()"
        );
    }

    #[test]
    fn test_first_time_flow_matches_ethers() {
        // Vector 3: full signPreparedCalls first-time flow
        let config = AlchemyConfig {
            api_key: "test".to_string(),
            gas_policy_id: "policy".to_string(),
        };
        let signer = test_signer();
        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(vec![])),
            signer,
        );

        // Mirrors real Alchemy response: UserOp entry has BOTH rawPayload and data.raw
        // with different values. We must use data.raw for UserOp, rawPayload for auth.
        let prepared = serde_json::json!({
            "type": "array",
            "data": [
                {
                    "type": "authorization",
                    "data": { "address": "0x69007702764179f14F51cdce752f4f775d74E139", "nonce": "0x0" },
                    "signatureRequest": {
                        "rawPayload": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "type": "eip7702Auth"
                    },
                    "chainId": "0xa4b1"
                },
                {
                    "type": "user-operation-v070",
                    "data": { "sender": "0x323f3d3cD440Ad067a8d6CeB8c9bF2252C5779Da" },
                    "signatureRequest": {
                        "data": { "raw": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                        "rawPayload": "0xff_DECOY_must_not_be_used_for_userop_signing_ffffffffffffffff",
                        "type": "personal_sign"
                    },
                    "chainId": "0xa4b1"
                }
            ]
        });

        let sign_result = client.sign_prepared_response(&prepared).unwrap();
        let data = sign_result["data"].as_array().unwrap();

        // Auth entry signature must match ethers
        assert_eq!(
            data[0]["signature"]["data"].as_str().unwrap(),
            "0xc1836abdb4aeee42d853020293f9f2a2ff34c19c745c96e8d2d7e4822f6916d42eb1e8b60447a6a17d4a2602e499352c9c9645f4aba95adc898588b8d302bc561b"
        );

        // UserOp entry signature must match ethers
        assert_eq!(
            data[1]["signature"]["data"].as_str().unwrap(),
            "0x220cccad90893eb6b523e98a17143a8959d4b57747e760356c181829fcf2bfea30afd418a6a3cd236f0fa6360bb40f9455068f48c364fccaceddbb132b15fd3e1b"
        );

        // Signed entries must only have type, data, chainId, signature (no signatureRequest)
        for entry in data {
            assert!(
                entry.get("signatureRequest").is_none(),
                "signatureRequest must not be forwarded to wallet_sendPreparedCalls"
            );
            assert!(entry.get("type").is_some());
            assert!(entry.get("data").is_some());
            assert!(entry.get("chainId").is_some());
            assert!(entry.get("signature").is_some());
        }
    }

    #[test]
    fn test_subsequent_flow_matches_ethers() {
        // Vector 4: full signPreparedCalls subsequent flow
        let config = AlchemyConfig {
            api_key: "test".to_string(),
            gas_policy_id: "policy".to_string(),
        };
        let signer = test_signer();
        let client = AlchemyGasClient::new(
            &config,
            Box::new(MockAlchemyHttpClient::new(vec![])),
            signer,
        );

        let prepared = serde_json::json!({
            "type": "user-operation-v070",
            "data": { "sender": "0x323f3d3cD440Ad067a8d6CeB8c9bF2252C5779Da" },
            "signatureRequest": {
                "data": { "raw": "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" },
                "rawPayload": "0xff_DECOY_must_not_be_used_ffffffffffffffffffffffffffffffff",
                "type": "personal_sign"
            },
            "chainId": "0xa4b1"
        });

        let sign_result = client.sign_prepared_response(&prepared).unwrap();

        assert_eq!(
            sign_result["signature"]["data"].as_str().unwrap(),
            "0xd0d8a4157083c95016141628bcbabf7a25f437674386440e2cd9e9e331f86b907a2733dade4f75818927b4b91f1af5966daba643a5e0473865a44608b7721f8a1b"
        );

        // Must not forward signatureRequest
        assert!(sign_result.get("signatureRequest").is_none());
    }
}
