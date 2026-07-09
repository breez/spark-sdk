use std::sync::Arc;

use anyhow::Result;
use breez_sdk_spark::signer::ExternalSparkSigner;
use breez_sdk_spark::*;
use log::info;

async fn sign_package(
    signer: &Arc<dyn ExternalSparkSigner>,
    unsigned: UnsignedTransferPackage,
) -> Result<SignedTransferPackage> {
    // ANCHOR: client-signing-sign-package
    let signature = match &unsigned {
        UnsignedTransferPackage::Transfer {
            prepare_transfer,
            amount_sat,
            fee_sat,
            target,
        } => {
            // Show the user what they are approving before signing
            let destination = match target {
                TransferTarget::Spark { address, .. } => address,
                TransferTarget::Lightning { bolt11, .. } => bolt11,
                TransferTarget::CoopExit { address, .. } => address,
            };
            info!("Approve sending {amount_sat} sats (fee {fee_sat} sats) to {destination}");
            TransferSignature::Transfer {
                signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
            }
        }
        UnsignedTransferPackage::Swap {
            prepare_transfer,
            amount_sat,
            fee_sat,
            ..
        } => {
            info!("Approve re-shaping funds for a {amount_sat} sat send (fee {fee_sat} sats)");
            TransferSignature::Transfer {
                signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
            }
        }
        UnsignedTransferPackage::Token {
            prepare_token_transaction,
            token_identifier,
            amount,
            fee,
            is_swap,
            ..
        } => {
            if *is_swap {
                info!("Approve combining token outputs for a {token_identifier} send");
            } else {
                info!("Approve sending {amount} of token {token_identifier} (fee {fee})");
            }
            TransferSignature::Token {
                signed: signer
                    .prepare_token_transaction(prepare_token_transaction.clone())
                    .await?,
            }
        }
    };

    let signed_package = SignedTransferPackage {
        unsigned,
        signature,
    };
    // ANCHOR_END: client-signing-sign-package
    Ok(signed_package)
}

async fn send_with_client_signing(
    sdk: &BreezSdk,
    signer: &Arc<dyn ExternalSparkSigner>,
) -> Result<Payment> {
    // ANCHOR: client-signing-send
    let prepare_response = sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: "<spark address or invoice>".to_string(),
            },
            amount: Some(5_000),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    loop {
        let unsigned = sdk
            .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
                prepare_response: prepare_response.clone(),
                options: None,
            })
            .await?;

        // Send the package to the user, who reviews and signs it
        let signed_package = sign_package(signer, unsigned).await?;

        match sdk
            .publish_signed_transfer_package(PublishSignedTransferPackageRequest { signed_package })
            .await?
        {
            // The wallet's funds were re-shaped first: build the payment again
            PublishSignedTransferPackageResponse::SwapCompleted => continue,
            PublishSignedTransferPackageResponse::PaymentSent { payment } => {
                return Ok(payment);
            }
        }
    }
    // ANCHOR_END: client-signing-send
}

async fn build_onchain_package(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: client-signing-build-onchain-options
    // For Bitcoin address sends, the confirmation speed is chosen when
    // building the package: the fee depends on it
    let unsigned = sdk
        .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
            prepare_response,
            options: Some(BuildTransferPackageOptions::BitcoinAddress {
                confirmation_speed: OnchainConfirmationSpeed::Medium,
            }),
        })
        .await?;
    // ANCHOR_END: client-signing-build-onchain-options
    info!("Unsigned package: {unsigned:?}");
    Ok(())
}

async fn build_bolt11_package(
    sdk: &BreezSdk,
    prepare_response: PrepareSendPaymentResponse,
) -> Result<()> {
    // ANCHOR: client-signing-build-bolt11-options
    let unsigned = sdk
        .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
            prepare_response,
            options: Some(BuildTransferPackageOptions::Bolt11Invoice {
                prefer_spark: true,
                completion_timeout_secs: Some(10),
            }),
        })
        .await?;
    // ANCHOR_END: client-signing-build-bolt11-options
    info!("Unsigned package: {unsigned:?}");
    Ok(())
}

async fn lnurl_pay_with_client_signing(
    sdk: &BreezSdk,
    signer: &Arc<dyn ExternalSparkSigner>,
    prepare_response: PrepareLnurlPayResponse,
) -> Result<LnurlPayResponse> {
    // ANCHOR: client-signing-lnurl-pay
    loop {
        let unsigned = sdk
            .build_unsigned_lnurl_pay_package(BuildUnsignedLnurlPayPackageRequest {
                prepare_response: prepare_response.clone(),
            })
            .await?;

        let signed_package = sign_package(signer, unsigned).await?;

        match sdk
            .publish_signed_lnurl_pay_package(PublishSignedLnurlPayPackageRequest {
                signed_package,
            })
            .await?
        {
            PublishSignedLnurlPayResponse::SwapCompleted => continue,
            PublishSignedLnurlPayResponse::PaymentSent { response } => {
                return Ok(response);
            }
        }
    }
    // ANCHOR_END: client-signing-lnurl-pay
}
