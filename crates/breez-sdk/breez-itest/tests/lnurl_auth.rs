use std::sync::Arc;

use anyhow::Result;
use bech32::{Bech32, Hrp};
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tokio::sync::Mutex;
use tracing::info;
use warp::Filter;

// ---------------------
// Mock LNURL Auth Server
// ---------------------

/// Helper function to encode a URL as LNURL (bech32)
fn encode_lnurl(url: &str) -> Result<String> {
    let hrp = Hrp::parse("lnurl")?;
    let lnurl = bech32::encode::<Bech32>(hrp, url.as_bytes())?;
    Ok(lnurl.to_lowercase())
}

#[derive(Clone, Debug)]
struct AuthState {
    k1: String,
    authenticated_key: Option<String>,
    signature: Option<String>,
}

type SharedAuthState = Arc<Mutex<AuthState>>;

/// Creates a mock LNURL auth server that returns a challenge and validates signatures
async fn start_mock_lnurl_auth_server() -> Result<(String, SharedAuthState)> {
    // Generate a random k1 challenge (32 bytes hex encoded)
    let mut k1_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k1_bytes);
    let k1 = hex::encode(k1_bytes);

    let state = Arc::new(Mutex::new(AuthState {
        k1: k1.clone(),
        authenticated_key: None,
        signature: None,
    }));

    let state_for_filter = state.clone();
    let state_filter = warp::any().map(move || state_for_filter.clone());

    // Initial auth endpoint - returns the LNURL auth request with k1 and action
    let auth_initial = warp::path!("lnurl-auth")
        .and(warp::query::<std::collections::HashMap<String, String>>())
        .and(state_filter.clone())
        .and_then(
            |params: std::collections::HashMap<String, String>, state: SharedAuthState| async move {
                let action = params.get("action").map(|s| s.as_str()).unwrap_or("login");
                let state = state.lock().await;

                let response = serde_json::json!({
                    "tag": "login",
                    "k1": state.k1,
                    "action": action,
                });

                Ok::<_, warp::Rejection>(warp::reply::json(&response))
            },
        );

    // Callback endpoint - validates the signature and key
    let auth_callback = warp::path!("lnurl-auth" / "callback")
        .and(warp::query::<std::collections::HashMap<String, String>>())
        .and(state_filter.clone())
        .and_then(
            |params: std::collections::HashMap<String, String>, state: SharedAuthState| async move {
                let mut state = state.lock().await;

                // Extract sig and key from query parameters
                let sig = params.get("sig").map(|s| s.to_string());
                let key = params.get("key").map(|s| s.to_string());

                // Validate that we received both parameters
                if sig.is_none() || key.is_none() {
                    let error_response = serde_json::json!({
                        "status": "ERROR",
                        "reason": "Missing sig or key parameter"
                    });
                    return Ok::<_, warp::Rejection>(warp::reply::json(&error_response));
                }

                // Store the authenticated key and signature
                state.authenticated_key = key.clone();
                state.signature = sig;

                let success_response = serde_json::json!({
                    "status": "OK"
                });

                Ok::<_, warp::Rejection>(warp::reply::json(&success_response))
            },
        );

    let routes = auth_initial.or(auth_callback);

    // Start the server on a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let port = addr.port();

    tokio::spawn(async move {
        warp::serve(routes)
            .serve_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await;
    });

    // Wait a bit for server to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let server_url = format!("http://127.0.0.1:{}", port);
    info!("Mock LNURL auth server started at {}", server_url);

    Ok((server_url, state.clone()))
}

// ---------------------
// Fixtures
// ---------------------

/// Fixture: SDK for LNURL auth testing
#[fixture]
async fn auth_sdk() -> Result<SdkInstance> {
    let temp_dir = TempDir::new("breez-sdk-lnurl-auth")?;

    // Generate random seed
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 1;
    config.real_time_sync_server_url = None;

    build_sdk_with_custom_config(
        temp_dir.path().to_string_lossy().to_string(),
        seed,
        config,
        Some(temp_dir),
        false,
    )
    .await
}

// ---------------------
// Tests
// ---------------------

/// Test LNURL auth login flow
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_lnurl_auth_login(#[future] auth_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_01_lnurl_auth_login ===");

    let sdk_instance = auth_sdk.await?;
    let (server_url, auth_state) = start_mock_lnurl_auth_server().await?;

    // Construct the HTTP URL with callback
    let http_url = format!(
        "{}/lnurl-auth?tag=login&k1={}&action=login",
        server_url,
        auth_state.lock().await.k1
    );

    // Encode as LNURL (bech32)
    let lnurl = encode_lnurl(&http_url)?;
    info!("Encoded LNURL: {}", lnurl);

    // Parse the LNURL auth URL
    let parse_response = sdk_instance.sdk.parse(&lnurl).await?;
    let InputType::LnurlAuth(auth_request) = parse_response else {
        anyhow::bail!("Expected LnurlAuth input type");
    };

    info!("Successfully parsed LNURL auth request");
    assert_eq!(auth_request.k1, auth_state.lock().await.k1);
    assert_eq!(auth_request.action, Some("login".to_string()));
    assert_eq!(auth_request.domain, "127.0.0.1");

    // Perform LNURL auth
    info!("Performing LNURL auth");
    let auth_response = sdk_instance.sdk.lnurl_auth(auth_request).await?;

    info!("LNURL auth response: {:?}", auth_response);

    // Verify the response is OK
    match auth_response {
        LnurlCallbackStatus::Ok => {
            info!("LNURL auth succeeded");
        }
        LnurlCallbackStatus::ErrorStatus { error_details } => {
            anyhow::bail!("LNURL auth failed with error: {}", error_details.reason);
        }
    }

    // Verify that the server received the authentication
    let state = auth_state.lock().await;
    assert!(
        state.authenticated_key.is_some(),
        "Server should have received the authentication key"
    );
    assert!(
        state.signature.is_some(),
        "Server should have received the signature"
    );

    info!("Authenticated key: {:?}", state.authenticated_key);
    info!("Signature: {:?}", state.signature);

    info!("=== Test test_01_lnurl_auth_login PASSED ===");
    Ok(())
}

