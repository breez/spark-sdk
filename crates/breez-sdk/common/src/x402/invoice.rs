use std::str::FromStr;

use bitcoin::{hashes::Hash, secp256k1::PublicKey};
use lightning::bolt11_invoice::{
    Currency, DEFAULT_EXPIRY_TIME, ExpiryTime, SignedRawBolt11Invoice, TaggedField,
};

use super::error::{X402Error, X402ErrorReason, X402Result};

/// The BOLT11 fields x402 checks. Decoding works on the raw invoice: it
/// verifies the signature and field counts but, unlike `Bolt11Invoice`, does
/// not require a features field, which BOLT11 readers need not reject. Whether
/// the invoice is payable is the payer's decision.
#[derive(Debug)]
pub(crate) struct DecodedInvoice {
    pub payment_hash: [u8; 32],
    /// Set only when the invoice has exactly one description hash and no
    /// inline description.
    pub description_hash: Option<[u8; 32]>,
    pub payee: PublicKey,
    pub currency: Currency,
    pub amount_msat: Option<u64>,
    pub created_secs: u64,
    pub expiry_secs: u64,
}

pub(crate) fn decode_invoice(invoice: &str) -> X402Result<DecodedInvoice> {
    let decode_failed =
        |detail: String| X402Error::new(X402ErrorReason::InvoiceDecodeFailed, detail);
    let signed = SignedRawBolt11Invoice::from_str(invoice)
        .map_err(|e| decode_failed(format!("invalid invoice: {e}")))?;
    if !signed.check_signature() {
        return Err(decode_failed("invalid invoice signature".to_string()));
    }
    // Parsing drops bech32 padding bits, and the signature is checked over
    // the re-encoded fields, so a string with non-zero padding would verify
    // too. Requiring the canonical encoding makes each invoice a single string.
    if signed.to_string() != invoice.to_ascii_lowercase() {
        return Err(decode_failed(
            "invoice is not canonically encoded".to_string(),
        ));
    }
    let raw = signed.raw_invoice();
    let payee = match raw.payee_pub_key() {
        Some(payee) => payee.0,
        None => {
            signed
                .recover_payee_pub_key()
                .map_err(|e| decode_failed(format!("cannot recover payee: {e}")))?
                .0
        }
    };

    let mut payment_hashes = Vec::new();
    let mut description_hashes = Vec::new();
    let mut has_inline_description = false;
    for field in raw.known_tagged_fields() {
        match field {
            TaggedField::PaymentHash(hash) => payment_hashes.push(hash.0.to_byte_array()),
            TaggedField::DescriptionHash(hash) => description_hashes.push(hash.0.to_byte_array()),
            TaggedField::Description(_) => has_inline_description = true,
            _ => {}
        }
    }
    let [payment_hash] = payment_hashes[..] else {
        return Err(decode_failed(
            "invoice must have exactly one payment hash".to_string(),
        ));
    };
    let description_hash = match description_hashes[..] {
        [hash] if !has_inline_description => Some(hash),
        _ => None,
    };

    // BOLT11 amounts are in pico-bitcoin; one that isn't a whole msat is invalid.
    let amount_msat = match raw.amount_pico_btc() {
        None => None,
        Some(pico) if pico % 10 == 0 => Some(pico / 10),
        Some(_) => {
            return Err(decode_failed(
                "invoice amount is not a whole msat".to_string(),
            ));
        }
    };

    Ok(DecodedInvoice {
        payment_hash,
        description_hash,
        payee,
        currency: raw.currency(),
        amount_msat,
        created_secs: raw.data.timestamp.as_unix_timestamp(),
        expiry_secs: raw
            .expiry_time()
            .map_or(DEFAULT_EXPIRY_TIME, ExpiryTime::as_seconds),
    })
}
