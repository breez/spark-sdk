use breez_sdk_common::input;
use spark_wallet::SspUserRequest;
use sparkscan::types::{
    AddressTransaction, AddressTransactionDirection, AddressTransactionStatus,
    AddressTransactionType, MultiIoDetails, TokenTransactionMetadata, TokenTransactionStatus,
};
use tracing::warn;

use crate::{
    Network, Payment, PaymentDetails, PaymentMethod, PaymentStatus, PaymentType, SdkError,
    TokenMetadata,
};

impl From<AddressTransactionStatus> for PaymentStatus {
    fn from(status: AddressTransactionStatus) -> Self {
        match status {
            AddressTransactionStatus::Confirmed => PaymentStatus::Completed,
            AddressTransactionStatus::Sent | AddressTransactionStatus::Pending => {
                PaymentStatus::Pending
            }
            AddressTransactionStatus::Expired | AddressTransactionStatus::Failed => {
                PaymentStatus::Failed
            }
        }
    }
}

impl From<TokenTransactionStatus> for PaymentStatus {
    fn from(status: TokenTransactionStatus) -> Self {
        match status {
            TokenTransactionStatus::Confirmed => PaymentStatus::Completed,
            TokenTransactionStatus::Sent | TokenTransactionStatus::Pending => {
                PaymentStatus::Pending
            }
            TokenTransactionStatus::Expired | TokenTransactionStatus::Failed => {
                PaymentStatus::Failed
            }
        }
    }
}

impl From<Network> for sparkscan::types::Network {
    fn from(network: Network) -> Self {
        match network {
            Network::Mainnet => sparkscan::types::Network::Mainnet,
            Network::Regtest => sparkscan::types::Network::Regtest,
        }
    }
}

impl TryFrom<TokenTransactionMetadata> for TokenMetadata {
    type Error = SdkError;

    fn try_from(value: TokenTransactionMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            identifier: value.token_address,
            issuer_public_key: value.issuer_public_key,
            name: value.name,
            ticker: value.ticker,
            decimals: value.decimals.try_into()?,
            max_supply: value
                .max_supply
                .ok_or(SdkError::Generic("Max supply is not set".to_string()))?
                .try_into()?, // max_supply will be changed to u128 or similar
            is_freezable: value
                .is_freezable
                .ok_or(SdkError::Generic("Is freezable is not set".to_string()))?,
        })
    }
}

/// Context for payment conversion containing common data
#[derive(Debug)]
struct PaymentCommonContext {
    timestamp: u64,
    status: PaymentStatus,
}

/// Information about payment method, details, and fees
#[derive(Debug)]
struct PaymentMethodInfo {
    method: PaymentMethod,
    details: Option<PaymentDetails>,
    fees: u64,
}

/// Converts a Sparkscan address transaction into Payment objects
pub(crate) fn payments_from_address_transaction_and_ssp_request(
    transaction: &AddressTransaction,
    ssp_user_request: Option<&SspUserRequest>,
    our_spark_address: &str,
) -> Result<Vec<Payment>, SdkError> {
    let context = extract_conversion_context(transaction)?;
    let method_info = extract_payment_method_and_details(transaction, ssp_user_request)?;

    if transaction.multi_io_details.is_some() {
        create_multi_io_payments(transaction, &method_info, &context, our_spark_address)
    } else {
        let payment = create_single_payment(transaction, &method_info, &context)?;
        Ok(vec![payment])
    }
}

/// Extracts common conversion context from transaction
fn extract_conversion_context(
    transaction: &AddressTransaction,
) -> Result<PaymentCommonContext, SdkError> {
    let timestamp = transaction
        .created_at
        .ok_or(SdkError::Generic(
            "Transaction created at is not set".to_string(),
        ))?
        .timestamp()
        .try_into()?;

    let status = transaction.status.into();

    Ok(PaymentCommonContext { timestamp, status })
}

