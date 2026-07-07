use spark_wallet::{
    CoopExitFeeQuote, ExitSpeed, PreparedTokenPackage, SendPackagePreparation, SparkAddress,
    TransferTokenOutput,
};

use crate::{
    BitcoinAddressDetails, FeePolicy, SendOnchainFeeQuote,
    error::SdkError,
    models::{
        BuildTransferPackageOptions, PrepareSendPaymentResponse, SendPaymentMethod,
        SignedTransferPackage, TransferSignature, TransferTarget, UnsignedTransferPackage,
    },
    sdk::BreezSdk,
    signer::{
        ExternalPrepareTokenTransactionRequest, ExternalPrepareTransferRequest,
        ExternalTokenTransactionKind,
    },
};

fn to_unsigned_package(
    prep: SendPackagePreparation,
    amount_sat: u64,
    fee_sat: u64,
    target: TransferTarget,
) -> Result<UnsignedTransferPackage, SdkError> {
    Ok(match prep {
        SendPackagePreparation::Ready(pt) => UnsignedTransferPackage::Transfer {
            prepare_transfer: ExternalPrepareTransferRequest::from_prepare_transfer_request(&pt)?,
            amount_sat,
            fee_sat,
            target,
        },
        SendPackagePreparation::SwapRequired {
            prepare_transfer,
            target_amounts,
        } => UnsignedTransferPackage::Swap {
            prepare_transfer: ExternalPrepareTransferRequest::from_prepare_transfer_request(
                &prepare_transfer,
            )?,
            target_amounts,
            amount_sat,
            fee_sat,
        },
    })
}

fn prefers_bolt11_spark_route(
    prefer_spark: bool,
    prepare_response: &PrepareSendPaymentResponse,
) -> bool {
    prefer_spark
        && matches!(
            &prepare_response.payment_method,
            SendPaymentMethod::Bolt11Invoice {
                spark_transfer_fee_sats: Some(_),
                ..
            }
        )
}

fn reject_conversion(response: &PrepareSendPaymentResponse) -> Result<(), SdkError> {
    if response.conversion_estimate.is_some() {
        return Err(SdkError::InvalidInput(
            "client signing is not supported for conversion sends".to_string(),
        ));
    }
    Ok(())
}

pub(in crate::sdk) async fn build_unsigned_transfer_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
    options: Option<&BuildTransferPackageOptions>,
) -> Result<UnsignedTransferPackage, SdkError> {
    reject_conversion(prepare_response)?;
    match &prepare_response.payment_method {
        SendPaymentMethod::SparkAddress { address, .. } => {
            build_spark_package(sdk, prepare_response, address, None).await
        }
        SendPaymentMethod::SparkInvoice {
            spark_invoice_details,
            ..
        } => {
            build_spark_package(
                sdk,
                prepare_response,
                &spark_invoice_details.invoice,
                Some(spark_invoice_details.invoice.clone()),
            )
            .await
        }
        SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            spark_transfer_fee_sats,
            lightning_fee_sats,
        } => {
            let (prefer_spark, completion_timeout_secs) = match options {
                Some(BuildTransferPackageOptions::Bolt11Invoice {
                    prefer_spark,
                    completion_timeout_secs,
                }) => (*prefer_spark, *completion_timeout_secs),
                _ => (sdk.config.prefer_spark_over_lightning, None),
            };
            if prefers_bolt11_spark_route(prefer_spark, prepare_response) {
                let spark_address = sdk
                    .spark_wallet
                    .extract_spark_address(&invoice_details.invoice.bolt11)?
                    .ok_or_else(|| {
                        SdkError::Generic("invoice expected to carry a spark address".to_string())
                    })?;
                let receiver = spark_address
                    .to_address_string()
                    .map_err(|e| SdkError::Generic(e.to_string()))?;
                if prepare_response.fee_policy == FeePolicy::FeesIncluded
                    && invoice_details.amount_msat.is_none()
                {
                    let mut adjusted = prepare_response.clone();
                    adjusted.amount = adjusted
                        .amount
                        .saturating_sub(u128::from(spark_transfer_fee_sats.unwrap_or(0)));
                    return build_spark_package(sdk, &adjusted, &receiver, None).await;
                }
                return build_spark_package(sdk, prepare_response, &receiver, None).await;
            }
            build_lightning_package(
                sdk,
                prepare_response,
                invoice_details,
                *lightning_fee_sats,
                completion_timeout_secs,
            )
            .await
        }
        SendPaymentMethod::BitcoinAddress { address, fee_quote } => {
            build_coop_exit_package(sdk, prepare_response, address, fee_quote, options).await
        }
        SendPaymentMethod::CrossChainAddress { .. } => Err(SdkError::InvalidInput(
            "client signing is not supported for cross-chain sends".to_string(),
        )),
    }
}

