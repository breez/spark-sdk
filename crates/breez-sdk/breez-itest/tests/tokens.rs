use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rstest::*;
use tracing::info;

async fn create_mint_test_token(instance: &SdkInstance) -> Result<TokenMetadata> {
    let issuer = instance.sdk.get_token_issuer();
    let token_metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "breez-itest token".to_string(),
            ticker: "BIT".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 1_000_000,
        })
        .await?;

    issuer
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;

    info!("Minted 1,000,000 tokens");

    wait_for_token_balance(&instance.sdk, &token_metadata.identifier, 1_000_000, 30).await?;
    Ok(token_metadata)
}

// ---------------------
// Tests
// ---------------------

/// Test 1: Send payment from Alice to Bob using token transfer
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_token_transfer(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_01_token_transfer ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;

    // Create and mint test token
    let token_metadata = create_mint_test_token(&alice).await?;
    info!(
        "Created token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Verify Alice's token balance after minting
    let alice_token_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        alice_token_balance, 1_000_000,
        "Alice should have 1,000,000 tokens after minting"
    );

    // Verify Bob has no tokens initially
    let bob_initial_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        bob_initial_token_balance, 0,
        "Bob should have no tokens initially"
    );

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice prepares and sends 5 token base units to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(5),
            token_identifier: Some(token_metadata.identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    info!("Prepare response amount: {:?}", prepare.amount);

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(
        matches!(
            send_resp.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Payment should be completed or pending"
    );

    // Verify Alice's payment details
    let alice_payment = alice
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id.clone(),
        })
        .await?
        .payment;

    assert_eq!(
        alice_payment.payment_type,
        PaymentType::Send,
        "Alice should have a Send payment"
    );
    assert_eq!(
        alice_payment.amount, 5,
        "Alice should have sent 5 token base units"
    );
    assert_eq!(
        alice_payment.method,
        PaymentMethod::Token,
        "Alice should have sent a token payment"
    );
    assert!(
        matches!(
            alice_payment.details,
            Some(PaymentDetails::Token {
                metadata,
                ..
            }) if metadata == token_metadata
        ),
        "Alice should have token payment details with correct metadata"
    );

    // Sync Bob's wallet to receive the payment
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    // Confirm payment is now completed for Bob
    let bob_payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: send_resp.payment.id.clone(),
        })
        .await?
        .payment;

    assert_eq!(
        bob_payment.status,
        PaymentStatus::Completed,
        "Bob's payment should be completed"
    );
    assert_eq!(
        bob_payment.payment_type,
        PaymentType::Receive,
        "Bob should have a Receive payment"
    );
    assert_eq!(
        bob_payment.amount, 5,
        "Bob should have received 5 token base units"
    );
    assert_eq!(
        bob_payment.method,
        PaymentMethod::Token,
        "Bob should have received a token payment"
    );
    assert!(
        matches!(
            bob_payment.details,
            Some(PaymentDetails::Token {
                metadata,
                ..
            }) if metadata == token_metadata
        ),
        "Bob should have token payment details with correct metadata"
    );

    info!(
        "Bob received payment: {} token units, status: {:?}",
        bob_payment.amount, bob_payment.status
    );

    // Verify final balances
    let alice_final_token_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        alice_final_token_balance,
        1_000_000 - 5,
        "Alice should have 999,995 tokens after sending"
    );

    let bob_final_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        bob_final_token_balance, 5,
        "Bob should have 5 tokens after receiving"
    );

    info!("=== Test test_01_token_transfer PASSED ===");
    Ok(())
}

