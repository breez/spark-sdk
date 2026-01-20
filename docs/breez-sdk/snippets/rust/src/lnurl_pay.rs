use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn prepare_pay(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: prepare-lnurl-pay
    // Endpoint can also be of the form:
    // lnurlp://domain.com/lnurl-pay?key=val
    // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
    let lnurl_pay_url = "lightning@address.com";

    if let Ok(InputType::LightningAddress(details)) = sdk.parse(lnurl_pay_url).await {
        let pay_amount = BitcoinPayAmount::Bitcoin { amount_sats: 5_000 };
        let optional_comment = Some("<comment>".to_string());
        let optional_validate_success_action_url = Some(true);
        // Optionally set to use token funds to pay via token conversion
        let optional_max_slippage_bps = Some(50);
        let optional_completion_timeout_secs = Some(30);
        let optional_conversion_options = Some(ConversionOptions {
            conversion_type: ConversionType::ToBitcoin {
                from_token_identifier: "<token identifier>".to_string(),
            },
            max_slippage_bps: optional_max_slippage_bps,
            completion_timeout_secs: optional_completion_timeout_secs,
        });

        let prepare_response = sdk
            .prepare_lnurl_pay(PrepareLnurlPayRequest {
                pay_amount,
                pay_request: details.pay_request,
                comment: optional_comment,
                validate_success_action_url: optional_validate_success_action_url,
                conversion_options: optional_conversion_options,
            })
            .await?;

        // If the fees are acceptable, continue to create the LNURL Pay
        if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
            info!("Estimated conversion amount: {} token base units", conversion_estimate.amount);
            info!("Estimated conversion fee: {} token base units", conversion_estimate.fee);
        }

        let fee_sats = prepare_response.fee_sats;
        info!("Fees: {fee_sats} sats");
    }
    // ANCHOR_END: prepare-lnurl-pay
    Ok(())
}

async fn pay(sdk: &BreezSdk, prepare_response: PrepareLnurlPayResponse) -> Result<()> {
    // ANCHOR: lnurl-pay
    let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
    let response = sdk
        .lnurl_pay(LnurlPayRequest {
            prepare_response,
            idempotency_key: optional_idempotency_key,
        })
        .await?;
    // ANCHOR_END: lnurl-pay
    info!("Response: {response:?}");
    Ok(())
}

async fn prepare_pay_drain(sdk: &BreezSdk, pay_request: LnurlPayRequestDetails) -> Result<()> {
    // ANCHOR: prepare-lnurl-pay-drain
    let optional_comment = Some("<comment>".to_string());
    let optional_validate_success_action_url = Some(true);
    let pay_amount = BitcoinPayAmount::Drain;

    let prepare_response = sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            pay_amount,
            pay_request,
            comment: optional_comment,
            validate_success_action_url: optional_validate_success_action_url,
            conversion_options: None,
        })
        .await?;

    // If the fees are acceptable, continue to create the LNURL Pay
    let fee_sats = prepare_response.fee_sats;
    info!("Fees: {fee_sats} sats");
    // ANCHOR_END: prepare-lnurl-pay-drain
    Ok(())
}
