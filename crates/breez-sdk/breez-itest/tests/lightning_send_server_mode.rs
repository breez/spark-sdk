use std::time::Instant;

use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tracing::info;

struct ServerModeFixture {
    #[allow(dead_code)]
    pg_container: ContainerAsync<Postgres>,
    connection_string: String,
}

impl ServerModeFixture {
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

    async fn build_alice(&self) -> Result<SdkInstance> {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        build_sdk_with_postgres_server_mode(&self.connection_string, seed).await
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_send_bolt11_invoice_server_mode(
    #[future] bob_sdk: Result<SdkInstance>,
) -> Result<()> {
    info!("=== Starting test_send_bolt11_invoice_server_mode ===");
    let invoice_amount_sats: u64 = 10_000;
    let completion_timeout_secs: u32 = 10;

    let fixture = ServerModeFixture::new().await?;
    let mut alice = fixture.build_alice().await?;
    let mut bob = bob_sdk.await?;

    // Fund Alice via polling (server mode has no ClaimedDeposits event).
    ensure_funded_via_polling(&mut alice, 100_000).await?;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;

    let bob_invoice = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "Server-mode bolt11 send test".to_string(),
                amount_sats: Some(invoice_amount_sats),
                expiry_secs: None,
                payment_hash: None,
            },
        })
        .await?
        .payment_request;

    let prepare = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_invoice.clone(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let start = Instant::now();
    let send_resp = alice
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(completion_timeout_secs),
            }),
            idempotency_key: None,
        })
        .await?;
    let elapsed = start.elapsed();
    info!(
        "Server-mode send_payment returned in {:?} with status {:?}",
        elapsed, send_resp.payment.status
    );

    assert_eq!(
        send_resp.payment.status,
        PaymentStatus::Completed,
        "server-mode send_payment should resolve to Completed via polling, not the Pending fallback",
    );

    // Confirm the payment reached Bob (client-mode receiver still drives
    // events normally).
    let received =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 30).await?;
    assert_eq!(received.amount, u128::from(invoice_amount_sats));
    assert_eq!(received.method, PaymentMethod::Lightning);

    Ok(())
}