/// Test 2: Send payment from Alice to Bob using token invoice
#[rstest]
#[test_log::test(tokio::test)]
async fn test_02_token_invoice(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_02_token_invoice ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;

    // Create and mint test token
    let token_metadata = create_mint_test_token(&alice).await?;
    info!(
        "Created token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Verify Alice has tokens before creating invoice
    let alice_initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        alice_initial_balance, 1_000_000,
        "Alice should have 1,000,000 tokens"
    );

    // Bob creates an invoice for 20 token units
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(20),
                token_identifier: Some(token_metadata.identifier.clone()),
                expiry_time: None,
                description: Some("test invoice".to_string()),
                sender_public_key: None,
            },
        })
        .await?;

    info!("Bob's invoice: {}", bob_invoice.payment_request);
    assert!(
        bob_invoice.payment_request.contains("spark"),
        "Invoice should be a spark invoice"
    );

    // Alice prepares payment using the invoice (amount is determined by invoice)
    let prepare_response = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_invoice.payment_request.clone(),
            },
            amount: None, // Amount comes from invoice
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!(
        "Alice's prepare response - amount: {:?}",
        prepare_response.amount
    );
    assert_eq!(
        prepare_response.amount, 20,
        "Prepare response should show invoice amount"
    );

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await?;

    let alice_payment = send_resp.payment;
    info!("Alice's payment: {:?}", alice_payment);

    assert_eq!(
        alice_payment.payment_type,
        PaymentType::Send,
        "Alice should have a Send payment"
    );
    assert_eq!(
        alice_payment.amount, 20,
        "Alice should have sent 20 token base units"
    );
    assert_eq!(
        alice_payment.method,
        PaymentMethod::Token,
        "Alice should have sent a token payment"
    );
    assert!(
        matches!(
            alice_payment.details,
            Some(PaymentDetails::Token {
                metadata,
                ..
            }) if metadata == token_metadata
        ),
        "Alice should have token payment details with correct metadata"
    );

    // Sync Bob's wallet
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let bob_payment = bob
        .sdk
        .get_payment(GetPaymentRequest {
            payment_id: alice_payment.id.clone(),
        })
        .await?
        .payment;

    info!("Bob's payment: {:?}", bob_payment);

    assert_eq!(
        bob_payment.payment_type,
        PaymentType::Receive,
        "Bob should have a Receive payment"
    );
    assert_eq!(
        bob_payment.amount, 20,
        "Bob should have received 20 token base units"
    );
    assert_eq!(
        bob_payment.method,
        PaymentMethod::Token,
        "Bob should have received a token payment"
    );
    assert!(
        matches!(
            bob_payment.details,
            Some(PaymentDetails::Token {
                metadata,
                ..
            }) if metadata == token_metadata
        ),
        "Bob should have token payment details with correct metadata"
    );

    // Verify final balances
    let alice_final_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        alice_final_balance,
        1_000_000 - 20,
        "Alice should have 999,980 tokens after payment"
    );

    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        bob_final_balance, 20,
        "Bob should have 20 tokens after receiving payment"
    );

    info!("=== Test test_02_token_invoice PASSED ===");
    Ok(())
}