async fn build_spark_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
    receiver: &str,
    spark_invoice: Option<String>,
) -> Result<UnsignedTransferPackage, SdkError> {
    if let Some(token_identifier) = prepare_response.token_identifier.clone() {
        let fee = match &prepare_response.payment_method {
            SendPaymentMethod::SparkAddress { fee, .. }
            | SendPaymentMethod::SparkInvoice { fee, .. } => *fee,
            _ => 0,
        };
        return build_token_package(
            sdk,
            receiver,
            spark_invoice,
            token_identifier,
            prepare_response.amount,
            fee,
        )
        .await;
    }
    let amount_sat: u64 = prepare_response.amount.try_into()?;
    let spark_address = receiver
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
    let address = SparkAddress::new(
        spark_address.identity_public_key,
        spark_address.network,
        None,
    )
    .to_address_string()
    .map_err(|e| SdkError::Generic(e.to_string()))?;
    let prep = sdk
        .spark_wallet
        .prepare_transfer_package(amount_sat, &spark_address, None)
        .await?;
    to_unsigned_package(
        prep,
        amount_sat,
        0,
        TransferTarget::Spark {
            address,
            spark_invoice,
        },
    )
}

async fn build_lightning_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
    invoice_details: &crate::Bolt11InvoiceDetails,
    lightning_fee_sats: u64,
    completion_timeout_secs: Option<u32>,
) -> Result<UnsignedTransferPackage, SdkError> {
    let amount_sat: u64 = prepare_response.amount.try_into()?;
    let fee_policy = prepare_response.fee_policy;

    let receiver_sat =
        if fee_policy == FeePolicy::FeesIncluded && invoice_details.amount_msat.is_none() {
            let receiver = amount_sat.saturating_sub(lightning_fee_sats);
            if receiver == 0 {
                return Err(SdkError::InvalidInput(
                    "Amount too small to cover fees".to_string(),
                ));
            }
            receiver
        } else {
            amount_sat
        };

    let prep = sdk
        .spark_wallet
        .prepare_lightning_send_package(
            &invoice_details.invoice.bolt11,
            Some(receiver_sat),
            Some(lightning_fee_sats),
            None,
        )
        .await?;

    let fee_sat = if fee_policy == FeePolicy::FeesIncluded {
        match &prep {
            SendPackagePreparation::Ready(pt) => pt
                .leaves
                .iter()
                .map(|l| l.node.value)
                .sum::<u64>()
                .saturating_sub(receiver_sat),
            SendPackagePreparation::SwapRequired { .. } => lightning_fee_sats,
        }
    } else {
        lightning_fee_sats
    };

    to_unsigned_package(
        prep,
        receiver_sat,
        fee_sat,
        TransferTarget::Lightning {
            bolt11: invoice_details.invoice.bolt11.clone(),
            lnurl_pay: None,
            fee_policy,
            completion_timeout_secs,
        },
    )
}

