use spark_wallet::{
    CoopExitFeeQuote, ExitSpeed, SendPackagePreparation, SparkAddress, TransferTokenOutput,
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

pub(in crate::sdk::payments) fn prefers_bolt11_spark_route(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
) -> bool {
    sdk.config.prefer_spark_over_lightning
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

fn reject_fees_included(response: &PrepareSendPaymentResponse) -> Result<(), SdkError> {
    if response.fee_policy == FeePolicy::FeesIncluded {
        return Err(SdkError::InvalidInput(
            "client signing does not yet support FeesIncluded".to_string(),
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
    reject_fees_included(prepare_response)?;
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
            lightning_fee_sats,
            ..
        } => {
            if prefers_bolt11_spark_route(sdk, prepare_response) {
                let spark_address = sdk
                    .spark_wallet
                    .extract_spark_address(&invoice_details.invoice.bolt11)?
                    .ok_or_else(|| {
                        SdkError::Generic("invoice expected to carry a spark address".to_string())
                    })?;
                let receiver = spark_address
                    .to_address_string()
                    .map_err(|e| SdkError::Generic(e.to_string()))?;
                return build_spark_package(sdk, prepare_response, &receiver, None).await;
            }
            let amount_sat: u64 = prepare_response.amount.try_into()?;
            let prep = sdk
                .spark_wallet
                .prepare_lightning_send_package(
                    &invoice_details.invoice.bolt11,
                    Some(amount_sat),
                    Some(*lightning_fee_sats),
                    None,
                )
                .await?;
            to_unsigned_package(
                prep,
                amount_sat,
                *lightning_fee_sats,
                TransferTarget::Lightning {
                    bolt11: invoice_details.invoice.bolt11.clone(),
                    lnurl_pay: None,
                },
            )
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

async fn build_token_package(
    sdk: &BreezSdk,
    receiver: &str,
    token_identifier: String,
    amount: u128,
    fee: u128,
) -> Result<UnsignedTransferPackage, SdkError> {
    let spark_address = receiver
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
    let prepared = sdk
        .spark_wallet
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
        .await?;
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
    let prep = sdk
        .spark_wallet
        .prepare_coop_exit_package(
            &address.address,
            amount_sat,
            exit_speed,
            coop_fee_quote,
            None,
        )
        .await?;
    to_unsigned_package(
        prep,
        amount_sat,
        fee_sat,
        TransferTarget::CoopExit {
            address: address.address.clone(),
            fee_quote: fee_quote.clone(),
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
