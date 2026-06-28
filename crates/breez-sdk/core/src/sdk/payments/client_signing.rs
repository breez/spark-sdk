use spark_wallet::{
    CoopExitFeeQuote, ExitSpeed, SendPackagePreparation, SparkAddress, TransferTokenOutput,
};

use crate::{
    BitcoinAddressDetails, SendOnchainFeeQuote,
    error::SdkError,
    models::{
        BuildTransferPackageOptions, PrepareSendPaymentResponse, SendPaymentMethod,
        SignedTransferPackage, UnsignedTransferPackage,
    },
    sdk::BreezSdk,
    signer::{
        ExternalPrepareTokenTransactionRequest, ExternalPrepareTransferRequest,
        ExternalTokenTransactionKind,
    },
};

fn to_unsigned_package(
    prep: SendPackagePreparation,
    swap_target_amounts: Vec<u64>,
) -> Result<UnsignedTransferPackage, SdkError> {
    Ok(match prep {
        SendPackagePreparation::Ready(pt) => UnsignedTransferPackage::Transfer {
            prepare_transfer: ExternalPrepareTransferRequest::from_prepare_transfer_request(&pt)?,
        },
        SendPackagePreparation::SwapRequired(pt) => UnsignedTransferPackage::Swap {
            prepare_transfer: ExternalPrepareTransferRequest::from_prepare_transfer_request(&pt)?,
            target_amounts: swap_target_amounts,
        },
    })
}

fn reject_conversion(response: &PrepareSendPaymentResponse) -> Result<(), SdkError> {
    if response.conversion_estimate.is_some() {
        return Err(SdkError::InvalidInput(
            "client signing is not supported for conversion sends".to_string(),
        ));
    }
    Ok(())
}

pub(in crate::sdk::payments) async fn build_unsigned_transfer_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendPaymentResponse,
    options: Option<&BuildTransferPackageOptions>,
) -> Result<UnsignedTransferPackage, SdkError> {
    reject_conversion(prepare_response)?;
    match &prepare_response.payment_method {
        SendPaymentMethod::SparkAddress { address, .. } => {
            build_spark_package(sdk, prepare_response, address).await
        }
        SendPaymentMethod::SparkInvoice {
            spark_invoice_details,
            ..
        } => build_spark_package(sdk, prepare_response, &spark_invoice_details.invoice).await,
        SendPaymentMethod::Bolt11Invoice {
            invoice_details,
            lightning_fee_sats,
            ..
        } => {
            let amount_sat: u64 = prepare_response.amount.try_into()?;
            let prep = sdk
                .spark_wallet
                .prepare_lightning_send_package(
                    &invoice_details.invoice.bolt11,
                    Some(amount_sat),
                    None,
                    None,
                )
                .await?;
            to_unsigned_package(prep, vec![amount_sat.saturating_add(*lightning_fee_sats)])
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
) -> Result<UnsignedTransferPackage, SdkError> {
    if let Some(token_identifier) = prepare_response.token_identifier.clone() {
        return build_token_package(sdk, receiver, token_identifier, prepare_response.amount).await;
    }
    let amount_sat: u64 = prepare_response.amount.try_into()?;
    let spark_address = receiver
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
    let prep = sdk
        .spark_wallet
        .prepare_transfer_package(amount_sat, &spark_address, None)
        .await?;
    to_unsigned_package(prep, vec![amount_sat])
}

async fn build_token_package(
    sdk: &BreezSdk,
    receiver: &str,
    token_identifier: String,
    amount: u128,
) -> Result<UnsignedTransferPackage, SdkError> {
    let spark_address = receiver
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?;
    let prepared = sdk
        .spark_wallet
        .prepare_token_package(
            vec![TransferTokenOutput {
                token_id: token_identifier,
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
    let fee_sats = coop_fee_quote.fee_sats(&exit_speed);
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
    to_unsigned_package(prep, vec![amount_sat, fee_sats])
}

pub(in crate::sdk::payments) async fn submit_swap(
    sdk: &BreezSdk,
    signed_package: &SignedTransferPackage,
) -> Result<(), SdkError> {
    let SignedTransferPackage::Swap {
        prepare_transfer,
        target_amounts,
        signed,
    } = signed_package
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
