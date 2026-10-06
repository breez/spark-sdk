use bitcoin::secp256k1::schnorr;
use spark_wallet::{
    CoopExitFeeQuote, ExitSpeed, PreparedTokenPackage, PreparedTokenPull, SendPackagePreparation,
    SparkAddress, TokenRecipient,
};

use crate::{
    BitcoinAddressDetails, FeePolicy, SendOnchainFeeQuote,
    error::SdkError,
    models::{
        BuildTransferPackageOptions, BuildUnsignedPullPackageRequest, PrepareSendBatchResponse,
        PrepareSendPaymentResponse, PullPaymentResponse, SendPaymentMethod, SignedTransferPackage,
        TransferSignature, TransferTarget, UnsignedTransferPackage,
    },
    sdk::BreezSdk,
    sdk::payments::send,
    sdk::token_allowances::{
        decode_prepare_response, decode_pull_context, describes_pull, pull_receivers,
    },
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
        .prepare_transfer_package(amount_sat, &spark_address, spark_invoice.as_deref(), None)
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

pub(in crate::sdk::payments) async fn build_unsigned_batch_package(
    sdk: &BreezSdk,
    prepare_response: &PrepareSendBatchResponse,
) -> Result<UnsignedTransferPackage, SdkError> {
    let recipients = send::batch::to_token_recipients(&prepare_response.recipients)?;
    let prepared = sdk
        .spark_wallet
        .prepare_token_package(recipients, None, None)
        .await?;
    // A consolidation package re-shapes the wallet's outputs rather than paying
    // the recipients, so it carries no totals to display. It is exposed as a
    // swap: publishing it returns SwapCompleted, like the single-recipient flow.
    let (prepared, is_swap, totals) = match prepared {
        PreparedTokenPackage::Ready(pt) => (pt, false, prepare_response.totals.clone()),
        PreparedTokenPackage::Consolidation(pt) => (pt, true, Vec::new()),
    };
    let digest = prepared.partial_token_transaction_hash.clone();
    let token_context = serde_json::to_vec(&prepared)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize token transfer: {e}")))?;
    Ok(UnsignedTransferPackage::TokenBatch {
        prepare_token_transaction: ExternalPrepareTokenTransactionRequest {
            kind: ExternalTokenTransactionKind::Partial,
            digest,
        },
        token_context,
        totals,
        is_swap,
    })
}

async fn build_token_package(
    sdk: &BreezSdk,
    receiver: &str,
    spark_invoice: Option<String>,
    token_identifier: String,
    amount: u128,
    fee: u128,
) -> Result<UnsignedTransferPackage, SdkError> {
    let recipient = if let Some(invoice) = spark_invoice {
        TokenRecipient::Invoice {
            invoice,
            amount: Some(amount),
        }
    } else {
        TokenRecipient::Address {
            token_id: token_identifier.clone(),
            amount,
            receiver_address: receiver
                .parse::<SparkAddress>()
                .map_err(|_| SdkError::InvalidInput("Invalid spark address".to_string()))?,
        }
    };
    let prepared = sdk
        .spark_wallet
        .prepare_token_package(vec![recipient], None, None)
        .await?;
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

pub(in crate::sdk) fn build_unsigned_pull_package(
    request: &BuildUnsignedPullPackageRequest,
) -> Result<UnsignedTransferPackage, SdkError> {
    let prepared = decode_prepare_response(&request.prepare_response)?;
    let digest = prepared
        .spend_digest()
        .map_err(|e| SdkError::InvalidInput(e.to_string()))?
        .to_vec();
    Ok(UnsignedTransferPackage::TokenPull {
        prepare_token_transaction: ExternalPrepareTokenTransactionRequest {
            kind: ExternalTokenTransactionKind::AllowanceSpend,
            digest,
        },
        pull_context: request.prepare_response.pull_context.clone(),
        payer_public_key: prepared.payer_public_key.to_string(),
        token_identifier: prepared.token_identifier.clone(),
        receivers: pull_receivers(&prepared),
        amount: prepared.total(),
        expiry_time: prepared.expiry_time(),
    })
}

fn decode_signed_pull_package(
    signed_package: &SignedTransferPackage,
) -> Result<(PreparedTokenPull, schnorr::Signature), SdkError> {
    let (
        UnsignedTransferPackage::TokenPull {
            pull_context,
            payer_public_key,
            token_identifier,
            receivers,
            amount,
            expiry_time,
            ..
        },
        TransferSignature::Token { signed },
    ) = (&signed_package.unsigned, &signed_package.signature)
    else {
        return Err(SdkError::InvalidInput(
            "publish_signed_pull_package needs a signed TokenPull package".to_string(),
        ));
    };
    let prepared = decode_pull_context(pull_context)?;
    if *expiry_time != prepared.expiry_time()
        || !describes_pull(
            &prepared,
            payer_public_key,
            token_identifier,
            receivers,
            *amount,
        )
    {
        return Err(SdkError::InvalidInput(
            "The pull package doesn't match its pull context".to_string(),
        ));
    }
    Ok((prepared, signed.to_prepared_token_transaction()?.signature))
}

pub(in crate::sdk) async fn publish_signed_pull_package(
    sdk: &BreezSdk,
    signed_package: &SignedTransferPackage,
) -> Result<PullPaymentResponse, SdkError> {
    let (prepared, signature) = decode_signed_pull_package(signed_package)?;
    let transaction = sdk
        .spark_wallet
        .broadcast_token_pull(&prepared, signature)
        .await?;
    Ok(sdk.complete_pull(transaction).await)
}

#[cfg(test)]
mod pull_package_tests {
    use crate::{
        BuildUnsignedPullPackageRequest, PullReceiver, SignedTransferPackage, TransferSignature,
        UnsignedTransferPackage,
        error::SdkError,
        sdk::token_allowances::{
            prepare_response_for_tests,
            tests::{OTHER_KEY, sample_pull},
        },
        signer::{
            ExternalPreparedTokenTransaction, ExternalTokenTransactionKind, SchnorrSignatureBytes,
        },
    };

    use super::{build_unsigned_pull_package, decode_signed_pull_package};

    type ReviewFieldsEdit =
        fn(&mut String, &mut String, &mut Vec<PullReceiver>, &mut u128, &mut u64);

    fn signed_package() -> SignedTransferPackage {
        let prepare_response = prepare_response_for_tests(&sample_pull()).unwrap();
        SignedTransferPackage {
            unsigned: build_unsigned_pull_package(&BuildUnsignedPullPackageRequest {
                prepare_response,
            })
            .unwrap(),
            signature: TransferSignature::Token {
                signed: ExternalPreparedTokenTransaction {
                    signature: SchnorrSignatureBytes { bytes: vec![1; 64] },
                },
            },
        }
    }

    #[test]
    fn pull_package_shows_the_prepared_pull() {
        let pull = sample_pull();
        let UnsignedTransferPackage::TokenPull {
            payer_public_key,
            token_identifier,
            receivers,
            amount,
            expiry_time,
            ..
        } = build_unsigned_pull_package(&BuildUnsignedPullPackageRequest {
            prepare_response: prepare_response_for_tests(&pull).unwrap(),
        })
        .unwrap()
        else {
            panic!("expected a TokenPull package");
        };
        assert_eq!(payer_public_key, pull.payer_public_key.to_string());
        assert_eq!(token_identifier, pull.token_identifier);
        assert_eq!(
            receivers,
            vec![PullReceiver {
                amount: 42,
                receiver_public_key: Some(pull.receivers[0].receiver_public_key.to_string()),
            }]
        );
        assert_eq!(amount, 42);
        assert_eq!(expiry_time, pull.expiry_time());
    }

    #[test]
    fn building_refuses_an_edited_prepare_response() {
        let mut prepare_response = prepare_response_for_tests(&sample_pull()).unwrap();
        prepare_response.amount = 1;
        assert!(matches!(
            build_unsigned_pull_package(&BuildUnsignedPullPackageRequest { prepare_response }),
            Err(SdkError::InvalidInput(_))
        ));
    }

    #[test]
    fn publishing_accepts_the_built_package() {
        let (prepared, _) = decode_signed_pull_package(&signed_package()).unwrap();
        assert_eq!(prepared.total(), 42);
    }

    #[test]
    fn publishing_refuses_a_package_edited_after_build() {
        let edits: [ReviewFieldsEdit; 5] = [
            |payer, _, _, _, _| *payer = OTHER_KEY.to_string(),
            |_, token, _, _, _| *token = "btknrt1other".to_string(),
            |_, _, receivers, _, _| {
                receivers[0].receiver_public_key = Some(OTHER_KEY.to_string());
            },
            |_, _, _, amount, _| *amount = 1,
            |_, _, _, _, expiry_time| *expiry_time += 1,
        ];
        for edit in edits {
            let mut package = signed_package();
            let UnsignedTransferPackage::TokenPull {
                payer_public_key,
                token_identifier,
                receivers,
                amount,
                expiry_time,
                ..
            } = &mut package.unsigned
            else {
                panic!("expected a TokenPull package");
            };
            edit(
                payer_public_key,
                token_identifier,
                receivers,
                amount,
                expiry_time,
            );
            assert!(matches!(
                decode_signed_pull_package(&package),
                Err(SdkError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn pull_package_asks_for_the_spend_digest() {
        let pull = sample_pull();
        let prepare_response =
            crate::sdk::token_allowances::prepare_response_for_tests(&pull).unwrap();
        let package =
            build_unsigned_pull_package(&BuildUnsignedPullPackageRequest { prepare_response })
                .unwrap();
        let UnsignedTransferPackage::TokenPull {
            prepare_token_transaction,
            amount,
            ..
        } = package
        else {
            panic!("expected a TokenPull package");
        };
        assert!(matches!(
            prepare_token_transaction.kind,
            ExternalTokenTransactionKind::AllowanceSpend
        ));
        assert_eq!(
            prepare_token_transaction.digest,
            pull.spend_digest().unwrap().to_vec()
        );
        assert_eq!(amount, 42);
    }
}
