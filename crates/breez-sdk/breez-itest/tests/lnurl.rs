use std::{borrow::Cow, sync::Arc};

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use nostr::util::JsonUtil;
use platform_utils::{DefaultHttpClient, HttpClient};
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tracing::{debug, info};

// ---------------------
// Fixtures
// ---------------------

/// Fixture: Lnurl service for LNURL testing
#[fixture]
async fn lnurl_fixture() -> LnurlFixture {
    LnurlFixture::new()
        .await
        .expect("Failed to start Lnurl service")
}

/// Fixture: Alice SDK for LNURL testing (sender)
#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    let temp_dir = TempDir::new("breez-sdk-alice-lnurl")?;

    // Generate random seed for Alice
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 1; // Faster sync for testing
    config.real_time_sync_server_url = None;
    config.lnurl_domain = None; // Alice doesn't need LNURL service

    build_sdk_with_custom_config(
        temp_dir.path().to_string_lossy().to_string(),
        seed,
        config,
        Some(temp_dir),
        false,
    )
    .await
}

/// Fixture: Bob SDK with Lnurl configured (receiver)
#[fixture]
async fn bob_sdk(#[future] lnurl_fixture: LnurlFixture) -> Result<SdkInstance> {
    let lnurl = Arc::new(lnurl_fixture.await);
    let lnurl_domain = lnurl.http_url().to_string();

    let temp_dir = TempDir::new("breez-sdk-bob-lnurl")?;

    // Generate random seed for Bob
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = Some(lnurl_domain.to_string());
    config.sync_interval_secs = 1; // Faster sync for testing
    config.real_time_sync_server_url = None;

    let mut sdk_instance = build_sdk_with_custom_config(
        temp_dir.path().to_string_lossy().to_string(),
        seed,
        config,
        Some(temp_dir),
        false,
    )
    .await?;
    sdk_instance.lnurl_fixture = Some(Arc::clone(&lnurl));
    Ok(sdk_instance)
}

// ---------------------
// Tests
// ---------------------

/// Test registering a Lightning address
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_register_lightning_address(#[future] bob_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_01_register_lightning_address ===");

    let bob = bob_sdk.await?;
    let username = "bobtest";

    // Register a Lightning address for Bob
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some("Bob's test Lightning address".to_string()),
        })
        .await?;

    info!(
        "Registered Lightning address: {}",
        register_response.lightning_address
    );

    // Verify the address format
    assert!(register_response.lightning_address.ends_with(&format!(
        "@{}",
        bob.lnurl_fixture.as_ref().unwrap().http_url().strip_prefix("http://").unwrap()
    )));
    assert!(register_response.lightning_address.starts_with(username));

    info!("=== Test test_01_register_lightning_address PASSED ===");
    Ok(())
}

/// Test checking Lightning address availability
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_check_lightning_address_available(
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_02_check_lightning_address_available ===");

    let bob = bob_sdk.await?;

    // Test available username
    let available_response = bob
        .sdk
        .check_lightning_address_available(CheckLightningAddressRequest {
            username: "availableuser".to_string(),
        })
        .await?;

    assert!(available_response);
    info!("Username 'availableuser' is available");

    // Register a username
    bob.sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: "takenuser".to_string(),
            description: Some("Test address".to_string()),
        })
        .await?;

    // Test taken username
    let taken_response = bob
        .sdk
        .check_lightning_address_available(CheckLightningAddressRequest {
            username: "takenuser".to_string(),
        })
        .await?;

    assert!(!taken_response);
    info!("Username 'takenuser' is not available");

    info!("=== Test test_02_check_lightning_address_available PASSED ===");
    Ok(())
}

