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

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    instance.sdk.sync_wallet(SyncWalletRequest {}).await?;
    Ok(token_metadata)
}

// ---------------------
// Tests
// ---------------------

/// Test 1: Send payment from Alice to Bob using token transfer
#[rstest]
#[test_log::test(tokio::test)]
async fn test_01_token_transfer(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_01_token_transfer ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

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
            payment_request: bob_spark_address.clone(),
            amount: Some(5),
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await?;
    info!("Prepare response amount: {} (token units)", prepare.amount);

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
async fn test_02_token_invoice(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_02_token_invoice ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

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
            payment_request: bob_invoice.payment_request.clone(),
            amount: None, // Amount comes from invoice
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await?;

    info!(
        "Alice's prepare response - amount: {}",
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
async fn test_03_token_burning(#[future] alice_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_03_token_burning ===");

    let alice = alice_sdk.await?;

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
#[rstest]
#[test_log::test(tokio::test)]
async fn test_04_token_freeze_unfreeze(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_04_token_freeze_unfreeze ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

    // Create a freezable token for this test
    let token_metadata = alice
        .sdk
        .get_token_issuer()
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "Freezable Token".to_string(),
            ticker: "FREEZE".to_string(),
            decimals: 2,
            is_freezable: true, // Make it freezable
            max_supply: 1_000_000,
        })
        .await?;

    alice
        .sdk
        .get_token_issuer()
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    info!(
        "Created freezable token: {} ({})",
        token_metadata.name, token_metadata.identifier
    );

    // Alice sends some tokens to Bob
    let bob_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare_send = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_address,
            amount: Some(100),
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await?;

    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_send,
            options: None,
            idempotency_key: None,
        })
        .await?;

    // Sync and verify Bob received tokens
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
        .unwrap()
        .balance;
    assert_eq!(bob_balance, 100, "Bob should have 100 tokens");

    // Get Bob's Spark address for freezing
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    // Alice freezes Bob's tokens
    let freeze_response = alice
        .sdk
        .get_token_issuer()
        .freeze_issuer_token(FreezeIssuerTokenRequest {
            address: bob_spark_address.clone(),
        })
        .await?;

    info!(
        "Froze tokens at address {}: {} tokens affected, {} outputs impacted",
        bob_spark_address,
        freeze_response.impacted_token_amount,
        freeze_response.impacted_output_ids.len()
    );

    // Verify the freeze affected the expected amount
    assert_eq!(
        freeze_response.impacted_token_amount, 100,
        "Should freeze all 100 of Bob's tokens"
    );

    // Wait for freeze operation to complete
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Now Bob tries to send tokens - should fail because frozen tokens cannot be sent
    let alice_address = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let bob_prepare_result = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_address.clone(),
            amount: Some(50),
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await;

    // Preparation might succeed, but sending should fail
    if let Ok(bob_prepare) = bob_prepare_result {
        let bob_send_result = bob
            .sdk
            .send_payment(SendPaymentRequest {
                prepare_response: bob_prepare,
                options: None,
                idempotency_key: None,
            })
            .await;

        // This should fail because Bob's tokens are frozen
        assert!(
            bob_send_result.is_err(),
            "Bob should not be able to send frozen tokens"
        );
        info!("Bob correctly failed to send frozen tokens");
    } else {
        // If preparation already fails, that's also acceptable
        info!("Bob correctly failed to prepare sending frozen tokens");
    }

    // Alice unfreezes Bob's tokens
    let unfreeze_response = alice
        .sdk
        .get_token_issuer()
        .unfreeze_issuer_token(UnfreezeIssuerTokenRequest {
            address: bob_spark_address,
        })
        .await?;

    info!(
        "Unfroze tokens: {} tokens affected, {} outputs impacted",
        unfreeze_response.impacted_token_amount,
        unfreeze_response.impacted_output_ids.len()
    );

    // Verify unfreeze affected the frozen tokens
    assert_eq!(
        unfreeze_response.impacted_token_amount, 100,
        "Should unfreeze the 100 frozen tokens"
    );

    // When attempting to send tokens, the SO temporarily locks outputs
    // (~3 minutes) even when they are frozen (low priority issue on SO side)
    // TODO: remove this sleep if/when the issue is fixed
    tokio::time::sleep(std::time::Duration::from_secs(60 * 3 + 30)).await;

    // Now Bob should be able to send tokens
    let bob_prepare_after_unfreeze = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_address,
            amount: Some(50),
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await?;

    let bob_send_after_unfreeze = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: bob_prepare_after_unfreeze,
            options: None,
            idempotency_key: None,
        })
        .await?;

    assert_eq!(
        bob_send_after_unfreeze.payment.amount, 50,
        "Bob should be able to send tokens after unfreeze"
    );

    info!("=== Test test_04_token_freeze_unfreeze PASSED ===");
    Ok(())
}

/// Test 5: Token invoice expiry functionality
#[rstest]
#[test_log::test(tokio::test)]
async fn test_05_invoice_expiry(
    #[future] alice_sdk: Result<SdkInstance>,
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_05_invoice_expiry ===");

    let alice = alice_sdk.await?;
    let bob = bob_sdk.await?;

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
            payment_request: bob_invoice.payment_request.clone(),
            amount: None,
            token_identifier: Some(token_metadata.identifier.clone()),
        })
        .await?;

    info!("Alice prepared payment successfully before expiry");

    // Wait for invoice to expire
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;

    // Now Alice tries to prepare the same payment - should fail
    let alice_prepare_expired = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_invoice.payment_request.clone(),
            amount: None,
            token_identifier: Some(token_metadata.identifier.clone()),
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
async fn test_06_supply_limits(#[future] alice_sdk: Result<SdkInstance>) -> Result<()> {
    info!("=== Starting test_06_supply_limits ===");

    let alice = alice_sdk.await?;

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