/// Test 3: Token burning functionality
#[rstest]
#[test_log::test(tokio::test)]
async fn test_03_token_burning(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_03_token_burning ===");

    let alice = env.create_wallet().await?;

    // Create and mint test token
    let token_metadata = create_mint_test_token(&alice).await?;
    info!(
        "Created token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Verify initial balance
    let initial_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;
    assert_eq!(
        initial_balance, 1_000_000,
        "Alice should have 1,000,000 tokens initially"
    );

    // Burn 100,000 tokens
    let burn_amount = 100_000;
    let burn_response = alice
        .sdk
        .get_token_issuer()
        .burn_issuer_token(BurnIssuerTokenRequest {
            amount: burn_amount,
        })
        .await?;

    info!(
        "Burned {} tokens, payment ID: {}",
        burn_amount, burn_response.id
    );
    assert_eq!(
        burn_response.payment_type,
        PaymentType::Send,
        "Burn should be recorded as send payment"
    );
    assert_eq!(
        burn_response.amount, burn_amount,
        "Burn amount should match request"
    );
    assert_eq!(
        burn_response.method,
        PaymentMethod::Token,
        "Burn should be token method"
    );

    // Verify token balance after burning
    //tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let after_burn_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;

    assert_eq!(
        after_burn_balance,
        initial_balance - burn_amount,
        "Balance should be reduced by burn amount"
    );

    // Verify issuer balance is also updated
    let issuer_balance = alice
        .sdk
        .get_token_issuer()
        .get_issuer_token_balance()
        .await?;
    assert_eq!(
        issuer_balance.balance, after_burn_balance,
        "Issuer balance should match wallet balance"
    );

    info!(
        "Successfully burned {} tokens. Balance: {} -> {}",
        burn_amount, initial_balance, after_burn_balance
    );

    info!("=== Test test_03_token_burning PASSED ===");
    Ok(())
}

/// Test 4: Token freezing and unfreezing functionality
///
/// Each half has a holder of its own. A send the operators reject as frozen keeps
/// its inputs locked until the transaction's validity window runs out, so the
/// holder who is unfrozen never attempts one while frozen.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_token_freeze_unfreeze(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_04_token_freeze_unfreeze ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;
    let carol = env.create_wallet().await?;

    let token_metadata = alice
        .sdk
        .get_token_issuer()
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "Freezable Token".to_string(),
            ticker: "FREEZE".to_string(),
            decimals: 2,
            is_freezable: true,
            max_supply: 1_000_000,
        })
        .await?;
    alice
        .sdk
        .get_token_issuer()
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;
    wait_for_token_balance(&alice.sdk, &token_metadata.identifier, 1_000_000, 30).await?;

    let token = &token_metadata.identifier;
    let bob_address = spark_address(&bob).await?;
    let carol_address = spark_address(&carol).await?;
    let alice_address = spark_address(&alice).await?;
    send_tokens(&alice, &bob_address, token, 100).await?;
    send_tokens(&alice, &carol_address, token, 100).await?;
    wait_for_token_balance(&bob.sdk, token, 100, 30).await?;
    wait_for_token_balance(&carol.sdk, token, 100, 30).await?;

    let issuer = alice.sdk.get_token_issuer();
    for address in [&bob_address, &carol_address] {
        let frozen = issuer
            .freeze_issuer_token(FreezeIssuerTokenRequest {
                address: address.clone(),
            })
            .await?;
        assert_eq!(
            frozen.impacted_token_amount, 100,
            "the freeze should cover all 100 of {address}'s tokens"
        );
    }

    // Preparing may already fail; if it does not, sending must.
    let bob_send = match prepare_token_send(&bob, &alice_address, token, 50).await {
        Ok(prepare_response) => bob
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response,
                options: None,
                idempotency_key: None,
            })
            .await
            .map(|_| ()),
        Err(e) => Err(e),
    };
    assert!(
        bob_send.is_err(),
        "Bob should not be able to send frozen tokens"
    );

    let unfrozen = issuer
        .unfreeze_issuer_token(UnfreezeIssuerTokenRequest {
            address: carol_address,
        })
        .await?;
    assert_eq!(
        unfrozen.impacted_token_amount, 100,
        "the unfreeze should cover Carol's 100 tokens"
    );

    let carol_send = send_tokens(&carol, &alice_address, token, 50).await?;
    assert_eq!(
        carol_send.payment.amount, 50,
        "Carol should be able to send tokens after the unfreeze"
    );

    info!("=== Test test_04_token_freeze_unfreeze PASSED ===");
    Ok(())
}

async fn spark_address(instance: &SdkInstance) -> Result<String> {
    Ok(instance
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request)
}

async fn prepare_token_send(
    instance: &SdkInstance,
    address: &str,
    token_identifier: &str,
    amount: u128,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    instance
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: address.to_string(),
            },
            amount: Some(amount),
            token_identifier: Some(token_identifier.to_string()),
            conversion_options: None,
            fee_policy: None,
        })
        .await
}

async fn send_tokens(
    instance: &SdkInstance,
    address: &str,
    token_identifier: &str,
    amount: u128,
) -> Result<SendPaymentResponse> {
    let prepare_response = prepare_token_send(instance, address, token_identifier, amount).await?;
    Ok(instance
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await?)
}