/// Test getting Lightning address
#[rstest]
#[test_log::test(tokio::test)]
async fn test_03_get_lightning_address(#[future] bob_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_03_get_lightning_address ===");

    let bob = bob_sdk.await?;
    let username = "bobgettest";
    let description = "Bob's get test Lightning address";

    // Register an address first
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some(description.to_string()),
        })
        .await?;

    info!(
        "Registered Lightning address: {}",
        register_response.lightning_address
    );

    // Get the Lightning address
    let get_response = bob.sdk.get_lightning_address().await?;

    let Some(address_info) = get_response else {
        anyhow::bail!("Expected Lightning address info");
    };

    assert_eq!(
        address_info.lightning_address,
        register_response.lightning_address
    );
    assert_eq!(address_info.description, description.to_string());

    info!(
        "Retrieved Lightning address: {}",
        address_info.lightning_address
    );

    info!("=== Test test_03_get_lightning_address PASSED ===");
    Ok(())
}

/// Test deleting a Lightning address
#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_delete_lightning_address(#[future] bob_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_04_delete_lightning_address ===");

    let bob = bob_sdk.await?;
    let username = "bobdeletetest";

    // Register an address first
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some("Address to be deleted".to_string()),
        })
        .await?;

    info!(
        "Registered Lightning address: {}",
        register_response.lightning_address
    );

    // Verify it exists
    let get_response = bob.sdk.get_lightning_address().await?;
    let Some(address_info) = get_response else {
        anyhow::bail!("Expected Lightning address info");
    };
    assert_eq!(
        address_info.lightning_address,
        register_response.lightning_address
    );

    // Delete the address
    bob.sdk.delete_lightning_address().await?;

    info!("Deleted Lightning address");

    // Verify it's gone - should return None when trying to get it
    let get_result = bob.sdk.get_lightning_address().await?;

    assert!(
        get_result.is_none(),
        "Expected None when getting deleted address"
    );
    info!("Confirmed Lightning address was deleted");

    info!("=== Test test_04_delete_lightning_address PASSED ===");
    Ok(())
}

/// Test LNURL payments between Alice and Bob
#[rstest]
#[test_log::test(tokio::test)]
async fn test_05_lnurl_payment_flow(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_05_lnurl_payment_flow ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    let username = "bobpayment";
    let description = "Bob's payment test Lightning address";
    let payment_amount_sats = 5_000;
    let payment_comment = "Test payment from Alice";

    // Bob registers a Lightning address
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some(description.to_string()),
        })
        .await?;

    let bob_lightning_address = register_response.lightning_address;
    info!(
        "Bob registered Lightning address: {}",
        bob_lightning_address
    );

    // Fund Alice with sats for testing
    receive_and_fund(&mut alice, 50_000, false).await?;
    info!("Alice funded with sats");

    // Alice parses Bob's Lightning address
    let parse_response = alice.sdk.parse(&bob_lightning_address).await?;
    let InputType::LightningAddress(details) = parse_response else {
        anyhow::bail!("Expected Lightning address");
    };

    info!("Alice parsed Lightning address successfully");

    // Alice prepares LNURL pay request
    let prepare_response = alice
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount_sats: payment_amount_sats,
            pay_request: details.pay_request,
            comment: Some(payment_comment.to_string()),
            validate_success_action_url: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let amount_sats = prepare_response.amount_sats;
    info!("Alice prepared payment for {amount_sats} sats to {bob_lightning_address}");

    // Alice sends the payment
    let pay_response = alice
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response,
            idempotency_key: None,
        })
        .await?;

    info!("Alice initiated payment to Bob");

    // Wait for payment to complete on Alice's side
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;
    info!("Payment completed on Alice's side");

    // Wait for payment to complete on Alice's side
    let bob_payment_from_event =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    info!("Payment completed on Bob's side");

    // Verify payment details on Alice's side
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: pay_response.payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.amount, payment_amount_sats as u128);
    assert_eq!(alice_payment.method, PaymentMethod::Lightning);
    assert_eq!(alice_payment.status, PaymentStatus::Completed);

    let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = alice_payment.details else {
        anyhow::bail!("Expected Lightning payment");
    };
    let Some(lnurl_pay_info) = lnurl_pay_info else {
        anyhow::bail!("Expected Lnurl pay info");
    };
    assert_eq!(lnurl_pay_info.ln_address, Some(bob_lightning_address));
    assert_eq!(lnurl_pay_info.comment, Some(payment_comment.to_string()));
    assert_eq!(
        lnurl_pay_info.extract_description(),
        Some(description.to_string())
    );
    info!("LNURL pay info verified on Alice's side");

    // Bob should see the incoming payment
    let bob_payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: bob_payment_from_event.id,
        })
        .await?
        .payment;

    assert_eq!(bob_payment.payment_type, PaymentType::Receive);
    assert_eq!(bob_payment.amount, payment_amount_sats as u128);
    assert_eq!(bob_payment.method, PaymentMethod::Lightning);
    assert_eq!(bob_payment.status, PaymentStatus::Completed);

    let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = bob_payment.details else {
        anyhow::bail!("Expected Lightning payment");
    };
    assert!(lnurl_pay_info.is_none()); // Only payer sees LNURL pay info

    info!("=== Test test_05_lnurl_payment_flow PASSED ===");
    Ok(())
}

