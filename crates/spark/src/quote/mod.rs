//! Fee-bearing (quoted) payments: the SSP signs a `TransferManifest` describing
//! how a payment splits between its receiver and the fee payees, and each party
//! signs a digest binding it to that manifest.
//!
//! Hashing delegates to `spark-primitives`, the shared core the SSP and the JS
//! SDK use. The golden vectors under `testdata/` are copied from
//! buildonspark/spark and are what keeps this crate byte-identical with them.

mod error;
mod hashing;
mod resolution;
mod validation;

pub use error::QuoteError;
pub use hashing::{
    QuoteReason, QuoteRole, quote_envelope_digest, receive_attestation_digest,
    receive_attestor_target, send_issuance_digest, send_target_v2,
    serialized_transfer_manifest_hash, transfer_manifest_hash, verify_identity_signature,
};
pub use resolution::{ResolvedEdge, allocate_manifest_edges, resolve_manifest_edges};
pub use validation::{
    ManifestDenomination, ReceiveAmountBasis, ReceiveQuoteFee, SendManifestBucket,
    manifest_denomination, manifest_fee_sats, manifest_gross_sats, manifest_net_sats_for,
    manifest_send_buckets, receive_quote_fee, validate_quoted_receive_manifest,
    validate_quoted_send_manifest,
};

#[cfg(test)]
mod tests;