/// Test 5: Token invoice expiry functionality
#[rstest]
#[test_log::test(tokio::test)]
async fn test_05_invoice_expiry(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_05_invoice_expiry ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;

    // Create and mint test token
    let token_metadata = create_mint_test_token(&alice).await?;
    info!(
        "Created token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Bob creates an invoice that expires in 5 seconds
    let expiry_time = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            + 5,
    );

    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(30),
                token_identifier: Some(token_metadata.identifier.clone()),
                expiry_time,
                description: Some("expiring invoice".to_string()),
                sender_public_key: None,
            },
        })
        .await?;

    info!(
        "Bob created expiring invoice: {}",
        bob_invoice.payment_request
    );

    // Alice should be able to prepare payment immediately
    let alice_prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_invoice.payment_request.clone(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    info!("Alice prepared payment successfully before expiry");

    // Wait for invoice to expire
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    // Now Alice tries to prepare the same payment - should fail
    let alice_prepare_expired = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_invoice.payment_request.clone(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await;

    // This should fail because the invoice has expired
    assert!(
        alice_prepare_expired.is_err(),
        "Payment preparation should fail for expired invoice"
    );

    // However, if Alice already has a prepared payment, she should still be able to send it
    // (this tests the expiry check during send vs prepare)
    let alice_send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: alice_prepare,
            options: None,
            idempotency_key: None,
        })
        .await;

    // This might succeed or fail depending on implementation, but should give a clear result
    match alice_send {
        Ok(send_resp) => {
            info!(
                "Payment succeeded even after expiry: {:?}",
                send_resp.payment.status
            );
            // If it succeeded, verify it was processed
            if send_resp.payment.status == PaymentStatus::Completed {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

                let bob_balance = bob
                    .sdk
                    .get_info(GetInfoRequest {
                        ensure_synced: Some(false),
                    })
                    .await?
                    .token_balances
                    .get(&token_metadata.identifier)
                    .map(|b| b.balance)
                    .unwrap_or(0);

                if bob_balance == 30 {
                    info!("Payment was processed successfully despite expiry");
                }
            }
        }
        Err(e) => {
            info!("Payment failed after expiry as expected: {}", e);
        }
    }

    info!("=== Test test_05_invoice_expiry PASSED ===");
    Ok(())
}

/// Test 6: Token supply limits and max supply validation
#[rstest]
#[test_log::test(tokio::test)]
async fn test_06_supply_limits(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_06_supply_limits ===");

    let alice = env.create_wallet().await?;

    // Create a token with small max supply
    let max_supply = 1000;
    let token_metadata = alice
        .sdk
        .get_token_issuer()
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "Limited Token".to_string(),
            ticker: "LIMIT".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply,
        })
        .await?;

    info!("Created limited token with max supply: {}", max_supply);

    // Mint up to the max supply
    alice
        .sdk
        .get_token_issuer()
        .mint_issuer_token(MintIssuerTokenRequest { amount: max_supply })
        .await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .unwrap()
        .balance;

    assert_eq!(balance, max_supply, "Should have minted up to max supply");

    // Try to mint more - should fail
    let mint_extra_result = alice
        .sdk
        .get_token_issuer()
        .mint_issuer_token(MintIssuerTokenRequest { amount: 100 })
        .await;

    // This should fail due to exceeding max supply
    assert!(
        mint_extra_result.is_err(),
        "Minting beyond max supply should fail"
    );

    info!("Successfully enforced max supply limit of {}", max_supply);

    info!("=== Test test_06_supply_limits PASSED ===");
    Ok(())
}