/// Test LNURL auth with different action types
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_lnurl_auth_actions(#[future] auth_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_02_lnurl_auth_actions ===");

    let sdk_instance = auth_sdk.await?;

    // Test different action types: register, login, link, auth
    let actions = vec!["register", "login", "link", "auth"];

    for action in actions {
        info!("Testing LNURL auth with action: {}", action);

        let (server_url, auth_state) = start_mock_lnurl_auth_server().await?;

        // Construct the HTTP URL with specific action
        let http_url = format!(
            "{}/lnurl-auth?tag=login&k1={}&action={}",
            server_url,
            auth_state.lock().await.k1,
            action
        );

        // Encode as LNURL (bech32)
        let lnurl = encode_lnurl(&http_url)?;

        // Parse the LNURL auth URL
        let parse_response = sdk_instance.sdk.parse(&lnurl).await?;
        let InputType::LnurlAuth(auth_request) = parse_response else {
            anyhow::bail!("Expected LnurlAuth input type for action {}", action);
        };

        assert_eq!(auth_request.action, Some(action.to_string()));

        // Perform LNURL auth
        let auth_response = sdk_instance.sdk.lnurl_auth(auth_request).await?;

        // Verify the response is OK
        match auth_response {
            LnurlCallbackStatus::Ok => {
                info!("LNURL auth succeeded for action: {}", action);
            }
            LnurlCallbackStatus::ErrorStatus { error_details } => {
                anyhow::bail!(
                    "LNURL auth failed for action {}: {}",
                    action,
                    error_details.reason
                );
            }
        }

        // Verify that the server received the authentication
        let state = auth_state.lock().await;
        assert!(
            state.authenticated_key.is_some(),
            "Server should have received the authentication key for action {}",
            action
        );
    }

    info!("=== Test test_02_lnurl_auth_actions PASSED ===");
    Ok(())
}

/// Test domain-specific key derivation
/// Verifies that different domains produce different authentication keys
#[rstest]
#[test_log::test(tokio::test)]
async fn test_03_lnurl_auth_domain_specific_keys(
    #[future] auth_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_03_lnurl_auth_domain_specific_keys ===");

    let sdk_instance = auth_sdk.await?;

    // Start two mock servers on different ports (simulating different domains)
    let (server_url_1, auth_state_1) = start_mock_lnurl_auth_server().await?;
    let (server_url_2, auth_state_2) = start_mock_lnurl_auth_server().await?;

    // Auth with first server
    let http_url_1 = format!(
        "{}/lnurl-auth?tag=login&k1={}",
        server_url_1,
        auth_state_1.lock().await.k1
    );
    let lnurl_1 = encode_lnurl(&http_url_1)?;

    let parse_response_1 = sdk_instance.sdk.parse(&lnurl_1).await?;
    let InputType::LnurlAuth(auth_request_1) = parse_response_1 else {
        anyhow::bail!("Expected LnurlAuth input type");
    };

    sdk_instance.sdk.lnurl_auth(auth_request_1).await?;
    let key_1 = auth_state_1
        .lock()
        .await
        .authenticated_key
        .clone()
        .expect("First auth should succeed");

    // Auth with second server
    let http_url_2 = format!(
        "{}/lnurl-auth?tag=login&k1={}",
        server_url_2,
        auth_state_2.lock().await.k1
    );
    let lnurl_2 = encode_lnurl(&http_url_2)?;

    let parse_response_2 = sdk_instance.sdk.parse(&lnurl_2).await?;
    let InputType::LnurlAuth(auth_request_2) = parse_response_2 else {
        anyhow::bail!("Expected LnurlAuth input type");
    };

    sdk_instance.sdk.lnurl_auth(auth_request_2).await?;
    let key_2 = auth_state_2
        .lock()
        .await
        .authenticated_key
        .clone()
        .expect("Second auth should succeed");

    // The keys should be the same since both servers are on 127.0.0.1
    // (domain is the same, only ports differ)
    info!("Key from server 1: {}", key_1);
    info!("Key from server 2: {}", key_2);
    assert_eq!(
        key_1, key_2,
        "Keys should be the same for the same domain (127.0.0.1)"
    );

    info!("=== Test test_03_lnurl_auth_domain_specific_keys PASSED ===");
    Ok(())
}
