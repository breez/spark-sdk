use std::sync::Arc;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tempfile::Builder;
use tracing::info;

/// Two SDK instances share a single ConnectionManager and successfully exchange
/// a Spark transfer. Verifies that sharing the manager doesn't break per-user
/// session isolation: alice's auth state must not leak to bob. The `None` case
/// uses the default single-connection manager; `Some(2)` uses a balanced pool
/// to ensure requests dispatched across multiple connections still complete
/// the transfer end-to-end.
#[rstest]
#[case::shared(None)]
#[case::balanced(Some(2))]
#[test_log::test(tokio::test)]
async fn test_shared_connection_manager_spark_transfer(
    #[case] connections_per_operator: Option<u32>,
) -> Result<()> {
    let context = new_shared_sdk_context(SdkContextConfig {
        connections_per_operator,
        ..SdkContextConfig::new(Network::Regtest)
    })
    .await?;

    let alice_dir = Builder::new()
        .prefix("breez-sdk-shared-cm-alice")
        .tempdir()?;
    let bob_dir = Builder::new().prefix("breez-sdk-shared-cm-bob").tempdir()?;

    let mut alice_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut alice_seed);
    let mut bob_seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bob_seed);

    let mut alice = build_sdk_with_shared_context(
        alice_dir.path().to_string_lossy().to_string(),
        alice_seed,
        Arc::clone(&context),
        Some(alice_dir),
    )
    .await?;
    let mut bob = build_sdk_with_shared_context(
        bob_dir.path().to_string_lossy().to_string(),
        bob_seed,
        Arc::clone(&context),
        Some(bob_dir),
    )
    .await?;

    let alice_pubkey = alice
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .identity_pubkey;
    let bob_pubkey = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .identity_pubkey;
    assert_ne!(
        alice_pubkey, bob_pubkey,
        "Alice and Bob must have distinct identities"
    );

    ensure_funded(&mut alice, 100).await?;

    let bob_initial = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address,
            },
            amount: Some(5),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;
    assert!(matches!(
        send.payment.status,
        PaymentStatus::Completed | PaymentStatus::Pending
    ));

    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert!(received.amount >= 5);

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    assert!(
        bob_final > bob_initial,
        "Bob's balance should have increased ({bob_initial} -> {bob_final})"
    );

    info!(
        "Shared SdkContext strong count: {}",
        Arc::strong_count(&context)
    );
    alice.sdk.disconnect().await?;
    bob.sdk.disconnect().await?;

    Ok(())
}
