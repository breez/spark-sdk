use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tracing::info;

struct ConversionServerModeFixture {
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    connection_string: String,
}

impl ConversionServerModeFixture {
    async fn new() -> Result<Self> {
        let pg_container = Postgres::default()
            .start()
            .await
            .expect("Failed to start PostgreSQL container");
        let host_port = pg_container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port");
        let connection_string = format!(
            "host=127.0.0.1 port={host_port} user=postgres password=postgres dbname=postgres"
        );
        Ok(Self {
            pg_container,
            connection_string,
        })
    }

    async fn build_instance(&self) -> Result<SdkInstance> {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        build_sdk_with_postgres_server_mode(&self.connection_string, seed).await
    }
}

#[test_log::test(tokio::test)]
#[ignore = "Requires regtest conversion liquidity; same precondition as token_conversion.rs"]
async fn test_conversion_send_server_mode_bitcoin_to_token() -> Result<()> {
    info!("=== Starting test_conversion_send_server_mode_bitcoin_to_token ===");
    let sats_to_token_amount: u128 = 20_000_000_000;

    let fixture = ConversionServerModeFixture::new().await?;
    let mut alice = fixture.build_instance().await?;
    let bob = fixture.build_instance().await?;

    // Fund Alice via polling (server mode has no ClaimedDeposits event).
    ensure_funded_via_polling(&mut alice, 10_000).await?;

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
            payment_request: bob_spark_address,
            amount: Some(sats_to_token_amount),
            token_identifier: Some(SHELL_REGTEST_TOKEN_ID.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: Some(200),
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;

    // Pre-fix, this call hangs ~30 s and returns
    // `Err(Generic("Timeout waiting for conversion to complete: …"))`.
    let send = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    assert!(
        matches!(
            send.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Bitcoin→Token send should not time out in server mode",
    );
    assert!(
        send.payment.conversion_details.is_some(),
        "Conversion-send payment should carry conversion_details"
    );

    // Receiver doesn't run a background processor either; sync explicitly
    // before reading balance.
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(SHELL_REGTEST_TOKEN_ID)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert!(
        bob_token_balance > 0,
        "Bob should receive tokens from the conversion-send"
    );
    info!("Bob token balance after Bitcoin→Token (server mode): {bob_token_balance}");
    Ok(())
}

#[test_log::test(tokio::test)]
#[ignore = "Requires regtest conversion liquidity; same precondition as token_conversion.rs"]
async fn test_conversion_send_server_mode_token_to_bitcoin() -> Result<()> {
    info!("=== Starting test_conversion_send_server_mode_token_to_bitcoin ===");
    let sats_to_token_amount: u128 = 20_000_000_000;
    let token_to_sats_amount: u64 = 2_500;

    let fixture = ConversionServerModeFixture::new().await?;
    let mut alice = fixture.build_instance().await?;
    let bob = fixture.build_instance().await?;

    // Seed Bob with tokens via Alice's Bitcoin → Token conversion, so we
    // can then exercise the Token → Bitcoin direction.
    ensure_funded_via_polling(&mut alice, 10_000).await?;
    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let prepare_seed = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: bob_spark_address,
            amount: Some(sats_to_token_amount),
            token_identifier: Some(SHELL_REGTEST_TOKEN_ID.to_string()),
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::FromBitcoin,
                max_slippage_bps: Some(200),
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;
    alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare_seed,
            options: None,
            idempotency_key: None,
        })
        .await?;

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;

    // Now Bob sends tokens → Alice receives sats via Lightning invoice.
    let alice_invoice = alice
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Server-mode Token→Bitcoin test".to_string(),
                amount_sats: Some(token_to_sats_amount),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = bob
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: alice_invoice,
            amount: None,
            token_identifier: None,
            conversion_options: Some(ConversionOptions {
                conversion_type: ConversionType::ToBitcoin {
                    from_token_identifier: SHELL_REGTEST_TOKEN_ID.to_string(),
                },
                max_slippage_bps: Some(200),
                completion_timeout_secs: None,
            }),
            fee_policy: None,
        })
        .await?;

    // Pre-fix, hangs because Bob's server-mode SDK never observes the
    // received-leg spark transfer that the conversion produces.
    let send = bob
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await?;

    assert!(
        matches!(
            send.payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Token→Bitcoin send should not time out in server mode",
    );
    assert!(
        send.payment.conversion_details.is_some(),
        "Conversion-send payment should carry conversion_details"
    );

    info!(
        "Token→Bitcoin server-mode send completed: status={:?}",
        send.payment.status
    );
    Ok(())
}
