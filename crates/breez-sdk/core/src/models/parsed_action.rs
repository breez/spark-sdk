use crate::{
    Bip21Details, BitcoinAddressDetails, Bolt11InvoiceDetails, Bolt12InvoiceDetails,
    Bolt12OfferDetails, InputType, LightningAddressDetails, LnurlAuthRequestDetails,
    LnurlPayRequestDetails, LnurlWithdrawRequestDetails, SparkAddressDetails, SparkInvoiceDetails,
};

/// A high-level action derived from parsing user input.
///
/// Instead of matching on the low-level [`InputType`] variants, callers can match
/// on [`ParsedAction`] to quickly determine what to do with parsed input:
///
/// - [`Send`](ParsedAction::Send) — the input represents a destination to send funds to
/// - [`Receive`](ParsedAction::Receive) — the input allows receiving funds (e.g. LNURL-withdraw)
/// - [`Authenticate`](ParsedAction::Authenticate) — the input is an LNURL-auth challenge
/// - [`Multi`](ParsedAction::Multi) — the input contains multiple payment methods (e.g. BIP21)
/// - [`Unsupported`](ParsedAction::Unsupported) — the input was parsed but is not actionable
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ParsedAction {
    /// The parsed input represents a payment destination.
    Send(SendAction),
    /// The parsed input allows receiving funds (e.g. LNURL-withdraw).
    Receive(ReceiveAction),
    /// The parsed input is an LNURL-auth challenge.
    Authenticate(AuthAction),
    /// The parsed input contains multiple payment methods (e.g. BIP21 URI).
    Multi {
        /// The original BIP21 details containing metadata about the URI.
        bip21_details: Bip21Details,
        /// The individual actions extracted from the BIP21 payment methods.
        actions: Vec<ParsedAction>,
    },
    /// The input was parsed but represents an unsupported or non-actionable type.
    Unsupported {
        /// A description of what was parsed.
        raw: String,
    },
}

/// A send action — the parsed input represents a payment destination.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum SendAction {
    /// A BOLT11 Lightning invoice.
    Bolt11 {
        invoice_details: Bolt11InvoiceDetails,
    },
    /// A BOLT12 Lightning invoice.
    Bolt12Invoice {
        invoice_details: Bolt12InvoiceDetails,
    },
    /// A BOLT12 offer.
    Bolt12Offer { offer_details: Bolt12OfferDetails },
    /// A Spark invoice.
    SparkInvoice {
        invoice_details: SparkInvoiceDetails,
    },
    /// A Spark address.
    SparkAddress {
        address_details: SparkAddressDetails,
    },
    /// A Bitcoin on-chain address.
    Bitcoin {
        address_details: BitcoinAddressDetails,
    },
    /// An LNURL-Pay endpoint.
    LnurlPay { pay_details: LnurlPayRequestDetails },
    /// A Lightning address (user@domain).
    LightningAddress {
        address_details: LightningAddressDetails,
    },
}

/// A receive action — the parsed input allows receiving/withdrawing funds.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ReceiveAction {
    /// An LNURL-withdraw endpoint.
    LnurlWithdraw {
        withdraw_details: LnurlWithdrawRequestDetails,
    },
}

/// An authentication action — the parsed input is an LNURL-auth challenge.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AuthAction {
    /// The domain requesting authentication.
    pub domain: String,
    /// An optional action description (e.g. "login", "register").
    pub action: Option<String>,
    /// The underlying LNURL-auth request details needed to complete authentication.
    pub request_data: LnurlAuthRequestDetails,
}

// --- Conversions ---

impl SendAction {
    /// Extracts a payment request string suitable for `prepare_send_payment`.
    pub fn payment_request(&self) -> String {
        match self {
            SendAction::Bolt11 { invoice_details } => invoice_details.invoice.bolt11.clone(),
            SendAction::Bolt12Invoice { invoice_details } => {
                invoice_details.invoice.invoice.clone()
            }
            SendAction::Bolt12Offer { offer_details } => offer_details.offer.offer.clone(),
            SendAction::SparkInvoice { invoice_details } => invoice_details.invoice.clone(),
            SendAction::SparkAddress { address_details } => address_details.address.clone(),
            SendAction::Bitcoin { address_details } => address_details.address.clone(),
            SendAction::LnurlPay { pay_details } => {
                // Use the address field if available (for lightning addresses resolved to LNURL-pay),
                // otherwise fall back to the URL
                pay_details
                    .address
                    .clone()
                    .unwrap_or_else(|| pay_details.url.clone())
            }
            SendAction::LightningAddress { address_details } => address_details.address.clone(),
        }
    }

