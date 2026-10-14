//! Quoted Lightning payments against Lightspark's dev environment, where the SSP
//! prices payments with partner fees. Skips unless the environment is configured;
//! see `breez_sdk_itest::dev` for the variables. The scenarios run in two tests,
//! receives and sends, because the dev faucet is rate limited, so each test's
//! scenarios share the wallet it funds.

use anyhow::{Result, bail};
use breez_sdk_itest::dev::{DevEnv, DevWalletOptions};
use breez_sdk_itest::ssp_fault::SspFaultProxy;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use tracing::info;

/// Each test funds one wallet and its scenarios share it: the dev faucet is
/// rate limited.
const RECEIVES_FUNDING_SATS: u64 = 60_000;
const SENDS_FUNDING_SATS: u64 = 30_000;

fn quoting() -> DevWalletOptions {
    DevWalletOptions {
        partner_jwt: true,
        ..Default::default()
    }
}

async fn invoice(
    wallet: &SdkInstance,
    amount_sats: Option<u64>,
    fee_policy: Option<FeePolicy>,
) -> Result<ReceivePaymentResponse> {
    Ok(wallet
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "dev quoted payment".to_string(),
                amount_sats,
                expiry_secs: None,
                payment_hash: None,
                receiver_identity_public_key: None,
                fee_policy,
            },
        })
        .await?)
}

async fn prepare(
    wallet: &SdkInstance,
    invoice: &str,
    amount: Option<u64>,
) -> Result<PrepareSendPaymentResponse> {
    Ok(wallet
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: invoice.to_string(),
            },
            amount: amount.map(u128::from),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?)
}