/// Test 7: Token payments arrive via SO event stream without manual sync
#[rstest]
#[test_log::test(tokio::test)]
async fn test_07_token_payment_realtime_event(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_07_token_payment_realtime_event ===");

    // Build SDKs with sync effectively disabled so any balance/event update
    // we observe must come from the SO real-time notification, not a background sync.
    let mut alice = env
        .create_wallet_with(|cfg| cfg.sync_interval_secs = u32::MAX)
        .await?;
    let mut bob = env
        .create_wallet_with(|cfg| cfg.sync_interval_secs = u32::MAX)
        .await?;

    // Create and mint test token
    let token_metadata = create_mint_test_token(&alice).await?;
    info!(
        "Created token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Drain any events that arrived during setup so the channel is clean
    // before the transfer we actually want to observe.
    clear_event_receiver(&mut alice.events).await;
    clear_event_receiver(&mut bob.events).await;

    // Bob exposes a Spark address
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    info!("Bob's Spark address: {}", bob_spark_address);

    // Alice sends 15 token units to Bob
    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(15),
            token_identifier: Some(token_metadata.identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    info!(
        "Alice sent payment (id: {}), waiting for events without sync ...",
        send_resp.payment.id
    );

    // Wait for Bob to receive a PaymentSucceeded(Token) event.
    // This must arrive via the SO's TokenTransaction notification, not from
    // sync_wallet, so we deliberately do NOT call sync here.
    let bob_payment = wait_for_payment_succeeded_event_with_method(
        &mut bob.events,
        PaymentType::Receive,
        PaymentMethod::Token,
        30,
    )
    .await?;

    info!(
        "Bob received PaymentSucceeded event: amount={}, status={:?}",
        bob_payment.amount, bob_payment.status
    );

    assert_eq!(bob_payment.amount, 15, "Bob should have received 15 tokens");
    assert_eq!(
        bob_payment.status,
        PaymentStatus::Completed,
        "Bob's payment should be completed"
    );
    assert_eq!(
        bob_payment.payment_type,
        PaymentType::Receive,
        "Bob's payment type should be Receive"
    );
    assert!(
        matches!(
            bob_payment.details,
            Some(PaymentDetails::Token { ref metadata, .. }) if *metadata == token_metadata
        ),
        "Bob's payment should carry the correct token metadata"
    );

    // Balances should be updated as part of event processing - no sync needed.
    let bob_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        bob_balance, 15,
        "Bob's token balance should be 15 immediately after the event"
    );

    // Also confirm Alice's send event arrived the same way
    let alice_payment = wait_for_payment_succeeded_event_with_method(
        &mut alice.events,
        PaymentType::Send,
        PaymentMethod::Token,
        30,
    )
    .await?;

    info!(
        "Alice received PaymentSucceeded(Send/Token) event: amount={}",
        alice_payment.amount
    );

    assert_eq!(
        alice_payment.amount, 15,
        "Alice's send event should show 15 tokens"
    );

    let alice_balance = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        alice_balance,
        1_000_000 - 15,
        "Alice's balance should be 999,985 without a manual sync"
    );

    // --- Second transfer: Bob sends 5 tokens back to Alice ---
    // This exercises the case where Bob's newly received outputs become
    // inputs in a new transaction, verifying that both spent-input removal
    // and new-output insertion work correctly across multiple hops.
    info!("Bob sending 5 tokens back to Alice ...");

    clear_event_receiver(&mut alice.events).await;
    clear_event_receiver(&mut bob.events).await;

    let alice_spark_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare2 = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: alice_spark_address.clone(),
            },
            amount: Some(5),
            token_identifier: Some(token_metadata.identifier.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare2,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Wait for Alice to receive the payment event
    let alice_recv = wait_for_payment_succeeded_event_with_method(
        &mut alice.events,
        PaymentType::Receive,
        PaymentMethod::Token,
        30,
    )
    .await?;
    assert_eq!(alice_recv.amount, 5, "Alice should receive 5 tokens back");

    // Wait for Bob's send event
    let bob_send = wait_for_payment_succeeded_event_with_method(
        &mut bob.events,
        PaymentType::Send,
        PaymentMethod::Token,
        30,
    )
    .await?;
    assert_eq!(bob_send.amount, 5, "Bob's send event should show 5 tokens");

    // Verify balances account for both inputs (spent) and outputs (received/change)
    let alice_balance_2 = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        alice_balance_2,
        1_000_000 - 15 + 5,
        "Alice's balance should be 999,990 (original - 15 sent + 5 received)"
    );

    let bob_balance_2 = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_metadata.identifier)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        bob_balance_2,
        15 - 5,
        "Bob's balance should be 10 (15 received - 5 sent)"
    );

    info!("=== Test test_07_token_payment_realtime_event PASSED ===");
    Ok(())
}

