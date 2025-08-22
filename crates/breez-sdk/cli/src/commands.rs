use breez_sdk_core::{
    BreezSdk, ClaimDepositRequest, Fee, GetInfoRequest, GetPaymentRequest, InputType,
    ListPaymentsRequest, ListUnclaimedDepositsRequest, LnurlPayRequest, PrepareLnurlPayRequest,
    PrepareReceivePaymentRequest, PrepareSendPaymentRequest, ReceivePaymentMethod,
    ReceivePaymentRequest, RefundDepositRequest, SendPaymentRequest, SyncWalletRequest, parse,
};
use clap::Parser;
use rustyline::{
    Completer, Editor, Helper, Hinter, Validator, highlight::Highlighter, hint::HistoryHinter,
    history::DefaultHistory,
};
use std::borrow::Cow::{self, Owned};

#[derive(Clone, Parser)]
pub enum Command {
    /// Exit the interactive shell (interactive mode only)
    #[command(hide = true)]
    Exit,

    /// Get balance information
    GetInfo,

    /// Get the payment with the given ID
    GetPayment {
        /// The ID of the payment to retrieve
        payment_id: String,
    },
    Sync,
    /// Lists payments
    ListPayments {
        /// Number of payments to show
        #[arg(short, long, default_value = "10")]
        limit: Option<u32>,

        /// Number of payments to skip
        #[arg(short, long, default_value = "0")]
        offset: Option<u32>,
    },

    /// Receive
    Receive {
        #[arg(short = 'm', long = "method")]
        payment_method: String,

        /// Optional description for the invoice
        #[clap(short = 'd', long = "description")]
        description: Option<String>,

        /// The amount the payer should send, in satoshi.
        #[arg(long)]
        amount_sat: Option<u64>,
    },

    /// Pay the given payment request
    Pay {
        /// The payment request to pay
        #[arg(short = 'r', long)]
        payment_request: String,

        /// Optional amount to pay in satoshis
        #[arg(short = 'a', long)]
        amount: Option<u64>,
    },

    /// Pay using LNURL
    LnurlPay {
        /// LN Address or LNURL-pay endpoint
        lnurl: String,

        /// Optional comment, which is to be included in the invoice request sent to the LNURL endpoint
        #[clap(short, long)]
        comment: Option<String>,

        /// Validates the success action URL
        #[clap(name = "validate_success_url", short = 'v', long = "validate")]
        validate_success_url: Option<bool>,
    },

    ClaimDeposit {
        /// The txid of the deposit
        txid: String,

        /// The vout of the deposit
        vout: u32,

        /// The max fee to claim the deposit
        #[arg(long)]
        fee_sat: Option<u64>,

        /// The max fee per vbyte to claim the deposit
        #[arg(long)]
        sat_per_vbyte: Option<u64>,
    },
    RefundDeposit {
        /// The txid of the deposit
        txid: String,

        /// The vout of the deposit
        vout: u32,

        /// Destination address
        destination_address: String,

        /// The max fee to refund the deposit
        #[arg(long)]
        fee_sat: Option<u64>,

        /// The max fee per vbyte to refund the deposit
        #[arg(long)]
        sat_per_vbyte: Option<u64>,
    },
    ListUnclaimedDeposits,
}

#[derive(Helper, Completer, Hinter, Validator)]
pub struct CliHelper {
    #[rustyline(Hinter)]
    pub hinter: HistoryHinter,
}

