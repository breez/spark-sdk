use std::sync::Arc;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tracing::info;

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
        })
        .await?;

    info!(
        "Alice prepared payment for {} sats to {}",
        prepare_response.amount_sats, bob_lightning_address
    );

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
    assert_eq!(alice_payment.amount, payment_amount_sats.into());
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
    assert_eq!(bob_payment.amount, payment_amount_sats.into());
    assert_eq!(bob_payment.method, PaymentMethod::Lightning);
    assert_eq!(bob_payment.status, PaymentStatus::Completed);

    let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = bob_payment.details else {
        anyhow::bail!("Expected Lightning payment");
    };
    assert!(lnurl_pay_info.is_none()); // Only payer sees LNURL pay info

    info!("=== Test test_05_lnurl_payment_flow PASSED ===");
    Ok(())
}