/// Test 8: One transaction paying two recipients, one by address and one by
/// invoice.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_08_token_batch(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_08_token_batch ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;
    let token = create_mint_test_token(&alice).await?.identifier;

    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(30),
                token_identifier: Some(token.clone()),
                expiry_time: None,
                description: Some("batch invoice".to_string()),
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;

    let prepare_response = alice
        .sdk
        .prepare_send_batch(PrepareSendBatchRequest {
            recipients: vec![
                BatchRecipient {
                    payment_request: bob_address,
                    amount: Some(70),
                    token_identifier: Some(token.clone()),
                },
                BatchRecipient {
                    payment_request: bob_invoice.clone(),
                    amount: None,
                    token_identifier: None,
                },
            ],
        })
        .await?;

    // One token, so one total covering both recipients.
    assert_eq!(prepare_response.totals.len(), 1);
    assert_eq!(
        prepare_response.totals[0].token_identifier.as_ref(),
        Some(&token)
    );
    assert_eq!(prepare_response.totals[0].amount, 100);
    assert_eq!(prepare_response.recipients.len(), 2);
    assert!(
        matches!(
            prepare_response.recipients[0].destination,
            BatchDestination::SparkAddress { .. }
        ),
        "the address recipient resolves to an address"
    );
    let BatchDestination::SparkInvoice { invoice_details } =
        &prepare_response.recipients[1].destination
    else {
        panic!("the invoice recipient carries the invoice it pays");
    };
    assert_eq!(invoice_details.amount, Some(30));

    let response = alice
        .sdk
        .send_batch(SendBatchRequest { prepare_response })
        .await?;

    assert_eq!(response.payments.len(), 2, "one payment per recipient");
    assert_eq!(
        response.payments[0].amount, 70,
        "payments come back in recipient order"
    );
    assert_eq!(response.payments[1].amount, 30);

    let tx_hashes: Vec<String> = response
        .payments
        .iter()
        .map(|p| match &p.details {
            Some(PaymentDetails::Token { tx_hash, .. }) => tx_hash.clone(),
            _ => panic!("a batch payment must carry token details"),
        })
        .collect();
    assert_eq!(
        tx_hashes[0], tx_hashes[1],
        "both payments come from one transaction"
    );

    // Both payments are listable, which is what a client groups on.
    let listed = alice
        .sdk
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![PaymentDetailsFilter::Token {
                conversion_refund_needed: None,
                tx_hash: Some(tx_hashes[0].clone()),
                tx_type: None,
            }]),
            ..Default::default()
        })
        .await?
        .payments;
    assert_eq!(listed.len(), 2, "the batch is listable by transaction hash");

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(bob_balance, 100, "Bob receives both outputs");

    info!("=== Test test_08_token_batch PASSED ===");
    Ok(())
}

/// Test 9: Prepare rejects a batch that cannot be paid as requested.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_09_token_batch_prepare_rejections(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_09_token_batch_prepare_rejections ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;
    let token = create_mint_test_token(&alice).await?.identifier;

    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(10),
                token_identifier: Some(token.clone()),
                expiry_time: None,
                description: None,
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;

    let prepare = |recipients: Vec<BatchRecipient>| {
        alice
            .sdk
            .prepare_send_batch(PrepareSendBatchRequest { recipients })
    };

    assert!(prepare(vec![]).await.is_err(), "empty batch");

    assert!(
        prepare(vec![
            BatchRecipient {
                payment_request: bob_invoice.clone(),
                amount: None,
                token_identifier: None,
            },
            BatchRecipient {
                payment_request: bob_invoice.clone(),
                amount: None,
                token_identifier: None,
            },
        ])
        .await
        .is_err(),
        "the same invoice twice would pay it twice"
    );

    assert!(
        prepare(vec![BatchRecipient {
            payment_request: bob_address.clone(),
            amount: Some(5),
            token_identifier: None,
        }])
        .await
        .is_err(),
        "an address recipient names no token"
    );

    assert!(
        prepare(vec![BatchRecipient {
            payment_request: bob_address.clone(),
            amount: None,
            token_identifier: Some(token.clone()),
        }])
        .await
        .is_err(),
        "an address recipient names no amount"
    );

    // Two outputs to one payee is a legitimate batch, not a duplicate.
    let ok = prepare(vec![
        BatchRecipient {
            payment_request: bob_address.clone(),
            amount: Some(5),
            token_identifier: Some(token.clone()),
        },
        BatchRecipient {
            payment_request: bob_address,
            amount: Some(7),
            token_identifier: Some(token.clone()),
        },
    ])
    .await?;
    assert_eq!(ok.totals[0].amount, 12, "one total covering both outputs");

    info!("=== Test test_09_token_batch_prepare_rejections PASSED ===");
    Ok(())
}

