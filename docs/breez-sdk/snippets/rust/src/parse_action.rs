use anyhow::Result;
use breez_sdk_spark::*;

// ANCHOR: parse-action
async fn parse_action_example(sdk: &BreezSdk) -> Result<()> {
    let input = "an input to be parsed...";

    match sdk.parse_action(input).await? {
        ParsedAction::Send(ref action) => match action {
            SendAction::Bolt11 { invoice_details } => {
                println!(
                    "BOLT11 invoice for {} msats",
                    invoice_details
                        .amount_msat
                        .map_or("unknown".to_string(), |a| a.to_string())
                );
                // Prepare and send the payment
                let prepare_response = sdk
                    .prepare_send(action, None, None)
                    .await?;
                let _payment = sdk
                    .send_payment(SendPaymentRequest {
                        prepare_response,
                        options: None,
                        idempotency_key: None,
                    })
                    .await?;
            }
            SendAction::SparkAddress { address_details } => {
                println!("Spark address: {}", address_details.address);
            }
            SendAction::SparkInvoice { invoice_details } => {
                println!("Spark invoice for {:?} sats", invoice_details.amount);
            }
            SendAction::LnurlPay { pay_details } => {
                println!(
                    "LNURL-Pay accepting {}-{} msats",
                    pay_details.min_sendable, pay_details.max_sendable
                );
            }
            _ => println!("Other send destination"),
        },
        ParsedAction::Receive(action) => match action {
            ReceiveAction::LnurlWithdraw { withdraw_details } => {
                println!(
                    "LNURL-Withdraw for {}-{} msats",
                    withdraw_details.min_withdrawable, withdraw_details.max_withdrawable
                );
                // Execute the withdraw
                let _response = sdk
                    .withdraw(
                        ReceiveAction::LnurlWithdraw { withdraw_details },
                        1000,
                        None,
                    )
                    .await?;
            }
        },
        ParsedAction::Authenticate(action) => {
            println!("LNURL-Auth for domain: {}", action.domain);
            // Perform authentication
            let _result = sdk.authenticate(action).await?;
        }
        ParsedAction::Multi {
            bip21_details,
            actions,
        } => {
            println!(
                "BIP21 URI with {} payment options (amount: {:?} sats)",
                actions.len(),
                bip21_details.amount_sat
            );
            // Pick the preferred action from the list
        }
        ParsedAction::Unsupported { raw } => {
            println!("Unsupported input: {raw}");
        }
    }
    Ok(())
}
// ANCHOR_END: parse-action

// ANCHOR: parse-action-static
async fn parse_action_static_example() -> Result<()> {
    let input = "lnbc100n1...";

    // Use parse_action() without an SDK instance
    let action = parse_action(input, None).await?;
    match action {
        ParsedAction::Send(send) => {
            println!("Can send to: {}", send.payment_request());
        }
        _ => println!("Other action type"),
    }
    Ok(())
}
// ANCHOR_END: parse-action-static
