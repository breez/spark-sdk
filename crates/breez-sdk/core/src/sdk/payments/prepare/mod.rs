mod bitcoin_address;
mod bolt11;
pub(in crate::sdk::payments) mod cross_chain;
mod spark_address;
mod spark_invoice;

use crate::{
    InputType,
    error::SdkError,
    models::{PaymentRequest, PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
};

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: PrepareSendPaymentRequest,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let input = match &request.payment_request {
        PaymentRequest::Input { input } => input.clone(),
        PaymentRequest::CrossChain { .. } => {
            return Err(SdkError::Generic(
                "prepare::prepare called with PaymentRequest::CrossChain — \
                 this variant must be dispatched at payments::mod.rs::prepare_send_payment"
                    .to_string(),
            ));
        }
    };
    let parsed_input = sdk.parse(&input).await?;

    let fee_policy = request.fee_policy.unwrap_or_default();
    let token_identifier = request.token_identifier.clone();

    match &parsed_input {
        InputType::SparkAddress(details) => {
            spark_address::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::SparkInvoice(details) => {
            spark_invoice::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::Bolt11Invoice(details) => {
            bolt11::prepare(sdk, &input, &request, details, fee_policy, token_identifier).await
        }
        InputType::BitcoinAddress(details) => {
            bitcoin_address::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::CrossChainAddress(_) => Err(SdkError::InvalidInput(
            "Cross-chain address detected. Use get_cross_chain_routes() to discover \
             routes, then PaymentRequest::CrossChain { address, route }."
                .to_string(),
        )),
        _ => Err(SdkError::InvalidInput(
            "Unsupported payment method".to_string(),
        )),
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::models::PrepareSendPaymentRequest;
    use crate::{BitcoinNetwork, Bolt11InvoiceDetails, FeePolicy, SparkInvoiceDetails};

    pub(crate) fn create_test_request() -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: crate::models::PaymentRequest::Input {
                input: "test_request".to_string(),
            },
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        }
    }

    pub(crate) fn create_bitcoin_amount_request(amount_sats: u64) -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: crate::models::PaymentRequest::Input {
                input: "test_request".to_string(),
            },
            amount: Some(u128::from(amount_sats)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        }
    }

    pub(crate) fn create_token_amount_request(
        amount: u128,
        token_identifier: &str,
    ) -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: crate::models::PaymentRequest::Input {
                input: "test_request".to_string(),
            },
            amount: Some(amount),
            token_identifier: Some(token_identifier.to_string()),
            conversion_options: None,
            fee_policy: None,
        }
    }

    pub(crate) fn create_fees_included_request(amount: u128) -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: crate::models::PaymentRequest::Input {
                input: "test_request".to_string(),
            },
            amount: Some(amount),
            token_identifier: None,
            conversion_options: None,
            fee_policy: Some(FeePolicy::FeesIncluded),
        }
    }

    pub(crate) fn create_test_invoice() -> SparkInvoiceDetails {
        SparkInvoiceDetails {
            invoice: "test_invoice".to_string(),
            identity_public_key: "test_identity_key".to_string(),
            network: BitcoinNetwork::Regtest,
            amount: None,
            token_identifier: None,
            expiry_time: None,
            description: None,
            sender_public_key: None,
        }
    }

    pub(crate) fn create_test_bolt11_invoice() -> Bolt11InvoiceDetails {
        use crate::{Bolt11Invoice, PaymentRequestSource};
        Bolt11InvoiceDetails {
            amount_msat: None,
            description: None,
            description_hash: None,
            expiry: 3600,
            invoice: Bolt11Invoice {
                bolt11: "lnbc1...".to_string(),
                source: PaymentRequestSource::default(),
            },
            min_final_cltv_expiry_delta: 144,
            network: BitcoinNetwork::Regtest,
            payee_pubkey: "test_pubkey".to_string(),
            payment_hash: "test_hash".to_string(),
            payment_secret: "test_secret".to_string(),
            routing_hints: vec![],
            timestamp: 0,
        }
    }
}
