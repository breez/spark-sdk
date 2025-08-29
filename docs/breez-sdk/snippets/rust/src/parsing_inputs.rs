use anyhow::Result;
use breez_sdk_spark::*;

async fn parse_input() -> Result<()> {
    // ANCHOR: parse-inputs
    let input = "an input to be parsed...";

    match parse(input).await? {
        InputType::BitcoinAddress(details) => {
            println!("Input is Bitcoin address {}", details.address);
        }
        InputType::Bolt11Invoice(details) => {
            println!(
                "Input is BOLT11 invoice for {} msats",
                details
                    .amount_msat
                    .map_or("unknown".to_string(), |a| a.to_string())
            );
        }
        InputType::LnurlPay(details) => {
            println!(
                "Input is LNURL-Pay/Lightning address accepting min/max {}/{} msats",
                details.min_sendable, details.max_sendable
            );
        }
        InputType::LnurlWithdraw(details) => {
            println!(
                "Input is LNURL-Withdraw for min/max {}/{} msats",
                details.min_withdrawable, details.max_withdrawable
            );
        }
        // Other input types are available
        _ => {}
    }
    // ANCHOR_END: parse-inputs
    Ok(())
}
