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

/// Fixture: DataSync service for RTSync testing
#[fixture]
async fn data_sync_fixture() -> DataSyncFixture {
    DataSyncFixture::new()
        .await
        .expect("Failed to start DataSync service")
}

/// Fixture: Lnurl service for RTSync testing
#[fixture]
async fn lnurl_fixture() -> LnurlFixture {
    LnurlFixture::new()
        .await
        .expect("Failed to start Lnurl service")
}

/// Fixture: Alice seed fixture
#[fixture]
fn alice_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    seed
}

/// Fixture: Alice SDKs with shared RTSync service
#[fixture]
async fn alice_sdks(
    #[future] data_sync_fixture: DataSyncFixture,
    alice_seed: [u8; 32],
) -> Result<(SdkInstance, SdkInstance)> {
    let data_sync = Arc::new(data_sync_fixture.await);
    let sync_url = data_sync.grpc_url().to_string();

    let mut alice1 = create_sdk_with_rtsync("alice1", alice_seed, &sync_url).await?;
    alice1.data_sync_fixture = Some(Arc::clone(&data_sync));

    let mut alice2 = create_sdk_with_rtsync("alice2", alice_seed, &sync_url).await?;
    alice2.data_sync_fixture = Some(Arc::clone(&data_sync));

    Ok((alice1, alice2))
}

/// Fixture: Bob's SDK with Lnurl configured
#[fixture]
async fn bob_sdk(#[future] lnurl_fixture: LnurlFixture) -> Result<SdkInstance> {
    let lnurl = Arc::new(lnurl_fixture.await);
    let lnurl_domain = lnurl.http_url().to_string();

    let temp_dir = TempDir::new("breez-sdk-bob")?;

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
// Helper Functions
// ---------------------

async fn create_sdk_with_rtsync(name: &str, seed: [u8; 32], sync_url: &str) -> Result<SdkInstance> {
    create_sdk_with_rtsync_and_lnurl(name, seed, sync_url, None).await
}

async fn create_sdk_with_rtsync_and_lnurl(
    name: &str,
    seed: [u8; 32],
    sync_url: &str,
    lnurl_domain: Option<String>,
) -> Result<SdkInstance> {
    let temp_dir = TempDir::new(&format!("breez-sdk-{name}"))?;

    let mut config = default_config(Network::Regtest);
    config.api_key = None;
    config.prefer_spark_over_lightning = true;
    config.sync_interval_secs = 1;
    config.real_time_sync_server_url = Some(sync_url.to_string());
    config.lnurl_domain = lnurl_domain;

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

/// Test real-time synchronization of payment metadata between multiple SDK instances
/// using data-sync service.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_rtsync_lnurl_info_sync(
    #[future] alice_sdks: Result<(SdkInstance, SdkInstance)>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_rtsync_lnurl_info_sync ===");

    let (mut alice1, mut alice2) = alice_sdks.await?;
    let bob = bob_sdk.await?;

    let ln_address_description = "Bob's Lightning address description".to_string();
    let ln_address_comment = "Test payment".to_string();

    // Fund Alice with sats for testing (allow other SDK instance to claim)
    receive_and_fund(&mut alice1, 50_000, false).await?;
    info!("Alice funded with sats");

    // Bob creates a Lightning address for receiving payments
    let bob_lightning_address = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: "bob".to_string(),
            description: Some(ln_address_description.clone()),
        })
        .await?
        .lightning_address;

    info!("Bob's Lightning address: {}", bob_lightning_address);

    // Alice1 prepares and sends payment to Bob
    let parse_response = alice1.sdk.parse(&bob_lightning_address).await?;
    let InputType::LightningAddress(details) = parse_response else {
        anyhow::bail!("Expected Lightning address");
    };

    let prepare_response = alice1
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount_sats: 10_000,
            pay_request: details.pay_request,
            comment: Some(ln_address_comment.clone()),
            validate_success_action_url: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let amount_sats = prepare_response.amount_sats;
    info!("Alice1 prepared payment for {amount_sats} sats");

    let pay_response = alice1
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response,
            idempotency_key: None,
        })
        .await?;
    info!("Alice1 initiated payment to Bob");

    // Wait for payment to complete on Alice1
    wait_for_payment_succeeded_event(&mut alice1.events, PaymentType::Send, 30).await?;
    info!("Payment completed on Alice1");

    // Wait for data-sync to propagate payment metadata to Alice2
    wait_for_synced_event(&mut alice2.events, 30).await?;
    alice2.sdk.sync_wallet(SyncWalletRequest {}).await?;

    // Alice2 should now see the payment, including LNURL information
    let alice2_payment = alice2
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: pay_response.payment.id,
        })
        .await?
        .payment;

    let Some(PaymentDetails::Lightning { lnurl_pay_info, .. }) = alice2_payment.details else {
        anyhow::bail!("Expected Lightning payment");
    };

    let Some(lnurl_pay_info) = lnurl_pay_info else {
        anyhow::bail!("Expected Lnurl pay info");
    };

    assert_eq!(lnurl_pay_info.ln_address, Some(bob_lightning_address));
    assert_eq!(lnurl_pay_info.comment, Some(ln_address_comment));
    assert_eq!(
        lnurl_pay_info.extract_description(),
        Some(ln_address_description)
    );

    info!("=== Test test_01_rtsync_lnurl_info_sync PASSED ===");
    Ok(())
}