/// Test client-side zap receipt creation and publishing
/// Bob has private mode enabled (prefer_spark_over_lightning = true)
/// Alice sends a zap to Bob's lightning address
/// Verify Bob creates and publishes a zap receipt
#[rstest]
#[test_log::test(tokio::test)]
async fn test_06_client_side_zap_receipt(
    #[future] lnurl_fixture: LnurlFixture,
    #[future] alice_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_06_client_side_zap_receipt ===");

    // Setup Bob with private mode enabled
    let lnurl = Arc::new(lnurl_fixture.await);
    let lnurl_domain = lnurl.http_url().to_string();

    let temp_dir = TempDir::new("breez-sdk-bob-zap")?;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = Some(lnurl_domain.clone());
    config.sync_interval_secs = 1;
    config.real_time_sync_server_url = None;
    config.private_enabled_default = true;

    let mut bob = build_sdk_with_custom_config(
        temp_dir.path().to_string_lossy().to_string(),
        seed,
        config,
        Some(temp_dir),
        false,
    )
    .await?;
    bob.lnurl_fixture = Some(Arc::clone(&lnurl));

    let mut alice = alice_sdk.await?;

    // Bob registers a Lightning address
    let username = "bobzap";
    let description = "Bob's zap test Lightning address";

    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some(description.to_string()),
        })
        .await?;

    let bob_lightning_address = register_response.lightning_address.clone();
    info!(
        "Bob registered Lightning address: {}",
        bob_lightning_address
    );

    // Fund Alice with sats for testing
    receive_and_fund(&mut alice, 50_000, false).await?;
    info!("Alice funded with sats");

    // Wait for sync to complete
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Alice parses Bob's Lightning address
    let details = match alice.sdk.parse(&bob_lightning_address).await? {
        InputType::LightningAddress(address) => address,
        _ => anyhow::bail!("Expected Lightning address"),
    };

    assert_eq!(details.pay_request.allows_nostr, Some(true));
    assert_eq!(details.pay_request.comment_allowed, 255u16);
    assert!(details.pay_request.nostr_pubkey.is_some());
    let bob_nostr_pubkey = details.pay_request.nostr_pubkey.unwrap();

    // Create a properly signed zap request (NIP-57 kind 9734 event) using the nostr crate
    let payment_amount_sats = 1000_u64;

    // Generate a temporary key for Alice (in production, this would be Alice's actual nostr key)
    let alice_keys = nostr::Keys::generate();

    // Parse Bob's nostr public key
    let bob_pubkey = nostr::PublicKey::from_hex(&bob_nostr_pubkey)?;

    // Build the zap request event
    let zap_request_builder =
        nostr::EventBuilder::new(nostr::Kind::ZapRequest, "Test zap from Alice to Bob")
            .tag(nostr::Tag::public_key(bob_pubkey))
            .tag(nostr::Tag::custom(
                nostr::TagKind::Custom(std::borrow::Cow::Borrowed("amount")),
                vec![(payment_amount_sats * 1000).to_string()],
            ))
            .tag(nostr::Tag::custom(
                nostr::TagKind::Custom(std::borrow::Cow::Borrowed("relays")),
                // Note there's nothing listening on this relay, but this makes the test pass.
                vec!["ws://localhost:7777".to_string()],
            ));

    let zap_request_event = zap_request_builder.sign_with_keys(&alice_keys)?;

    let zap_request_str = zap_request_event.as_json();

    info!("Created properly signed zap request using nostr crate");

    // For this test, we need to manually trigger the zap request flow
    // since the SDK doesn't automatically include zap requests yet
    //
    // Step 1: Get an invoice from the LNURL server with the zap request
    let encoded_zap = percent_encode(&zap_request_str);

    let callback_url = format!(
        "{}?amount={}&nostr={encoded_zap}&comment={}",
        details.pay_request.callback,
        payment_amount_sats * 1000, // amount in millisats
        percent_encode("Test zap from Alice to Bob")
    );

    info!("Calling LNURL callback with zap request: {callback_url}");

    let http_client = DefaultHttpClient::default();
    let response = http_client
        .get(callback_url.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send callback request: {e:?}"))?;

    if !(200..300).contains(&response.status) {
        anyhow::bail!("Callback request failed: {}", response.status);
    }

    let callback_json: serde_json::Value = response.json()?;
    info!("Callback response: {}", callback_json);

    // Extract the invoice from the callback response
    let invoice = callback_json["pr"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No invoice in callback response: {callback_json}"))?
        .to_string();

    info!("Got invoice with zap request: {invoice}");

    // Step 2: Alice pays the invoice using the standard send_payment flow
    let prepare_response = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: invoice.clone(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let _pay_response = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!("Alice initiated payment to Bob");

    // Wait for payment to complete on both sides
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;
    info!("Payment completed on Alice's side");

    let bob_payment_from_event =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    info!("Payment completed on Bob's side");

    // Verify bob_payment_from_event has all metadata
    let Some(PaymentDetails::Lightning {
        lnurl_receive_metadata: event_metadata,
        ..
    }) = &bob_payment_from_event.details
    else {
        anyhow::bail!("Expected Lightning payment in bob_payment_from_event");
    };

    let Some(event_lnurl_metadata) = event_metadata else {
        anyhow::bail!("Expected LNURL receive metadata in bob_payment_from_event");
    };

    // Verify zap request was stored in the event
    assert!(
        event_lnurl_metadata.nostr_zap_request.is_some(),
        "Zap request should be stored in bob_payment_from_event"
    );
    info!("Verified zap request is stored in bob_payment_from_event");

    // Verify comment is present in the event
    assert!(
        event_lnurl_metadata.sender_comment.is_some(),
        "Comment should be stored in bob_payment_from_event"
    );
    assert_eq!(
        event_lnurl_metadata.sender_comment.as_deref(),
        Some("Test zap from Alice to Bob"),
        "Comment should match in bob_payment_from_event"
    );
    info!("Verified comment is stored in bob_payment_from_event");

    // Wait for Bob's SDK to sync metadata and process zap receipts
    // Poll until the zap receipt is present in the payment
    let payment_id = bob_payment_from_event.id.clone();
    let bob_sdk = bob.sdk.clone();

    let payment_lnurl_metadata = wait_for(
        || async {
            debug!("Checking for zap receipt in Bob's payment {}", payment_id);
            let payment = bob_sdk
                .get_payment(GetPaymentRequest {
                    payment_id: payment_id.clone(),
                })
                .await?
                .payment;

            let Some(PaymentDetails::Lightning {
                lnurl_receive_metadata,
                ..
            }) = payment.details
            else {
                anyhow::bail!("Expected Lightning payment");
            };

            let Some(metadata) = lnurl_receive_metadata else {
                anyhow::bail!("Expected LNURL receive metadata");
            };

            if metadata.nostr_zap_receipt.is_none() {
                anyhow::bail!("Zap receipt not yet created");
            }

            Ok(metadata)
        },
        20,
    )
    .await?;

    info!("Zap receipt detected in Bob's payment");

    // NOTE: The zap receipt will not be present in the event because it is created
    // asynchronously after the payment event is generated.

    // Now verify bob_payment (fetched separately) also has all metadata
    // Verify zap request was stored in bob_payment
    assert!(
        payment_lnurl_metadata.nostr_zap_request.is_some(),
        "Zap request should be stored in bob_payment"
    );
    info!("Verified zap request is stored in bob_payment");

    // Verify comment is present in bob_payment
    assert!(
        payment_lnurl_metadata.sender_comment.is_some(),
        "Comment should be stored in bob_payment"
    );
    assert_eq!(
        payment_lnurl_metadata.sender_comment.as_deref(),
        Some("Test zap from Alice to Bob"),
        "Comment should match in bob_payment"
    );
    info!("Verified comment is stored in bob_payment");

    // Verify zap receipt was created and published in bob_payment
    assert!(
        payment_lnurl_metadata.nostr_zap_receipt.is_some(),
        "Zap receipt should be created and stored in bob_payment"
    );

    let zap_receipt_json = payment_lnurl_metadata.nostr_zap_receipt.unwrap();
    info!("Zap receipt created: {}", zap_receipt_json);

    // Parse and validate the zap receipt
    let zap_receipt: serde_json::Value = serde_json::from_str(&zap_receipt_json)?;
    assert_eq!(
        zap_receipt["kind"].as_i64(),
        Some(9735),
        "Zap receipt should be kind 9735"
    );
    info!("Verified zap receipt is valid NIP-57 event (kind 9735)");

    info!("=== Test test_06_client_side_zap_receipt PASSED ===");
    Ok(())
}

fn percent_encode(input: &str) -> Cow<'_, str> {
    let mut result = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{:02X}", byte));
            }
        }
    }
    Cow::Owned(result)
}

