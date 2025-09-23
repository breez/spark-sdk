use anyhow::Result;
use breez_sdk_spark::BreezSdk;

async fn list_fiat_currencies(sdk: BreezSdk) -> Result<()> {
    // ANCHOR: list-fiat-currencies
    let response = sdk.list_fiat_currencies().await?;
    // ANCHOR_END: list-fiat-currencies

    Ok(())
}

async fn list_fiat_rates(sdk: BreezSdk) -> Result<()> {
    // ANCHOR: list-fiat-rates
    let response = sdk.list_fiat_rates().await?;
    // ANCHOR_END: list-fiat-rates

    Ok(())
}