async fn pay(
    wallet: &SdkInstance,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<Payment> {
    Ok(wallet
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await?
        .payment)
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

/// The payee's side of a paid invoice, once settled.
async fn received(payee: &mut SdkInstance) -> Result<Payment> {
    let event =
        wait_for_payment_succeeded_event(&mut payee.events, PaymentType::Receive, 120).await?;
    settled(payee, &event.id).await
}

fn invoice_amount_sats(prepare_response: &PrepareSendPaymentResponse) -> u64 {
    u64::try_from(prepare_response.amount).expect("an amount in sats")
}

fn fixed_fee(fee: &ReceiveFee) -> u64 {
    match fee {
        ReceiveFee::Fixed { amount } => u64::try_from(*amount).expect("a fee in sats"),
        ReceiveFee::Rate { .. } => panic!("expected a fixed fee, got {fee:?}"),
    }
}

/// Checks the fee payout wallets each recorded their share of `payment_id`, when
/// their mnemonics are configured, and that together they got less than the fee.
async fn check_fee_shares(dev: &DevEnv, payment_id: &str, fee_sats: u64) -> Result<()> {
    let mut shares = 0u64;
    for mnemonic in [&dev.affiliate_mnemonic, &dev.partner_mnemonic]
        .into_iter()
        .flatten()
    {
        let payout = dev
            .wallet(DevWalletOptions {
                mnemonic: Some(mnemonic.clone()),
                ..Default::default()
            })
            .await?;
        let share = settled(&payout, payment_id).await?;
        assert_eq!(share.payment_type, PaymentType::Receive);
        assert_eq!(share.method, PaymentMethod::Spark);
        assert_eq!(share.fees, 0);
        let amount = u64::try_from(share.amount)?;
        assert!(amount > 0, "a payout wallet's share is empty");
        info!("Fee share of {payment_id}: {amount} sats");
        shares += amount;
    }
    // Lightspark keeps a cut of every fee, so the payout wallets get less.
    assert!(
        shares < fee_sats,
        "shares {shares} exceed the fee {fee_sats}"
    );
    Ok(())
}

async fn a_quoted_receive_charges_the_payer_the_fee(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;

    let response = invoice(&payee, Some(10_000), None).await?;
    let fee = fixed_fee(&response.fee);
    assert!(
        fee > 0,
        "the quote carries no fee: is the partner JWT attributed?"
    );

    let prepare_response = prepare(payer, &response.payment_request, None).await?;
    assert_eq!(invoice_amount_sats(&prepare_response), 10_000 + fee);
    pay(payer, prepare_response).await?;

    let payment = received(&mut payee).await?;
    assert_eq!(payment.amount, 10_000);
    assert_eq!(u64::try_from(payment.fees)?, fee);
    assert_eq!(payment.method, PaymentMethod::Lightning);

    check_fee_shares(dev, &payment.id, fee).await
}

async fn a_quoted_receive_with_fees_included_takes_the_fee_from_the_amount(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;

    let response = invoice(&payee, Some(10_000), Some(FeePolicy::FeesIncluded)).await?;
    let fee = fixed_fee(&response.fee);
    assert!(fee > 0);

    let prepare_response = prepare(payer, &response.payment_request, None).await?;
    assert_eq!(invoice_amount_sats(&prepare_response), 10_000);
    pay(payer, prepare_response).await?;

    let payment = received(&mut payee).await?;
    assert_eq!(u64::try_from(payment.amount)?, 10_000 - fee);
    assert_eq!(u64::try_from(payment.fees)?, fee);
    Ok(())
}

async fn an_amountless_quoted_receive_takes_a_rate_of_what_is_sent(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;

    let response = invoice(&payee, None, None).await?;
    let ReceiveFee::Rate { bps, fixed_sats } = response.fee else {
        bail!("expected a rate, got {:?}", response.fee);
    };
    assert!(bps > 0);

    let prepare_response = prepare(payer, &response.payment_request, Some(10_000)).await?;
    pay(payer, prepare_response).await?;

    let payment = received(&mut payee).await?;
    let fees = u64::try_from(payment.fees)?;
    assert_eq!(u64::try_from(payment.amount)? + fees, 10_000);
    let expected = 10_000 * u64::from(bps) / 10_000 + fixed_sats;
    assert!(
        fees.abs_diff(expected) <= 1,
        "fee {fees} is not {bps} bps + {fixed_sats} of 10000"
    );
    Ok(())
}

async fn a_receive_with_a_spark_address_is_not_quoted(dev: &DevEnv) -> Result<()> {
    let payee = dev
        .wallet(DevWalletOptions {
            partner_jwt: true,
            prefer_spark_over_lightning: true,
            ..Default::default()
        })
        .await?;

    let response = invoice(&payee, Some(10_000), None).await?;

    assert_eq!(fixed_fee(&response.fee), 0);
    Ok(())
}

async fn balance(wallet: &SdkInstance) -> Result<u64> {
    Ok(wallet
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?
        .balance_sats)
}

/// What a quoted send left behind: the payer's settled payment, the fee it was
/// quoted, and the payee's settled receive.
struct QuotedSend {
    sent: Payment,
    quoted_fee: u64,
    debited: u64,
    received: Payment,
}

/// `payer` pays `payee`'s invoice for `invoice_sats` (amountless when unset),
/// sending `send_sats` when the invoice names no amount, under a fee quote.
async fn quoted_send(
    payer: &SdkInstance,
    payee: &mut SdkInstance,
    invoice_sats: Option<u64>,
    send_sats: Option<u64>,
    fee_policy: Option<FeePolicy>,
) -> Result<QuotedSend> {
    let before = balance(payer).await?;
    let response = invoice(payee, invoice_sats, None).await?;
    let prepare_response = payer
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: response.payment_request,
            },
            amount: send_sats.map(u128::from),
            token_identifier: None,
            conversion_options: None,
            fee_policy,
        })
        .await?;
    let SendPaymentMethod::Bolt11Invoice {
        lightning_fee_sats,
        fee_quote,
        ..
    } = &prepare_response.payment_method
    else {
        bail!(
            "not a Lightning send: {:?}",
            prepare_response.payment_method
        );
    };
    assert!(fee_quote.is_some(), "the send was not quoted");
    let quoted_fee = *lightning_fee_sats;
    let sent = pay(payer, prepare_response).await?;

    let sent = settled(payer, &sent.id).await?;
    let received = received(payee).await?;
    let debited = before - balance(payer).await?;
    info!(
        "Quoted send: amount {} fees {} (quoted {quoted_fee}), debited {debited}; received {} fees {}",
        sent.amount, sent.fees, received.amount, received.fees
    );
    Ok(QuotedSend {
        sent,
        quoted_fee,
        debited,
        received,
    })
}

