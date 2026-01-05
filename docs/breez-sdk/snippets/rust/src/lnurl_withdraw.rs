use breez_sdk_spark::*;
use anyhow::Result;
use log::info;

async fn withdraw(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: lnurl-withdraw
    // Endpoint can also be of the form:
    // lnurlw://domain.com/lnurl-withdraw?key=val
    let lnurl_withdraw_url = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

    if let Ok(InputType::LnurlWithdraw(withdraw_request)) = sdk.parse(lnurl_withdraw_url).await {
        // Amount to withdraw in sats between min/max withdrawable amounts
        let amount_sats = 5_000;
        let optional_completion_timeout_secs = Some(30);

        let response = sdk
            .lnurl_withdraw(LnurlWithdrawRequest {
                amount_sats,
                withdraw_request,
                completion_timeout_secs: optional_completion_timeout_secs,
            })
            .await?;

        let payment = response.payment;
        info!("Payment: {payment:?}");
    }
    // ANCHOR_END: lnurl-withdraw

    Ok(())
}
