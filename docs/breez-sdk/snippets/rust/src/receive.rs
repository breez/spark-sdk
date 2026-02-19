use anyhow::Result;
use breez_sdk_spark::*;

// ANCHOR: create-invoice
async fn create_invoice_example(sdk: &BreezSdk) -> Result<()> {
    // Create a Lightning invoice for 1000 sats
    let result = sdk
        .create_invoice(Some(InvoiceOptions {
            amount_sats: Some(1000),
            description: Some("Coffee payment".to_string()),
            expiry_secs: Some(3600),
        }))
        .await?;
    println!("Invoice: {}", result.bolt11);
    println!("Fee: {} sats", result.fee_sats);
    Ok(())
}
// ANCHOR_END: create-invoice

// ANCHOR: create-spark-invoice
async fn create_spark_invoice_example(sdk: &BreezSdk) -> Result<()> {
    // Create a Spark invoice for 500 sats
    let result = sdk
        .create_spark_invoice(Some(SparkInvoiceOptions {
            amount: Some(500),
            description: Some("Spark payment".to_string()),
            ..Default::default()
        }))
        .await?;
    println!("Spark invoice: {}", result.invoice);
    println!("Fee: {}", result.fee);
    Ok(())
}
// ANCHOR_END: create-spark-invoice

// ANCHOR: get-bitcoin-address
async fn get_bitcoin_address_example(sdk: &BreezSdk) -> Result<()> {
    let result = sdk.get_bitcoin_address().await?;
    println!("Deposit address: {}", result.address);
    Ok(())
}
// ANCHOR_END: get-bitcoin-address

// ANCHOR: get-spark-address
async fn get_spark_address_example(sdk: &BreezSdk) -> Result<()> {
    let result = sdk.get_spark_address().await?;
    println!("Spark address: {}", result.address);
    Ok(())
}
// ANCHOR_END: get-spark-address