async fn a_quoted_send_pays_the_invoice_and_the_fee(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(DevWalletOptions::default()).await?;

    let send = quoted_send(payer, &mut payee, Some(5_000), None, None).await?;

    assert_eq!(send.sent.amount, 5_000);
    assert_eq!(u64::try_from(send.sent.fees)?, send.quoted_fee);
    assert_eq!(send.debited, 5_000 + send.quoted_fee);
    assert_eq!(send.received.amount, 5_000);
    check_fee_shares(dev, &send.sent.id, send.quoted_fee).await
}

async fn a_quoted_send_pays_an_amountless_invoice(dev: &DevEnv, payer: &SdkInstance) -> Result<()> {
    let mut payee = dev.wallet(DevWalletOptions::default()).await?;

    let send = quoted_send(payer, &mut payee, None, Some(3_000), None).await?;

    assert_eq!(send.sent.amount, 3_000);
    assert_eq!(u64::try_from(send.sent.fees)?, send.quoted_fee);
    assert_eq!(send.debited, 3_000 + send.quoted_fee);
    assert_eq!(send.received.amount, 3_000);
    check_fee_shares(dev, &send.sent.id, send.quoted_fee).await
}

/// A quote fixes every amount it pays, so it cannot land on a requested total:
/// a fees-included send goes unquoted and spends exactly that total.
async fn a_fees_included_send_is_not_quoted_and_spends_the_total(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(DevWalletOptions::default()).await?;
    let before = balance(payer).await?;

    let response = invoice(&payee, None, None).await?;
    let prepare_response = payer
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: response.payment_request,
            },
            amount: Some(4_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;
    let SendPaymentMethod::Bolt11Invoice { fee_quote, .. } = &prepare_response.payment_method
    else {
        bail!("not a Lightning send");
    };
    assert!(fee_quote.is_none(), "a fees-included send was quoted");
    let sent = pay(payer, prepare_response).await?;

    let sent = settled(payer, &sent.id).await?;
    let received = received(&mut payee).await?;
    info!(
        "Fees-included send: amount {} fees {}; received {}",
        sent.amount, sent.fees, received.amount
    );
    assert_eq!(before - balance(payer).await?, 4_000);
    assert_eq!(sent.amount + sent.fees, 4_000);
    assert_eq!(received.amount, sent.amount);
    Ok(())
}

async fn a_quoted_send_pays_a_quoted_invoice(dev: &DevEnv, payer: &SdkInstance) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;

    let send = quoted_send(payer, &mut payee, Some(3_000), None, None).await?;

    // The payee's quote adds its fee to the invoice, which the payer pays.
    let invoice_sats = u64::try_from(send.sent.amount)?;
    assert!(invoice_sats > 3_000);
    assert_eq!(send.debited, invoice_sats + send.quoted_fee);
    assert_eq!(send.received.amount, 3_000);
    assert_eq!(u64::try_from(send.received.fees)?, invoice_sats - 3_000);
    Ok(())
}

async fn a_quoted_send_pays_a_quoted_amountless_invoice(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;

    let send = quoted_send(payer, &mut payee, None, Some(3_000), None).await?;

    assert_eq!(send.sent.amount, 3_000);
    assert_eq!(send.debited, 3_000 + send.quoted_fee);
    assert!(send.received.fees > 0, "the payee's rate takes nothing");
    assert_eq!(send.received.amount + send.received.fees, 3_000);
    Ok(())
}