    /// Converts back to the corresponding [`InputType`].
    pub fn to_input_type(&self) -> InputType {
        match self {
            SendAction::Bolt11 { invoice_details } => {
                InputType::Bolt11Invoice(invoice_details.clone())
            }
            SendAction::Bolt12Invoice { invoice_details } => {
                InputType::Bolt12Invoice(invoice_details.clone())
            }
            SendAction::Bolt12Offer { offer_details } => {
                InputType::Bolt12Offer(offer_details.clone())
            }
            SendAction::SparkInvoice { invoice_details } => {
                InputType::SparkInvoice(invoice_details.clone())
            }
            SendAction::SparkAddress { address_details } => {
                InputType::SparkAddress(address_details.clone())
            }
            SendAction::Bitcoin { address_details } => {
                InputType::BitcoinAddress(address_details.clone())
            }
            SendAction::LnurlPay { pay_details } => InputType::LnurlPay(pay_details.clone()),
            SendAction::LightningAddress { address_details } => {
                InputType::LightningAddress(address_details.clone())
            }
        }
    }
}

impl From<InputType> for ParsedAction {
    fn from(input: InputType) -> Self {
        match input {
            InputType::Bolt11Invoice(details) => ParsedAction::Send(SendAction::Bolt11 {
                invoice_details: details,
            }),
            InputType::Bolt12Invoice(details) => ParsedAction::Send(SendAction::Bolt12Invoice {
                invoice_details: details,
            }),
            InputType::Bolt12Offer(details) => ParsedAction::Send(SendAction::Bolt12Offer {
                offer_details: details,
            }),
            InputType::SparkInvoice(details) => ParsedAction::Send(SendAction::SparkInvoice {
                invoice_details: details,
            }),
            InputType::SparkAddress(details) => ParsedAction::Send(SendAction::SparkAddress {
                address_details: details,
            }),
            InputType::BitcoinAddress(details) => ParsedAction::Send(SendAction::Bitcoin {
                address_details: details,
            }),
            InputType::LnurlPay(details) => ParsedAction::Send(SendAction::LnurlPay {
                pay_details: details,
            }),
            InputType::LightningAddress(details) => {
                ParsedAction::Send(SendAction::LightningAddress {
                    address_details: details,
                })
            }
            InputType::LnurlWithdraw(details) => {
                ParsedAction::Receive(ReceiveAction::LnurlWithdraw {
                    withdraw_details: details,
                })
            }
            InputType::LnurlAuth(details) => ParsedAction::Authenticate(AuthAction {
                domain: details.domain.clone(),
                action: details.action.clone(),
                request_data: details,
            }),
            InputType::Bip21(details) => {
                let actions: Vec<ParsedAction> = details
                    .payment_methods
                    .iter()
                    .map(|m| ParsedAction::from(m.clone()))
                    .collect();
                if actions.is_empty() {
                    ParsedAction::Unsupported {
                        raw: format!("BIP21 URI with no payment methods: {}", details.uri),
                    }
                } else {
                    ParsedAction::Multi {
                        bip21_details: details,
                        actions,
                    }
                }
            }
            InputType::Url(url) => ParsedAction::Unsupported {
                raw: format!("URL: {url}"),
            },
            InputType::SilentPaymentAddress(details) => ParsedAction::Unsupported {
                raw: format!("Silent payment address: {}", details.address),
            },
            InputType::Bolt12InvoiceRequest(_) => ParsedAction::Unsupported {
                raw: "BOLT12 invoice request".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Bolt11Invoice, Bolt11InvoiceDetails, InputType, LnurlAuthRequestDetails,
        LnurlWithdrawRequestDetails, PaymentRequestSource,
    };

    use super::*;

    fn make_bolt11_details() -> Bolt11InvoiceDetails {
        Bolt11InvoiceDetails {
            amount_msat: Some(10_000),
            description: Some("test".to_string()),
            description_hash: None,
            expiry: 3600,
            invoice: Bolt11Invoice {
                bolt11: "lnbc100n1...".to_string(),
                source: PaymentRequestSource::default(),
            },
            min_final_cltv_expiry_delta: 18,
            network: crate::BitcoinNetwork::Bitcoin,
            payee_pubkey: "02abc...".to_string(),
            payment_hash: "abc123".to_string(),
            payment_secret: "secret".to_string(),
            routing_hints: vec![],
            timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn test_bolt11_maps_to_send() {
        let details = make_bolt11_details();
        let action = ParsedAction::from(InputType::Bolt11Invoice(details.clone()));
        match action {
            ParsedAction::Send(SendAction::Bolt11 { invoice_details }) => {
                assert_eq!(invoice_details.invoice.bolt11, details.invoice.bolt11);
            }
            other => panic!("Expected Send(Bolt11), got {other:?}"),
        }
    }

    #[test]
    fn test_lnurl_withdraw_maps_to_receive() {
        let details = LnurlWithdrawRequestDetails {
            callback: "https://example.com/callback".to_string(),
            k1: "k1value".to_string(),
            default_description: "withdraw".to_string(),
            min_withdrawable: 1000,
            max_withdrawable: 100_000,
        };
        let action = ParsedAction::from(InputType::LnurlWithdraw(details.clone()));
        match action {
            ParsedAction::Receive(ReceiveAction::LnurlWithdraw { withdraw_details }) => {
                assert_eq!(withdraw_details.callback, details.callback);
                assert_eq!(withdraw_details.min_withdrawable, details.min_withdrawable);
            }
            other => panic!("Expected Receive(LnurlWithdraw), got {other:?}"),
        }
    }

    #[test]
    fn test_lnurl_auth_maps_to_authenticate() {
        let details = LnurlAuthRequestDetails {
            k1: "k1".to_string(),
            action: Some("login".to_string()),
            domain: "example.com".to_string(),
            url: "https://example.com/auth?k1=k1".to_string(),
        };
        let action = ParsedAction::from(InputType::LnurlAuth(details.clone()));
        match action {
            ParsedAction::Authenticate(auth) => {
                assert_eq!(auth.domain, "example.com");
                assert_eq!(auth.action, Some("login".to_string()));
                assert_eq!(auth.request_data.k1, "k1");
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    #[test]
    fn test_bip21_maps_to_multi() {
        let bolt11 = make_bolt11_details();
        let bip21 = Bip21Details {
            amount_sat: Some(1000),
            asset_id: None,
            uri: "bitcoin:bc1q...?lightning=lnbc100n1...".to_string(),
            extras: vec![],
            label: None,
            message: None,
            payment_methods: vec![InputType::Bolt11Invoice(bolt11)],
        };
        let action = ParsedAction::from(InputType::Bip21(bip21));
        match action {
            ParsedAction::Multi {
                bip21_details,
                actions,
            } => {
                assert_eq!(actions.len(), 1);
                assert_eq!(bip21_details.amount_sat, Some(1000));
                assert!(matches!(
                    actions[0],
                    ParsedAction::Send(SendAction::Bolt11 { .. })
                ));
            }
            other => panic!("Expected Multi, got {other:?}"),
        }
    }

    #[test]
    fn test_bip21_empty_payment_methods_maps_to_unsupported() {
        let bip21 = Bip21Details {
            uri: "bitcoin:bc1q...".to_string(),
            payment_methods: vec![],
            ..Default::default()
        };
        let action = ParsedAction::from(InputType::Bip21(bip21));
        assert!(matches!(action, ParsedAction::Unsupported { .. }));
    }

    #[test]
    fn test_url_maps_to_unsupported() {
        let action = ParsedAction::from(InputType::Url("https://example.com".to_string()));
        match action {
            ParsedAction::Unsupported { raw } => {
                assert!(raw.contains("https://example.com"));
            }
            other => panic!("Expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn test_send_action_payment_request() {
        let details = make_bolt11_details();
        let send = SendAction::Bolt11 {
            invoice_details: details.clone(),
        };
        assert_eq!(send.payment_request(), details.invoice.bolt11);
    }

    #[test]
    fn test_send_action_round_trip() {
        let details = make_bolt11_details();
        let send = SendAction::Bolt11 {
            invoice_details: details.clone(),
        };
        let input_type = send.to_input_type();
        match input_type {
            InputType::Bolt11Invoice(d) => {
                assert_eq!(d.invoice.bolt11, details.invoice.bolt11);
            }
            other => panic!("Expected Bolt11Invoice, got {other:?}"),
        }
    }
}