/// Determines payment method, details, and fees based on transaction type
fn extract_payment_method_and_details(
    transaction: &AddressTransaction,
    ssp_user_request: Option<&SspUserRequest>,
) -> Result<PaymentMethodInfo, SdkError> {
    match transaction.type_ {
        AddressTransactionType::SparkTransfer => Ok(PaymentMethodInfo {
            method: PaymentMethod::Spark,
            details: Some(PaymentDetails::Spark),
            fees: 0,
        }),
        AddressTransactionType::LightningPayment => {
            create_lightning_payment_info(transaction, ssp_user_request)
        }
        AddressTransactionType::BitcoinDeposit => {
            Ok(create_deposit_payment_info(transaction, ssp_user_request))
        }
        AddressTransactionType::BitcoinWithdrawal => {
            Ok(create_withdraw_payment_info(transaction, ssp_user_request))
        }
        AddressTransactionType::TokenTransfer
        | AddressTransactionType::TokenMint
        | AddressTransactionType::TokenBurn
        | AddressTransactionType::TokenMultiTransfer
        | AddressTransactionType::UnknownTokenOp => create_token_payment_info(transaction),
    }
}

/// Creates payment info for Lightning transactions
fn create_lightning_payment_info(
    transaction: &AddressTransaction,
    ssp_user_request: Option<&SspUserRequest>,
) -> Result<PaymentMethodInfo, SdkError> {
    if let Some(request) = ssp_user_request {
        let invoice = request.get_lightning_invoice().ok_or(SdkError::Generic(
            "No invoice in SspUserRequest".to_string(),
        ))?;
        let invoice_details = input::parse_invoice(&invoice).ok_or(SdkError::Generic(
            "Invalid invoice in SspUserRequest::LightningReceiveRequest".to_string(),
        ))?;
        let preimage = request.get_lightning_preimage();
        let fees = request.get_total_fees_sats();

        Ok(PaymentMethodInfo {
            method: PaymentMethod::Lightning,
            details: Some(PaymentDetails::Lightning {
                description: invoice_details.description.clone(),
                preimage,
                invoice,
                payment_hash: invoice_details.payment_hash.clone(),
                destination_pubkey: invoice_details.payee_pubkey.clone(),
                lnurl_pay_info: None,
            }),
            fees,
        })
    } else {
        warn!(
            "No SspUserRequest found for LightningPayment with transfer id {}",
            transaction.id
        );
        Ok(PaymentMethodInfo {
            method: PaymentMethod::Lightning,
            details: None,
            fees: 0,
        })
    }
}

/// Creates payment info for Bitcoin deposit transactions
fn create_deposit_payment_info(
    transaction: &AddressTransaction,
    ssp_user_request: Option<&SspUserRequest>,
) -> PaymentMethodInfo {
    if let Some(SspUserRequest::ClaimStaticDeposit(request)) = ssp_user_request {
        let fees = request.get_total_fees_sats();
        PaymentMethodInfo {
            method: PaymentMethod::Deposit,
            details: Some(PaymentDetails::Deposit {
                tx_id: request.transaction_id.clone(),
            }),
            fees,
        }
    } else {
        warn!(
            "No SspUserRequest found for BitcoinDeposit with transfer id {}",
            transaction.id
        );
        PaymentMethodInfo {
            method: PaymentMethod::Deposit,
            details: None,
            fees: 0,
        }
    }
}

/// Creates payment info for Bitcoin withdrawal transactions
fn create_withdraw_payment_info(
    transaction: &AddressTransaction,
    ssp_user_request: Option<&SspUserRequest>,
) -> PaymentMethodInfo {
    if let Some(SspUserRequest::CoopExitRequest(request)) = ssp_user_request {
        let fees = request.get_total_fees_sats();
        PaymentMethodInfo {
            method: PaymentMethod::Withdraw,
            details: Some(PaymentDetails::Withdraw {
                tx_id: request.coop_exit_txid.clone(),
            }),
            fees,
        }
    } else {
        warn!(
            "No SspUserRequest found for BitcoinWithdrawal with transfer id {}",
            transaction.id
        );
        PaymentMethodInfo {
            method: PaymentMethod::Withdraw,
            details: None,
            fees: 0,
        }
    }
}

/// Creates payment info for token transactions
fn create_token_payment_info(
    transaction: &AddressTransaction,
) -> Result<PaymentMethodInfo, SdkError> {
    let Some(metadata) = &transaction.token_metadata else {
        return Err(SdkError::Generic(
            "No token metadata in transaction".to_string(),
        ));
    };

    Ok(PaymentMethodInfo {
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: metadata.clone().try_into()?,
            tx_hash: transaction.id.clone(),
        }),
        fees: 0,
    })
}