/// The SSP refuses a commit whose total disagrees with the signed split, which
/// it does before opening a send request, so the leaves stay with the
/// operators' swap. The send stays pending and its quote is committed again,
/// which completes it once the SSP accepts.
async fn a_refused_quoted_commit_is_committed_again(
    dev: &DevEnv,
    funder: &SdkInstance,
) -> Result<()> {
    let proxy = SspFaultProxy::start(
        &dev.spark_config.ssp_config.base_url,
        "RequestLightningSend",
    )
    .await?;
    proxy.rewrite_requests(|body| {
        if let Some(total) = body.pointer_mut("/variables/input/committed_quote/total_sats") {
            *total = serde_json::json!(total.as_u64().unwrap_or_default() + 1);
        }
    });
    let payer = dev
        .wallet(DevWalletOptions {
            partner_jwt: true,
            ssp_base_url: Some(proxy.base_url()),
            ..Default::default()
        })
        .await?;
    send_over_spark(funder, &payer, 5_000).await?;
    let mut payee = dev.wallet(DevWalletOptions::default()).await?;
    let before = balance(&payer).await?;

    let response = invoice(&payee, Some(2_000), None).await?;
    let prepare_response = prepare(&payer, &response.payment_request, None).await?;
    let SendPaymentMethod::Bolt11Invoice {
        fee_quote,
        lightning_fee_sats,
        ..
    } = &prepare_response.payment_method
    else {
        bail!("not a Lightning send");
    };
    assert!(fee_quote.is_some(), "the send was not quoted");
    let quoted_fee = *lightning_fee_sats;
    let result = payer
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await;
    info!(
        "Refused quoted commit: {:?}",
        result.as_ref().map(|r| &r.payment)
    );
    assert!(result.is_err(), "the tampered commit was accepted");

    proxy.stop_rewriting();
    let received = received(&mut payee).await?;
    assert_eq!(received.amount, 2_000);
    let sends = payer
        .sdk
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Send]),
            ..Default::default()
        })
        .await?
        .payments;
    let sent = settled(&payer, &sends[0].id).await?;
    assert_eq!(sent.amount, 2_000);
    assert_eq!(u64::try_from(sent.fees)?, quoted_fee);
    assert_eq!(before - balance(&payer).await?, 2_000 + quoted_fee);
    Ok(())
}

async fn a_quoted_send_refuses_an_idempotency_key(dev: &DevEnv, payer: &SdkInstance) -> Result<()> {
    let payee = dev.wallet(DevWalletOptions::default()).await?;

    let response = invoice(&payee, Some(1_000), None).await?;
    let prepare_response = prepare(payer, &response.payment_request, None).await?;
    let result = payer
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: Some(uuid::Uuid::now_v7().to_string()),
        })
        .await;

    assert!(
        matches!(result, Err(SdkError::InvalidInput(_))),
        "{result:?}"
    );
    Ok(())
}

async fn send_over_spark(from: &SdkInstance, to: &SdkInstance, amount_sats: u64) -> Result<()> {
    let address = to
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;
    let initial = to
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;
    let prepare_response = prepare(from, &address, Some(amount_sats)).await?;
    pay(from, prepare_response).await?;
    wait_for_balance(&to.sdk, Some(initial + amount_sats), None, 120).await?;
    Ok(())
}