/// Test 10: A batch fulfilling two invoices of different amounts plus a
/// plain-address output. Different amounts make attribution unambiguous, so both
/// the sender's response and the receiver's synced records must attach each
/// invoice to the one payment whose amount it names.
///
/// The batch stays on one token because the operators reject a transaction that
/// carries an invoice and pays more than one, which prepare refuses up front.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_10_token_batch_invoice_attribution(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_10_token_batch_invoice_attribution ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;
    let token_a = create_mint_test_token(&alice).await?.identifier;
    let token_b = create_mint_test_token(&bob).await?.identifier;

    let invoice_a = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(30),
                token_identifier: Some(token_a.clone()),
                expiry_time: None,
                description: Some("invoice a".to_string()),
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;
    let invoice_b = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(55),
                token_identifier: Some(token_a.clone()),
                expiry_time: None,
                description: Some("invoice b".to_string()),
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;
    let invoice_other_token = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkInvoice {
                amount: Some(40),
                token_identifier: Some(token_b.clone()),
                expiry_time: None,
                description: Some("invoice on the other token".to_string()),
                sender_public_key: None,
            },
        })
        .await?
        .payment_request;
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // A batch carrying an invoice may not span two tokens: the operators read
    // the whole transaction as paying the token of its first output, whether the
    // second token is invoiced or paid to a plain address.
    assert!(
        alice
            .sdk
            .prepare_send_batch(PrepareSendBatchRequest {
                recipients: vec![
                    BatchRecipient {
                        payment_request: invoice_a.clone(),
                        amount: None,
                        token_identifier: None,
                    },
                    BatchRecipient {
                        payment_request: invoice_other_token.clone(),
                        amount: None,
                        token_identifier: None,
                    },
                ],
            })
            .await
            .is_err(),
        "invoices of two tokens in one batch"
    );
    assert!(
        alice
            .sdk
            .prepare_send_batch(PrepareSendBatchRequest {
                recipients: vec![
                    BatchRecipient {
                        payment_request: invoice_a.clone(),
                        amount: None,
                        token_identifier: None,
                    },
                    BatchRecipient {
                        payment_request: bob_address.clone(),
                        amount: Some(70),
                        token_identifier: Some(token_b.clone()),
                    },
                ],
            })
            .await
            .is_err(),
        "an invoice alongside an address paying another token"
    );

    let prepare_response = alice
        .sdk
        .prepare_send_batch(PrepareSendBatchRequest {
            recipients: vec![
                BatchRecipient {
                    payment_request: invoice_a.clone(),
                    amount: None,
                    token_identifier: None,
                },
                BatchRecipient {
                    payment_request: invoice_b.clone(),
                    amount: None,
                    token_identifier: None,
                },
                BatchRecipient {
                    payment_request: bob_address,
                    amount: Some(70),
                    token_identifier: Some(token_a.clone()),
                },
            ],
        })
        .await?;

    let totals: Vec<(Option<String>, u128)> = prepare_response
        .totals
        .iter()
        .map(|t| (t.token_identifier.clone(), t.amount))
        .collect();
    assert_eq!(
        totals,
        vec![(Some(token_a.clone()), 155)],
        "one total covering every recipient of the one token"
    );

    let response = alice
        .sdk
        .send_batch(SendBatchRequest { prepare_response })
        .await?;
    assert_eq!(response.payments.len(), 3, "one payment per recipient");

    // The expected (amount, token, attached invoice) for each output. The
    // address recipient carries no invoice.
    let expected = vec![
        (30_u128, token_a.clone(), Some(invoice_a.clone())),
        (55_u128, token_a.clone(), Some(invoice_b.clone())),
        (70_u128, token_a.clone(), None),
    ];

    // Sender side: payments come back in recipient order.
    let sender_attribution: Vec<(u128, String, Option<String>)> = response
        .payments
        .iter()
        .map(|p| match &p.details {
            Some(PaymentDetails::Token {
                metadata,
                invoice_details,
                ..
            }) => (
                p.amount,
                metadata.identifier.clone(),
                invoice_details.as_ref().map(|d| d.invoice.clone()),
            ),
            _ => panic!("a batch payment must carry token details"),
        })
        .collect();
    assert_eq!(
        sender_attribution, expected,
        "each sent payment carries the invoice its amount names"
    );

    let tx_hash = match &response.payments[0].details {
        Some(PaymentDetails::Token { tx_hash, .. }) => tx_hash.clone(),
        _ => unreachable!(),
    };

    // Receiver side: attribution is reconstructed from the synced transaction
    // alone, so it must land on the same invoice per amount.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_payments = bob
        .sdk
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![PaymentDetailsFilter::Token {
                conversion_refund_needed: None,
                tx_hash: Some(tx_hash),
                tx_type: None,
            }]),
            ..Default::default()
        })
        .await?
        .payments;
    assert_eq!(bob_payments.len(), 3, "the receiver records every output");

    let mut receiver_attribution: Vec<(u128, String, Option<String>)> = bob_payments
        .iter()
        .map(|p| {
            assert_eq!(p.payment_type, PaymentType::Receive);
            match &p.details {
                Some(PaymentDetails::Token {
                    metadata,
                    invoice_details,
                    ..
                }) => (
                    p.amount,
                    metadata.identifier.clone(),
                    invoice_details.as_ref().map(|d| d.invoice.clone()),
                ),
                _ => panic!("a batch payment must carry token details"),
            }
        })
        .collect();
    // Listing order is not guaranteed, so compare by amount.
    receiver_attribution.sort_by_key(|(amount, _, _)| *amount);
    assert_eq!(
        receiver_attribution, expected,
        "the receiver attaches each invoice to the payment its amount names"
    );

    // No invoice is attached to two payments.
    let mut attached: Vec<&String> = receiver_attribution
        .iter()
        .filter_map(|(_, _, inv)| inv.as_ref())
        .collect();
    let attached_count = attached.len();
    attached.sort();
    attached.dedup();
    assert_eq!(
        attached.len(),
        attached_count,
        "no invoice is attached to more than one payment"
    );

    info!("=== Test test_10_token_batch_invoice_attribution PASSED ===");
    Ok(())
}