/// Creates multiple payments for multi-IO token transactions
fn create_multi_io_payments(
    transaction: &AddressTransaction,
    method_info: &PaymentMethodInfo,
    context: &PaymentCommonContext,
    our_spark_address: &str,
) -> Result<Vec<Payment>, SdkError> {
    let multi_io_details = transaction.multi_io_details.as_ref().unwrap();

    let payment_type = determine_multi_io_payment_type(multi_io_details, our_spark_address)?;

    let mut payments = Vec::new();

    for (index, output) in multi_io_details.outputs.iter().enumerate() {
        // Create payments for outputs that are not ours (for send payments) or ours (for receive payments)
        if should_include_output(payment_type, &output.address, our_spark_address) {
            let id = format!("{}:{}", transaction.id, index);
            let amount = output.amount.try_into()?;

            payments.push(Payment {
                id,
                payment_type,
                status: context.status,
                amount,
                fees: 0,
                timestamp: context.timestamp,
                method: method_info.method,
                details: method_info.details.clone(),
            });
        }
    }

    Ok(payments)
}

/// Determines payment type for multi-IO transactions based on input ownership
fn determine_multi_io_payment_type(
    multi_io_details: &MultiIoDetails,
    our_spark_address: &str,
) -> Result<PaymentType, SdkError> {
    let first_input = multi_io_details.inputs.first().ok_or(SdkError::Generic(
        "No inputs in multi IO details".to_string(),
    ))?;

    if first_input.address == our_spark_address {
        Ok(PaymentType::Send)
    } else {
        Ok(PaymentType::Receive)
    }
}

/// Determines if an output should be included in the payment list
fn should_include_output(
    payment_type: PaymentType,
    output_address: &str,
    our_spark_address: &str,
) -> bool {
    match payment_type {
        PaymentType::Send => output_address != our_spark_address,
        PaymentType::Receive => output_address == our_spark_address,
    }
}

/// Creates a single payment for non-multi-IO transactions
fn create_single_payment(
    transaction: &AddressTransaction,
    method_info: &PaymentMethodInfo,
    context: &PaymentCommonContext,
) -> Result<Payment, SdkError> {
    let id = transaction.id.clone();
    let payment_type = determine_single_payment_type(transaction, method_info.method)?;
    let amount = calculate_payment_amount(transaction, method_info)?;

    Ok(Payment {
        id,
        payment_type,
        status: context.status,
        amount,
        fees: method_info.fees,
        timestamp: context.timestamp,
        method: method_info.method,
        details: method_info.details.clone(),
    })
}

/// Determines payment type for single transactions based on method and transaction details
fn determine_single_payment_type(
    transaction: &AddressTransaction,
    method: PaymentMethod,
) -> Result<PaymentType, SdkError> {
    match method {
        PaymentMethod::Lightning | PaymentMethod::Spark => match transaction.direction {
            AddressTransactionDirection::Incoming => Ok(PaymentType::Receive),
            AddressTransactionDirection::Outgoing => Ok(PaymentType::Send),
            _ => Err(SdkError::Generic(format!(
                "Invalid direction in transaction {}",
                transaction.id
            ))),
        },
        PaymentMethod::Token => match transaction.type_ {
            AddressTransactionType::TokenMint => Ok(PaymentType::Receive),
            AddressTransactionType::TokenBurn => Ok(PaymentType::Send),
            _ => Err(SdkError::Generic(format!(
                "Invalid type in TokenTransaction transaction {}",
                transaction.id
            ))),
        },
        PaymentMethod::Deposit => Ok(PaymentType::Receive),
        PaymentMethod::Withdraw => Ok(PaymentType::Send),
        PaymentMethod::Unknown => Err(SdkError::Generic(format!(
            "Unexpected payment method in transaction {}",
            transaction.id
        ))),
    }
}

/// Calculates the payment amount, considering fees for different payment methods
fn calculate_payment_amount(
    transaction: &AddressTransaction,
    method_info: &PaymentMethodInfo,
) -> Result<u64, SdkError> {
    let transaction_amount: u64 = transaction
        .amount_sats
        .or(transaction.token_amount)
        .ok_or(SdkError::Generic(
            "Amount not found in transaction".to_string(),
        ))?
        .try_into()?;

    // For deposits, we don't subtract fees from the amount
    if method_info.method == PaymentMethod::Deposit {
        Ok(transaction_amount)
    } else {
        Ok(transaction_amount.saturating_sub(method_info.fees))
    }
}