/// A quoted HODL invoice for 10,000 sats net: the payer pays, the HTLC is
/// held, and the payee releases the preimage.
async fn a_quoted_hodl_receive_is_claimed_by_its_payee(
    dev: &DevEnv,
    payer: &SdkInstance,
) -> Result<()> {
    let mut payee = dev.wallet(quoting()).await?;
    let (preimage, payment_hash) = generate_preimage_hash_pair();
    let response = payee
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "dev quoted hodl".to_string(),
                amount_sats: Some(10_000),
                expiry_secs: None,
                payment_hash: Some(payment_hash),
                receiver_identity_public_key: None,
                fee_policy: None,
            },
        })
        .await?;
    let fee = fixed_fee(&response.fee);
    assert!(fee > 0, "the HODL invoice was not quoted");

    let prepare_response = prepare(payer, &response.payment_request, None).await?;
    assert_eq!(invoice_amount_sats(&prepare_response), 10_000 + fee);
    let sent = payer
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(1),
            }),
            idempotency_key: None,
        })
        .await?
        .payment;
    assert_eq!(sent.status, PaymentStatus::Pending, "the HTLC was not held");

    // The SSP hands over the leaves once it holds the HTLC.
    wait_for(
        || async {
            payee.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let pending = payee
                .sdk
                .list_payments(ListPaymentsRequest {
                    status_filter: Some(vec![PaymentStatus::Pending]),
                    type_filter: Some(vec![PaymentType::Receive]),
                    ..Default::default()
                })
                .await?
                .payments;
            match pending.first() {
                Some(payment) => Ok(payment.clone()),
                None => bail!("no held receive yet"),
            }
        },
        90,
    )
    .await?;
    payee
        .sdk
        .claim_htlc_payment(ClaimHtlcPaymentRequest { preimage })
        .await?;

    let payment = received(&mut payee).await?;
    assert_eq!(payment.amount, 10_000);
    assert_eq!(u64::try_from(payment.fees)?, fee);
    check_fee_shares(dev, &payment.id, fee).await
}

/// Only the wallet that creates a quoted receive can release its preimage, so a
/// HODL invoice is always the creator's own.
async fn a_hodl_receive_for_another_wallet_is_rejected(dev: &DevEnv) -> Result<()> {
    let creator = dev.wallet(quoting()).await?;
    let payee = dev.wallet(DevWalletOptions::default()).await?;
    let payee_key = payee
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .identity_pubkey;
    let (_, payment_hash) = generate_preimage_hash_pair();

    let result = creator
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "dev delegated hodl".to_string(),
                amount_sats: Some(10_000),
                expiry_secs: None,
                payment_hash: Some(payment_hash),
                receiver_identity_public_key: Some(payee_key),
                fee_policy: None,
            },
        })
        .await;

    assert!(
        matches!(result, Err(SdkError::InvalidInput(_))),
        "{:?}",
        result.map(|r| r.payment_request)
    );
    Ok(())
}

#[test_log::test(tokio::test)]
async fn quoted_receives_on_dev() -> Result<()> {
    let Some(dev) = DevEnv::from_env() else {
        return Ok(());
    };
    let payer = dev.wallet(DevWalletOptions::default()).await?;
    dev.fund(&payer, RECEIVES_FUNDING_SATS).await?;

    a_quoted_receive_charges_the_payer_the_fee(&dev, &payer).await?;
    a_quoted_receive_with_fees_included_takes_the_fee_from_the_amount(&dev, &payer).await?;
    an_amountless_quoted_receive_takes_a_rate_of_what_is_sent(&dev, &payer).await?;
    a_receive_with_a_spark_address_is_not_quoted(&dev).await?;
    a_quoted_hodl_receive_is_claimed_by_its_payee(&dev, &payer).await?;
    a_hodl_receive_for_another_wallet_is_rejected(&dev).await
}

