mod issuer;

use bitcoin::hashes::{Hash, sha256};
use breez_sdk_spark::{
    AssetFilter, BreezSdk, BuyBitcoinRequest, CheckLightningAddressRequest, ClaimDepositRequest,
    ClaimHtlcPaymentRequest, ConversionOptions, ConversionType, Fee, FeePolicy,
    FetchConversionLimitsRequest, GetInfoRequest, GetPaymentRequest, GetTokensMetadataRequest,
    InputType, LightningAddressDetails, ListPaymentsRequest, ListUnclaimedDepositsRequest,
    LnurlPayRequest, LnurlWithdrawRequest, MaxFee, OnchainConfirmationSpeed, PaymentDetailsFilter,
    PaymentStatus, PaymentType, PrepareLnurlPayRequest, PrepareSendPaymentRequest,
    ReceivePaymentMethod, ReceivePaymentRequest, RefundDepositRequest,
    RegisterLightningAddressRequest, SendPaymentMethod, SendPaymentOptions, SendPaymentRequest,
    SparkHtlcOptions, SparkHtlcStatus, SyncWalletRequest, TokenIssuer, TokenTransactionType,
    UpdateUserSettingsRequest,
};
use clap::Parser;
use rand::RngCore;
use rustyline::{
    Completer, Editor, Helper, Hinter, Validator, highlight::Highlighter, hint::HistoryHinter,
    history::DefaultHistory,
};
use std::{
    borrow::Cow::{self, Owned},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::command::issuer::IssuerCommand;

#[derive(Clone, Parser)]
pub enum Command {
    /// Exit the interactive shell (interactive mode only)
    #[command(hide = true)]
    Exit,

    /// Get balance information
    GetInfo {
        /// Force sync
        #[arg(short, long)]
        ensure_synced: Option<bool>,
    },

    /// Get the payment with the given ID
    GetPayment {
        /// The ID of the payment to retrieve
        payment_id: String,
    },
    Sync,
    /// Lists payments
    ListPayments {
        /// Filter by payment type
        #[arg(short, long)]
        type_filter: Option<Vec<PaymentType>>,

        /// Filter by payment status
        #[arg(short, long)]
        status_filter: Option<Vec<PaymentStatus>>,

        /// Filter by asset
        #[arg(short, long)]
        asset_filter: Option<AssetFilter>,

        /// Filter by Spark HTLC status
        #[arg(long)]
        spark_htlc_status_filter: Option<Vec<SparkHtlcStatus>>,

        /// Filter by token transaction hash
        #[arg(long)]
        tx_hash: Option<String>,

        /// Filter by token transaction type
        #[arg(long)]
        tx_type: Option<TokenTransactionType>,

        /// Only include payments created after this timestamp (inclusive)
        #[arg(long)]
        from_timestamp: Option<u64>,

        /// Only include payments created before this timestamp (exclusive)
        #[arg(long)]
        to_timestamp: Option<u64>,

        /// Number of payments to show
        #[arg(short, long, default_value = "10")]
        limit: Option<u32>,

        /// Number of payments to skip
        #[arg(short, long, default_value = "0")]
        offset: Option<u32>,

        /// Sort payments in ascending order
        #[arg(long)]
        sort_ascending: Option<bool>,
    },

    /// Receive
    Receive {
        #[arg(short = 'm', long = "method")]
        payment_method: String,

        /// Optional description for the invoice
        #[clap(short = 'd', long = "description")]
        description: Option<String>,

        /// The amount the payer should send, in sats or token base units.
        #[arg(short = 'a', long)]
        amount: Option<u128>,

        /// Optional token identifier. Only used if the payment method is a spark invoice. Absence indicates sats payment.
        #[arg(short = 't', long)]
        token_identifier: Option<String>,

        /// Optional expiry time for the invoice in seconds from now. Used for spark invoice and bolt11 invoice.
        #[arg(short = 'e', long)]
        expiry_secs: Option<u32>,

        /// Optional sender public key. Only used if the payment method is a spark invoice.
        #[arg(short = 's', long)]
        sender_public_key: Option<String>,

        /// Create a HODL invoice (bolt11 only). Generates a preimage locally and prints it.
        #[arg(long)]
        hodl: bool,
    },

    /// Pay the given payment request
    Pay {
        /// The payment request to pay
        #[arg(short = 'r', long)]
        payment_request: String,

        /// Optional amount to pay. By default is denominated in sats.
        /// If a token identifier is provided, the amount will be denominated in the token base units.
        #[arg(short = 'a', long)]
        amount: Option<u128>,

        /// Optional token identifier. May only be provided if the payment request is a spark address.
        #[arg(short = 't', long)]
        token_identifier: Option<String>,

        /// Optional idempotency key to ensure only one payment is made for multiple requests.
        #[arg(short = 'i', long)]
        idempotency_key: Option<String>,

        /// If provided, the payment will include a token conversion step, converting from Bitcoin
        /// to the specified token to fulfill the payment.
        #[clap(long = "from-bitcoin", conflicts_with = "convert_from_token_identifier", action = clap::ArgAction::SetTrue)]
        convert_from_bitcoin: Option<bool>,

        // If provided, the payment will include a token conversion step, converting from the
        // specified token to Bitcoin to fulfill the payment.
        #[arg(long = "from-token", conflicts_with = "convert_from_bitcoin")]
        convert_from_token_identifier: Option<String>,

        /// The optional maximum slippage in basis points (1/100 of a percent) allowed when
        /// a token conversion is needed to fulfill the payment. Defaults to 50 bps (0.5%) if not set.
        #[arg(short = 's', long)]
        convert_max_slippage_bps: Option<u32>,

        /// If set, fees will be deducted from the specified amount instead of added on top.
        #[arg(long = "fees-included", action = clap::ArgAction::SetTrue)]
        fees_included: bool,
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

        /// Optional idempotency key to ensure only one payment is made for multiple requests.
        #[arg(short = 'i', long)]
        idempotency_key: Option<String>,

        // If provided, the payment will include a token conversion step, converting from the
        // specified token to Bitcoin to fulfill the payment.
        #[arg(long = "from-token")]
        convert_from_token_identifier: Option<String>,

        /// The optional maximum slippage in basis points (1/100 of a percent) allowed when
        /// a token conversion is needed to fulfill the payment. Defaults to 50 bps (0.5%) if not set.
        #[arg(short = 's', long)]
        convert_max_slippage_bps: Option<u32>,

        /// If set, fees will be deducted from the specified amount instead of added on top.
        #[arg(long = "fees-included", action = clap::ArgAction::SetTrue)]
        fees_included: bool,
    },

    /// Withdraw using LNURL
    LnurlWithdraw {
        /// LNURL-withdraw endpoint
        lnurl: String,

        /// Optional completion timeout in seconds
        #[clap(short = 't', long = "timeout")]
        completion_timeout_secs: Option<u32>,
    },

    /// Authenticate using LNURL
    LnurlAuth {
        /// LNURL-auth endpoint
        lnurl: String,
    },

    /// Claim an HTLC payment
    ClaimHtlcPayment {
        /// The preimage of the HTLC (hex string)
        preimage: String,
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

        /// If provided, the max fee per vbyte will be set to the fastest recommended fee at time of claim, plus the leeway.
        #[arg(long)]
        recommended_fee_leeway: Option<u64>,
    },
    Parse {
        input: String,
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
    /// Buy Bitcoin using an external provider (`MoonPay`)
    BuyBitcoin {
        /// Lock the purchase to a specific amount in satoshis. When provided, the user cannot change the amount in the purchase flow.
        #[arg(long)]
        locked_amount_sat: Option<u64>,

        /// Custom redirect URL after purchase completion
        #[arg(long)]
        redirect_url: Option<String>,
    },
    CheckLightningAddressAvailable {
        /// The username to check
        username: String,
    },
    GetLightningAddress,
    RegisterLightningAddress {
        /// The lightning address username
        username: String,

        /// Description in the lnurl response and the invoice.
        description: Option<String>,
    },
    DeleteLightningAddress,
    /// List fiat currencies
    ListFiatCurrencies,
    /// List available fiat rates
    ListFiatRates,
    /// Get the recommended BTC fees based on the configured chain service
    RecommendedFees,
    GetTokensMetadata {
        /// The token identifiers to get metadata for
        token_identifiers: Vec<String>,
    },
    FetchConversionLimits {
        /// Whether we are converting from or to Bitcoin
        #[clap(short = 'f', long, action = clap::ArgAction::SetTrue)]
        from_bitcoin: bool,

        /// The token identifier of the token
        token_identifier: String,
    },
    GetUserSettings,
    SetUserSettings {
        /// Whether spark private mode is enabled.
        #[clap(short = 'p', long = "private")]
        spark_private_mode_enabled: Option<bool>,
    },

    /// Get the status of the Spark network services
    GetSparkStatus,

    /// Issuer related commands
    #[command(subcommand)]
    Issuer(IssuerCommand),
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
    token_issuer: &TokenIssuer,
) -> Result<bool, anyhow::Error> {
    match command {
        Command::Exit => {
            sdk.disconnect().await?;
            Ok(false)
        }
        Command::GetInfo { ensure_synced } => {
            let value = sdk.get_info(GetInfoRequest { ensure_synced }).await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::GetPayment { payment_id } => {
            let value = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
            print_value(&value)?;
            Ok(true)
        }
        Command::ListPayments {
            limit,
            offset,
            type_filter,
            status_filter,
            spark_htlc_status_filter,
            tx_hash,
            tx_type,
            asset_filter,
            from_timestamp,
            to_timestamp,
            sort_ascending,
        } => {
            let mut payment_details_filter = Vec::new();
            if let Some(statuses) = spark_htlc_status_filter {
                payment_details_filter.push(PaymentDetailsFilter::Spark {
                    htlc_status: Some(statuses),
                    conversion_refund_needed: None,
                });
            }
            if let Some(tx_hash) = tx_hash {
                payment_details_filter.push(PaymentDetailsFilter::Token {
                    conversion_refund_needed: None,
                    tx_type: None,
                    tx_hash: Some(tx_hash),
                });
            }
            if let Some(tx_type) = tx_type {
                payment_details_filter.push(PaymentDetailsFilter::Token {
                    conversion_refund_needed: None,
                    tx_type: Some(tx_type),
                    tx_hash: None,
                });
            }
            let payment_details_filter = if payment_details_filter.is_empty() {
                None
            } else {
                Some(payment_details_filter)
            };
            let value = sdk
                .list_payments(ListPaymentsRequest {
                    limit,
                    offset,
                    type_filter,
                    status_filter,
                    asset_filter,
                    payment_details_filter,
                    from_timestamp,
                    to_timestamp,
                    sort_ascending,
                })
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
            recommended_fee_leeway,
        } => {
            let max_fee = if let Some(recommended_fee_leeway) = recommended_fee_leeway {
                if fee_sat.is_some() || sat_per_vbyte.is_some() {
                    return Err(anyhow::anyhow!(
                        "Cannot specify fee_sat or sat_per_vbyte when using recommended fee"
                    ));
                }
                Some(MaxFee::NetworkRecommended {
                    leeway_sat_per_vbyte: recommended_fee_leeway,
                })
            } else {
                match (fee_sat, sat_per_vbyte) {
                    (Some(_), Some(_)) => {
                        return Err(anyhow::anyhow!(
                            "Cannot specify both fee_sat and sat_per_vbyte"
                        ));
                    }
                    (Some(fee_sat), None) => Some(MaxFee::Fixed { amount: fee_sat }),
                    (None, Some(sat_per_vbyte)) => Some(MaxFee::Rate { sat_per_vbyte }),
                    (None, None) => None,
                }
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
        Command::Parse { input } => {
            let value = sdk.parse(&input).await?;
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
        Command::BuyBitcoin {
            locked_amount_sat,
            redirect_url,
        } => {
            let value = sdk
                .buy_bitcoin(BuyBitcoinRequest {
                    locked_amount_sat,
                    redirect_url,
                })
                .await?;
            println!("Open this URL in a browser to complete the purchase:");
            println!("{}", value.url);
            Ok(true)
        }
        Command::Receive {
            payment_method,
            description,
            amount,
            token_identifier,
            expiry_secs,
            sender_public_key,
            hodl,
        } => {
            let payment_method = match payment_method.as_str() {
                "sparkaddress" => ReceivePaymentMethod::SparkAddress,
                "sparkinvoice" => ReceivePaymentMethod::SparkInvoice {
                    amount,
                    token_identifier,
                    expiry_time: expiry_secs
                        .map(|secs| {
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)?
                                .as_secs()
                                .checked_add(u64::from(secs))
                                .ok_or(anyhow::anyhow!("Invalid expiry time"))
                        })
                        .transpose()?,
                    description,
                    sender_public_key,
                },
                "bitcoin" => ReceivePaymentMethod::BitcoinAddress,
                "bolt11" => {
                    let payment_hash = if hodl {
                        let mut preimage_bytes = [0u8; 32];
                        rand::thread_rng().fill_bytes(&mut preimage_bytes);
                        let preimage = hex::encode(preimage_bytes);
                        let hash = sha256::Hash::hash(&preimage_bytes).to_string();

                        println!("HODL invoice preimage: {preimage}");
                        println!("Payment hash: {hash}");
                        println!("Save the preimage! Use `claim-htlc-payment` with it to settle.");

                        Some(hash)
                    } else {
                        None
                    };

                    ReceivePaymentMethod::Bolt11Invoice {
                        description: description.unwrap_or_default(),
                        amount_sats: amount.map(TryInto::try_into).transpose()?,
                        expiry_secs,
                        payment_hash,
                    }
                }
                _ => return Err(anyhow::anyhow!("Invalid payment method")),
            };

            let receive_result = sdk
                .receive_payment(ReceivePaymentRequest { payment_method })
                .await?;

            if receive_result.fee > 0 {
                println!(
                    "Prepared payment requires fee of {} sats/token base units\n ",
                    receive_result.fee
                );
            }

            print_value(&receive_result)?;
            Ok(true)
        }
        Command::Pay {
            payment_request,
            amount,
            token_identifier,
            idempotency_key,
            convert_from_bitcoin,
            convert_from_token_identifier,
            convert_max_slippage_bps: max_slippage_bps,
            fees_included,
        } => {
            let conversion_options = match (convert_from_bitcoin, convert_from_token_identifier) {
                (Some(true), _) => Some(ConversionOptions {
                    conversion_type: ConversionType::FromBitcoin,
                    max_slippage_bps,
                    completion_timeout_secs: None,
                }),
                (_, Some(from_token_identifier)) => Some(ConversionOptions {
                    conversion_type: ConversionType::ToBitcoin {
                        from_token_identifier,
                    },
                    max_slippage_bps,
                    completion_timeout_secs: None,
                }),
                _ => None,
            };
            let fee_policy = if fees_included {
                Some(FeePolicy::FeesIncluded)
            } else {
                None
            };
            let prepared_payment = sdk
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request,
                    amount,
                    token_identifier,
                    conversion_options,
                    fee_policy,
                })
                .await;

            let Ok(prepare_response) = prepared_payment else {
                return Err(anyhow::anyhow!(
                    "Failed to prepare payment: {}",
                    prepared_payment.err().unwrap()
                ));
            };

            if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
                let units =
                    if conversion_estimate.options.conversion_type == ConversionType::FromBitcoin {
                        "sats"
                    } else {
                        "token base units"
                    };
                println!(
                    "Estimated conversion of {} {} with a {} {} fee",
                    conversion_estimate.amount, units, conversion_estimate.fee, units
                );
                let line = rl
                    .readline_with_initial("Do you want to continue (y/n): ", ("y", ""))?
                    .to_lowercase();
                if line != "y" {
                    return Err(anyhow::anyhow!("Payment cancelled"));
                }
            }

            let payment_options =
                read_payment_options(prepare_response.payment_method.clone(), rl)?;

            let send_payment_response = Box::pin(sdk.send_payment(SendPaymentRequest {
                prepare_response,
                options: payment_options,
                idempotency_key,
            }))
            .await?;

            print_value(&send_payment_response)?;
            Ok(true)
        }
        Command::LnurlPay {
            lnurl,
            comment,
            validate_success_url,
            idempotency_key,
            convert_from_token_identifier,
            convert_max_slippage_bps: max_slippage_bps,
            fees_included,
        } => {
            let conversion_options =
                convert_from_token_identifier.map(|from_token_identifier| ConversionOptions {
                    conversion_type: ConversionType::ToBitcoin {
                        from_token_identifier,
                    },
                    max_slippage_bps,
                    completion_timeout_secs: None,
                });
            let fee_policy = if fees_included {
                Some(FeePolicy::FeesIncluded)
            } else {
                None
            };

            let input = sdk.parse(&lnurl).await?;
            let res = match input {
                InputType::LightningAddress(LightningAddressDetails { pay_request, .. })
                | InputType::LnurlPay(pay_request) => {
                    let min_sendable = pay_request.min_sendable.div_ceil(1000);
                    let max_sendable = pay_request.max_sendable / 1000;
                    let prompt =
                        format!("Amount to pay (min {min_sendable} sat, max {max_sendable} sat): ");
                    let amount_sats = rl.readline(&prompt)?.parse::<u64>()?;

                    let prepare_response = sdk
                        .prepare_lnurl_pay(PrepareLnurlPayRequest {
                            amount_sats,
                            comment,
                            pay_request,
                            validate_success_action_url: validate_success_url,
                            conversion_options,
                            fee_policy,
                        })
                        .await?;

                    if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
                        println!(
                            "Estimated conversion of {} token base units with a {} token base units fee",
                            conversion_estimate.amount, conversion_estimate.fee
                        );
                        let line = rl
                            .readline_with_initial("Do you want to continue (y/n): ", ("y", ""))?
                            .to_lowercase();
                        if line != "y" {
                            return Err(anyhow::anyhow!("Payment cancelled"));
                        }
                    }

                    println!(
                        "Prepared payment: {prepare_response:#?}\n Do you want to continue? (y/n)"
                    );
                    let line = rl.readline_with_initial("", ("y", ""))?.to_lowercase();
                    if line != "y" {
                        return Ok(true);
                    }

                    let pay_res = Box::pin(sdk.lnurl_pay(LnurlPayRequest {
                        prepare_response,
                        idempotency_key,
                    }))
                    .await?;
                    Ok(pay_res)
                }
                _ => Err(anyhow::anyhow!("Invalid input")),
            }?;

            print_value(&res)?;
            Ok(true)
        }
        Command::LnurlWithdraw {
            lnurl,
            completion_timeout_secs,
        } => {
            let input = sdk.parse(&lnurl).await?;
            let res = match input {
                InputType::LnurlWithdraw(withdraw_request) => {
                    let min_withdrawable = withdraw_request.min_withdrawable.div_ceil(1000);
                    let max_withdrawable = withdraw_request.max_withdrawable / 1000;
                    let prompt = format!(
                        "Amount to withdraw (min {min_withdrawable} sat, max {max_withdrawable} sat): "
                    );
                    let amount_sats = rl.readline(&prompt)?.parse::<u64>()?;

                    let withdraw_res = sdk
                        .lnurl_withdraw(LnurlWithdrawRequest {
                            amount_sats,
                            withdraw_request,
                            completion_timeout_secs,
                        })
                        .await?;
                    Ok(withdraw_res)
                }
                _ => Err(anyhow::anyhow!("Invalid input")),
            }?;

            print_value(&res)?;
            Ok(true)
        }
        Command::LnurlAuth { lnurl } => {
            let input = sdk.parse(&lnurl).await?;
            let res = match input {
                InputType::LnurlAuth(auth_request) => {
                    let action = auth_request.action.as_deref().unwrap_or("auth");
                    let prompt = format!(
                        "Authenticate with {} (action: {})? (y/n): ",
                        auth_request.domain, action
                    );
                    let line = rl.readline_with_initial(&prompt, ("y", ""))?.to_lowercase();
                    if line != "y" {
                        return Ok(true);
                    }
                    sdk.lnurl_auth(auth_request).await?
                }
                _ => return Err(anyhow::anyhow!("Invalid input: expected LNURL-auth")),
            };

            print_value(&res)?;
            Ok(true)
        }

        Command::ClaimHtlcPayment { preimage } => {
            let res = sdk
                .claim_htlc_payment(ClaimHtlcPaymentRequest { preimage })
                .await?;
            print_value(&res.payment)?;
            Ok(true)
        }
        Command::CheckLightningAddressAvailable { username } => {
            let res = sdk
                .check_lightning_address_available(CheckLightningAddressRequest { username })
                .await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::GetLightningAddress => {
            let res = sdk.get_lightning_address().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::RegisterLightningAddress {
            username,
            description,
        } => {
            let res = sdk
                .register_lightning_address(RegisterLightningAddressRequest {
                    username,
                    description,
                })
                .await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::DeleteLightningAddress => {
            sdk.delete_lightning_address().await?;
            Ok(true)
        }
        Command::ListFiatCurrencies => {
            let res = sdk.list_fiat_currencies().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::ListFiatRates => {
            let res = sdk.list_fiat_rates().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::RecommendedFees => {
            let res = sdk.recommended_fees().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::GetTokensMetadata { token_identifiers } => {
            let res = sdk
                .get_tokens_metadata(GetTokensMetadataRequest { token_identifiers })
                .await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::FetchConversionLimits {
            from_bitcoin,
            token_identifier,
        } => {
            let request = if from_bitcoin {
                FetchConversionLimitsRequest {
                    conversion_type: ConversionType::FromBitcoin,
                    token_identifier: Some(token_identifier),
                }
            } else {
                FetchConversionLimitsRequest {
                    conversion_type: ConversionType::ToBitcoin {
                        from_token_identifier: token_identifier,
                    },
                    token_identifier: None,
                }
            };
            let res = sdk.fetch_conversion_limits(request).await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::GetUserSettings => {
            let res = sdk.get_user_settings().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::SetUserSettings {
            spark_private_mode_enabled,
        } => {
            sdk.update_user_settings(UpdateUserSettingsRequest {
                spark_private_mode_enabled,
            })
            .await?;
            Ok(true)
        }
        Command::GetSparkStatus => {
            let res = breez_sdk_spark::get_spark_status().await?;
            print_value(&res)?;
            Ok(true)
        }
        Command::Issuer(issuer_command) => {
            issuer::handle_command(token_issuer, issuer_command).await
        }
    }
}

fn read_payment_options(
    method: SendPaymentMethod,
    rl: &mut Editor<CliHelper, DefaultHistory>,
) -> Result<Option<SendPaymentOptions>, anyhow::Error> {
    match method {
        SendPaymentMethod::BitcoinAddress { fee_quote, .. } => {
            println!("Please choose payment fee:");
            println!("1. Fast: {}", fee_quote.speed_fast.total_fee_sat());
            println!("2. Medium: {}", fee_quote.speed_medium.total_fee_sat());
            println!("3. Slow: {}", fee_quote.speed_slow.total_fee_sat());

            let line = rl.readline_with_initial("", ("1", ""))?.to_lowercase();
            let confirmation_speed = match line.as_str() {
                "1" => OnchainConfirmationSpeed::Fast,
                "2" => OnchainConfirmationSpeed::Medium,
                "3" => OnchainConfirmationSpeed::Slow,
                _ => return Err(anyhow::anyhow!("Invalid confirmation speed")),
            };
            Ok(Some(SendPaymentOptions::BitcoinAddress {
                confirmation_speed,
            }))
        }
        SendPaymentMethod::Bolt11Invoice {
            spark_transfer_fee_sats,
            lightning_fee_sats,
            ..
        } => {
            if let Some(spark_transfer_fee_sats) = spark_transfer_fee_sats {
                println!("Choose payment option:");
                println!("1. Spark transfer fee: {spark_transfer_fee_sats} sats");
                println!("2. Lightning fee: {lightning_fee_sats} sats");
                let line = rl.readline_with_initial("", ("1", ""))?.to_lowercase();
                if line == "1" {
                    return Ok(Some(SendPaymentOptions::Bolt11Invoice {
                        prefer_spark: true,
                        completion_timeout_secs: Some(0),
                    }));
                }
            }
            Ok(Some(SendPaymentOptions::Bolt11Invoice {
                prefer_spark: false,
                completion_timeout_secs: Some(0),
            }))
        }
        SendPaymentMethod::SparkAddress {
            token_identifier, ..
        } => {
            // HTLC options are only valid for Bitcoin payments, not token payments
            if token_identifier.is_some() {
                return Ok(None);
            }

            let line = rl
                .readline_with_initial("Do you want to create an HTLC transfer? (y/n)", ("n", ""))?
                .to_lowercase();
            if line != "y" {
                return Ok(None);
            }

            let payment_hash = rl.readline("Please enter the HTLC payment hash (hex string) or leave empty to generate a new preimage and associated hash:")?;
            let payment_hash = if payment_hash.is_empty() {
                let mut preimage_bytes = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut preimage_bytes);
                let preimage = hex::encode(preimage_bytes);
                let payment_hash = sha256::Hash::hash(&preimage_bytes).to_string();

                println!("Generated preimage: {preimage}");
                println!("Associated payment hash: {payment_hash}");
                payment_hash
            } else {
                payment_hash
            };

            let expiry_duration_secs = rl
                .readline("Please enter the HTLC expiry duration in seconds:")?
                .parse::<u64>()?;

            Ok(Some(SendPaymentOptions::SparkAddress {
                htlc_options: Some(SparkHtlcOptions {
                    payment_hash,
                    expiry_duration_secs,
                }),
            }))
        }
        SendPaymentMethod::SparkInvoice { .. } => Ok(None),
    }
}

pub(crate) fn print_value<T: serde::Serialize>(value: &T) -> Result<(), serde_json::Error> {
    let serialized = serialize(value)?;
    println!("{serialized}");
    Ok(())
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}
