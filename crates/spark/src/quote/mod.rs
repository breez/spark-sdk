//! Fee-bearing (quoted) payments: the SSP signs a `TransferManifest` describing
//! how a payment splits between its receiver and the fee payees, and each party
//! signs a digest binding it to that manifest.
//!
//! Hashing delegates to `spark-primitives`, the shared core the SSP and the JS
//! SDK use. The golden vectors under `testdata/` are copied from
//! buildonspark/spark and are what keeps this crate byte-identical with them.

mod error;
mod hashing;
mod validation;

pub use error::QuoteError;
pub use hashing::{
    QuoteReason, QuoteRole, quote_envelope_digest, receive_attestation_digest,
    receive_attestor_target, send_issuance_digest, send_target_v2,
    serialized_transfer_manifest_hash, transfer_manifest_hash, verify_identity_signature,
};
pub use validation::{
    ManifestDenomination, ReceiveAmountBasis, ReceiveQuoteFee, SendManifestBucket,
    manifest_denomination, manifest_fee_sats, manifest_gross_sats, manifest_has_partner_markup,
    manifest_net_sats_for, receive_quote_fee, validate_quoted_receive_manifest,
    validate_quoted_send_manifest,
};

#[cfg(all(test, feature = "quotes"))]
#[allow(clippy::indexing_slicing)] // Test suites: a panic there is a failed assertion.
mod tests;
