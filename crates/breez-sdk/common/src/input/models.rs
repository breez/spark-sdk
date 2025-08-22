use serde::{Deserialize, Serialize};

use crate::{
    lnurl::{auth::LnurlAuthRequestDetails, pay::LnurlPayRequestDetails},
    network::BitcoinNetwork,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum Amount {
    Bitcoin {
        amount_msat: u64,
    },
    /// An amount of currency specified using ISO 4712.
    Currency {
        /// The currency that the amount is denominated in.
        iso4217_code: String,
        /// The amount in the currency unit adjusted by the ISO 4712 exponent (e.g., USD cents).
        fractional_amount: u64,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Details {
    pub amount_sat: Option<u64>,
    pub asset_id: Option<String>,
    pub uri: String,
    pub extras: Vec<Bip21Extra>,
    pub label: Option<String>,
    pub message: Option<String>,
    pub payment_methods: Vec<InputType>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bip21Extra {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BitcoinAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11Invoice {
    pub bolt11: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHint {
    pub hops: Vec<Bolt11RouteHintHop>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11RouteHintHop {
    /// The `node_id` of the non-target end of the route
    pub src_node_id: String,
    /// The `short_channel_id` of this channel
    pub short_channel_id: String,
    /// The fees which must be paid to use this channel
    pub fees_base_msat: u32,
    pub fees_proportional_millionths: u32,

    /// The difference in CLTV values between this node and the next node.
    pub cltv_expiry_delta: u16,
    /// The minimum value, in msat, which must be relayed to the next hop.
    pub htlc_minimum_msat: Option<u64>,
    /// The maximum value in msat available for routing with a single HTLC.
    pub htlc_maximum_msat: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Invoice {
    pub invoice: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceRequestDetails {
    // TODO: Fill fields
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferBlindedPath {
    pub blinded_hops: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt11InvoiceDetails {
    pub amount_msat: Option<u64>,
    pub description: Option<String>,
    pub description_hash: Option<String>,
    pub expiry: u64,
    pub invoice: Bolt11Invoice,
    pub min_final_cltv_expiry_delta: u64,
    pub network: BitcoinNetwork,
    pub payee_pubkey: String,
    pub payment_hash: String,
    pub payment_secret: String,
    pub routing_hints: Vec<Bolt11RouteHint>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12InvoiceDetails {
    // TODO: Fill fields
    pub amount_msat: u64,
    pub invoice: Bolt12Invoice,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12Offer {
    pub offer: String,
    pub source: PaymentRequestSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Bolt12OfferDetails {
    pub absolute_expiry: Option<u64>,
    pub chains: Vec<String>,
    pub description: Option<String>,
    pub issuer: Option<String>,
    pub min_amount: Option<Amount>,
    pub offer: Bolt12Offer,
    pub paths: Vec<Bolt12OfferBlindedPath>,
    pub signing_pubkey: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InputType {
    BitcoinAddress(BitcoinAddressDetails),
    Bolt11Invoice(Bolt11InvoiceDetails),
    Bolt12Invoice(Bolt12InvoiceDetails),
    Bolt12Offer(Bolt12OfferDetails),
    LightningAddress(LightningAddressDetails),
    LnurlPay(LnurlPayRequestDetails),
    SilentPaymentAddress(SilentPaymentAddressDetails),
    LnurlAuth(LnurlAuthRequestDetails),
    Url(String),
    Bip21(Bip21Details),
    Bolt12InvoiceRequest(Bolt12InvoiceRequestDetails),
    LnurlWithdraw(LnurlWithdrawRequestDetails),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LightningAddressDetails {
    pub address: String,
    pub pay_request: LnurlPayRequestDetails,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LnurlWithdrawRequestDetails {
    pub callback: String,
    pub k1: String,
    pub default_description: String,
    /// The minimum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub min_withdrawable: u64,
    /// The maximum amount, in millisats, that this LNURL-withdraw endpoint accepts
    pub max_withdrawable: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentRequestSource {
    pub bip_21_uri: Option<String>,
    pub bip_353_address: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SilentPaymentAddressDetails {
    pub address: String,
    pub network: BitcoinNetwork,
    pub source: PaymentRequestSource,
}
