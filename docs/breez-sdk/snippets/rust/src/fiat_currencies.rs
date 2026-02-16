use anyhow::Result;
use breez_sdk_spark::BreezClient;

async fn list_fiat_currencies(client: BreezClient) -> Result<()> {
    // ANCHOR: list-fiat-currencies
    let response = client.fiat().currencies().await?;
    // ANCHOR_END: list-fiat-currencies

    Ok(())
}

async fn list_fiat_rates(client: BreezClient) -> Result<()> {
    // ANCHOR: list-fiat-rates
    let response = client.fiat().rates().await?;
    // ANCHOR_END: list-fiat-rates

    Ok(())
}