async fn build_token_package(
    sdk: &BreezSdk,
    receiver: &str,
    spark_invoice: Option<String>,
    token_identifier: String,
    amount: u128,
    fee: u128,
) -> Result<UnsignedTransferPackage, SdkError> {
    let prepared = if let Some(invoice) = spark_invoice {
        sdk.spark_wallet
            .prepare_spark_invoice_token_package(&invoice, Some(amount))
            .await?
    } else {
        let spark_address = receiver
            .parse::<SparkAddress>()
            .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
        sdk.spark_wallet
            .prepare_token_package(
                vec![TransferTokenOutput {
                    token_id: token_identifier.clone(),
                    amount,
                    receiver_address: spark_address,
                    spark_invoice: None,
                }],
                None,
                None,
            )
            .await?
    };
    // A consolidation package re-shapes the wallet's outputs rather than paying
    // the receiver, so it carries no send amount or fee to display. It is exposed
    // as a swap: publishing it returns SwapCompleted, like the sats flow.
    let (prepared, is_swap, amount, fee) = match prepared {
        PreparedTokenPackage::Ready(pt) => (pt, false, amount, fee),
        PreparedTokenPackage::Consolidation(pt) => (pt, true, 0, 0),
    };
    let digest = prepared.partial_token_transaction_hash.clone();
    let token_context = serde_json::to_vec(&prepared)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize token transfer: {e}")))?;
    Ok(UnsignedTransferPackage::Token {
        prepare_token_transaction: ExternalPrepareTokenTransactionRequest {
            kind: ExternalTokenTransactionKind::Partial,
            digest,
        },
        token_context,
        token_identifier,
        amount,
        fee,
        is_swap,
    })
}

async fn build_coop_exit_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
    address: &BitcoinAddressDetails,
    fee_quote: &SendOnchainFeeQuote,
    options: Option<&BuildTransferPackageOptions>,
) -> Result<UnsignedTransferPackage, SdkError> {
    let Some(BuildTransferPackageOptions::BitcoinAddress { confirmation_speed }) = options else {
        return Err(SdkError::InvalidInput(
            "confirmation_speed is required for cooperative exit client signing".to_string(),
        ));
    };
    let amount_sat: u64 = prepare_response.amount.try_into()?;
    let exit_speed: ExitSpeed = confirmation_speed.clone().into();
    let coop_fee_quote: CoopExitFeeQuote = fee_quote.clone().into();
    let fee_sat = coop_fee_quote.fee_sats(&exit_speed);
    let receiver_sat = if prepare_response.fee_policy == FeePolicy::FeesIncluded {
        amount_sat.saturating_sub(fee_sat)
    } else {
        amount_sat
    };
    let dust_limit_sats = crate::utils::bitcoin_dust::get_dust_limit_sats(&address.address)?;
    if receiver_sat < dust_limit_sats {
        return Err(SdkError::InvalidInput(format!(
            "Amount is below the minimum of {dust_limit_sats} sats required for this address"
        )));
    }
    let prep = sdk
        .spark_wallet
        .prepare_coop_exit_package(
            &address.address,
            receiver_sat,
            exit_speed,
            coop_fee_quote,
            None,
        )
        .await?;
    to_unsigned_package(
        prep,
        receiver_sat,
        fee_sat,
        TransferTarget::CoopExit {
            address: address.address.clone(),
            fee_quote: fee_quote.clone(),
            confirmation_speed: confirmation_speed.clone(),
        },
    )
}

pub(in crate::sdk::payments) async fn submit_swap(
    sdk: &BreezSdk,
    signed_package: &SignedTransferPackage,
) -> Result<(), SdkError> {
    let (
        UnsignedTransferPackage::Swap {
            prepare_transfer,
            target_amounts,
            ..
        },
        TransferSignature::Transfer { signed },
    ) = (&signed_package.unsigned, &signed_package.signature)
    else {
        return Err(SdkError::InvalidInput(
            "submit_swap requires a Swap package".to_string(),
        ));
    };
    sdk.spark_wallet
        .publish_swap_package(
            prepare_transfer.transfer_id()?,
            prepare_transfer.leaf_ids()?,
            target_amounts.clone(),
            signed.to_prepared_transfer()?,
        )
        .await?;
    Ok(())
}