/// Test LNURL full balance payment - sends entire balance via LNURL
#[rstest]
#[test_log::test(tokio::test)]
async fn test_07_lnurl_send_all_payment(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_07_lnurl_send_all_payment ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    let username = "bobfullbalance";
    let description = "Bob's full balance test Lightning address";

    // Bob registers a Lightning address
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some(description.to_string()),
        })
        .await?;

    let bob_lightning_address = register_response.lightning_address;
    info!(
        "Bob registered Lightning address: {}",
        bob_lightning_address
    );

    // Fund Alice with a specific amount for testing
    let funding_amount = 10_000u64;
    receive_and_fund(&mut alice, funding_amount, false).await?;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice balance after funding: {} sats", alice_balance);

    // Alice parses Bob's Lightning address
    let parse_response = alice.sdk.parse(&bob_lightning_address).await?;
    let InputType::LightningAddress(details) = parse_response else {
        anyhow::bail!("Expected Lightning address");
    };

    info!("Alice parsed Lightning address successfully");
    info!(
        "LNURL min_sendable: {} msats, max_sendable: {} msats",
        details.pay_request.min_sendable, details.pay_request.max_sendable
    );

    // Alice prepares LNURL FeesIncluded (sends entire balance)
    let prepare_response = alice
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount_sats: alice_balance,
            pay_request: details.pay_request,
            comment: Some("FeesIncluded test from Alice".to_string()),
            validate_success_action_url: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;

    // For FeesIncluded, amount is in invoice details
    let invoice_amount_sats = prepare_response.invoice_details.amount_msat.unwrap() / 1000;
    info!(
        "Alice prepared FeesIncluded payment: {invoice_amount_sats} sats (fee: {} sats)",
        prepare_response.fee_sats
    );

    // Verify the invoice amount is exactly balance - fees
    let expected_amount = alice_balance.saturating_sub(prepare_response.fee_sats);
    assert_eq!(
        invoice_amount_sats, expected_amount,
        "Invoice amount should be exactly balance - fees"
    );

    // Alice sends the full balance
    let pay_response = alice
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response: prepare_response.clone(),
            idempotency_key: None,
        })
        .await?;

    info!("Alice initiated full balance payment to Bob");

    // Wait for payment to complete on both sides
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;
    info!("Payment completed on Alice's side");

    wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    info!("Payment completed on Bob's side");

    // Verify Alice's balance is now zero
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice final balance: {} sats", alice_final);

    assert_eq!(alice_final, 0, "Alice's balance should be fully spent");

    // Verify payment details
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: pay_response.payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.method, PaymentMethod::Lightning);
    assert_eq!(alice_payment.status, PaymentStatus::Completed);
    assert_eq!(alice_payment.amount, expected_amount.into());
    assert_eq!(alice_payment.fees, prepare_response.fee_sats.into());

    info!("=== Test test_07_lnurl_send_all_payment PASSED ===");
    Ok(())
}

