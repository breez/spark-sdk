use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, ecdsa};
use prost::Message as _;

use super::QuoteError;
use crate::operator::rpc::spark::{Network, TransferManifest};

/// Which flow a quote was issued for, so a quote for one flow cannot be
/// replayed into another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteReason {
    Receive = 1,
    Send = 2,
    CoopExit = 3,
    StaticDeposit = 4,
}

/// Which party a quote signature comes from. Both roles sign the same envelope
/// over the same manifest, so the role is what keeps an issuer signature from
/// being replayed as an attestor one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteRole {
    Issuer = 1,
    Attestor = 2,
}

/// The digest a sender signs with its identity key to consent to a manifest:
/// `tagged_hash(["spark","transfer","manifest"], protohash(canonicalize(manifest)))`.
pub fn transfer_manifest_hash(manifest: &TransferManifest) -> Result<[u8; 32], QuoteError> {
    serialized_transfer_manifest_hash(&manifest.encode_to_vec())
}

/// [`transfer_manifest_hash`] over a manifest received as bytes, so a peer's
/// copy is hashed exactly as the operators will bind it.
pub fn serialized_transfer_manifest_hash(manifest_bytes: &[u8]) -> Result<[u8; 32], QuoteError> {
    to_digest(spark_primitives::hash_transfer_manifest(
        manifest_bytes.to_vec(),
    )?)
}

/// The digest a quote signature is taken over:
/// `tagged_hash(["spark","quote","v1"], network, manifest_hash, reason, role, target)`.
///
/// An issuer may sign an empty `target` (at quote time there is nothing else to
/// bind); an attestor may not.
pub fn quote_envelope_digest(
    network: Network,
    manifest_hash: &[u8; 32],
    reason: QuoteReason,
    role: QuoteRole,
    target: &[u8],
) -> Result<[u8; 32], QuoteError> {
    envelope_digest(
        u64::try_from(i32::from(network)).unwrap_or_default(),
        manifest_hash,
        reason as u64,
        role as u64,
        target,
    )
}

// Takes the wire integers so the golden vectors can also drive the values the
// typed API cannot express.
pub(super) fn envelope_digest(
    network: u64,
    manifest_hash: &[u8],
    reason: u64,
    role: u64,
    target: &[u8],
) -> Result<[u8; 32], QuoteError> {
    // spark-primitives takes u32s, and a truncating cast would let a value with
    // high bits set alias a valid one.
    let narrow = |value: u64, name: &str| {
        u32::try_from(value)
            .map_err(|_| QuoteError::NotHashable(format!("unsupported {name} {value}")))
    };
    to_digest(spark_primitives::quote_envelope_digest(
        narrow(network, "network")?,
        manifest_hash.to_vec(),
        narrow(reason, "quote reason")?,
        narrow(role, "quote role")?,
        target.to_vec(),
    )?)
}

/// Binds a receive attestation to the invoice it covers, so one attestation
/// cannot cover another invoice of the same gross from the same attestor.
pub fn receive_attestor_target(payment_hash: &[u8; 32]) -> Result<[u8; 32], QuoteError> {
    to_digest(spark_primitives::receive_attestor_target(
        payment_hash.to_vec(),
    )?)
}

/// Binds a send quote to the invoice and to the amount paid for it: a
/// zero-amount invoice takes its amount from the request, so the hash alone
/// would let a quote priced for one amount commit against another.
pub fn send_target_v2(payment_hash: &[u8; 32], payment_msats: u64) -> Result<[u8; 32], QuoteError> {
    to_digest(spark_primitives::send_target_v2(
        payment_hash.to_vec(),
        payment_msats,
    )?)
}

/// What the attestor of a quoted receive signs: the receive envelope under the
/// attestor role, bound to the invoice's payment hash. The network is read from
/// the manifest bytes, as the verifiers do, so an envelope cannot name a network
/// its manifest denies.
pub fn receive_attestation_digest(
    manifest_bytes: &[u8],
    payment_hash: &[u8; 32],
) -> Result<[u8; 32], QuoteError> {
    let network = manifest_network(manifest_bytes)?;
    quote_envelope_digest(
        network,
        &serialized_transfer_manifest_hash(manifest_bytes)?,
        QuoteReason::Receive,
        QuoteRole::Attestor,
        &receive_attestor_target(payment_hash)?,
    )
}

/// What the SSP signs when it issues a send quote. A wallet checks the SSP's
/// signature against it before funding anything.
pub fn send_issuance_digest(
    manifest_bytes: &[u8],
    payment_hash: &[u8; 32],
    payment_msats: u64,
) -> Result<[u8; 32], QuoteError> {
    let network = manifest_network(manifest_bytes)?;
    quote_envelope_digest(
        network,
        &serialized_transfer_manifest_hash(manifest_bytes)?,
        QuoteReason::Send,
        QuoteRole::Issuer,
        &send_target_v2(payment_hash, payment_msats)?,
    )
}

/// Checks an identity-key signature over a quote digest the way the operators
/// and the SSP do: strict DER and low-S only, so a signature accepted here is
/// one the server accepts.
pub fn verify_identity_signature(
    digest: &[u8; 32],
    der_signature: &[u8],
    identity_public_key: &PublicKey,
) -> Result<(), QuoteError> {
    let signature = ecdsa::Signature::from_der(der_signature)
        .map_err(|e| QuoteError::InvalidSignature(format!("not strict DER: {e}")))?;
    // libsecp256k1 rejects high-S on verify, which is the canonical form the
    // verifiers require.
    Secp256k1::verification_only()
        .verify_ecdsa(
            &Message::from_digest(*digest),
            &signature,
            identity_public_key,
        )
        .map_err(|e| QuoteError::InvalidSignature(e.to_string()))
}

fn manifest_network(manifest_bytes: &[u8]) -> Result<Network, QuoteError> {
    let manifest = TransferManifest::decode(manifest_bytes)
        .map_err(|e| QuoteError::NotHashable(format!("failed to decode TransferManifest: {e}")))?;
    Network::try_from(manifest.network)
        .map_err(|_| QuoteError::NotHashable(format!("unsupported network {}", manifest.network)))
}

fn to_digest(bytes: Vec<u8>) -> Result<[u8; 32], QuoteError> {
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        QuoteError::NotHashable(format!("digest is {} bytes", bytes.len()))
    })
}
