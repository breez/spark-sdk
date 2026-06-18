mod bitcoin_address;
mod bolt11;
mod spark_address;
mod spark_invoice;

use crate::{
    InputType,
    error::SdkError,
    models::{PrepareSendPaymentRequest, PrepareSendPaymentResponse},
    sdk::BreezSdk,
};

pub(super) async fn prepare(
    sdk: &BreezSdk,
    request: PrepareSendPaymentRequest,
) -> Result<PrepareSendPaymentResponse, SdkError> {
    let parsed_input = sdk.parse(&request.payment_request).await?;

    // A BIP21 URI wraps one or more concrete payment methods. Resolve it to the
    // preferred method so callers can pay a `bitcoin:` URI directly. A bare
    // address carries no amount, so fall back to the URI's `amount` when the
    // caller didn't pass one (an invoice carries its own amount).
    let (payment_method, request) = match &parsed_input {
        InputType::Bip21(details) => {
            let method =
                select_bip21_payment_method(&details.payment_methods).ok_or_else(|| {
                    SdkError::InvalidInput("BIP21 URI has no supported payment method".to_string())
                })?;
            let mut request = request;
            if request.amount.is_none()
                && matches!(
                    method,
                    InputType::BitcoinAddress(_) | InputType::SparkAddress(_)
                )
                && let Some(amount_sat) = details.amount_sat
            {
                request.amount = Some(u128::from(amount_sat));
            }
            (method, request)
        }
        other => (other, request),
    };

    let fee_policy = request.fee_policy.unwrap_or_default();
    let token_identifier = request.token_identifier.clone();

    match payment_method {
        InputType::SparkAddress(details) => {
            spark_address::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::SparkInvoice(details) => {
            spark_invoice::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::Bolt11Invoice(details) => {
            bolt11::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        InputType::BitcoinAddress(details) => {
            bitcoin_address::prepare(sdk, &request, details, fee_policy, token_identifier).await
        }
        _ => Err(SdkError::InvalidInput(
            "Unsupported payment method".to_string(),
        )),
    }
}

/// Picks the method to pay from a BIP21 URI's `payment_methods`, in the SDK's
/// cost/speed order: Spark (instant, no chain fee) before Lightning before
/// on-chain. Methods the send flow can't action (BOLT12 offers, silent
/// payments) are skipped. Returns None when no payable method is present.
fn select_bip21_payment_method(methods: &[InputType]) -> Option<&InputType> {
    fn rank(method: &InputType) -> u8 {
        match method {
            InputType::SparkInvoice(_) => 0,
            InputType::SparkAddress(_) => 1,
            InputType::Bolt11Invoice(_) => 2,
            InputType::BitcoinAddress(_) => 3,
            _ => u8::MAX,
        }
    }
    methods
        .iter()
        .filter(|method| rank(method) != u8::MAX)
        .min_by_key(|method| rank(method))
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::models::PrepareSendPaymentRequest;
    use crate::{BitcoinNetwork, Bolt11InvoiceDetails, FeePolicy, SparkInvoiceDetails};

    pub(crate) fn create_test_request() -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: "test_request".to_string(),
            amount: None,
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        }
    }

    pub(crate) fn create_bitcoin_amount_request(amount_sats: u64) -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: "test_request".to_string(),
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
            payment_request: "test_request".to_string(),
            amount: Some(amount),
            token_identifier: Some(token_identifier.to_string()),
            conversion_options: None,
            fee_policy: None,
        }
    }

    pub(crate) fn create_fees_included_request(amount: u128) -> PrepareSendPaymentRequest {
        PrepareSendPaymentRequest {
            payment_request: "test_request".to_string(),
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

#[cfg(test)]
mod selection_tests {
    use super::test_helpers::{create_test_bolt11_invoice, create_test_invoice};
    use super::*;
    use crate::{BitcoinAddressDetails, BitcoinNetwork, PaymentRequestSource, SparkAddressDetails};

    fn bitcoin_address() -> InputType {
        InputType::BitcoinAddress(BitcoinAddressDetails {
            address: "bc1qaddr".to_string(),
            network: BitcoinNetwork::Bitcoin,
            source: PaymentRequestSource::default(),
        })
    }

    fn spark_address() -> InputType {
        InputType::SparkAddress(SparkAddressDetails {
            address: "sp1addr".to_string(),
            identity_public_key: "pubkey".to_string(),
            network: BitcoinNetwork::Regtest,
            source: PaymentRequestSource::default(),
        })
    }

    #[test]
    fn prefers_spark_over_lightning_and_onchain() {
        let methods = vec![
            bitcoin_address(),
            InputType::Bolt11Invoice(create_test_bolt11_invoice()),
            spark_address(),
        ];
        assert!(matches!(
            select_bip21_payment_method(&methods),
            Some(InputType::SparkAddress(_))
        ));
    }

    #[test]
    fn prefers_lightning_over_onchain() {
        let methods = vec![
            bitcoin_address(),
            InputType::Bolt11Invoice(create_test_bolt11_invoice()),
        ];
        assert!(matches!(
            select_bip21_payment_method(&methods),
            Some(InputType::Bolt11Invoice(_))
        ));
    }

    #[test]
    fn falls_back_to_onchain() {
        let methods = vec![bitcoin_address()];
        assert!(matches!(
            select_bip21_payment_method(&methods),
            Some(InputType::BitcoinAddress(_))
        ));
    }

    #[test]
    fn prefers_spark_invoice() {
        let methods = vec![
            InputType::Bolt11Invoice(create_test_bolt11_invoice()),
            InputType::SparkInvoice(create_test_invoice()),
        ];
        assert!(matches!(
            select_bip21_payment_method(&methods),
            Some(InputType::SparkInvoice(_))
        ));
    }

    #[test]
    fn none_when_no_payable_method() {
        let methods: Vec<InputType> = vec![];
        assert!(select_bip21_payment_method(&methods).is_none());
    }
}