/// Test 11: One batch paying two tokens, both to plain addresses. Prepare
/// reports a total per token, and every payment carries its own token's metadata
/// while sharing the one transaction.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_11_token_batch_across_tokens(#[future] env: Result<Environment>) -> Result<()> {
    let env = env.await?;
    info!("=== Starting test_11_token_batch_across_tokens ===");

    let alice = env.create_wallet().await?;
    let bob = env.create_wallet().await?;
    let token_a = create_mint_test_token(&alice).await?.identifier;
    let token_b = create_mint_test_token(&bob).await?.identifier;

    // Fund Alice with the second token so her batch can span both.
    let alice_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let funding_prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: alice_address,
            },
            amount: Some(500),
            token_identifier: Some(token_b.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    bob.sdk
        .send_payment(SendPaymentRequest {
            prepare_response: funding_prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare_response = alice
        .sdk
        .prepare_send_batch(PrepareSendBatchRequest {
            recipients: vec![
                BatchRecipient {
                    payment_request: bob_address.clone(),
                    amount: Some(70),
                    token_identifier: Some(token_a.clone()),
                },
                BatchRecipient {
                    payment_request: bob_address,
                    amount: Some(55),
                    token_identifier: Some(token_b.clone()),
                },
            ],
        })
        .await?;

    let totals: Vec<(Option<String>, u128)> = prepare_response
        .totals
        .iter()
        .map(|t| (t.token_identifier.clone(), t.amount))
        .collect();
    assert_eq!(
        totals,
        vec![(Some(token_a.clone()), 70), (Some(token_b.clone()), 55)],
        "one total per token, in first-requested order"
    );

    let response = alice
        .sdk
        .send_batch(SendBatchRequest { prepare_response })
        .await?;
    assert_eq!(response.payments.len(), 2, "one payment per recipient");

    let sent: Vec<(u128, String, String)> = response
        .payments
        .iter()
        .map(|p| match &p.details {
            Some(PaymentDetails::Token {
                metadata, tx_hash, ..
            }) => (p.amount, metadata.identifier.clone(), tx_hash.clone()),
            _ => panic!("a batch payment must carry token details"),
        })
        .collect();
    assert_eq!(sent[0].0, 70, "payments come back in recipient order");
    assert_eq!(sent[0].1, token_a, "each payment names its own token");
    assert_eq!(sent[1].0, 55);
    assert_eq!(sent[1].1, token_b);
    assert_eq!(
        sent[0].2, sent[1].2,
        "both tokens move in the one transaction"
    );

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let balances = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances;
    assert_eq!(balances.get(&token_a).map(|b| b.balance), Some(70));
    assert_eq!(
        balances.get(&token_b).map(|b| b.balance),
        Some(1_000_000 - 500 + 55),
        "Bob keeps what he did not send Alice, plus the batch output"
    );

    info!("=== Test test_11_token_batch_across_tokens PASSED ===");
    Ok(())
}