/// Test LNURL full balance payment with fee overpayment - verifies it works when fee steps down
///
/// This test specifically targets the scenario where:
/// - fee(balance) > fee(balance - fee(balance))
/// - The SDK must overpay the fee to fully spend the balance
#[rstest]
#[test_log::test(tokio::test)]
async fn test_08_lnurl_send_all_with_fee_overpayment(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_08_lnurl_send_all_with_fee_overpayment ===");

    let mut alice = alice_sdk.await?;
    let mut bob = bob_sdk.await?;

    let username = "boboverpay";
    let description = "Bob's overpayment test Lightning address";

    // Bob registers a Lightning address
    let register_response = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some(description.to_string()),
        })
        .await?;

    let bob_lightning_address = register_response.lightning_address;
    info!(
        "Bob registered Lightning address: {}",
        bob_lightning_address
    );

    // Fund Alice with max faucet amount to have room for searching
    let funding_amount = 50_000u64;
    receive_and_fund(&mut alice, funding_amount, false).await?;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice balance after funding: {} sats", alice_balance);

    // Parse Bob's Lightning address to get the pay request
    let parse_response = alice.sdk.parse(&bob_lightning_address).await?;
    let InputType::LightningAddress(details) = parse_response else {
        anyhow::bail!("Expected Lightning address");
    };

    let min_sendable_sats = details.pay_request.min_sendable.div_ceil(1000);
    info!(
        "LNURL min_sendable: {} sats, max_sendable: {} sats",
        min_sendable_sats,
        details.pay_request.max_sendable / 1000
    );

    // Search for a balance where fee stepping occurs using binary search
    // We look for the fee tier boundary - stepping naturally occurs there
    info!("Searching for fee tier boundary using binary search...");

    // Helper to get fee for an amount
    async fn get_fee(
        sdk: &BreezSdk,
        pay_request: &LnurlPayRequestDetails,
        amount: u64,
    ) -> Result<u64> {
        let prepare = sdk
            .prepare_lnurl_pay(PrepareLnurlPayRequest {
                amount_sats: amount,
                pay_request: pay_request.clone(),
                comment: None,
                validate_success_action_url: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;
        Ok(prepare.fee_sats)
    }

    let fee_at_min = get_fee(&alice.sdk, &details.pay_request, min_sendable_sats).await?;
    let fee_at_max = get_fee(&alice.sdk, &details.pay_request, alice_balance).await?;

    info!(
        "Fee at min ({} sats): {} sats, fee at max ({} sats): {} sats",
        min_sendable_sats, fee_at_min, alice_balance, fee_at_max
    );

    if fee_at_min >= fee_at_max {
        anyhow::bail!(
            "No fee tier boundary found - fees are constant or decreasing ({} -> {})",
            fee_at_min,
            fee_at_max
        );
    }

    // Binary search to find where fee changes
    let mut low = min_sendable_sats;
    let mut high = alice_balance;
    let fee_low = fee_at_min;

    while high - low > 1 {
        let mid = low + (high - low) / 2;
        let fee_mid = get_fee(&alice.sdk, &details.pay_request, mid).await?;
        debug!(
            "Binary search: low={}, mid={}, high={}, fee_mid={}",
            low, mid, high, fee_mid
        );
        if fee_mid == fee_low {
            low = mid;
        } else {
            high = mid;
        }
    }

    // high is now the boundary where fee increases
    let target_balance = high;
    let fee1 = get_fee(&alice.sdk, &details.pay_request, target_balance).await?;
    let adjusted = target_balance.saturating_sub(fee1);
    let fee2 = get_fee(&alice.sdk, &details.pay_request, adjusted).await?;

    info!(
        "Found fee tier boundary at {} sats: fee1={}, fee2={} (for adjusted={})",
        target_balance, fee1, fee2, adjusted
    );

    if fee2 >= fee1 {
        anyhow::bail!(
            "Fee stepping not found at boundary: fee({})={}, fee({})={}",
            target_balance,
            fee1,
            adjusted,
            fee2
        );
    }

    let (expected_fee1, expected_fee2) = (fee1, fee2);

    info!(
        "Using stepping balance: {} sats (fee will step from {} to {})",
        target_balance, expected_fee1, expected_fee2
    );

    // Adjust Alice's balance to target using Spark transfer
    if alice_balance > target_balance {
        let excess = alice_balance - target_balance;
        info!(
            "Adjusting Alice's balance: sending {} sats to Bob via Spark",
            excess
        );

        // Bob creates a Spark address
        let bob_spark_address = bob
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request;

        // Alice sends excess to Bob
        let prepare = alice
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: bob_spark_address,
                amount: Some(excess as u128),
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        alice
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: prepare,
                options: None,
                idempotency_key: None,
            })
            .await?;

        // Wait for Spark transfer to complete
        wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 60).await?;
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;

        // Sync and verify
        alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
        let new_balance = alice
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;

        info!(
            "Alice balance after adjustment: {} sats (target was {})",
            new_balance, target_balance
        );
        assert_eq!(
            new_balance, target_balance,
            "Alice's balance should match target"
        );
    }

    // Execute payment
    info!("Executing payment with fee overpayment...");

    let prepare_response = alice
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount_sats: target_balance,
            pay_request: details.pay_request,
            comment: Some("FeesIncluded with overpayment test".to_string()),
            validate_success_action_url: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;

    // For FeesIncluded, amount is in invoice details
    let invoice_amount_sats = prepare_response.invoice_details.amount_msat.unwrap() / 1000;
    info!(
        "Prepared payment: amount={invoice_amount_sats} sats, fee={} sats",
        prepare_response.fee_sats
    );

    // The fee should be expected_fee1 (the higher fee for full balance)
    assert_eq!(
        prepare_response.fee_sats, expected_fee1,
        "Payment fee should match expected fee for full balance"
    );

    // Execute the full balance payment
    let pay_response = alice
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response: prepare_response.clone(),
            idempotency_key: None,
        })
        .await?;

    info!("Full balance payment initiated");

    // Wait for payment to complete on both sides
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;
    info!("Full balance payment completed on Alice's side");

    let bob_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    info!("Full balance payment completed on Bob's side");

    // Verify Alice's balance is zero
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let alice_final = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    info!("Alice final balance: {} sats", alice_final);

    assert_eq!(alice_final, 0, "Alice's balance should be fully spent");

    // Verify Bob received the expected amount (the invoice amount, not more)
    assert_eq!(
        bob_payment.amount,
        invoice_amount_sats.into(),
        "Bob should receive exactly the prepared amount"
    );

    // Verify payment details
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: pay_response.payment.id,
        })
        .await?
        .payment;

    assert_eq!(alice_payment.payment_type, PaymentType::Send);
    assert_eq!(alice_payment.method, PaymentMethod::Lightning);
    assert_eq!(alice_payment.status, PaymentStatus::Completed);

    info!(
        "Fee overpayment test passed! Expected overpayment: {} sats",
        expected_fee1 - expected_fee2
    );
    info!("=== Test test_08_lnurl_send_all_with_fee_overpayment PASSED ===");
    Ok(())
}
