use anyhow::Result;
use breez_sdk_spark::*;
use rstest::*;
use tokio_with_wasm::alias as tokio;
use tracing::{debug, info};

// ---------------------
// SDK Builder and helpers (remote regtest, faucet funding)
// ---------------------

async fn build_sdk(storage_dir: String, seed_bytes: [u8; 32]) -> Result<BreezSdk> {
    let mut config = default_config(Network::Regtest);
    config.api_key = None; // Regtest: no API key needed
    config.lnurl_domain = None; // Avoid lnurl server in tests
    config.prefer_spark_over_lightning = true; // prefer spark transfers when possible
    config.sync_interval_secs = 5;

    let storage = default_storage(storage_dir)?;
    let seed = Seed::Entropy(seed_bytes.to_vec());

    let builder = SdkBuilder::new(config, seed, storage);
    let sdk = builder.build().await?;

    // Ensure initial sync completes
    let _ = sdk
        .get_info(GetInfoRequest { ensure_synced: Some(true) })
        .await?;
    Ok(sdk)
}

async fn wait_for_balance(
    sdk: &BreezSdk,
    min_balance: u64,
    timeout_secs: u64,
) -> Result<u64> {
    let start = std::time::Instant::now();
    loop {
        let _ = sdk.sync_wallet(SyncWalletRequest {}).await?;
        let info = sdk
            .get_info(GetInfoRequest { ensure_synced: Some(false) })
            .await?;
        if info.balance_sats >= min_balance {
            return Ok(info.balance_sats);
        }
        if start.elapsed().as_secs() > timeout_secs {
            anyhow::bail!(
                "Timeout waiting for balance >= {} sats, current = {}",
                min_balance,
                info.balance_sats
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

// ---------------------
// Tests
// ---------------------

#[rstest]
#[test_log::test(tokio::test)]
async fn test_breez_sdk_deposit_claim() -> Result<()> {
    // Create SDK (alice)
    let data_dir = tempdir::TempDir::new("breez-sdk-deposit")?;
    let sdk = build_sdk(data_dir.path().to_string_lossy().to_string(), [1u8; 32]).await?;

    // Get a static deposit address
    let receive = sdk
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::BitcoinAddress })
        .await?;
    let deposit_address = receive.payment_request;
    info!("Generated deposit address: {}", deposit_address);
    info!("Fund using faucet: https://app.lightspark.com/regtest-faucet (address={}, amount e.g. 100000)", deposit_address);

    // Wait until funds are detected and claimed by background sync
    let balance = wait_for_balance(&sdk, 1, 180).await?;
    debug!("Wallet balance after claim: {} sats", balance);
    assert!(balance > 0, "Balance should increase after deposit claim");
    Ok(())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_breez_sdk_send_payment_prefer_spark() -> Result<()> {
    // Create SDKs for Alice and Bob
    let alice_dir = tempdir::TempDir::new("breez-sdk-alice")?;
    let bob_dir = tempdir::TempDir::new("breez-sdk-bob")?;

    let alice = build_sdk(alice_dir.path().to_string_lossy().to_string(), [2u8; 32]).await?;
    let bob = build_sdk(bob_dir.path().to_string_lossy().to_string(), [3u8; 32]).await?;

    // Fund Alice via faucet and wait for balance
    let alice_deposit_addr = alice
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::BitcoinAddress })
        .await?
        .payment_request;
    info!("Alice deposit address: {}", alice_deposit_addr);
    info!("Fund using faucet: https://app.lightspark.com/regtest-faucet (address={}, amount e.g. 120000)", alice_deposit_addr);
    let alice_balance = wait_for_balance(&alice, 10_000, 240).await?; // wait until some balance is available
    info!("Alice balance after funding: {} sats", alice_balance);

    // Bob exposes a Spark address (no SSP required)
    let bob_spark_address = bob
        .receive_payment(ReceivePaymentRequest { payment_method: ReceivePaymentMethod::SparkAddress })
        .await?
        .payment_request;

    // Alice prepares and sends the payment, preferring spark transfer
    let prepare = alice
        .prepare_send_payment(PrepareSendPaymentRequest { payment_request: bob_spark_address.clone(), amount_sats: Some(5_000) })
        .await?;

    let send_resp = alice
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
        })
        .await?;

    info!("Alice send payment status: {:?}", send_resp.payment.status);
    assert!(matches!(send_resp.payment.status, PaymentStatus::Completed | PaymentStatus::Pending));

    // Bob syncs and verifies he received the payment
    bob.sync_wallet(SyncWalletRequest {}).await?;
    let payments = bob
        .list_payments(ListPaymentsRequest { offset: Some(0), limit: Some(50) })
        .await?
        .payments;
    let received = payments
        .into_iter()
        .find(|p| p.payment_type == PaymentType::Receive && p.amount >= 5_000);
    assert!(received.is_some(), "Bob should have a received payment >= 5000 sats");

    Ok(())
}
