use std::sync::Arc;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use platform_utils::{DefaultHttpClient, HttpClient};
use rand::RngCore;
use rstest::*;
use tempdir::TempDir;
use tracing::{Instrument, debug, info};

// ---------------------
// Setup helpers
// ---------------------

/// Start an LNURL server fixture.
/// When `use_postgres` is true the server runs against a PostgreSQL
/// testcontainer; otherwise it uses in-memory SQLite.
async fn setup_lnurl(use_postgres: bool) -> LnurlFixture {
    if use_postgres {
        LnurlFixture::new_with_postgres()
            .await
            .expect("Failed to start Lnurl service with PostgreSQL")
    } else {
        LnurlFixture::new()
            .await
            .expect("Failed to start Lnurl service")
    }
}

/// Fixture: Alice SDK for LNURL testing (sender)
#[fixture]
async fn alice_sdk() -> Result<SdkInstance> {
    async {
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
    .instrument(tracing::info_span!(target: "breez_sdk_spark", "alice"))
    .await
}

/// Set up Bob SDK with an LNURL server (receiver).
/// When `use_postgres` is true the LNURL server runs against a PostgreSQL
/// testcontainer; otherwise it uses in-memory SQLite.
async fn setup_bob(use_postgres: bool) -> Result<SdkInstance> {
    async {
        let lnurl = Arc::new(setup_lnurl(use_postgres).await);
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
    .instrument(tracing::info_span!(target: "breez_sdk_spark", "bob"))
    .await
}

// ---------------------
// Tests
// ---------------------

/// Test registering a Lightning address
#[rstest]
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_01_register_lightning_address(#[case] use_postgres: bool) -> Result<()> {
    info!("=== Starting test_01_register_lightning_address ===");

    let bob = setup_bob(use_postgres).await?;
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
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_02_check_lightning_address_available(#[case] use_postgres: bool) -> Result<()> {
    info!("=== Starting test_02_check_lightning_address_available ===");

    let bob = setup_bob(use_postgres).await?;

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
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_03_get_lightning_address(#[case] use_postgres: bool) -> Result<()> {
    info!("=== Starting test_03_get_lightning_address ===");

    let bob = setup_bob(use_postgres).await?;
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
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_04_delete_lightning_address(#[case] use_postgres: bool) -> Result<()> {
    info!("=== Starting test_04_delete_lightning_address ===");

    let bob = setup_bob(use_postgres).await?;
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
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_05_lnurl_payment_flow(
    #[future] alice_sdk: Result<SdkInstance>,

    #[case] use_postgres: bool,
) -> Result<()> {
    info!("=== Starting test_05_lnurl_payment_flow ===");

    let mut alice = alice_sdk.await?;
    let mut bob = setup_bob(use_postgres).await?;

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
            amount: payment_amount_sats as u128,
            pay_request: details.pay_request,
            comment: Some(payment_comment.to_string()),
            validate_success_action_url: None,
            token_identifier: None,
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

/// Fixture: Lnurl service with include_spark_address enabled
#[fixture]
async fn lnurl_spark_address_fixture() -> LnurlFixture {
    LnurlFixture::with_config(LnurlImageConfig::default().with_include_spark_address(true))
        .await
        .expect("Failed to start Lnurl service with Spark address")
}

/// Test LNURL full balance payment - sends entire balance via LNURL
#[rstest]
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_07_lnurl_send_all_payment(
    #[future] alice_sdk: Result<SdkInstance>,

    #[case] use_postgres: bool,
) -> Result<()> {
    info!("=== Starting test_07_lnurl_send_all_payment ===");

    let mut alice = alice_sdk.await?;
    let mut bob = setup_bob(use_postgres).await?;

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
            amount: alice_balance.into(),
            pay_request: details.pay_request,
            comment: Some("FeesIncluded test from Alice".to_string()),
            validate_success_action_url: None,
            token_identifier: None,
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
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_08_lnurl_send_all_with_fee_overpayment(
    #[future] alice_sdk: Result<SdkInstance>,

    #[case] use_postgres: bool,
) -> Result<()> {
    info!("=== Starting test_08_lnurl_send_all_with_fee_overpayment ===");

    let mut alice = alice_sdk.await?;
    let mut bob = setup_bob(use_postgres).await?;

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
                amount: amount.into(),
                pay_request: pay_request.clone(),
                comment: None,
                validate_success_action_url: None,
                token_identifier: None,
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
                payment_request: PaymentRequest::Input(bob_spark_address),
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

        // Wait for Spark transfer to complete (filter by method to avoid picking up other payments)
        wait_for_payment_succeeded_event_with_method(
            &mut alice.events,
            PaymentType::Send,
            PaymentMethod::Spark,
            60,
        )
        .await?;
        wait_for_payment_succeeded_event_with_method(
            &mut bob.events,
            PaymentType::Receive,
            PaymentMethod::Spark,
            60,
        )
        .await?;

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
            amount: target_balance.into(),
            pay_request: details.pay_request,
            comment: Some("FeesIncluded with overpayment test".to_string()),
            validate_success_action_url: None,
            token_identifier: None,
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

    // Wait for payment to complete on both sides (filter by Lightning method)
    wait_for_payment_succeeded_event_with_method(
        &mut alice.events,
        PaymentType::Send,
        PaymentMethod::Lightning,
        30,
    )
    .await?;
    info!("Full balance payment completed on Alice's side");

    let bob_payment = wait_for_payment_succeeded_event_with_method(
        &mut bob.events,
        PaymentType::Receive,
        PaymentMethod::Lightning,
        30,
    )
    .await?;
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

/// Test that the invoice expiry query parameter is passed through to the generated invoice
#[rstest]
#[case::sqlite(false)]
#[case::postgres(true)]
#[test_log::test(tokio::test)]
async fn test_09_invoice_expiry_parameter(#[case] use_postgres: bool) -> Result<()> {
    info!("=== Starting test_09_invoice_expiry_parameter ===");

    let bob = setup_bob(use_postgres).await?;
    let username = "bobexpiry";

    // Register a Lightning address for Bob
    bob.sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: username.to_string(),
            description: Some("Expiry test address".to_string()),
        })
        .await?;

    // Parse the lightning address to get the callback URL
    let bob_lnurl_domain = bob
        .lnurl_fixture
        .as_ref()
        .unwrap()
        .http_url()
        .strip_prefix("http://")
        .unwrap();
    let lightning_address = format!("{username}@{bob_lnurl_domain}");
    let parse_response = bob.sdk.parse(&lightning_address).await?;
    let InputType::LightningAddress(details) = parse_response else {
        anyhow::bail!("Expected Lightning address");
    };

    let callback = &details.pay_request.callback;
    let amount_msat = 10_000_000; // 10k sats in msats
    let custom_expiry_secs = 600_u32;

    // Request invoice WITH custom expiry
    let http_client = DefaultHttpClient::default();
    let url_with_expiry = format!("{callback}?amount={amount_msat}&expiry={custom_expiry_secs}");
    let response = http_client
        .get(url_with_expiry.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("Invoice request failed: {e:?}"))?;
    assert!(
        response.is_success(),
        "Invoice request failed: {}",
        response.status
    );
    let json: serde_json::Value = response.json()?;
    let invoice_str = json["pr"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No invoice in response: {json}"))?;

    info!("Got invoice string {invoice_str}");
    // Parse the invoice and verify the expiry
    let parsed = bob.sdk.parse(invoice_str).await?;
    let InputType::Bolt11Invoice(invoice_details) = parsed else {
        anyhow::bail!("Expected Bolt11Invoice");
    };
    assert_eq!(
        invoice_details.expiry, custom_expiry_secs as u64,
        "Invoice expiry should match the requested expiry parameter"
    );
    info!(
        "Invoice with custom expiry verified: {} secs",
        invoice_details.expiry
    );

    // Request invoice WITHOUT expiry (should use server default)
    let url_without_expiry = format!("{callback}?amount={amount_msat}");
    let response = http_client
        .get(url_without_expiry.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("Invoice request failed: {e:?}"))?;
    assert!(
        response.is_success(),
        "Invoice request failed: {}",
        response.status
    );
    let json: serde_json::Value = response.json()?;
    let invoice_str = json["pr"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No invoice in response: {json}"))?;

    let parsed = bob.sdk.parse(invoice_str).await?;
    let InputType::Bolt11Invoice(invoice_details) = parsed else {
        anyhow::bail!("Expected Bolt11Invoice");
    };
    assert_ne!(
        invoice_details.expiry, custom_expiry_secs as u64,
        "Default expiry should differ from the custom value"
    );
    info!(
        "Invoice with default expiry verified: {} secs",
        invoice_details.expiry
    );

    info!("=== Test test_09_invoice_expiry_parameter PASSED ===");
    Ok(())
}

/// Test LNURL payment when the LNURL server includes a Spark address routing hint.
/// When both sender and receiver are on Spark, the invoice returned by the LNURL server
/// will contain a Spark routing hint, causing the payment to go via Spark transfer
/// instead of Lightning. This previously caused an "Expected Lightning payment details"
/// error despite the funds being transferred successfully.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_11_lnurl_spark_address_payment(
    #[future] lnurl_spark_address_fixture: LnurlFixture,
    #[future] alice_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_11_lnurl_spark_address_payment ===");

    // Setup Bob with the Spark address LNURL server
    let lnurl = Arc::new(lnurl_spark_address_fixture.await);
    let lnurl_domain = lnurl.http_url().to_string();

    let mut bob = async {
        let temp_dir = TempDir::new("breez-sdk-bob-spark-addr")?;
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);

        let mut config = default_config(Network::Regtest);
        config.api_key = None;
        config.lnurl_domain = Some(lnurl_domain.clone());
        config.sync_interval_secs = 1;
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
        Ok::<_, anyhow::Error>(sdk_instance)
    }
    .instrument(tracing::info_span!(target: "breez_sdk_spark", "bob"))
    .await?;

    let mut alice = alice_sdk.await?;

    let username = "bobsparkaddr";
    let description = "Bob's Spark address test Lightning address";
    let payment_amount_sats = 5_000;
    let payment_comment = "Spark address LNURL payment from Alice";

    // Bob registers a Lightning address (LNURL server has include_spark_address=true)
    let bob_lightning_address = async {
        let register_response = bob
            .sdk
            .register_lightning_address(RegisterLightningAddressRequest {
                username: username.to_string(),
                description: Some(description.to_string()),
            })
            .await?;

        let addr = register_response.lightning_address.clone();
        info!("Registered Lightning address: {}", addr);
        Ok::<_, anyhow::Error>(addr)
    }
    .instrument(bob.span.clone())
    .await?;

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
            amount: payment_amount_sats as u128,
            pay_request: details.pay_request,
            comment: Some(payment_comment.to_string()),
            validate_success_action_url: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let amount_sats = prepare_response.amount_sats;
    info!("Alice prepared payment for {amount_sats} sats to {bob_lightning_address}");

    // Alice sends the payment via lnurl_pay
    // This is the key part: the LNURL server returns an invoice with a Spark routing hint,
    // so the SDK pays via Spark transfer. Previously this returned
    // "Expected Lightning payment details" error.
    let pay_response = alice
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response,
            idempotency_key: None,
        })
        .await?;

    info!("Alice initiated Spark-routed LNURL payment to Bob");

    // Wait for payment to complete on Alice's side
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 30).await?;
    info!("Payment completed on Alice's side");

    // Wait for payment to complete on Bob's side
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
    assert_eq!(alice_payment.status, PaymentStatus::Completed);
    // The payment went via Spark, so method should be Spark
    assert_eq!(alice_payment.method, PaymentMethod::Spark);

    // The payment went via Spark, so details should be Spark variant
    assert!(
        matches!(&alice_payment.details, Some(PaymentDetails::Spark { .. })),
        "Expected Spark payment details, got: {:?}",
        alice_payment.details,
    );

    info!("=== Test test_11_lnurl_spark_address_payment PASSED ===");
    Ok(())
}