/// Test real-time synchronization of lightning address changes between SDK instances.
/// Instance 1 registers a lightning address, instance 2 receives the change event.
/// Instance 1 deletes the address, instance 2 receives the deletion event.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_rtsync_lightning_address_sync(
    #[future] data_sync_fixture: DataSyncFixture,
    #[future] lnurl_fixture: LnurlFixture,
    alice_seed: [u8; 32],
) -> Result<()> {
    info!("=== Starting test_02_rtsync_lightning_address_sync ===");

    let data_sync = Arc::new(data_sync_fixture.await);
    let sync_url = data_sync.grpc_url().to_string();
    let lnurl = Arc::new(lnurl_fixture.await);
    let lnurl_domain = lnurl.http_url().to_string();

    // Create two instances from the same seed with rtsync + lnurl
    let mut alice1 = create_sdk_with_rtsync_and_lnurl(
        "alice1-la",
        alice_seed,
        &sync_url,
        Some(lnurl_domain.clone()),
    )
    .await?;
    alice1.data_sync_fixture = Some(Arc::clone(&data_sync));
    alice1.lnurl_fixture = Some(Arc::clone(&lnurl));

    let mut alice2 = create_sdk_with_rtsync_and_lnurl(
        "alice2-la",
        alice_seed,
        &sync_url,
        Some(lnurl_domain.clone()),
    )
    .await?;
    alice2.data_sync_fixture = Some(Arc::clone(&data_sync));
    alice2.lnurl_fixture = Some(Arc::clone(&lnurl));

    // Instance 1 registers a lightning address
    let registered = alice1
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: "alicesync".to_string(),
            description: Some("Alice's synced address".to_string()),
        })
        .await?;
    info!(
        "Alice1 registered lightning address: {}",
        registered.lightning_address
    );

    // Instance 2 should receive a LightningAddressChanged event
    let changed_addr = wait_for_lightning_address_changed_event(&mut alice2.events, 30).await?;
    let changed_addr = changed_addr.expect("Expected Some(address) after register");
    assert_eq!(changed_addr.lightning_address, registered.lightning_address);
    assert_eq!(changed_addr.username, registered.username);
    info!(
        "Alice2 received LightningAddressChanged: {}",
        changed_addr.lightning_address
    );

    // Verify alice2 can also fetch it via the API
    let alice2_addr = alice2.sdk.get_lightning_address().await?;
    assert_eq!(
        alice2_addr.as_ref().map(|a| &a.lightning_address),
        Some(&registered.lightning_address)
    );
    info!("Alice2 get_lightning_address matches");

    // Instance 1 deletes the lightning address
    alice1.sdk.delete_lightning_address().await?;
    info!("Alice1 deleted lightning address");

    // Instance 2 should receive a LightningAddressChanged event with None
    let deleted_addr = wait_for_lightning_address_changed_event(&mut alice2.events, 30).await?;
    assert!(
        deleted_addr.is_none(),
        "Expected None after delete, got: {deleted_addr:?}"
    );
    info!("Alice2 received LightningAddressChanged: None");

    // Verify alice2's API also returns None
    let alice2_addr = alice2.sdk.get_lightning_address().await?;
    assert!(
        alice2_addr.is_none(),
        "Expected None from get_lightning_address after delete"
    );
    info!("Alice2 get_lightning_address returns None");

    info!("=== Test test_02_rtsync_lightning_address_sync PASSED ===");
    Ok(())
}
