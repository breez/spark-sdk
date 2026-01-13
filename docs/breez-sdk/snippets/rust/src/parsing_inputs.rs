use anyhow::Result;
use breez_sdk_spark::*;

async fn parse_input(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: parse-inputs
    let input = "an input to be parsed...";

    match sdk.parse(input).await? {
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
        InputType::SparkAddress(details) => {
            println!("Input is Spark address {}", details.address);
        }
        InputType::SparkInvoice(invoice) => {
            println!("Input is Spark invoice:");
            if let Some(token_identifier) = &invoice.token_identifier {
                println!(
                    "  Amount: {:?} base units of token with id {}",
                    invoice.amount, token_identifier
                );
            } else {
                println!("  Amount: {:?} sats", invoice.amount);
            }

            if let Some(description) = &invoice.description {
                println!("  Description: {}", description);
            }

            if let Some(expires_at) = invoice.expires_at {
                println!("  Expires at: {}", expires_at);
            }

            if let Some(sender_public_key) = &invoice.sender_public_key {
                println!("  Sender public key: {}", sender_public_key);
            }
        }
        // Other input types are available
        _ => {}
    }
    // ANCHOR_END: parse-inputs
    Ok(())
}

pub(crate) async fn set_external_input_parsers() -> Result<()> {
    // ANCHOR: set-external-input-parsers
    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Configure external parsers
    config.external_input_parsers = Some(vec![
        ExternalInputParser {
            provider_id: "provider_a".to_string(),
            input_regex: "^provider_a".to_string(),
            parser_url: "https://parser-domain.com/parser?input=<input>".to_string(),
        },
        ExternalInputParser {
            provider_id: "provider_b".to_string(),
            input_regex: "^provider_b".to_string(),
            parser_url: "https://parser-domain.com/parser?input=<input>".to_string(),
        },
    ]);
    // ANCHOR_END: set-external-input-parsers
    Ok(())
}
