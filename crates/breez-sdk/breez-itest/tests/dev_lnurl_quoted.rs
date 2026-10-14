//! Lightning-address receives quoted by the LNURL server against Lightspark's dev
//! environment, where the SSP prices payments with partner fees. Skips unless the
//! environment is configured; see `breez_sdk_itest::dev` for the variables. Both
//! cases run in one test because the dev faucet is rate limited, so they share
//! one funded payer.

use anyhow::{Result, bail};
use breez_sdk_itest::dev::{DevEnv, DevWalletOptions};
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use spark_wallet::SparkDeployment;
use tracing::info;

const FUNDING_SATS: u64 = 22_000;
const AMOUNT_SATS: u64 = 10_000;

/// Alice pays Bob's lightning address `AMOUNT_SATS`, and returns the payment
/// hash of the invoice she paid and Bob's settled receive.
async fn pay_address(
    alice: &mut SdkInstance,
    bob: &mut SdkInstance,
    address: &str,
) -> Result<(String, Payment)> {
    let InputType::LightningAddress(details) = alice.sdk.parse(address).await? else {
        bail!("expected a lightning address");
    };
    let prepare_response = alice
        .sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount: u128::from(AMOUNT_SATS),
            pay_request: details.pay_request,
            comment: None,
            validate_success_action_url: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;
    // LUD-06: the invoice is for exactly what the payer asked for.
    assert_eq!(
        prepare_response.invoice_details.amount_msat,
        Some(AMOUNT_SATS * 1000)
    );
    let payment_hash = prepare_response.invoice_details.payment_hash.clone();
    alice
        .sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response,
            idempotency_key: None,
        })
        .await?;
    wait_for_payment_succeeded_event(&mut alice.events, PaymentType::Send, 120).await?;
    let event =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 120).await?;
    Ok((payment_hash, settled(bob, &event.id).await?))
}

/// The payment once it is completed and its fees are known, which a sync can
/// lag behind the payment event.
async fn settled(wallet: &SdkInstance, payment_id: &str) -> Result<Payment> {
    wait_for(
        || async {
            wallet.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let payment = wallet
                .sdk
                .get_payment(GetPaymentRequest {
                    payment_id: payment_id.to_string(),
                })
                .await?
                .payment;
            if payment.status != PaymentStatus::Completed {
                bail!("payment {payment_id} is {:?}", payment.status);
            }
            Ok(payment)
        },
        120,
    )
    .await
}

/// The partner fee the server recorded for the invoice with `payment_hash`.
async fn recorded_fee(lnurl: &LnurlFixture, payment_hash: &str) -> Result<Option<i64>> {
    let row = lnurl
        .db()
        .await?
        .query_one(
            "SELECT fee_sat FROM invoices WHERE payment_hash = $1",
            &[&payment_hash],
        )
        .await?;
    Ok(row.try_get(0)?)
}

#[test_log::test(tokio::test)]
async fn test_dev_lnurl_quoted_receive() -> Result<()> {
    let Some(dev) = DevEnv::from_env() else {
        return Ok(());
    };

    let mut config = LnurlImageConfig::default()
        .with_include_spark_address(false)
        .with_spark_deployment(SparkDeployment::from(&dev.spark_config))
        .with_partner_jwt(dev.partner_jwt());
    // The fixture reuses any image under its tag, and one built before fee
    // quoting lacks the schema and the dev partner JWT this test needs.
    config.docker_config.image_tag = "dev-quoted".to_string();
    let lnurl = LnurlFixture::with_config(config).await?;
    // Fails fast on a server that can't quote, before the faucet is spent.
    lnurl.set_fee_quoting(false).await?;

    let mut bob = dev
        .wallet(DevWalletOptions {
            lnurl_domain: Some(lnurl.http_url().to_string()),
            ..Default::default()
        })
        .await?;
    let address = bob
        .sdk
        .register_lightning_address(RegisterLightningAddressRequest {
            username: "bobquoted".to_string(),
            description: None,
        })
        .await?
        .lightning_address;
    info!("Bob registered {address}");

    let mut alice = dev.wallet(DevWalletOptions::default()).await?;
    dev.fund(&alice, FUNDING_SATS).await?;

    // Control: the domain is known but does not quote, so Bob gets it all.
    let (payment_hash, received) = pay_address(&mut alice, &mut bob, &address).await?;
    info!(
        "Unquoted receive: amount {} fees {}",
        received.amount, received.fees
    );
    assert_eq!(received.amount, u128::from(AMOUNT_SATS));
    assert_eq!(received.fees, 0);
    assert_eq!(recorded_fee(&lnurl, &payment_hash).await?, None);

    // Quoted: the fee comes out of what Bob gets.
    lnurl.set_fee_quoting(true).await?;
    let (payment_hash, received) = pay_address(&mut alice, &mut bob, &address).await?;
    let fee = recorded_fee(&lnurl, &payment_hash).await?;
    info!(
        "Quoted receive: amount {} fees {}, recorded fee {fee:?}",
        received.amount, received.fees
    );
    assert!(received.fees > 0, "a quoted receive carries a fee");
    assert_eq!(received.amount + received.fees, u128::from(AMOUNT_SATS));
    assert_eq!(fee, Some(i64::try_from(received.fees)?));

    Ok(())
}
