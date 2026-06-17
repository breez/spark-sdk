//! Live coverage for the Turnkey-backed `ExternalBreezSigner` methods that the
//! signer-backend flows never touch (LNURL and real-time sync are off there):
//! ECDSA signing and recovery, HMAC, and the ECIES round-trip. Turnkey signs;
//! every assertion verifies locally against the key derived at the same path.
#![cfg(feature = "turnkey")]

use anyhow::Result;
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, ecdsa};
use breez_sdk_itest::turnkey::provision_turnkey_wallet;
use breez_sdk_spark::signer::external_types::MessageBytes;
use tracing::info;

/// An LNURL-auth style derivation path, applied relative to the identity
/// master like the seed backend does.
const TEST_PATH: &str = "m/138'/1'/2'/3'";

#[test_log::test(tokio::test)]
async fn breez_signer_signing_flows() -> Result<()> {
    let (config, _guard) = provision_turnkey_wallet().await?;
    let signers = breez_sdk_spark::turnkey::create_turnkey_signer(config)
        .await
        .map_err(|e| anyhow::anyhow!("create_turnkey_signer failed: {e}"))?;
    let breez = signers.breez_signer;

    let secp = Secp256k1::new();
    let digest = sha256::Hash::hash(b"turnkey breez signer itest").to_byte_array();
    let message = Message::from_digest(digest);

    // The signature must verify against the key derived at the same path, and
    // must be canonical low-S like the seed backend's.
    let pubkey =
        PublicKey::from_slice(&breez.derive_public_key(TEST_PATH.to_string()).await?.bytes)?;
    let sig_bytes = breez
        .sign_ecdsa(MessageBytes::new(digest.to_vec()), TEST_PATH.to_string())
        .await?;
    let signature = ecdsa::Signature::from_compact(&sig_bytes.bytes)?;
    secp.verify_ecdsa(&message, &signature, &pubkey)?;
    let mut normalized = signature;
    normalized.normalize_s();
    assert_eq!(signature, normalized, "expected a low-S signature");
    info!("[Turnkey] sign_ecdsa verified against the derived key");

    // The recoverable signature must recover to that same key, which proves
    // the recovery id handling end to end.
    let rec_bytes = breez
        .sign_ecdsa_recoverable(MessageBytes::new(digest.to_vec()), TEST_PATH.to_string())
        .await?
        .bytes;
    assert_eq!(rec_bytes.len(), 65, "expected [31 + recid] || r || s");
    let recovery_id = RecoveryId::from_i32(i32::from(rec_bytes[0]).saturating_sub(31))?;
    let recoverable = RecoverableSignature::from_compact(&rec_bytes[1..], recovery_id)?;
    let recovered = secp.recover_ecdsa(&message, &recoverable)?;
    assert_eq!(
        recovered, pubkey,
        "recoverable signature must recover to the derived key"
    );
    info!("[Turnkey] sign_ecdsa_recoverable recovered the derived key");

    // HMAC: deterministic per path, distinct across paths.
    let msg = b"hmac input".to_vec();
    let h1 = breez
        .hmac_sha256(msg.clone(), TEST_PATH.to_string())
        .await?;
    let h2 = breez
        .hmac_sha256(msg.clone(), TEST_PATH.to_string())
        .await?;
    let h3 = breez.hmac_sha256(msg, "m/138'/9'".to_string()).await?;
    assert_eq!(h1.bytes, h2.bytes, "hmac must be deterministic per path");
    assert_ne!(h1.bytes, h3.bytes, "hmac must differ across paths");

    // ECIES round-trips on the dedicated exported key.
    let plaintext = b"ecies payload".to_vec();
    let ciphertext = breez
        .encrypt_ecies(plaintext.clone(), TEST_PATH.to_string())
        .await?;
    let decrypted = breez
        .decrypt_ecies(ciphertext, TEST_PATH.to_string())
        .await?;
    assert_eq!(decrypted, plaintext, "ecies must round-trip");

    info!("[Turnkey] breez signer flows verified");
    Ok(())
}
