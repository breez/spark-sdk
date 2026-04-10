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
        let amount_sats = 5_000;
        let optional_comment = Some("<comment>".to_string());
        let optional_validate_success_action_url = Some(true);

        let prepare_response = sdk
            .prepare_lnurl_pay(PrepareLnurlPayRequest {
                amount: amount_sats,
                pay_request: details.pay_request,
                comment: optional_comment,
                validate_success_action_url: optional_validate_success_action_url,
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        // If the fees are acceptable, continue to create the LNURL Pay
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

async fn prepare_pay_fees_included(sdk: &BreezSdk, pay_request: LnurlPayRequestDetails) -> Result<()> {
    // ANCHOR: prepare-lnurl-pay-fees-included
    // By default (FeePolicy::FeesExcluded), fees are added on top of the amount.
    // Use FeePolicy::FeesIncluded to deduct fees from the amount instead.
    // The receiver gets amount minus fees.
    let optional_comment = Some("<comment>".to_string());
    let optional_validate_success_action_url = Some(true);
    let amount_sats = 5_000;

    let prepare_response = sdk
        .prepare_lnurl_pay(PrepareLnurlPayRequest {
            amount: amount_sats,
            pay_request,
            comment: optional_comment,
            validate_success_action_url: optional_validate_success_action_url,
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        })
        .await?;

    // If the fees are acceptable, continue to create the LNURL Pay
    let fee_sats = prepare_response.fee_sats;
    info!("Fees: {fee_sats} sats");
    // The receiver gets amount_sats - fee_sats
    // ANCHOR_END: prepare-lnurl-pay-fees-included
    Ok(())
}