#[test_log::test(tokio::test)]
async fn quoted_sends_on_dev() -> Result<()> {
    let Some(dev) = DevEnv::from_env() else {
        return Ok(());
    };
    let funder = dev.wallet(DevWalletOptions::default()).await?;
    dev.fund(&funder, SENDS_FUNDING_SATS).await?;
    let payer = dev.wallet(quoting()).await?;
    send_over_spark(&funder, &payer, SENDS_FUNDING_SATS - 6_000).await?;

    a_quoted_send_pays_the_invoice_and_the_fee(&dev, &payer).await?;
    a_quoted_send_pays_an_amountless_invoice(&dev, &payer).await?;
    a_fees_included_send_is_not_quoted_and_spends_the_total(&dev, &payer).await?;
    a_quoted_send_pays_a_quoted_invoice(&dev, &payer).await?;
    a_quoted_send_pays_a_quoted_amountless_invoice(&dev, &payer).await?;
    a_quoted_send_refuses_an_idempotency_key(&dev, &payer).await?;
    a_refused_quoted_commit_is_committed_again(&dev, &funder).await
}

/// A send committed after its quote expired, as when an app stops between the
/// preimage swap and the SSP request and resumes later: the operators hold the
/// swap whatever the quote's expiry. Waits out the quote's lifetime, so it runs
/// only when asked for.
#[test_log::test(tokio::test)]
#[ignore = "waits out a quote's five minute lifetime"]
async fn a_commit_after_the_quote_expires_on_dev() -> Result<()> {
    let Some(dev) = DevEnv::from_env() else {
        return Ok(());
    };
    let proxy = SspFaultProxy::start(
        &dev.spark_config.ssp_config.base_url,
        "RequestLightningSend",
    )
    .await?;
    proxy.start_failing();
    let payer = dev
        .wallet(DevWalletOptions {
            partner_jwt: true,
            ssp_base_url: Some(proxy.base_url()),
            ..Default::default()
        })
        .await?;
    dev.fund(&payer, 10_000).await?;
    let mut payee = dev.wallet(DevWalletOptions::default()).await?;
    let before = balance(&payer).await?;

    let response = invoice(&payee, Some(2_000), None).await?;
    let prepare_response = prepare(&payer, &response.payment_request, None).await?;
    let SendPaymentMethod::Bolt11Invoice {
        fee_quote: Some(fee_quote),
        lightning_fee_sats,
        ..
    } = &prepare_response.payment_method
    else {
        bail!("the send was not quoted");
    };
    let (expires_at, quoted_fee) = (fee_quote.expires_at, *lightning_fee_sats);
    let result = payer
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response,
            options: None,
            idempotency_key: None,
        })
        .await;
    info!(
        "Send while the SSP is unreachable: {:?}",
        result.as_ref().err()
    );
    info!(
        "Balance with the swap held: {} of {before}",
        balance(&payer).await?
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let wait = expires_at.saturating_sub(now) + 10;
    info!("Waiting {wait}s for the quote to expire at {expires_at}");
    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    proxy.stop_failing();

    let outcome = wait_for(
        || async {
            payer.sdk.sync_wallet(SyncWalletRequest {}).await?;
            let send = payer
                .sdk
                .list_payments(ListPaymentsRequest {
                    type_filter: Some(vec![PaymentType::Send]),
                    ..Default::default()
                })
                .await?
                .payments
                .into_iter()
                .next();
            let after = balance(&payer).await?;
            let status = send.as_ref().map(|p| p.status);
            info!(
                "After expiry: send {status:?}, balance {after} of {before}, commits {}",
                proxy.forwarded()
            );
            match status {
                Some(PaymentStatus::Completed) => Ok(format!("completed, balance {after}")),
                Some(PaymentStatus::Failed) if after == before => {
                    Ok("failed, balance restored".to_string())
                }
                _ => bail!("send {status:?}, balance {after} of {before}"),
            }
        },
        300,
    )
    .await;
    let commits = proxy.forwarded();
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    info!(
        "Commits reaching the SSP in the 30s after the outcome: {}",
        proxy.forwarded() - commits
    );
    let outcome = outcome?;
    info!("Commit after expiry: {outcome} (quoted fee {quoted_fee})");
    if outcome.starts_with("completed") {
        assert_eq!(received(&mut payee).await?.amount, 2_000);
    }
    Ok(())
}
