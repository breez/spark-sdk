use std::sync::Arc;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
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

/// Fixture: Alice seed fixture
#[fixture]
fn alice_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    seed
}

// ---------------------
// Helper Functions
// ---------------------

/// A wallet with identity `seed` that syncs through `data_sync`, and registers
/// lightning addresses with `lnurl` when given one.
async fn create_sdk_with_rtsync(
    env: &Environment,
    seed: [u8; 32],
    data_sync: &Arc<DataSyncFixture>,
    lnurl: Option<&Arc<LnurlFixture>>,
) -> Result<SdkInstance> {
    let sync_url = data_sync.grpc_url().to_string();
    let lnurl_domain = lnurl.map(|lnurl| lnurl.http_url().to_string());
    let mut sdk = env
        .create_wallet_from_seed(seed, move |config| {
            config.sync_interval_secs = 1;
            config.real_time_sync_server_url = Some(sync_url);
            config.lnurl_domain = lnurl_domain;
        })
        .await?;
    sdk.data_sync_fixture = Some(Arc::clone(data_sync));
    sdk.lnurl_fixture = lnurl.cloned();
    Ok(sdk)
}

/// Bob's wallet, which registers lightning addresses with `lnurl`.
async fn create_bob_sdk(env: &Environment, lnurl: &Arc<LnurlFixture>) -> Result<SdkInstance> {
    let lnurl_domain = lnurl.http_url().to_string();
    let mut sdk = env
        .create_wallet_with(move |config| {
            config.sync_interval_secs = 1;
            config.lnurl_domain = Some(lnurl_domain);
        })
        .await?;
    sdk.lnurl_fixture = Some(Arc::clone(lnurl));
    Ok(sdk)
}

// ---------------------
// Tests
// ---------------------

/// Test real-time synchronization of payment metadata between multiple SDK instances
/// using data-sync service.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_rtsync_lnurl_info_sync(
    #[future] env: Result<Environment>,
    #[future] data_sync_fixture: DataSyncFixture,
    alice_seed: [u8; 32],
) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_01_rtsync_lnurl_info_sync ===");

    let data_sync = Arc::new(data_sync_fixture.await);
    let mut alice1 = create_sdk_with_rtsync(&env, alice_seed, &data_sync, None).await?;
    let mut alice2 = create_sdk_with_rtsync(&env, alice_seed, &data_sync, None).await?;
    let lnurl = Arc::new(env.lnurl_server().await?);
    let bob = create_bob_sdk(&env, &lnurl).await?;

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
            amount: 10_000,
            pay_request: details.pay_request,
            comment: Some(ln_address_comment.clone()),
            validate_success_action_url: None,
            token_identifier: None,
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
    #[future] env: Result<Environment>,
    #[future] data_sync_fixture: DataSyncFixture,
    alice_seed: [u8; 32],
) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_02_rtsync_lightning_address_sync ===");

    let data_sync = Arc::new(data_sync_fixture.await);
    let lnurl = Arc::new(env.lnurl_server().await?);

    // Create two instances from the same seed with rtsync + lnurl
    let alice1 = create_sdk_with_rtsync(&env, alice_seed, &data_sync, Some(&lnurl)).await?;
    let mut alice2 = create_sdk_with_rtsync(&env, alice_seed, &data_sync, Some(&lnurl)).await?;

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