impl Highlighter for CliHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn execute_command(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    command: Command,
    sdk: &BreezSdk,
) -> Result<bool, anyhow::Error> {
    match command {
        Command::Exit => Ok(false),
        Command::GetInfo => {
            let value = sdk.get_info(GetInfoRequest {}).await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::GetPayment { payment_id } => {
            let value = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::ListPayments { limit, offset } => {
            let value = sdk
                .list_payments(ListPaymentsRequest { limit, offset })
                .await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::Sync => {
            let value = sdk.sync_wallet(SyncWalletRequest {}).await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::ListUnclaimedDeposits => {
            let value = sdk
                .list_unclaimed_deposits(ListUnclaimedDepositsRequest {})
                .await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::ClaimDeposit {
            txid,
            vout,
            fee_sat,
            sat_per_vbyte,
        } => {
            let max_fee = match (fee_sat, sat_per_vbyte) {
                (Some(_), Some(_)) => {
                    return Err(anyhow::anyhow!(
                        "Cannot specify both fee_sat and sat_per_vbyte"
                    ));
                }
                (Some(fee_sat), None) => Some(Fee::Fixed { amount: fee_sat }),
                (None, Some(sat_per_vbyte)) => Some(Fee::Rate { sat_per_vbyte }),
                (None, None) => None,
            };
            let value = sdk
                .claim_deposit(ClaimDepositRequest {
                    txid,
                    vout,
                    max_fee,
                })
                .await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::RefundDeposit {
            txid,
            vout,
            destination_address,
            fee_sat,
            sat_per_vbyte,
        } => {
            let fee = match (fee_sat, sat_per_vbyte) {
                (Some(_), Some(_)) => {
                    return Err(anyhow::anyhow!(
                        "Cannot specify both fee_sat and sat_per_vbyte"
                    ));
                }
                (Some(fee_sat), None) => Fee::Fixed { amount: fee_sat },
                (None, Some(sat_per_vbyte)) => Fee::Rate { sat_per_vbyte },
                (None, None) => {
                    return Err(anyhow::anyhow!(
                        "Must specify either fee_sat or sat_per_vbyte"
                    ));
                }
            };
            let value = sdk
                .refund_deposit(RefundDepositRequest {
                    txid,
                    vout,
                    destination_address,
                    fee,
                })
                .await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::Receive {
            payment_method,
            description,
            amount_sat,
        } => {
            let payment_method = match payment_method.as_str() {
                "spark" => ReceivePaymentMethod::SparkAddress,
                "bitcoin" => ReceivePaymentMethod::BitcoinAddress,
                "bolt11" => ReceivePaymentMethod::Bolt11Invoice {
                    description: description.unwrap_or_default(),
                    amount_sats: amount_sat,
                },
                _ => return Err(anyhow::anyhow!("Invalid payment method")),
            };

            let prepare_response = sdk
                .prepare_receive_payment(PrepareReceivePaymentRequest { payment_method })
                .await?;

            if prepare_response.fee_sats > 0 {
                println!(
                    "Prepared payment requires fee of {} sats\n Do you want to continue? (y/n)",
                    prepare_response.fee_sats
                );
                let line = rl.readline_with_initial("", ("y", ""))?.to_lowercase();
                if line != "y" {
                    return Ok(true);
                }
            }

            let receive_result = sdk
                .receive_payment(ReceivePaymentRequest { prepare_response })
                .await?;

            print_value(&receive_result)?;
            Ok(true)
        }
        Command::Pay {
            payment_request,
            amount,
        } => {
            let prepared_payment = sdk
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request,
                    amount_sats: amount,
                    prefer_spark: None,
                })
                .await;

            let Ok(prepare_response) = prepared_payment else {
                return Err(anyhow::anyhow!(
                    "Failed to prepare payment: {}",
                    prepared_payment.err().unwrap()
                ));
            };
            println!("Prepared payment: {prepare_response:#?}\n Do you want to continue? (y/n)");
            let line = rl.readline_with_initial("", ("y", ""))?.to_lowercase();
            if line != "y" {
                return Ok(true);
            }

            let send_payment_response = sdk
                .send_payment(SendPaymentRequest { prepare_response })
                .await?;

            print_value(&send_payment_response)?;
            Ok(true)
        }
        Command::LnurlPay {
            lnurl,
            comment,
            validate_success_url,
        } => {
            let input = parse(&lnurl).await?;
            let res = match input {
                InputType::LnurlPay(data) => {
                    let min_sendable = data.min_sendable.div_ceil(1000);
                    let max_sendable = data.max_sendable / 1000;
                    let prompt =
                        format!("Amount to pay (min {min_sendable} sat, max {max_sendable} sat): ");
                    let amount_sats = rl.readline(&prompt)?.parse::<u64>()?;

                    let prepare_response = sdk
                        .prepare_lnurl_pay(PrepareLnurlPayRequest {
                            amount_sats,
                            comment,
                            data,
                            validate_success_action_url: validate_success_url,
                        })
                        .await?;

                    println!(
                        "Prepared payment: {prepare_response:#?}\n Do you want to continue? (y/n)"
                    );
                    let line = rl.readline_with_initial("", ("y", ""))?.to_lowercase();
                    if line != "y" {
                        return Ok(true);
                    }

                    let pay_res =
                        Box::pin(sdk.lnurl_pay(LnurlPayRequest { prepare_response })).await?;
                    Ok(pay_res)
                }
                _ => Err(anyhow::anyhow!("Invalid input")),
            }?;

            print_value(&res)?;
            Ok(true)
        }
    }
}

fn print_value<T: serde::Serialize>(value: &T) -> Result<(), serde_json::Error> {
    let serialized = serialize(value)?;
    println!("{serialized}");
    Ok(())
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}
